//! Integration tests for M3-4's A101 parse-damage policy
//! (`nsd-plan-final.md` *Diagnostics*). Every scenario commits real sources
//! to a scratch repository and goes through the real diff and analysis.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use git2::{Oid, Repository};
use nsd::analysis::{analyze_file, FileAnalysis};
use nsd::git::diff::{diff_commit_to_commit, Change};
use nsd::git::path::RepoPath;
use nsd::git::snapshot::CommitSnapshot;
use nsd::policy::damage::evaluate_damage;
use nsd::policy::diagnostics::CODE_PARSE_DAMAGE;
use nsd::policy::findings::FindingFile;

const MODE_REGULAR: i32 = 0o100644;
const SHIFT_LINES: usize = 20;
const PAD_METHODS: usize = 10;
const CC_BRANCHES: usize = 15;
const BRANCH_LINE_OFFSET: usize = 3;

const LEGACY: &str = "class W {\n    void ok1() {\n        a();\n    }\n    void broken(int a {\n        return;\n    }\n    void ok2() {\n        b();\n    }\n}\n";
const LEGACY_DAMAGE_LINE: usize = 5;
const JSX_LEGACY: &str =
    "export const c = <div>a & b</div>;\nexport function ok() {\n  return 1;\n}\n";

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

fn padding() -> String {
    (0..SHIFT_LINES).map(|k| format!("// pad {k}\n")).collect()
}

fn branching(branch_text: impl Fn(usize) -> String) -> String {
    let branches: String = (0..CC_BRANCHES).map(branch_text).collect();
    format!("class W {{\n    int run(int x) {{\n{branches}        return x;\n    }}\n}}\n")
}

fn clean_branch(index: usize) -> String {
    format!("        if (x > {index}) {{ x++; }}\n")
}

struct Analyzed {
    path: RepoPath,
    source: Vec<u8>,
    analysis: FileAnalysis,
}

fn analyzed(
    snapshot: &CommitSnapshot,
    repo: &Repository,
    wanted: &BTreeSet<RepoPath>,
) -> Vec<Analyzed> {
    let mut out = Vec::new();
    for entry in snapshot.entries.iter().filter(|e| wanted.contains(&e.path)) {
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

type Site = (String, usize, usize);

#[test]
fn test_block_overlapping_a_raised_callable_is_not_raised_again() {
    let base_text = "if (flag) {\n  const c = <div>a & b</div>;\n  function f() {\n    return <i>a & b</i>;\n  }\n  g();\n}\n";
    let edited = "if (flag) {\n  const c = <div>a & b</div>;\n  function f() {\n    h();\n    return <i>a & b</i>;\n  }\n  g();\n}\n";
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited.to_string())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.sites, vec![site("w.tsx", 3, 6)]);
}

#[test]
fn test_nested_excluded_callables_are_each_raised() {
    let base_text = "function outer() {\n  const c = <div>a & b</div>;\n  function inner() {\n    return <i>a & b</i>;\n  }\n  return 1;\n}\n";
    let edited = "function outer() {\n  const c = <div>a & b</div>;\n  function inner() {\n    h();\n    return <i>a & b</i>;\n  }\n  return 1;\n}\n";
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited.to_string())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        evaluation.sites,
        vec![site("w.tsx", 1, 8), site("w.tsx", 3, 6)]
    );
}

/// `evaluate_damage` alone over an added file of `count` broken methods;
/// returns the minimum of three timings and the A101 count.
fn added_damaged_methods_run(count: usize) -> (std::time::Duration, usize) {
    const SAMPLES: usize = 3;
    let text = (0..count).fold(String::from("class W {\n"), |mut text, index| {
        text.push_str(&format!(
            "    void broken{index}(int a {{\n        return;\n    }}\n"
        ));
        text
    }) + "}\n";
    let path = RepoPath::from_bytes(b"W.java".to_vec());
    let analysis = analyze_file(Path::new("W.java"), text.as_bytes()).expect("candidate");
    let candidate_files = [FindingFile {
        path: path.clone(),
        source: text.as_bytes(),
        analysis: &analysis,
    }];
    let changes = [Change::Added {
        path,
        kind: nsd::git::snapshot::EntryKind::Regular,
    }];
    let mut fastest = std::time::Duration::MAX;
    let mut sites = 0;
    for _ in 0..SAMPLES {
        let started = std::time::Instant::now();
        let found = evaluate_damage(&[], &candidate_files, &changes).expect("evaluate");
        fastest = fastest.min(started.elapsed());
        sites = found.len();
    }
    (fastest, sites)
}

#[test]
fn test_overlap_lookups_do_not_scale_quadratically_in_damage_sites() {
    const METHOD_COUNT: usize = 1_000;
    const SCALED_METHOD_COUNT: usize = METHOD_COUNT * 8;
    const SCALING_ASSERT_MULTIPLIER: u32 = 32;

    let (elapsed, sites) = added_damaged_methods_run(METHOD_COUNT);
    let (scaled_elapsed, scaled_sites) = added_damaged_methods_run(SCALED_METHOD_COUNT);

    assert_eq!((sites, scaled_sites), (METHOD_COUNT, SCALED_METHOD_COUNT));
    assert!(
        scaled_elapsed < elapsed * SCALING_ASSERT_MULTIPLIER,
        "a quadratic overlap scan scales ~64x from N to 8N, an indexed lookup ~8x: evaluate_damage \
         at N={METHOD_COUNT} took {elapsed:?}, at 8N={SCALED_METHOD_COUNT} took {scaled_elapsed:?}, \
         expected 8N < {SCALING_ASSERT_MULTIPLIER}x N"
    );
}

struct Evaluation {
    sites: Vec<Site>,
    changes: Vec<Change>,
    candidate_callables: Vec<String>,
}

fn evaluate_with(repo: &Repository, base: Oid, candidate: Oid, reversed: bool) -> Evaluation {
    let mut changes = diff_commit_to_commit(repo, Some(base), candidate).expect("diff commits");
    let (base_paths, candidate_paths) = changed_paths(&changes);
    let base_snapshot = CommitSnapshot::at(repo, base).expect("base snapshot");
    let candidate_snapshot = CommitSnapshot::at(repo, candidate).expect("candidate snapshot");
    let mut base_files = analyzed(&base_snapshot, repo, &base_paths);
    let mut candidate_files = analyzed(&candidate_snapshot, repo, &candidate_paths);
    if reversed {
        base_files.reverse();
        candidate_files.reverse();
        changes.reverse();
    }
    let diagnostics = evaluate_damage(
        &finding_files(&base_files),
        &finding_files(&candidate_files),
        &changes,
    )
    .expect("evaluate damage");
    let sites = diagnostics
        .iter()
        .map(|d| {
            assert_eq!(d.code, CODE_PARSE_DAMAGE);
            (
                d.candidate_path.render(),
                d.candidate_start_line,
                d.candidate_end_line,
            )
        })
        .collect();
    let candidate_callables = candidate_files
        .iter()
        .flat_map(|file| file.analysis.callables.iter())
        .map(|callable| callable.metrics.name.clone())
        .collect();
    Evaluation {
        sites,
        changes,
        candidate_callables,
    }
}

fn evaluate(repo: &Repository, base: Oid, candidate: Oid) -> Evaluation {
    evaluate_with(repo, base, candidate, false)
}

fn site(path: &str, start: usize, end: usize) -> Site {
    (path.to_string(), start, end)
}

// ---------------------------------------------------------------------
// Scenarios.
// ---------------------------------------------------------------------

#[test]
fn test_changed_line_in_a_salvage_dropped_callable_raises_a101() {
    let broken_line = BRANCH_LINE_OFFSET + 7;
    let base_text = branching(clean_branch);
    let candidate_text = branching(|index| {
        if index == 7 {
            "        if (x > 7 { x++; }\n".to_string()
        } else {
            clean_branch(index)
        }
    });
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("W.java", base_text)]);
    let candidate = commit(&repo, &[("W.java", candidate_text)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert!(
        evaluation.candidate_callables.is_empty(),
        "salvage drops `run`, so the diff alone reads as a deletion: {:?}",
        evaluation.candidate_callables
    );
    assert_eq!(
        evaluation.sites,
        vec![site("W.java", broken_line, broken_line)]
    );
}

#[test]
fn test_legacy_damage_moved_through_unchanged_lines_is_tolerated() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("W.java", LEGACY.to_string())]);
    let candidate = commit(&repo, &[("W.java", format!("{}{LEGACY}", padding()))]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.changes.len(), 1);
    assert!(evaluation.sites.is_empty(), "{:?}", evaluation.sites);
}

#[test]
fn test_edit_elsewhere_in_a_damaged_file_is_tolerated() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("W.java", LEGACY.to_string())]);
    let edited = LEGACY.replace("a();", "a();\n        z();");
    let candidate = commit(&repo, &[("W.java", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.changes.len(), 1);
    assert!(evaluation.sites.is_empty(), "{:?}", evaluation.sites);
}

#[test]
fn test_edit_inside_a_legacy_damaged_callable_raises_a101() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("W.java", LEGACY.to_string())]);
    let edited = LEGACY.replace("return;", "return;\n        x();");
    let candidate = commit(&repo, &[("W.java", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        evaluation.sites,
        vec![site("W.java", LEGACY_DAMAGE_LINE, LEGACY_DAMAGE_LINE + 3)]
    );
}

#[test]
fn test_new_damage_on_unchanged_lines_raises_a101() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("W.java", LEGACY.to_string())]);
    let edited = LEGACY.replace("        b();\n    }\n", "        b();\n");
    let candidate = commit(&repo, &[("W.java", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.sites.len(), 1, "{:?}", evaluation.sites);
    assert_ne!(
        evaluation.sites[0].1, LEGACY_DAMAGE_LINE,
        "the legacy span is tolerated; the raise is the new one"
    );
    assert_eq!(evaluation.sites, vec![site("W.java", 10, 10)]);
}

#[test]
fn test_worsened_damage_raises_a101() {
    let base_text = "export const c = <div>\n  a & b\n</div>;\nexport const d = 1;\n";
    let grown_text = "export const c = <div>\n  a &\n  b\n</div>;\nexport const d = 1;\n";
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", grown_text.to_string())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.sites, vec![site("w.tsx", 2, 3)]);
}

#[test]
fn test_damage_on_a_line_with_no_base_mapping_raises_a101() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("W.java", LEGACY.to_string())]);
    let edited = LEGACY.replace("return;", "return;\n        x(;");
    let candidate = commit(&repo, &[("W.java", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        evaluation.sites,
        vec![site(
            "W.java",
            LEGACY_DAMAGE_LINE + 2,
            LEGACY_DAMAGE_LINE + 2
        )]
    );
}

#[test]
fn test_added_file_with_damage_raises_a101() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("Keep.java", "class Keep {}\n".to_string())]);
    let candidate = commit(
        &repo,
        &[
            ("Keep.java", "class Keep {}\n".to_string()),
            ("W.java", LEGACY.to_string()),
        ],
    );

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        evaluation.sites,
        vec![site("W.java", LEGACY_DAMAGE_LINE, LEGACY_DAMAGE_LINE)]
    );
}

#[test]
fn test_renamed_file_maps_its_legacy_damage() {
    let pads: String = (0..PAD_METHODS)
        .map(|k| format!("    void pad{k}() {{\n        work();\n    }}\n"))
        .collect();
    let text = format!("class W {{\n{pads}    void broken(int a {{\n        return;\n    }}\n}}\n");
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("Old.java", text.clone())]);
    let candidate = commit(&repo, &[("New.java", text)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert!(
        matches!(evaluation.changes.as_slice(), [Change::Renamed { .. }]),
        "{:?}",
        evaluation.changes
    );
    assert!(evaluation.sites.is_empty(), "{:?}", evaluation.sites);
}

#[test]
fn test_removed_damage_raises_nothing() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("W.java", LEGACY.to_string())]);
    let candidate = commit(&repo, &[("W.java", LEGACY.replace("int a {", "int a) {"))]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.changes.len(), 1);
    assert!(evaluation.sites.is_empty(), "{:?}", evaluation.sites);
}

#[test]
fn test_jsts_legacy_damage_is_tolerated_and_new_damage_raises_a101() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", JSX_LEGACY.to_string())]);
    let moved = commit(&repo, &[("w.tsx", format!("{}{JSX_LEGACY}", padding()))]);
    let evaluation = evaluate(&repo, base, moved);
    assert_eq!(evaluation.changes.len(), 1);
    assert!(evaluation.sites.is_empty(), "{:?}", evaluation.sites);

    let broken = format!("{JSX_LEGACY}export function bad() {{\n  foo(;\n}}\n");
    let candidate = commit(&repo, &[("w.tsx", broken)]);
    let evaluation = evaluate(&repo, base, candidate);
    assert_eq!(evaluation.sites.len(), 1, "{:?}", evaluation.sites);
    assert_eq!(evaluation.sites[0].0, "w.tsx");
}

#[test]
fn test_output_is_sorted_and_deterministic() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("Keep.java", "class Keep {}\n".to_string())]);
    let candidate = commit(
        &repo,
        &[
            ("Keep.java", "class Keep {}\n".to_string()),
            ("m.java", LEGACY.to_string()),
            ("b/Z.java", LEGACY.to_string()),
            ("A.java", LEGACY.to_string()),
        ],
    );

    let forward = evaluate_with(&repo, base, candidate, false);
    let reversed = evaluate_with(&repo, base, candidate, true);

    let paths: Vec<&str> = forward.sites.iter().map(|s| s.0.as_str()).collect();
    assert_eq!(paths, vec!["A.java", "b/Z.java", "m.java"]);
    assert_eq!(forward.sites, reversed.sites);
}

#[test]
fn test_measured_callable_swallowed_by_legacy_damage_raises_a101() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("W.java", LEGACY.to_string())]);
    let edited = LEGACY.replace("        a();\n    }\n", "        a();\n");
    let candidate = commit(&repo, &[("W.java", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.sites, vec![site("W.java", 2, 6)]);
}

#[test]
fn test_damage_growing_from_an_unchanged_start_line_raises_a101() {
    let base_text = "export const c = <div>a &\n</div>;\nexport const d = 1;\n";
    let grown_text = "export const c = <div>a &\nb\n</div>;\nexport const d = 1;\n";
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", grown_text.to_string())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.sites, vec![site("w.tsx", 1, 2)]);
}

#[test]
fn test_callable_sharing_a_line_with_legacy_damage_is_not_swallowed_by_a_shift() {
    let text = "export const c = () => <div>a & b</div>; export const d = (x: number) => { if (x) { return 1; } return 2; };\nexport function ok() {\n  return 1;\n}\n";
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", format!("{}{text}", padding()))]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.changes.len(), 1);
    assert!(evaluation.sites.is_empty(), "{:?}", evaluation.sites);
}

/// Ledger row 105: the extra `(1,1)` line-granularity site. Legacy `c`
/// shares line 1 with `d`'s header; when `d` becomes damaged at line 3,
/// line-granular tolerance also names the unchanged `c` on line 1, next to
/// the real `(3,3)` damage site.
#[test]
fn test_line_sharing_legacy_callable_is_named_when_its_neighbour_becomes_damaged() {
    let base_text = "export const c = () => <div>a & b</div>; export function d(x: number) {\n  if (x) { return 1; }\n  return 2;\n}\nexport function ok() {\n  return 1;\n}\n";
    let damaged = base_text.replace("  return 2;\n}\n", "  return 2 +; }\n");
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", damaged)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        evaluation.sites,
        vec![site("w.tsx", 1, 1), site("w.tsx", 3, 3)]
    );
}

#[test]
fn test_deleted_line_inside_a_legacy_damaged_callable_raises_a101() {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[(
            "W.java",
            LEGACY.replace("        return;\n", "        w();\n        return;\n"),
        )],
    );
    let candidate = commit(&repo, &[("W.java", LEGACY.to_string())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        evaluation.sites,
        vec![site("W.java", LEGACY_DAMAGE_LINE, LEGACY_DAMAGE_LINE + 2)]
    );
}

#[test]
fn test_edit_inside_a_legacy_damaged_top_level_block_raises_a101() {
    let base_text = "if (flag) {\n  const c = <div>a & b</div>;\n  g();\n}\n";
    let edited =
        "if (flag) {\n  const c = <div>a & b</div>;\n  try { h(); } catch (e) {}\n  g();\n}\n";
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited.to_string())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.sites, vec![site("w.tsx", 1, 5)]);
}

#[test]
fn test_end_line_edit_of_a_line_sharing_clean_callable_is_not_swallowed() {
    let base_text = "export const c = () => <div>a & b</div>; export const d = (x: number) => {\n  if (x) { return 1; }\n  return 2;\n};\nexport function ok() {\n  return 1;\n}\n";
    let edited = base_text.replace("  return 2;\n};\n", "  return 2;\n}; // edited\n");
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.changes.len(), 1);
    assert!(evaluation.sites.is_empty(), "{:?}", evaluation.sites);
}

#[test]
fn test_start_line_edit_of_a_line_sharing_clean_callable_is_not_swallowed() {
    let base_text = "export function d(x: number) {\n  if (x) { return 1; }\n  return 2;\n} export const c = () => <div>a & b</div>;\nexport function ok() {\n  return 1;\n}\n";
    let edited = base_text.replace("d(x: number)", "d(y: number)");
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.changes.len(), 1);
    assert!(evaluation.sites.is_empty(), "{:?}", evaluation.sites);
}

#[test]
fn test_deleted_line_inside_a_legacy_damaged_top_level_block_raises_a101() {
    let base_text = "if (flag) {\n  const c = <div>a & b</div>;\n  w();\n  g();\n}\n";
    let edited = "if (flag) {\n  const c = <div>a & b</div>;\n  g();\n}\n";
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited.to_string())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.sites, vec![site("w.tsx", 1, 4)]);
}

#[test]
fn test_header_deleted_before_a_line_sharing_legacy_damage_raises_a101() {
    let base_text = "export function d(x: number) {\n  if (x) { return 1; }\n  return 2;\n} export const c = () => <div>a & b</div>;\nexport function ok() {\n  return 1;\n}\n";
    let edited = base_text.replacen("export function d(x: number) {\n", "", 1);
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.sites, vec![site("w.tsx", 3, 3)]);
}

#[test]
fn test_callable_matching_only_the_start_image_is_swallowed() {
    let base_text = "export function e() { return 1; } export function d(x: number) {\n  if (x) { return 1; }\n  return 2;\n} export const c = () => <div>a & b</div>;\nexport function ok() {\n  return 1;\n}\n";
    let edited = base_text.replace("  return 2;\n", "  const z = <p>q & r</p>;\n  return 2;\n");
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        evaluation.sites,
        vec![site("w.tsx", 3, 3), site("w.tsx", 5, 5)]
    );
}

#[test]
fn test_callable_matching_only_the_end_image_is_swallowed() {
    let base_text = "export const c = () => <div>a & b</div>; export function d(x: number) {\n  if (x) { return 1; }\n  return 2;\n} export function e() { return 1; }\nexport function ok() {\n  return 1;\n}\n";
    let edited = base_text.replace("  return 2;\n", "  const z = <p>q & r</p>;\n  return 2;\n");
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("w.tsx", base_text.to_string())]);
    let candidate = commit(&repo, &[("w.tsx", edited)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        evaluation.sites,
        vec![site("w.tsx", 1, 1), site("w.tsx", 3, 3)]
    );
}

/// `evaluate_damage` alone (analysis excluded) over a legacy JSX error plus
/// `count` unchanged functions and one prepended comment line; returns the
/// minimum of three timings and the A101 count.
fn shifted_callables_damage_run(count: usize) -> (std::time::Duration, usize) {
    const SAMPLES: usize = 3;
    let base_text = (0..count).fold(
        String::from("export const c = <div>a & b</div>;\n"),
        |mut text, index| {
            text.push_str(&format!("export function f{index}() {{ return 1; }}\n"));
            text
        },
    );
    let candidate_text = format!("// pad\n{base_text}");
    let path = RepoPath::from_bytes(b"x.tsx".to_vec());
    let base_analysis = analyze_file(Path::new("x.tsx"), base_text.as_bytes()).expect("base");
    let candidate_analysis =
        analyze_file(Path::new("x.tsx"), candidate_text.as_bytes()).expect("candidate");
    let base_files = [FindingFile {
        path: path.clone(),
        source: base_text.as_bytes(),
        analysis: &base_analysis,
    }];
    let candidate_files = [FindingFile {
        path: path.clone(),
        source: candidate_text.as_bytes(),
        analysis: &candidate_analysis,
    }];
    let changes = [Change::Modified {
        path,
        kind: nsd::git::snapshot::EntryKind::Regular,
    }];
    let mut fastest = std::time::Duration::MAX;
    let mut sites = 0;
    for _ in 0..SAMPLES {
        let started = std::time::Instant::now();
        let found = evaluate_damage(&base_files, &candidate_files, &changes).expect("evaluate");
        fastest = fastest.min(started.elapsed());
        sites = found.len();
    }
    (fastest, sites)
}

#[test]
fn test_still_measured_lookup_does_not_scale_quadratically() {
    const CALLABLE_COUNT: usize = 1_000;
    const SCALED_CALLABLE_COUNT: usize = CALLABLE_COUNT * 8;
    const SCALING_ASSERT_MULTIPLIER: u32 = 32;

    let (elapsed, sites) = shifted_callables_damage_run(CALLABLE_COUNT);
    let (scaled_elapsed, scaled_sites) = shifted_callables_damage_run(SCALED_CALLABLE_COUNT);

    assert_eq!((sites, scaled_sites), (0, 0));
    assert!(
        scaled_elapsed < elapsed * SCALING_ASSERT_MULTIPLIER,
        "a quadratic scan scales ~64x from N to 8N, a sub-linear lookup ~8x: evaluate_damage at \
         N={CALLABLE_COUNT} took {elapsed:?}, at 8N={SCALED_CALLABLE_COUNT} took {scaled_elapsed:?}, \
         expected 8N < {SCALING_ASSERT_MULTIPLIER}x N"
    );
}
