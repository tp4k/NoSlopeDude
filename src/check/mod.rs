//! M5-4: one uncached `check` run over snapshots (`nsd-plan-final.md` *CLI
//! and configuration*). Every internal failure is a diagnostic in the
//! outcome, never an `Err`, so the exit status is always 2-class for them.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use git2::{ErrorCode, ObjectType, Oid, Repository};
use rayon::prelude::*;

use crate::analysis::{analyze_file, FileAnalysis, UnanalyzableReason};
use crate::cache::{Cache, CacheError, CacheKey, CachedAnalysis, CachedReason};
use crate::clones::Candidate as CloneRun;
use crate::config::{Config, PolicyConfig, Severity};
use crate::git::diff::{
    diff_commit_to_commit, diff_commit_to_index, diff_commit_to_worktree, Change,
};
use crate::git::discovery::{discover, IncludedEntry};
use crate::git::mergebase::merge_base;
use crate::git::path::RepoPath;
use crate::git::snapshot::{CommitSnapshot, Entry, IndexSnapshot, WorktreeSnapshot};
use crate::git::{wrap_git_error, GitError, CODE_SNAPSHOT_UNAVAILABLE};
use crate::identity::matching::{match_callables, FileCallables};
use crate::model::Grammar;
use crate::policy;
use crate::policy::clones::{evaluate_clones_precomputed, UnchangedCandidates};
use crate::policy::complexity::{classify, FileMetrics};
use crate::policy::coverage::{evaluate_coverage, CoverageInput};
use crate::policy::damage::evaluate_damage;
use crate::policy::diagnostics::{
    CloneDiagnostic, CoverageDiagnostic, DamageDiagnostic, FindingDiagnostic, PolicyDiagnostic,
    SuppressionDiagnostic,
};
use crate::policy::exit::exit_status;
use crate::policy::findings::{match_findings, FindingFile};
use crate::policy::Candidate;
use crate::profile::{fingerprint, MeasurementProfileInputs};
use crate::suppress::apply_suppressions;

/// What the candidate side of a check is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckMode {
    /// Base is `HEAD`, candidate is the Git index.
    Staged,
    /// Base is `merge-base(HEAD, reference)`; candidate is `HEAD`, or the
    /// worktree overlay when `worktree` is set.
    Base { reference: String, worktree: bool },
}

/// One check invocation.
#[derive(Debug, Clone)]
pub struct CheckRequest<'a> {
    /// The repository work-tree directory, opened as is (no upward search).
    pub repository: &'a Path,
    pub mode: CheckMode,
    /// A trusted configuration file replacing repository policy.
    pub config_path: Option<&'a Path>,
    pub allow_new_suppressions: bool,
}

/// One diagnostic of a check, whichever stage raised it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckDiagnostic {
    /// E101, E102 or G102.
    Complexity(Box<PolicyDiagnostic>),
    /// V101.
    Finding(FindingDiagnostic),
    /// S101 or S102.
    Suppression(SuppressionDiagnostic),
    /// A101.
    Damage(DamageDiagnostic),
    /// A102.
    Coverage(CoverageDiagnostic),
    /// V102.
    Clone(Box<CloneDiagnostic>),
    /// C101 or C102 about the candidate's `nsd.yml`.
    Config(policy::Diagnostic),
    /// A failure with no policy verdict behind it: G101 for a repository,
    /// snapshot, diff or read failure, or the code of a `resolve` failure.
    Failure { code: &'static str, message: String },
}

impl CheckDiagnostic {
    /// The stable `NSD-` code.
    pub fn code(&self) -> &'static str {
        match self {
            CheckDiagnostic::Complexity(found) => found.code,
            CheckDiagnostic::Finding(found) => found.code,
            CheckDiagnostic::Suppression(found) => found.code,
            CheckDiagnostic::Damage(found) => found.code,
            CheckDiagnostic::Coverage(found) => found.code,
            CheckDiagnostic::Clone(found) => found.code,
            CheckDiagnostic::Config(found) => found.code,
            CheckDiagnostic::Failure { code, .. } => code,
        }
    }
}

/// The complete diagnostic set of one check and its exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutcome {
    /// Grouped in a fixed order: configuration, read failures, complexity,
    /// findings, suppressions, damage, coverage, clones; each group in its
    /// evaluator's own sorted order.
    pub diagnostics: Vec<CheckDiagnostic>,
    /// `policy::exit::exit_status` over `diagnostics`.
    pub exit_status: u8,
    /// Exit-neutral notes about the run, outside `diagnostics`: at most one,
    /// raised when the analysis cache failed and the run continued uncached.
    pub warnings: Vec<String>,
}

/// Runs one check. Never fails: an internal error is an `NSD-G101` (or the
/// `resolve` error's own code) in the outcome.
pub fn run_check(request: &CheckRequest<'_>) -> CheckOutcome {
    let (diagnostics, policy, warnings) = match execute(request) {
        Ok(checked) => checked,
        Err(diagnostics) => (diagnostics, Config::default().policy, Vec::new()),
    };
    let exit_status = exit_status(
        diagnostics.iter().map(CheckDiagnostic::code),
        &policy,
        request.allow_new_suppressions,
    );
    CheckOutcome {
        diagnostics,
        exit_status,
        warnings,
    }
}

enum Snapshot {
    Commit(CommitSnapshot),
    Index(IndexSnapshot),
    Worktree(WorktreeSnapshot),
}

impl Snapshot {
    fn entries(&self) -> &[Entry] {
        match self {
            Snapshot::Commit(snapshot) => &snapshot.entries,
            Snapshot::Index(snapshot) => &snapshot.entries,
            Snapshot::Worktree(snapshot) => &snapshot.entries,
        }
    }

    fn read(&self, repo: &Repository, entry: &Entry) -> Result<Option<Vec<u8>>, GitError> {
        match self {
            Snapshot::Commit(snapshot) => snapshot.read(repo, entry),
            Snapshot::Index(snapshot) => snapshot.read(repo, entry),
            Snapshot::Worktree(snapshot) => snapshot.read(repo, entry),
        }
    }

    fn as_candidate(&self) -> Candidate<'_> {
        match self {
            Snapshot::Commit(snapshot) => Candidate::Commit(snapshot),
            Snapshot::Index(snapshot) => Candidate::Index(snapshot),
            Snapshot::Worktree(snapshot) => Candidate::Worktree(snapshot),
        }
    }
}

fn git_failure(err: &GitError) -> Vec<CheckDiagnostic> {
    vec![CheckDiagnostic::Failure {
        code: err.code(),
        message: err.to_string(),
    }]
}

/// `HEAD`'s commit id, `None` when `HEAD` is unborn.
fn head_oid(repo: &Repository) -> Result<Option<Oid>, GitError> {
    match repo.head() {
        Ok(head) => head
            .peel_to_commit()
            .map(|commit| Some(commit.id()))
            .map_err(|err| wrap_git_error("HEAD does not resolve to a commit", &err)),
        Err(err) if err.code() == ErrorCode::UnbornBranch => Ok(None),
        Err(err) => Err(wrap_git_error("cannot resolve HEAD", &err)),
    }
}

/// The base snapshot, the candidate snapshot and the diff between them.
fn open_sides(
    repo: &Repository,
    mode: &CheckMode,
) -> Result<(CommitSnapshot, Snapshot, Vec<Change>), GitError> {
    match mode {
        CheckMode::Staged => {
            let base_oid = head_oid(repo)?;
            let base = CommitSnapshot::head_or_empty(repo)?;
            let index = IndexSnapshot::open(repo)?;
            let changes = diff_commit_to_index(repo, base_oid)?;
            Ok((base, Snapshot::Index(index), changes))
        }
        CheckMode::Base {
            reference,
            worktree,
        } => {
            let base_oid = merge_base(repo, reference)?;
            let base = CommitSnapshot::at(repo, base_oid)?;
            if *worktree {
                let overlay = WorktreeSnapshot::open(repo)?;
                let changes = diff_commit_to_worktree(repo, Some(base_oid), &overlay)?;
                Ok((base, Snapshot::Worktree(overlay), changes))
            } else {
                let head = CommitSnapshot::head_or_empty(repo)?;
                let head_oid = head_oid(repo)?.ok_or_else(|| {
                    GitError::new(CODE_SNAPSHOT_UNAVAILABLE, "HEAD is unborn after merge-base")
                })?;
                let changes = diff_commit_to_commit(repo, Some(base_oid), head_oid)?;
                Ok((base, Snapshot::Commit(head), changes))
            }
        }
    }
}

type Checked = (Vec<CheckDiagnostic>, PolicyConfig, Vec<String>);

fn execute(request: &CheckRequest<'_>) -> Result<Checked, Vec<CheckDiagnostic>> {
    let repo = Repository::open(request.repository)
        .map_err(|err| git_failure(&wrap_git_error("cannot open the repository", &err)))?;
    let (base, candidate, changes) =
        open_sides(&repo, &request.mode).map_err(|e| git_failure(&e))?;
    let resolution = policy::resolve(&repo, request.config_path, &base, candidate.as_candidate())
        .map_err(|err| {
        vec![CheckDiagnostic::Failure {
            code: err.code(),
            message: err.to_string(),
        }]
    })?;
    let scope = resolution.config.compiled_scope().map_err(|err| {
        vec![CheckDiagnostic::Failure {
            code: err.code(),
            message: err.to_string(),
        }]
    })?;

    let mut diagnostics: Vec<CheckDiagnostic> = resolution
        .diagnostics
        .iter()
        .copied()
        .map(CheckDiagnostic::Config)
        .collect();
    let session = CacheSession::open(&repo, resolution.config.measurement.min_clone_lines);
    let sides = Sides {
        repository: request.repository,
        cache: &session,
        base: &base,
        candidate: &candidate,
        changes: &changes,
        config: &resolution.config,
        scope: &scope,
    };
    diagnostics.extend(sides.evaluate());
    Ok((
        diagnostics,
        resolution.config.policy,
        session.into_warnings(),
    ))
}

/// One analyzed, read-once file: the bytes and analysis `FindingFile`,
/// `FileCallables` and `FileMetrics` are all built from.
struct Loaded {
    path: RepoPath,
    bytes: Vec<u8>,
    analysis: FileAnalysis,
}

impl Loaded {
    fn finding_file(&self) -> FindingFile<'_> {
        FindingFile {
            path: self.path.clone(),
            source: &self.bytes,
            analysis: &self.analysis,
        }
    }

    fn callables(&self) -> FileCallables {
        FileCallables {
            path: self.path.clone(),
            callables: self
                .analysis
                .callables
                .iter()
                .map(|callable| (callable.identity.clone(), callable.body_fingerprint.clone()))
                .collect(),
        }
    }

    fn metrics(&self) -> FileMetrics {
        FileMetrics {
            path: self.path.clone(),
            callables: self
                .analysis
                .callables
                .iter()
                .map(|callable| callable.metrics.clone())
                .collect(),
        }
    }
}

/// The candidate-side paths of `changes`.
fn candidate_paths(changes: &[Change]) -> HashSet<&RepoPath> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Added { path, .. }
            | Change::Modified { path, .. }
            | Change::Typechange { path, .. } => Some(path),
            Change::Renamed { to, .. } => Some(to),
            Change::Deleted { .. } => None,
        })
        .collect()
}

/// The base-side paths of `changes`, deletions included.
fn base_paths(changes: &[Change]) -> HashSet<&RepoPath> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Modified { path, .. }
            | Change::Typechange { path, .. }
            | Change::Deleted { path, .. } => Some(path),
            Change::Renamed { from, .. } => Some(from),
            Change::Added { .. } => None,
        })
        .collect()
}

/// Each base-side path of `changes` mapped to its candidate-side path;
/// deletions and additions have no pair.
fn candidate_counterparts(changes: &[Change]) -> HashMap<&RepoPath, &RepoPath> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Modified { path, .. } | Change::Typechange { path, .. } => Some((path, path)),
            Change::Renamed { from, to, .. } => Some((from, to)),
            Change::Added { .. } | Change::Deleted { .. } => None,
        })
        .collect()
}

/// The bytes of an included entry that must be readable, or the G101 that
/// says its snapshot is unavailable. A regular, under-ceiling entry whose
/// read yields `None` is never an analysis reason: the blob vanished or
/// changed after enumeration.
fn required_bytes(
    path: &RepoPath,
    read: Result<Option<Vec<u8>>, GitError>,
) -> Result<Vec<u8>, CheckDiagnostic> {
    match read {
        Ok(Some(bytes)) => Ok(bytes),
        Ok(None) => Err(CheckDiagnostic::Failure {
            code: CODE_SNAPSHOT_UNAVAILABLE,
            message: format!(
                "required snapshot unavailable: {} is unreadable",
                path.render()
            ),
        }),
        Err(err) => Err(CheckDiagnostic::Failure {
            code: err.code(),
            message: format!("cannot read {}: {err}", path.render()),
        }),
    }
}

/// Analyzes one file's bytes under its repository path.
fn analyze_bytes(path: &RepoPath, bytes: &[u8]) -> Result<FileAnalysis, UnanalyzableReason> {
    match std::str::from_utf8(path.as_bytes()) {
        Ok(path) => analyze_file(Path::new(path), bytes),
        Err(_) => Err(UnanalyzableReason::NonUtf8Path),
    }
}

/// The entry of `entries` (sorted by path) at `path`.
fn find_entry<'a>(entries: &'a [Entry], path: &RepoPath) -> Option<&'a Entry> {
    entries
        .binary_search_by(|entry| entry.path.cmp(path))
        .ok()
        .map(|position| &entries[position])
}

/// The grammar `path`'s extension selects, the cache key's language part.
fn path_grammar(path: &RepoPath) -> Option<Grammar> {
    let text = std::str::from_utf8(path.as_bytes()).ok()?;
    let extension = Path::new(text).extension()?.to_str()?;
    Grammar::for_extension(extension)
}

/// The analysis cache of one run. After the first I/O failure it is off for
/// the rest of the run and holds the single warning the outcome reports; the
/// message names the failure's kind, never a path.
struct CacheSession {
    cache: Option<Cache>,
    fingerprint: String,
    disabled: AtomicBool,
    warning: Mutex<Option<String>>,
}

impl CacheSession {
    fn open(repo: &Repository, min_clone_lines: u32) -> CacheSession {
        let inputs = MeasurementProfileInputs {
            min_clone_lines,
            ..MeasurementProfileInputs::current()
        };
        let session = CacheSession {
            cache: None,
            fingerprint: fingerprint(&inputs),
            disabled: AtomicBool::new(false),
            warning: Mutex::new(None),
        };
        match Cache::open(repo) {
            Ok(cache) => CacheSession {
                cache: Some(cache),
                ..session
            },
            Err(err) => {
                session.fail(&err);
                session
            }
        }
    }

    fn fail(&self, err: &CacheError) {
        if self.disabled.swap(true, Ordering::SeqCst) {
            return;
        }
        let reason = match err {
            CacheError::Io { source, .. } => source.kind().to_string(),
            CacheError::Serialize(_) => "entry not serializable".to_string(),
        };
        *self.warning.lock().unwrap_or_else(PoisonError::into_inner) = Some(format!(
            "analysis cache unavailable ({reason}); continuing without it"
        ));
    }

    fn active(&self) -> Option<&Cache> {
        if self.disabled.load(Ordering::SeqCst) {
            return None;
        }
        self.cache.as_ref()
    }

    fn get(&self, blob: Oid, grammar: Grammar) -> Option<CachedAnalysis> {
        let key = CacheKey::from_parts(blob, grammar, &self.fingerprint);
        match self.active()?.get(&key) {
            Ok(hit) => hit,
            Err(err) => {
                self.fail(&err);
                None
            }
        }
    }

    fn put(&self, blob: Oid, grammar: Grammar, payload: &CachedAnalysis) {
        let key = CacheKey::from_parts(blob, grammar, &self.fingerprint);
        if let Some(Err(err)) = self.active().map(|cache| cache.put(&key, payload)) {
            self.fail(&err);
        }
    }

    fn into_warnings(self) -> Vec<String> {
        self.warning
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .into_iter()
            .collect()
    }
}

type ReadBlob<'a> = &'a (dyn Fn(&Repository, &Entry) -> Result<Option<Vec<u8>>, GitError> + Sync);

/// One file to read and analyze, with its snapshot entry when it has one.
struct LoadTask<'a> {
    included: &'a IncludedEntry,
    entry: Option<&'a Entry>,
    changed: bool,
}

/// The outcome of loading one file in the parallel pass.
enum Load {
    Failure(CheckDiagnostic),
    Changed(Box<Loaded>),
    Unchanged(UnchangedCandidates),
    Unanalyzable(UnanalyzableReason),
}

/// Runs `load` over `tasks` in parallel, keeping input order. Each worker
/// opens its own `Repository`, which is `Send` but not `Sync`.
fn load_in_parallel(
    repository: &Path,
    tasks: &[LoadTask<'_>],
    load: impl Fn(&Repository, &LoadTask<'_>) -> Load + Sync,
) -> Vec<Load> {
    tasks
        .par_iter()
        .map_init(
            || {
                Repository::open(repository)
                    .map_err(|err| wrap_git_error("cannot open the repository", &err))
            },
            |repo, task| match repo {
                Ok(repo) => load(repo, task),
                Err(err) => Load::Failure(CheckDiagnostic::Failure {
                    code: err.code(),
                    message: err.to_string(),
                }),
            },
        )
        .collect()
}

fn read_task(
    repo: &Repository,
    task: &LoadTask<'_>,
    read: ReadBlob<'_>,
) -> Result<Vec<u8>, CheckDiagnostic> {
    let result = task
        .entry
        .map_or_else(|| Ok(None), |entry| read(repo, entry));
    required_bytes(&task.included.path, result)
}

/// A base-side file: read and fully analyzed, never cached. One that cannot
/// be analyzed leaves its callables unmatched, so the candidate side is
/// judged as new code.
fn load_base(repo: &Repository, task: &LoadTask<'_>, read: ReadBlob<'_>) -> Load {
    let bytes = match read_task(repo, task, read) {
        Ok(bytes) => bytes,
        Err(failure) => return Load::Failure(failure),
    };
    match analyze_bytes(&task.included.path, &bytes) {
        Ok(analysis) => Load::Changed(Box::new(Loaded {
            path: task.included.path.clone(),
            bytes,
            analysis,
        })),
        Err(reason) => Load::Unanalyzable(reason),
    }
}

/// An unchanged file's cached payload as the clone candidates and outcome
/// clone evaluation needs.
fn unchanged_load(included: &IncludedEntry, found: CachedAnalysis) -> Load {
    match found {
        CachedAnalysis::Analyzed(payload) => Load::Unchanged(UnchangedCandidates {
            path: included.path.clone(),
            language: included.language,
            candidates: payload
                .clone_candidates
                .into_iter()
                .map(|stored| {
                    (
                        stored.key,
                        CloneRun {
                            file_index: 0,
                            container: stored.container,
                            first_statement: stored.first_statement,
                            start_line: stored.start_line,
                            end_line: stored.end_line,
                            source_lines: stored.source_lines,
                            statement_count: stored.statement_count,
                        },
                    )
                })
                .collect(),
        }),
        CachedAnalysis::Unanalyzable(CachedReason::InvalidEncoding) => {
            Load::Unanalyzable(UnanalyzableReason::InvalidEncoding)
        }
        CachedAnalysis::Unanalyzable(CachedReason::ParserUnavailable) => {
            Load::Unanalyzable(UnanalyzableReason::ParserUnavailable)
        }
    }
}

/// A candidate-side file. An unchanged one is looked up first (by its entry's
/// blob id when it has one, so a hit reads nothing), and only its payload's
/// candidates and outcome survive. A miss, and every changed file, is read and
/// analyzed, then written under the hash of the bytes actually analyzed.
fn load_candidate(
    repo: &Repository,
    task: &LoadTask<'_>,
    read: ReadBlob<'_>,
    cache: &CacheSession,
    min_clone_lines: u32,
    v102: Severity,
) -> Load {
    let included = task.included;
    let grammar = path_grammar(&included.path);
    let listed_blob = task.entry.and_then(|entry| entry.oid);
    if let (false, Some(grammar), Some(blob)) = (task.changed, grammar, listed_blob) {
        if let Some(found) = cache.get(blob, grammar) {
            return unchanged_load(included, found);
        }
    }
    let bytes = match read_task(repo, task, read) {
        Ok(bytes) => bytes,
        Err(failure) => return Load::Failure(failure),
    };
    let analyzed_blob = Oid::hash_object(ObjectType::Blob, &bytes).ok();
    if let (false, None, Some(grammar), Some(blob)) =
        (task.changed, listed_blob, grammar, analyzed_blob)
    {
        if let Some(found) = cache.get(blob, grammar) {
            return unchanged_load(included, found);
        }
    }
    let analysis = analyze_bytes(&included.path, &bytes);
    let payload_wanted = !task.changed || (cache.active().is_some() && v102 != Severity::Off);
    let payload = match &analysis {
        _ if !payload_wanted => None,
        Ok(analysis) => std::str::from_utf8(&bytes)
            .ok()
            .map(|source| CachedAnalysis::from_analysis(analysis, source, min_clone_lines)),
        Err(reason) => CachedAnalysis::unanalyzable(*reason),
    };
    if let (Some(payload), Some(grammar), Some(blob)) = (&payload, grammar, analyzed_blob) {
        cache.put(blob, grammar, payload);
    }
    match analysis {
        Ok(analysis) if task.changed => Load::Changed(Box::new(Loaded {
            path: included.path.clone(),
            bytes,
            analysis,
        })),
        Ok(_) => payload.map_or(
            Load::Unanalyzable(UnanalyzableReason::InvalidEncoding),
            |payload| unchanged_load(included, payload),
        ),
        Err(reason) => Load::Unanalyzable(reason),
    }
}

struct Sides<'a> {
    repository: &'a Path,
    cache: &'a CacheSession,
    base: &'a CommitSnapshot,
    candidate: &'a Snapshot,
    changes: &'a [Change],
    config: &'a Config,
    scope: &'a crate::config::CompiledScope,
}

/// The files and failures one check loads from its two snapshots. Unchanged
/// files are held as clone candidates only.
#[derive(Default)]
struct Loads {
    base: Vec<Loaded>,
    changed: Vec<Loaded>,
    unchanged: Vec<UnchangedCandidates>,
    coverage: Vec<CoverageInput>,
    failures: Vec<CheckDiagnostic>,
}

impl Sides<'_> {
    fn evaluate(&self) -> Vec<CheckDiagnostic> {
        let loads = self.load();
        let mut diagnostics = loads.failures;
        let policy = &self.config.policy;
        match self.run_evaluators(
            &loads.base,
            &loads.changed,
            &loads.unchanged,
            &loads.coverage,
            policy,
        ) {
            Ok(found) => diagnostics.extend(found),
            Err(err) => diagnostics.extend(git_failure(&err)),
        }
        diagnostics
    }

    fn load(&self) -> Loads {
        let mut loads = Loads::default();
        let base_discovery = discover(&self.base.entries, self.scope);
        let candidate_discovery = discover(self.candidate.entries(), self.scope);
        let changed_candidate = candidate_paths(self.changes);
        let changed_base = base_paths(self.changes);
        let counterparts = candidate_counterparts(self.changes);
        let candidate_included: HashSet<&RepoPath> = candidate_discovery
            .included
            .iter()
            .map(|included| &included.path)
            .collect();

        let mut base_tasks = Vec::new();
        let mut base_coverage = Vec::new();
        for included in &base_discovery.included {
            if !changed_base.contains(&included.path) {
                continue;
            }
            // A changed file the base cannot analyze is an analysis gap of its
            // own, even when its candidate side fits; a deletion, or a change
            // whose candidate side is out of scope, has nothing to compare.
            if included.too_large || included.non_utf8_path {
                let compared = counterparts
                    .get(&included.path)
                    .is_some_and(|to| candidate_included.contains(to));
                if compared {
                    base_coverage.push(CoverageInput {
                        entry: included.clone(),
                        changed: true,
                        failure: None,
                    });
                }
                continue;
            }
            base_tasks.push(LoadTask {
                included,
                entry: find_entry(&self.base.entries, &included.path),
                changed: true,
            });
        }
        let base_read = |repo: &Repository, entry: &Entry| self.base.read(repo, entry);
        for load in load_in_parallel(self.repository, &base_tasks, |repo, task| {
            load_base(repo, task, &base_read)
        }) {
            match load {
                Load::Failure(failure) => loads.failures.push(failure),
                Load::Changed(loaded) => loads.base.push(*loaded),
                Load::Unchanged(_) | Load::Unanalyzable(_) => {}
            }
        }

        let unchanged_required = self.config.policy.nsd_v102 != Severity::Off;
        let mut candidate_tasks = Vec::new();
        for included in &candidate_discovery.included {
            let changed = changed_candidate.contains(&included.path);
            if !changed && !unchanged_required {
                continue;
            }
            if included.too_large || included.non_utf8_path {
                loads.coverage.push(CoverageInput {
                    entry: included.clone(),
                    changed,
                    failure: None,
                });
                continue;
            }
            candidate_tasks.push(LoadTask {
                included,
                entry: find_entry(self.candidate.entries(), &included.path),
                changed,
            });
        }
        let candidate_read = |repo: &Repository, entry: &Entry| self.candidate.read(repo, entry);
        let min_clone_lines = self.config.measurement.min_clone_lines;
        let v102 = self.config.policy.nsd_v102;
        let results = load_in_parallel(self.repository, &candidate_tasks, |repo, task| {
            load_candidate(
                repo,
                task,
                &candidate_read,
                self.cache,
                min_clone_lines,
                v102,
            )
        });
        for (task, load) in candidate_tasks.iter().zip(results) {
            match load {
                Load::Failure(failure) => loads.failures.push(failure),
                Load::Changed(loaded) => loads.changed.push(*loaded),
                Load::Unchanged(candidates) => loads.unchanged.push(candidates),
                Load::Unanalyzable(reason) => loads.coverage.push(CoverageInput {
                    entry: task.included.clone(),
                    changed: task.changed,
                    failure: Some(reason),
                }),
            }
        }
        let reported: HashSet<RepoPath> = loads
            .coverage
            .iter()
            .map(|input| input.entry.path.clone())
            .collect();
        loads.coverage.extend(
            base_coverage
                .into_iter()
                .filter(|input| !reported.contains(&input.entry.path)),
        );
        loads
    }

    fn run_evaluators(
        &self,
        base: &[Loaded],
        changed: &[Loaded],
        unchanged: &[UnchangedCandidates],
        coverage: &[CoverageInput],
        policy: &PolicyConfig,
    ) -> Result<Vec<CheckDiagnostic>, GitError> {
        let base_files: Vec<FindingFile<'_>> = base.iter().map(Loaded::finding_file).collect();
        let candidate_files: Vec<FindingFile<'_>> =
            changed.iter().map(Loaded::finding_file).collect();
        let base_callables: Vec<FileCallables> = base.iter().map(Loaded::callables).collect();
        let candidate_callables: Vec<FileCallables> =
            changed.iter().map(Loaded::callables).collect();
        let base_metrics: Vec<FileMetrics> = base.iter().map(Loaded::metrics).collect();
        let candidate_metrics: Vec<FileMetrics> = changed.iter().map(Loaded::metrics).collect();

        let matched = match_callables(&base_callables, &candidate_callables, self.changes);
        let complexity = classify(&base_metrics, &candidate_metrics, &matched, policy);
        let findings = match_findings(
            &base_files,
            &candidate_files,
            &matched,
            self.changes,
            policy,
        )?;
        let suppressions = apply_suppressions(
            &base_files,
            &candidate_files,
            &findings,
            self.changes,
            policy,
        )?;
        let damage = evaluate_damage(&base_files, &candidate_files, self.changes)?;
        let coverage = evaluate_coverage(coverage, policy);
        let clones = evaluate_clones_precomputed(
            &base_files,
            &candidate_files,
            unchanged,
            self.changes,
            self.config.measurement.min_clone_lines,
            policy,
        )?;

        let mut diagnostics = Vec::new();
        diagnostics.extend(
            complexity
                .into_iter()
                .map(|found| CheckDiagnostic::Complexity(Box::new(found))),
        );
        diagnostics.extend(
            suppressions
                .findings
                .into_iter()
                .map(CheckDiagnostic::Finding),
        );
        diagnostics.extend(
            suppressions
                .diagnostics
                .into_iter()
                .map(CheckDiagnostic::Suppression),
        );
        diagnostics.extend(damage.into_iter().map(CheckDiagnostic::Damage));
        diagnostics.extend(coverage.into_iter().map(CheckDiagnostic::Coverage));
        diagnostics.extend(
            clones
                .into_iter()
                .map(|found| CheckDiagnostic::Clone(Box::new(found))),
        );
        Ok(diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::CODE_SNAPSHOT_UNAVAILABLE;

    #[test]
    fn test_unreadable_under_ceiling_changed_blob_maps_to_g101() {
        let path = RepoPath::from_bytes(b"src/Foo.java".to_vec());

        let unreadable = required_bytes(&path, Ok(None)).expect_err("an unreadable blob fails");
        let readable = required_bytes(&path, Ok(Some(b"class Foo {}".to_vec())));

        assert!(matches!(unreadable, CheckDiagnostic::Failure { .. }));
        assert_eq!(unreadable.code(), CODE_SNAPSHOT_UNAVAILABLE);
        assert_eq!(
            readable.expect("readable bytes pass through"),
            b"class Foo {}"
        );
    }
}
