//! Integration tests for M4-2's V102 clone-regression policy
//! (`nsd-plan-final.md` *Diagnostics*, A3). Every scenario commits real
//! sources to a scratch repository and goes through the real diff and
//! analysis.

mod common;

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use git2::{Oid, Repository};
use nsd::analysis::{analyze_file, FileAnalysis};
use nsd::config::{PolicyConfig, Severity};
use nsd::git::diff::{diff_commit_to_commit, Change};
use nsd::git::path::RepoPath;
use nsd::git::snapshot::CommitSnapshot;
use nsd::model::DEFAULT_MIN_CLONE_LINES;
use nsd::policy::clones::evaluate_clones;
use nsd::policy::diagnostics::{CloneDiagnostic, CODE_CLONE_REGRESSION};
use nsd::policy::findings::FindingFile;

const MODE_REGULAR: i32 = 0o100644;
const BLOCK_LINES: usize = 12;
const FIRST_BODY_LINE: usize = 3;
const TS_FIRST_BODY_LINE: usize = 2;
const EXTENSION_BASE_SMALL: usize = 10;
const EXTENSION_BASE_LARGE: usize = 100;
const LARGE_THRESHOLD: usize = 10;
const SCALING_BLOCK_STATEMENTS: usize = 10;
const SCALING_BLOCKS: usize = 400;
const SCALING_FACTOR: usize = 8;
const SCALING_SAMPLES: usize = 3;
const SCALING_ASSERT_MULTIPLIER: u32 = 32;
const FILLER_STATEMENTS: usize = 30;
const REINDENT: &str = "    ";

// ---------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------

fn commit(repo: &Repository, files: &[(&str, String)]) -> Oid {
    let entries: Vec<_> = files
        .iter()
        .map(|(file, text)| {
            (
                file.as_bytes().to_vec(),
                MODE_REGULAR,
                text.clone().into_bytes(),
            )
        })
        .collect();
    common::commit_entries(repo, &entries)
}

/// `count` one-line statements; every token is unique to `tag` and the index.
fn statements(tag: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!("        int {tag}{index} = {tag}Call{index}(x);\n"))
        .collect()
}

fn java(name: &str, body: &str) -> String {
    format!("class {name} {{\n    void run(int x) {{\n{body}    }}\n}}\n")
}

fn block(tag: &str) -> String {
    statements(tag, BLOCK_LINES)
}

/// Adds four spaces of indentation to the body line at `index`.
fn reindent_line(body: &str, index: usize) -> String {
    body.lines()
        .enumerate()
        .map(|(position, line)| {
            if position == index {
                format!("{REINDENT}{line}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect()
}

fn reindent_all(body: &str) -> String {
    body.lines()
        .map(|line| format!("{REINDENT}{line}\n"))
        .collect()
}

fn ts_file(body: &str) -> String {
    format!("export function run(x: number) {{\n{body}}}\n")
}

fn ts_block(tag: &str) -> String {
    (0..BLOCK_LINES)
        .map(|index| format!("  const {tag}{index} = {tag}Call{index}(x);\n"))
        .collect()
}

struct Analyzed {
    path: RepoPath,
    source: Vec<u8>,
    analysis: FileAnalysis,
}

fn analyzed(
    snapshot: &CommitSnapshot,
    repo: &Repository,
    wanted: impl Fn(&RepoPath) -> bool,
) -> Vec<Analyzed> {
    let mut out = Vec::new();
    for entry in snapshot.entries.iter().filter(|e| wanted(&e.path)) {
        let Some(bytes) = snapshot.read(repo, entry).expect("read blob") else {
            continue;
        };
        let Ok(analysis) = analyze_file(Path::new(&entry.path.render()), &bytes) else {
            continue;
        };
        out.push(Analyzed {
            path: entry.path.clone(),
            source: bytes,
            analysis,
        });
    }
    out
}

fn changed_paths(changes: &[Change]) -> (BTreeSet<RepoPath>, BTreeSet<RepoPath>) {
    let mut base_paths = BTreeSet::new();
    let mut candidate_paths = BTreeSet::new();
    for change in changes {
        match change {
            Change::Added { path, .. } => {
                candidate_paths.insert(path.clone());
            }
            Change::Deleted { path, .. } => {
                base_paths.insert(path.clone());
            }
            Change::Modified { path, .. } | Change::Typechange { path, .. } => {
                base_paths.insert(path.clone());
                candidate_paths.insert(path.clone());
            }
            Change::Renamed { from, to, .. } => {
                base_paths.insert(from.clone());
                candidate_paths.insert(to.clone());
            }
        }
    }
    (base_paths, candidate_paths)
}

fn finding_files(files: &[Analyzed]) -> Vec<FindingFile<'_>> {
    files
        .iter()
        .map(|file| FindingFile {
            path: file.path.clone(),
            source: &file.source,
            analysis: &file.analysis,
        })
        .collect()
}

struct Sides {
    base: Vec<Analyzed>,
    candidate: Vec<Analyzed>,
    unchanged: Vec<Analyzed>,
    changes: Vec<Change>,
}

fn load(repo: &Repository, base: Oid, candidate: Oid) -> Sides {
    let changes = diff_commit_to_commit(repo, Some(base), candidate).expect("diff commits");
    let (base_paths, candidate_paths) = changed_paths(&changes);
    let base_snapshot = CommitSnapshot::at(repo, base).expect("base snapshot");
    let candidate_snapshot = CommitSnapshot::at(repo, candidate).expect("candidate snapshot");
    Sides {
        base: analyzed(&base_snapshot, repo, |path| base_paths.contains(path)),
        candidate: analyzed(&candidate_snapshot, repo, |path| {
            candidate_paths.contains(path)
        }),
        unchanged: analyzed(&candidate_snapshot, repo, |path| {
            !candidate_paths.contains(path)
        }),
        changes,
    }
}

fn evaluate_sides(
    sides: &Sides,
    min_clone_lines: u32,
    policy: &PolicyConfig,
    reversed: bool,
) -> Vec<CloneDiagnostic> {
    let mut base = finding_files(&sides.base);
    let mut candidate = finding_files(&sides.candidate);
    let mut unchanged = finding_files(&sides.unchanged);
    let mut changes = sides.changes.clone();
    if reversed {
        base.reverse();
        candidate.reverse();
        unchanged.reverse();
        changes.reverse();
    }
    evaluate_clones(
        &base,
        &candidate,
        &unchanged,
        &changes,
        min_clone_lines,
        policy,
    )
    .expect("evaluate clones")
}

fn evaluate_min(
    repo: &Repository,
    base: Oid,
    candidate: Oid,
    min_clone_lines: u32,
) -> Vec<CloneDiagnostic> {
    let sides = load(repo, base, candidate);
    evaluate_sides(&sides, min_clone_lines, &policy_with(Severity::Deny), false)
}

fn evaluate(repo: &Repository, base: Oid, candidate: Oid) -> Vec<CloneDiagnostic> {
    evaluate_min(repo, base, candidate, DEFAULT_MIN_CLONE_LINES)
}

fn policy_with(nsd_v102: Severity) -> PolicyConfig {
    PolicyConfig {
        nsd_e101: Severity::Deny,
        nsd_e102: Severity::Deny,
        nsd_v101: Severity::Deny,
        nsd_v102,
        nsd_s102: Severity::Warn,
    }
}

type Site = (String, usize, usize);

fn sites(diagnostics: &[CloneDiagnostic]) -> Vec<Site> {
    diagnostics
        .iter()
        .map(|d| {
            assert_eq!(d.code, CODE_CLONE_REGRESSION);
            (
                d.candidate_path.render(),
                d.candidate_start_line,
                d.candidate_end_line,
            )
        })
        .collect()
}

fn site(path: &str, start: usize, end: usize) -> Site {
    (path.to_string(), start, end)
}

fn body_site(path: &str, lines: usize) -> Site {
    site(path, FIRST_BODY_LINE, FIRST_BODY_LINE + lines - 1)
}

fn has_change(changes: &[Change], wanted: impl Fn(&Change) -> bool) -> bool {
    changes.iter().any(wanted)
}

// ---------------------------------------------------------------------
// Extension thresholds.
// ---------------------------------------------------------------------

/// `A.java` holds a clone of `base_count` lines and is extended by `added`
/// lines to match `B.java`, which already holds the extended form.
fn extension_run(base_count: usize, added: usize) -> Vec<CloneDiagnostic> {
    let base_body = statements("a", base_count);
    let extended = format!("{base_body}{}", statements("e", added));
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("A.java", java("A", &base_body)),
            ("B.java", java("B", &extended)),
        ],
    );
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &extended)),
            ("B.java", java("B", &extended)),
        ],
    );
    evaluate(&repo, base, candidate)
}

#[test]
fn test_extension_by_one_line_of_a_ten_line_clone_passes() {
    assert!(
        extension_run(EXTENSION_BASE_SMALL, 1).is_empty(),
        "+1 on a 10-line base is within max(1, 10 / 10)"
    );
    assert_eq!(
        extension_run(EXTENSION_BASE_SMALL, 2).len(),
        1,
        "the same scenario at +2 must raise, so the empty result above is a verdict"
    );
}

#[test]
fn test_extension_by_two_lines_of_a_ten_line_clone_raises_v102() {
    let found = extension_run(EXTENSION_BASE_SMALL, 2);

    assert_eq!(found.len(), 1, "{found:?}");
    let diagnostic = &found[0];
    assert_eq!(diagnostic.code, CODE_CLONE_REGRESSION);
    assert_eq!(
        sites(&found),
        vec![body_site("A.java", EXTENSION_BASE_SMALL + 2)]
    );
    assert_eq!(diagnostic.base_lines, Some(EXTENSION_BASE_SMALL));
    assert_eq!(diagnostic.added_lines, 2);
    assert_eq!(diagnostic.matched_path.render(), "B.java");
}

#[test]
fn test_extension_by_ten_lines_of_a_hundred_line_clone_passes() {
    assert!(
        extension_run(EXTENSION_BASE_LARGE, LARGE_THRESHOLD).is_empty(),
        "+10 on a 100-line base is within max(1, 100 / 10)"
    );
    assert_eq!(
        extension_run(EXTENSION_BASE_LARGE, LARGE_THRESHOLD + 1).len(),
        1,
        "the same scenario at +11 must raise, so the empty result above is a verdict"
    );
}

#[test]
fn test_extension_by_eleven_lines_of_a_hundred_line_clone_raises_v102() {
    let found = extension_run(EXTENSION_BASE_LARGE, LARGE_THRESHOLD + 1);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].base_lines, Some(EXTENSION_BASE_LARGE));
    assert_eq!(found[0].added_lines, LARGE_THRESHOLD + 1);
}

// ---------------------------------------------------------------------
// Moves and copies (A3).
// ---------------------------------------------------------------------

/// `A.java` and `B.java` hold the same block in base.
fn two_copies(repo: &Repository) -> Oid {
    commit(
        repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
        ],
    )
}

#[test]
fn test_block_copied_across_files_raises_v102() {
    let (_dir, repo) = common::init_repo();
    let base = two_copies(&repo);
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
            ("New.java", java("New", &block("a"))),
        ],
    );

    let found = evaluate(&repo, base, candidate);

    assert_eq!(sites(&found), vec![body_site("New.java", BLOCK_LINES)]);
    assert_eq!(found[0].base_lines, None);
    assert_eq!(found[0].added_lines, BLOCK_LINES);
    assert_eq!(found[0].matched_path.render(), "A.java");
    assert_eq!(
        (found[0].matched_start_line, found[0].matched_end_line),
        (FIRST_BODY_LINE, FIRST_BODY_LINE + BLOCK_LINES - 1)
    );
}

#[test]
fn test_block_moved_across_files_passes() {
    let (_dir, repo) = common::init_repo();
    let base = two_copies(&repo);
    let moved = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", "")),
            ("New.java", java("New", &block("a"))),
        ],
    );
    let copied = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
            ("New.java", java("New", &block("a"))),
        ],
    );

    assert!(evaluate(&repo, base, moved).is_empty());
    assert_eq!(evaluate(&repo, base, copied).len(), 1);
}

#[test]
fn test_moved_block_edited_inside_its_body_raises_v102() {
    let edited = block("a").replace("aCall6(", "bCall6(");
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
            ("C.java", java("C", &edited)),
        ],
    );
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", "")),
            ("C.java", java("C", &edited)),
            ("New.java", java("New", &edited)),
        ],
    );

    let found = evaluate(&repo, base, candidate);

    assert_eq!(sites(&found), vec![body_site("New.java", BLOCK_LINES)]);
    assert_eq!(found[0].matched_path.render(), "C.java");
}

/// `Orig.java` holds a 10-line block that moves to `New.java`, inside a
/// context of `extra` more lines that `Keep.java` carries too.
fn wrapped_move(extra: usize) -> Vec<CloneDiagnostic> {
    let core = statements("a", EXTENSION_BASE_SMALL);
    let wrapped = format!("{}{core}", statements("w", extra));
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("Orig.java", java("Orig", &core)),
            ("Keep.java", java("Keep", &wrapped)),
        ],
    );
    let candidate = commit(
        &repo,
        &[
            ("Orig.java", java("Orig", "")),
            ("Keep.java", java("Keep", &wrapped)),
            ("New.java", java("New", &wrapped)),
        ],
    );
    evaluate(&repo, base, candidate)
}

#[test]
fn test_moved_block_wrapped_within_the_threshold_passes_and_beyond_it_raises() {
    assert!(
        wrapped_move(1).is_empty(),
        "1 extra line is within max(1, 10 / 10)"
    );

    let beyond = wrapped_move(2);

    assert_eq!(
        sites(&beyond),
        vec![body_site("New.java", EXTENSION_BASE_SMALL + 2)]
    );
    assert_eq!(beyond[0].base_lines, None);
}

#[test]
fn test_block_moved_out_of_a_deleted_file_passes() {
    let filler = statements("z", FILLER_STATEMENTS);
    let with_filler = format!(
        "class B {{\n    void run(int x) {{\n{}    }}\n    void other(int x) {{\n{filler}    }}\n}}\n",
        block("a")
    );
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[("A.java", java("A", &block("a"))), ("B.java", with_filler)],
    );
    let moved = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("New.java", java("New", &block("a"))),
        ],
    );
    let moved_changes = diff_commit_to_commit(&repo, Some(base), moved).expect("diff");
    assert!(
        has_change(&moved_changes, |c| matches!(c, Change::Deleted { .. })),
        "the fixture must be a delete plus an add, not a rename: {moved_changes:?}"
    );
    assert!(has_change(&moved_changes, |c| matches!(
        c,
        Change::Added { .. }
    )));

    assert!(evaluate(&repo, base, moved).is_empty());

    let kept = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
            ("New.java", java("New", &block("a"))),
        ],
    );
    assert_eq!(evaluate(&repo, base, kept).len(), 1);
}

#[test]
fn test_repeated_identical_moved_blocks_pair_kth_to_kth() {
    let two_methods = |name: &str| {
        format!(
            "class {name} {{\n    void m1(int x) {{\n{}    }}\n    void m2(int x) {{\n{}    }}\n}}\n",
            block("a"),
            block("a")
        )
    };
    let one_method = format!(
        "class S {{\n    void m2(int x) {{\n{}    }}\n}}\n",
        block("a")
    );
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("S.java", two_methods("S"))]);
    let both_moved = commit(
        &repo,
        &[
            ("S.java", "class S {\n}\n".to_string()),
            ("New.java", two_methods("New")),
        ],
    );
    let one_deleted = commit(
        &repo,
        &[("S.java", one_method), ("New.java", two_methods("New"))],
    );

    assert!(evaluate(&repo, base, both_moved).is_empty());

    let found = evaluate(&repo, base, one_deleted);

    let second_start = FIRST_BODY_LINE + BLOCK_LINES + 2;
    assert_eq!(
        sites(&found),
        vec![site(
            "New.java",
            second_start,
            second_start + BLOCK_LINES - 1
        )]
    );
}

#[test]
fn test_moved_block_leaving_its_interior_comment_behind_passes() {
    let body = block("a");
    let split = body
        .match_indices('\n')
        .nth(5)
        .map(|(position, _)| position + 1)
        .expect("six lines");
    let (first, second) = body.split_at(split);
    let commented = format!("{first}        // note\n{second}");
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("S.java", java("S", &commented)),
        ],
    );
    let moved = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("S.java", java("S", "        // note\n")),
            ("New.java", java("New", &block("a"))),
        ],
    );
    let copied = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("S.java", java("S", &commented)),
            ("New.java", java("New", &block("a"))),
        ],
    );

    assert!(evaluate(&repo, base, moved).is_empty());
    assert_eq!(
        sites(&evaluate(&repo, base, copied)),
        vec![body_site("New.java", BLOCK_LINES)]
    );
}

#[test]
fn test_base_unique_block_moved_and_copied_raises_one_v102_on_the_copy() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("A.java", java("A", &block("a")))]);
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", "")),
            ("B.java", java("B", &block("a"))),
            ("C.java", java("C", &block("a"))),
        ],
    );

    let found = evaluate(&repo, base, candidate);

    assert_eq!(sites(&found), vec![body_site("C.java", BLOCK_LINES)]);
    assert_eq!(found[0].matched_path.render(), "B.java");
}

#[test]
fn test_base_unique_block_edited_in_place_and_copied_raises_only_on_the_copy() {
    let extended = format!("{}{}", block("a"), statements("e", 1));
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("A.java", java("A", &block("a")))]);
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &extended)),
            ("New.java", java("New", &extended)),
        ],
    );

    let found = evaluate(&repo, base, candidate);

    assert_eq!(sites(&found), vec![body_site("New.java", BLOCK_LINES + 1)]);
    assert_eq!(found[0].base_lines, None);
    assert_eq!(found[0].matched_path.render(), "A.java");
}

#[test]
fn test_token_containment_respects_token_boundaries() {
    let forged = format!("x = \"\u{1}foo\u{1};\u{1}bar\u{1};\";\ny = 1;\n");
    let genuine = "foo;\nbar;\n".to_string();
    let min_lines = 2;
    let (_dir, repo) = common::init_repo();

    let forged_base = commit(
        &repo,
        &[("Src.js", genuine.clone()), ("Other.js", forged.clone())],
    );
    let forged_candidate = commit(
        &repo,
        &[
            ("Src.js", "// moved\n".to_string()),
            ("Other.js", forged.clone()),
            ("New.js", forged.clone()),
        ],
    );
    let found = evaluate_min(&repo, forged_base, forged_candidate, min_lines);
    assert_eq!(sites(&found), vec![site("New.js", 1, 2)]);

    let (_dir, repo) = common::init_repo();
    let genuine_base = commit(
        &repo,
        &[("Src.js", genuine.clone()), ("Other.js", genuine.clone())],
    );
    let genuine_candidate = commit(
        &repo,
        &[
            ("Src.js", "// moved\n".to_string()),
            ("Other.js", genuine.clone()),
            ("New.js", genuine),
        ],
    );
    assert!(evaluate_min(&repo, genuine_base, genuine_candidate, min_lines).is_empty());
}

// ---------------------------------------------------------------------
// Candidate and base set construction.
// ---------------------------------------------------------------------

#[test]
fn test_new_copy_of_an_untouched_files_block_raises_one_v102_on_the_changed_side() {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("Other.java", java("Other", &statements("o", 3))),
        ],
    );
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("Other.java", java("Other", &statements("o", 4))),
            ("New.java", java("New", &block("a"))),
        ],
    );

    let found = evaluate(&repo, base, candidate);

    assert_eq!(sites(&found), vec![body_site("New.java", BLOCK_LINES)]);
    assert_eq!(found[0].matched_path.render(), "A.java");
}

#[test]
fn test_rewritten_block_does_not_match_its_own_replaced_base_version() {
    let rewritten = reindent_all(&block("a"));
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("A.java", java("A", &block("a")))]);
    let rewrite = commit(&repo, &[("A.java", java("A", &rewritten))]);
    let rewrite_and_copy = commit(
        &repo,
        &[
            ("A.java", java("A", &rewritten)),
            ("New.java", java("New", &rewritten)),
        ],
    );

    assert!(evaluate(&repo, base, rewrite).is_empty());
    assert_eq!(
        sites(&evaluate(&repo, base, rewrite_and_copy)),
        vec![body_site("New.java", BLOCK_LINES)]
    );
}

#[test]
fn test_clones_between_unchanged_files_raise_nothing() {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
            ("Other.java", java("Other", "")),
        ],
    );
    let unrelated = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
            ("Other.java", java("Other", &statements("o", 3))),
        ],
    );
    let copy = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
            ("Other.java", java("Other", &block("a"))),
        ],
    );

    assert!(evaluate(&repo, base, unrelated).is_empty());
    assert_eq!(
        sites(&evaluate(&repo, base, copy)),
        vec![body_site("Other.java", BLOCK_LINES)]
    );
}

#[test]
fn test_pure_line_shift_raises_nothing() {
    let shifted = format!("// pad\n\n// pad again\n\n{}", java("A", &block("a")));
    let (_dir, repo) = common::init_repo();
    let base = two_copies(&repo);
    let shift = commit(
        &repo,
        &[
            ("A.java", shifted.clone()),
            ("B.java", java("B", &block("a"))),
        ],
    );
    let shift_and_copy = commit(
        &repo,
        &[
            ("A.java", shifted),
            ("B.java", java("B", &block("a"))),
            ("C.java", java("C", &block("a"))),
        ],
    );

    assert!(evaluate(&repo, base, shift).is_empty());
    assert_eq!(
        sites(&evaluate(&repo, base, shift_and_copy)),
        vec![body_site("C.java", BLOCK_LINES)]
    );
}

#[test]
fn test_comment_only_edit_inside_a_clone_raises_nothing() {
    let commented: String = block("a")
        .lines()
        .enumerate()
        .map(|(index, line)| {
            if index % 4 == 0 {
                format!("        // note {index}\n{line}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    let (_dir, repo) = common::init_repo();
    let base = two_copies(&repo);
    let edit = commit(
        &repo,
        &[
            ("A.java", java("A", &commented)),
            ("B.java", java("B", &block("a"))),
        ],
    );
    let edit_and_copy = commit(
        &repo,
        &[
            ("A.java", java("A", &commented)),
            ("B.java", java("B", &block("a"))),
            ("C.java", java("C", &block("a"))),
        ],
    );

    assert!(evaluate(&repo, base, edit).is_empty());
    assert_eq!(
        sites(&evaluate(&repo, base, edit_and_copy)),
        vec![body_site("C.java", BLOCK_LINES)]
    );
}

#[test]
fn test_renamed_file_keeps_its_existing_clone_without_v102() {
    let with_other = |class: &str, body: &str, other: &str| {
        format!(
            "class {class} {{\n    void run(int x) {{\n{body}    }}\n    void other(int y) {{\n{other}    }}\n}}\n"
        )
    };
    let core = statements("a", EXTENSION_BASE_SMALL);
    let extended = format!("{core}{}", statements("e", 2));
    let other = "        y++;\n        y--;\n";
    let other_edited = "        y++;\n        y--;\n        y = 0;\n";
    let (_dir, repo) = common::init_repo();

    let base = commit(
        &repo,
        &[
            ("A.java", java("A", &core)),
            ("B.java", with_other("B", &core, other)),
        ],
    );
    let renamed = commit(
        &repo,
        &[
            ("A.java", java("A", &core)),
            ("R.java", with_other("R", &core, other_edited)),
        ],
    );
    let renames = diff_commit_to_commit(&repo, Some(base), renamed).expect("diff");
    assert!(
        has_change(&renames, |c| matches!(c, Change::Renamed { .. })),
        "the fixture must be a git rename: {renames:?}"
    );
    assert!(evaluate(&repo, base, renamed).is_empty());

    let extended_base = commit(
        &repo,
        &[
            ("A.java", java("A", &extended)),
            ("B.java", with_other("B", &core, other)),
        ],
    );
    let extended_rename = commit(
        &repo,
        &[
            ("A.java", java("A", &extended)),
            ("R.java", with_other("R", &extended, other)),
        ],
    );
    let found = evaluate(&repo, extended_base, extended_rename);

    assert_eq!(
        sites(&found),
        vec![body_site("R.java", EXTENSION_BASE_SMALL + 2)]
    );
    assert_eq!(found[0].base_lines, Some(EXTENSION_BASE_SMALL));
    assert_eq!(found[0].added_lines, 2);
}

#[test]
fn test_overlapping_sub_runs_raise_one_diagnostic_per_maximal_occurrence() {
    let long_block = statements("a", 30);
    let min_lines = 2;
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("A.java", java("A", &long_block))]);
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &long_block)),
            ("New.java", java("New", &long_block)),
        ],
    );

    let found = evaluate_min(&repo, base, candidate, min_lines);

    assert_eq!(sites(&found), vec![body_site("New.java", 30)]);
}

#[test]
fn test_min_clone_lines_sets_the_qualifying_size() {
    let small = statements("a", 6);
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("A.java", java("A", &small))]);
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &small)),
            ("New.java", java("New", &small)),
        ],
    );

    assert!(evaluate_min(&repo, base, candidate, 10).is_empty());
    assert_eq!(
        sites(&evaluate_min(&repo, base, candidate, 5)),
        vec![body_site("New.java", 6)]
    );
}

#[test]
fn test_jsts_block_copied_across_files_raises_v102() {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("a.ts", ts_file(&ts_block("a"))),
            ("b.ts", ts_file(&ts_block("a"))),
        ],
    );
    let copied = commit(
        &repo,
        &[
            ("a.ts", ts_file(&ts_block("a"))),
            ("b.ts", ts_file(&ts_block("a"))),
            ("c.ts", ts_file(&ts_block("a"))),
        ],
    );
    let moved = commit(
        &repo,
        &[
            ("a.ts", ts_file(&ts_block("a"))),
            ("b.ts", ts_file("")),
            ("c.ts", ts_file(&ts_block("a"))),
        ],
    );

    let found = evaluate(&repo, base, copied);

    assert_eq!(
        sites(&found),
        vec![site(
            "c.ts",
            TS_FIRST_BODY_LINE,
            TS_FIRST_BODY_LINE + BLOCK_LINES - 1
        )]
    );
    assert!(evaluate(&repo, base, moved).is_empty());
}

// ---------------------------------------------------------------------
// Edges of the line accounting.
// ---------------------------------------------------------------------

#[test]
fn test_reindented_first_statement_of_a_clone_raises_nothing() {
    reindented_statement_scenario(0);
}

#[test]
fn test_reindented_last_statement_of_a_clone_raises_nothing() {
    reindented_statement_scenario(EXTENSION_BASE_SMALL - 1);
}

/// Reindenting one end statement of `A.java`'s 10-line clone is a 1-line
/// extension; the same edit plus two added statements is not.
fn reindented_statement_scenario(index: usize) {
    let core = statements("a", EXTENSION_BASE_SMALL);
    let reindented = reindent_line(&core, index);
    let (_dir, repo) = common::init_repo();

    let base = commit(
        &repo,
        &[("A.java", java("A", &core)), ("B.java", java("B", &core))],
    );
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &reindented)),
            ("B.java", java("B", &core)),
        ],
    );
    assert!(evaluate(&repo, base, candidate).is_empty());

    let extension = statements("e", 2);
    let extended = format!("{core}{extension}");
    let extended_base = commit(
        &repo,
        &[
            ("A.java", java("A", &core)),
            ("B.java", java("B", &extended)),
        ],
    );
    let extended_candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &format!("{reindented}{extension}"))),
            ("B.java", java("B", &extended)),
        ],
    );
    let found = evaluate(&repo, extended_base, extended_candidate);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].base_lines, Some(EXTENSION_BASE_SMALL));
}

// ---------------------------------------------------------------------
// Policy switch, ordering, scale.
// ---------------------------------------------------------------------

#[test]
fn test_v102_off_raises_nothing_and_warn_still_reports() {
    let (_dir, repo) = common::init_repo();
    let base = two_copies(&repo);
    let candidate = commit(
        &repo,
        &[
            ("A.java", java("A", &block("a"))),
            ("B.java", java("B", &block("a"))),
            ("New.java", java("New", &block("a"))),
        ],
    );
    let sides = load(&repo, base, candidate);
    let with = policy_with;

    let denied = evaluate_sides(
        &sides,
        DEFAULT_MIN_CLONE_LINES,
        &with(Severity::Deny),
        false,
    );
    let warned = evaluate_sides(
        &sides,
        DEFAULT_MIN_CLONE_LINES,
        &with(Severity::Warn),
        false,
    );
    let off = evaluate_sides(&sides, DEFAULT_MIN_CLONE_LINES, &with(Severity::Off), false);

    assert_eq!(denied.len(), 1, "{denied:?}");
    assert_eq!(warned, denied);
    assert!(off.is_empty(), "{off:?}");
}

#[test]
fn test_output_is_sorted_and_deterministic() {
    let two_blocks = |name: &str| {
        format!(
            "class {name} {{\n    void m1(int x) {{\n{}    }}\n    void m2(int x) {{\n{}    }}\n}}\n",
            block("a"),
            block("b")
        )
    };
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("A.java", two_blocks("A"))]);
    let candidate = commit(
        &repo,
        &[
            ("A.java", two_blocks("A")),
            ("Zed.java", two_blocks("Zed")),
            ("Mid.java", two_blocks("Mid")),
            ("New.java", two_blocks("New")),
        ],
    );
    let sides = load(&repo, base, candidate);
    let policy = policy_with(Severity::Deny);

    let forward = evaluate_sides(&sides, DEFAULT_MIN_CLONE_LINES, &policy, false);
    let backward = evaluate_sides(&sides, DEFAULT_MIN_CLONE_LINES, &policy, true);

    let second = FIRST_BODY_LINE + BLOCK_LINES + 2;
    let expected: Vec<Site> = ["Mid.java", "New.java", "Zed.java"]
        .iter()
        .flat_map(|path| {
            [
                site(path, FIRST_BODY_LINE, FIRST_BODY_LINE + BLOCK_LINES - 1),
                site(path, second, second + BLOCK_LINES - 1),
            ]
        })
        .collect();
    assert_eq!(sites(&forward), expected);
    assert_eq!(backward, forward);
}

/// A class with one method per index in `indices`, each holding its own
/// distinct 10-statement block.
fn blocks_file(class: &str, indices: impl Iterator<Item = usize>) -> String {
    let methods: String = indices
        .map(|index| {
            format!(
                "    void m{index}(int x) {{\n{}    }}\n",
                statements(&format!("b{index}x"), SCALING_BLOCK_STATEMENTS)
            )
        })
        .collect();
    format!("class {class} {{\n{methods}}}\n")
}

fn fastest_evaluation(sides: &Sides) -> (Duration, usize) {
    let policy = policy_with(Severity::Deny);
    let mut fastest = Duration::MAX;
    let mut count = 0;
    for _ in 0..SCALING_SAMPLES {
        let started = Instant::now();
        let found = evaluate_sides(sides, DEFAULT_MIN_CLONE_LINES, &policy, false);
        fastest = fastest.min(started.elapsed());
        count = found.len();
    }
    (fastest, count)
}

fn assert_scales_linearly(what: &str, small: (Duration, usize), large: (Duration, usize)) {
    assert!(
        large.0 < small.0 * SCALING_ASSERT_MULTIPLIER,
        "a quadratic pass scales ~64x from N to 8N, a linear one ~8x: {what} at \
         N={SCALING_BLOCKS} took {:?}, at 8N={} took {:?}, ratio {:.1}x, expected < \
         {SCALING_ASSERT_MULTIPLIER}x",
        small.0,
        SCALING_BLOCKS * SCALING_FACTOR,
        large.0,
        large.0.as_secs_f64() / small.0.as_secs_f64().max(f64::EPSILON)
    );
}

fn copy_run(blocks: usize) -> (Duration, usize) {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("U.java", blocks_file("U", 0..blocks))]);
    let candidate = commit(
        &repo,
        &[
            ("U.java", blocks_file("U", 0..blocks)),
            ("D.java", blocks_file("D", 0..blocks)),
        ],
    );
    fastest_evaluation(&load(&repo, base, candidate))
}

#[test]
fn test_evaluation_does_not_scale_quadratically_in_changed_occurrences() {
    let small = copy_run(SCALING_BLOCKS);
    let large = copy_run(SCALING_BLOCKS * SCALING_FACTOR);

    assert_eq!(
        (small.1, large.1),
        (SCALING_BLOCKS, SCALING_BLOCKS * SCALING_FACTOR)
    );
    assert_scales_linearly("evaluate_clones over copied blocks", small, large);
}

/// Every block of `S.java` moves to `D.java`, except the first `kept` blocks
/// that stay in `S.java` as well.
fn move_run(blocks: usize, kept: usize) -> (Duration, usize) {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("U.java", blocks_file("U", 0..blocks)),
            ("S.java", blocks_file("S", 0..blocks)),
        ],
    );
    let candidate = commit(
        &repo,
        &[
            ("U.java", blocks_file("U", 0..blocks)),
            ("S.java", blocks_file("S", 0..kept)),
            ("D.java", blocks_file("D", 0..blocks)),
        ],
    );
    fastest_evaluation(&load(&repo, base, candidate))
}

#[test]
fn test_move_mapping_does_not_scale_quadratically_in_moved_blocks() {
    let small = move_run(SCALING_BLOCKS, 0);
    let large = move_run(SCALING_BLOCKS * SCALING_FACTOR, 0);
    let one_copied = move_run(SCALING_BLOCKS, 1);

    assert_eq!((small.1, large.1), (0, 0), "every pair is a move");
    assert_eq!(one_copied.1, 1, "one block copied, not moved");
    assert_scales_linearly("evaluate_clones over moved blocks", small, large);
}

/// The largest block count whose eight-fold file stays under the 1 MiB
/// source ceiling; a larger file is silently skipped by `analyzed`.
const CEILING_SAFE_BLOCKS: usize = 250;

fn assert_ratio_below_bound(what: &str, small: Duration, large: Duration) {
    assert!(
        large < small * SCALING_ASSERT_MULTIPLIER,
        "{what} at N={CEILING_SAFE_BLOCKS} took {small:?}, at 8N={} took {large:?}, ratio \
         {:.1}x, expected < {SCALING_ASSERT_MULTIPLIER}x",
        CEILING_SAFE_BLOCKS * SCALING_FACTOR,
        large.as_secs_f64() / small.as_secs_f64().max(f64::EPSILON)
    );
}

#[test]
fn test_scaling_fixtures_stay_under_the_source_ceiling() {
    let largest = blocks_file("D", 0..CEILING_SAFE_BLOCKS * SCALING_FACTOR);
    assert!(
        (largest.len() as u64) < nsd::git::snapshot::SOURCE_CEILING_BYTES,
        "a {} byte fixture is skipped by the analyzer, so the scaling run is vacuous",
        largest.len()
    );
}

#[test]
fn test_copy_evaluation_scales_linearly_within_the_source_ceiling() {
    let small = copy_run(CEILING_SAFE_BLOCKS);
    let large = copy_run(CEILING_SAFE_BLOCKS * SCALING_FACTOR);

    assert_eq!(
        (small.1, large.1),
        (CEILING_SAFE_BLOCKS, CEILING_SAFE_BLOCKS * SCALING_FACTOR)
    );
    assert_ratio_below_bound("evaluate_clones over copied blocks", small.0, large.0);
}

#[test]
fn test_move_mapping_scales_linearly_within_the_source_ceiling() {
    let small = move_run(CEILING_SAFE_BLOCKS, 0);
    let large = move_run(CEILING_SAFE_BLOCKS * SCALING_FACTOR, 0);
    let one_copied = move_run(CEILING_SAFE_BLOCKS, 1);

    assert_eq!((small.1, large.1), (0, 0), "every pair is a move");
    assert_eq!(one_copied.1, 1, "one block copied, not moved");
    assert_ratio_below_bound("evaluate_clones over moved blocks", small.0, large.0);
}

const SUB_TEN_BASE: usize = 5;

#[test]
fn test_extension_of_a_sub_ten_line_clone_keeps_the_one_line_floor() {
    let run = |added: usize| {
        let base_body = statements("a", SUB_TEN_BASE);
        let extended = format!("{base_body}{}", statements("e", added));
        let (_dir, repo) = common::init_repo();
        let base = commit(
            &repo,
            &[
                ("A.java", java("A", &base_body)),
                ("B.java", java("B", &extended)),
            ],
        );
        let candidate = commit(
            &repo,
            &[
                ("A.java", java("A", &extended)),
                ("B.java", java("B", &extended)),
            ],
        );
        evaluate_min(&repo, base, candidate, SUB_TEN_BASE as u32)
    };

    assert!(run(1).is_empty(), "+1 on a 5-line base is max(1, 5 / 10)");
    let found = run(2);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].base_lines, Some(SUB_TEN_BASE));
}
