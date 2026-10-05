//! M5-4: one uncached `check` run over snapshots (`nsd-plan-final.md` *CLI
//! and configuration*). Every internal failure is a diagnostic in the
//! outcome, never an `Err`, so the exit status is always 2-class for them.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use git2::{ErrorCode, Oid, Repository};
use rayon::prelude::*;

use crate::analysis::{analyze_file, FileAnalysis, UnanalyzableReason};
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
use crate::policy;
use crate::policy::clones::evaluate_clones;
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
}

/// Runs one check. Never fails: an internal error is an `NSD-G101` (or the
/// `resolve` error's own code) in the outcome.
pub fn run_check(request: &CheckRequest<'_>) -> CheckOutcome {
    let (diagnostics, policy) = match execute(request) {
        Ok(checked) => checked,
        Err(diagnostics) => (diagnostics, Config::default().policy),
    };
    let exit_status = exit_status(
        diagnostics.iter().map(CheckDiagnostic::code),
        &policy,
        request.allow_new_suppressions,
    );
    CheckOutcome {
        diagnostics,
        exit_status,
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

type Checked = (Vec<CheckDiagnostic>, PolicyConfig);

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
    let sides = Sides {
        repo: &repo,
        base: &base,
        candidate: &candidate,
        changes: &changes,
        config: &resolution.config,
        scope: &scope,
    };
    diagnostics.extend(sides.evaluate());
    Ok((diagnostics, resolution.config.policy))
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

/// Analyzes each read file in parallel, keeping input order.
fn analyze_all(
    reads: Vec<(&IncludedEntry, Vec<u8>)>,
) -> Vec<(
    &IncludedEntry,
    Vec<u8>,
    Result<FileAnalysis, UnanalyzableReason>,
)> {
    reads
        .into_par_iter()
        .map(|(entry, bytes)| {
            let analysis = match std::str::from_utf8(entry.path.as_bytes()) {
                Ok(path) => analyze_file(Path::new(path), &bytes),
                Err(_) => Err(UnanalyzableReason::NonUtf8Path),
            };
            (entry, bytes, analysis)
        })
        .collect()
}

struct Sides<'a> {
    repo: &'a Repository,
    base: &'a CommitSnapshot,
    candidate: &'a Snapshot,
    changes: &'a [Change],
    config: &'a Config,
    scope: &'a crate::config::CompiledScope,
}

/// The files and failures one check loads from its two snapshots.
#[derive(Default)]
struct Loads {
    base: Vec<Loaded>,
    changed: Vec<Loaded>,
    unchanged: Vec<Loaded>,
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
        let base_entries: HashMap<&RepoPath, &Entry> = self
            .base
            .entries
            .iter()
            .map(|entry| (&entry.path, entry))
            .collect();
        let candidate_entries: HashMap<&RepoPath, &Entry> = self
            .candidate
            .entries()
            .iter()
            .map(|entry| (&entry.path, entry))
            .collect();

        let mut base_reads = Vec::new();
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
            let read = base_entries
                .get(&included.path)
                .map_or_else(|| Ok(None), |entry| self.base.read(self.repo, entry));
            match required_bytes(&included.path, read) {
                Ok(bytes) => base_reads.push((included, bytes)),
                Err(failure) => loads.failures.push(failure),
            }
        }
        for (included, bytes, analysis) in analyze_all(base_reads) {
            // A base file that cannot be analyzed leaves its callables unmatched, so
            // the candidate side is judged as new code.
            if let Ok(analysis) = analysis {
                loads.base.push(Loaded {
                    path: included.path.clone(),
                    bytes,
                    analysis,
                });
            }
        }

        let unchanged_required = self.config.policy.nsd_v102 != Severity::Off;
        let mut candidate_reads = Vec::new();
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
            let read = candidate_entries
                .get(&included.path)
                .map_or_else(|| Ok(None), |entry| self.candidate.read(self.repo, entry));
            match required_bytes(&included.path, read) {
                Ok(bytes) => candidate_reads.push((included, bytes)),
                Err(failure) => loads.failures.push(failure),
            }
        }
        for (included, bytes, analysis) in analyze_all(candidate_reads) {
            match analysis {
                Ok(analysis) => {
                    let loaded = Loaded {
                        path: included.path.clone(),
                        bytes,
                        analysis,
                    };
                    if changed_candidate.contains(&included.path) {
                        loads.changed.push(loaded);
                    } else {
                        loads.unchanged.push(loaded);
                    }
                }
                Err(reason) => loads.coverage.push(CoverageInput {
                    entry: included.clone(),
                    changed: changed_candidate.contains(&included.path),
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
        unchanged: &[Loaded],
        coverage: &[CoverageInput],
        policy: &PolicyConfig,
    ) -> Result<Vec<CheckDiagnostic>, GitError> {
        let base_files: Vec<FindingFile<'_>> = base.iter().map(Loaded::finding_file).collect();
        let candidate_files: Vec<FindingFile<'_>> =
            changed.iter().map(Loaded::finding_file).collect();
        let unchanged_files: Vec<FindingFile<'_>> =
            unchanged.iter().map(Loaded::finding_file).collect();
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
        let clones = evaluate_clones(
            &base_files,
            &candidate_files,
            &unchanged_files,
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
