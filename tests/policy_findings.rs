//! Integration tests for M3-2's diff-aware finding matching and V101
//! (`nsd-plan-final.md` *Diagnostics* and *Stable data model*). Every
//! scenario commits real sources to a scratch repository and goes through
//! the real diff, analysis and callable matcher.

mod common;

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use git2::{Oid, Repository};
use nsd::analysis::{analyze_file, FileAnalysis};
use nsd::config::{Config, PolicyConfig, Severity};
use nsd::git::diff::{diff_commit_to_commit, Change};
use nsd::git::path::RepoPath;
use nsd::git::snapshot::CommitSnapshot;
use nsd::identity::matching::{match_callables, FileCallables, MatchOutput};
use nsd::policy::diagnostics::CODE_UNMATCHED_FINDING;
use nsd::policy::findings::{match_findings, FindingFile, FindingMatchOutput};
use nsd::rules;

const MODE_REGULAR: i32 = 0o100644;
const BOUND: Duration = Duration::from_secs(30);
const LARGE_GROUP: usize = 50_000;
const SHIFT_LINES: usize = 20;
const PAD_METHODS: usize = 10;
const CATCH: &str = "try { work(); } catch (Exception e) { }";

// ---------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------

fn path(text: &str) -> RepoPath {
    RepoPath::from_bytes(text.as_bytes().to_vec())
}

fn method(name: &str, body: &[&str]) -> String {
    let lines: String = body
        .iter()
        .map(|line| format!("        {line}\n"))
        .collect();
    format!("    void {name}() {{\n{lines}    }}\n")
}

fn class(name: &str, methods: &[String]) -> String {
    format!("class {name} {{\n{}}}\n", methods.concat())
}

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

fn catch_lines(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("catch"))
        .map(|(index, _)| index + 1)
        .collect()
}

fn default_policy() -> PolicyConfig {
    Config::default().policy
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

struct Evaluation {
    output: FindingMatchOutput,
    matched: MatchOutput,
    changes: Vec<Change>,
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

fn callables_of(files: &[Analyzed]) -> Vec<FileCallables> {
    files
        .iter()
        .map(|file| FileCallables {
            path: file.path.clone(),
            callables: file
                .analysis
                .callables
                .iter()
                .map(|c| (c.identity.clone(), c.body_fingerprint.clone()))
                .collect(),
        })
        .collect()
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

fn evaluate_with(
    repo: &Repository,
    base: Oid,
    candidate: Oid,
    policy: &PolicyConfig,
    reversed: bool,
) -> Evaluation {
    let changes = diff_commit_to_commit(repo, Some(base), candidate).expect("diff commits");
    let (base_paths, candidate_paths) = changed_paths(&changes);
    let base_snapshot = CommitSnapshot::at(repo, base).expect("base snapshot");
    let candidate_snapshot = CommitSnapshot::at(repo, candidate).expect("candidate snapshot");
    let mut base_files = analyzed(&base_snapshot, repo, &base_paths);
    let mut candidate_files = analyzed(&candidate_snapshot, repo, &candidate_paths);
    if reversed {
        base_files.reverse();
        candidate_files.reverse();
    }
    let matched = match_callables(
        &callables_of(&base_files),
        &callables_of(&candidate_files),
        &changes,
    );
    let output = match_findings(
        &finding_files(&base_files),
        &finding_files(&candidate_files),
        &matched,
        &changes,
        policy,
    )
    .expect("match findings");
    Evaluation {
        output,
        matched,
        changes,
    }
}

fn evaluate(repo: &Repository, base: Oid, candidate: Oid) -> Evaluation {
    evaluate_with(repo, base, candidate, &default_policy(), false)
}

fn v101_sites(evaluation: &Evaluation) -> Vec<(&'static str, String, usize)> {
    evaluation
        .output
        .diagnostics
        .iter()
        .map(|d| {
            assert_eq!(d.code, CODE_UNMATCHED_FINDING);
            (d.rule_id, d.candidate_path.render(), d.candidate_start_line)
        })
        .collect()
}

// ---------------------------------------------------------------------
// Scenarios.
// ---------------------------------------------------------------------

#[test]
fn test_scratch_repo_new_empty_catch_raises_v101() {
    let (_dir, repo) = common::init_repo();
    let base_text = class("Widget", &[method("run", &["work();"])]);
    let candidate_text = class("Widget", &[method("run", &["work();", CATCH])]);
    let base = commit(&repo, &[("Widget.java", base_text)]);
    let candidate = commit(&repo, &[("Widget.java", candidate_text.clone())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        v101_sites(&evaluation),
        vec![(
            rules::JAVA_EMPTY_CATCH,
            "Widget.java".to_string(),
            catch_lines(&candidate_text)[0]
        )]
    );
    assert_eq!(evaluation.output.unmatched_candidates.len(), 1);
    assert!(evaluation.output.pairs.is_empty());

    let added = commit(
        &repo,
        &[
            ("Widget.java", candidate_text.clone()),
            (
                "Extra.java",
                class("Extra", &[method("go", &["work();", CATCH])]),
            ),
        ],
    );
    let evaluation = evaluate(&repo, candidate, added);
    assert_eq!(
        v101_sites(&evaluation),
        vec![(
            rules::JAVA_EMPTY_CATCH,
            "Extra.java".to_string(),
            catch_lines(&class("Extra", &[method("go", &["work();", CATCH])]))[0]
        )]
    );
}

#[test]
fn test_line_only_move_is_not_a_regression() {
    let (_dir, repo) = common::init_repo();
    let text = class("Widget", &[method("run", &["work();", CATCH])]);
    let pad: String = (0..SHIFT_LINES).map(|k| format!("// pad {k}\n")).collect();
    let base = commit(&repo, &[("Widget.java", text.clone())]);
    let candidate = commit(&repo, &[("Widget.java", format!("{pad}{text}"))]);

    let evaluation = evaluate(&repo, base, candidate);

    assert!(evaluation.output.diagnostics.is_empty());
    assert!(evaluation.output.unmatched_candidates.is_empty());
    assert_eq!(evaluation.output.pairs.len(), 1);
}

#[test]
fn test_renamed_file_keeps_its_findings_matched() {
    let pads: Vec<String> = (0..PAD_METHODS)
        .map(|k| method(&format!("pad{k}"), &["work();"]))
        .collect();
    let with = |run: String| {
        let mut methods = pads.clone();
        methods.push(run);
        class("Widget", &methods)
    };
    let in_method = method("run", &["work();", CATCH]);

    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("Old.java", with(in_method.clone()))]);
    let candidate = commit(&repo, &[("New.java", with(in_method.clone()))]);
    let evaluation = evaluate(&repo, base, candidate);
    assert!(
        matches!(evaluation.changes.as_slice(), [Change::Renamed { .. }]),
        "{:?}",
        evaluation.changes
    );
    assert!(evaluation.output.diagnostics.is_empty());
    assert_eq!(evaluation.output.pairs.len(), 1);

    // The callable is renamed and edited too, so both findings fall to file
    // context; the renamed path must still map base to candidate.
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("Old.java", with(in_method))]);
    let edited = method("execute", &["work();", "extra();", CATCH]);
    let candidate = commit(&repo, &[("New.java", with(edited))]);
    let evaluation = evaluate(&repo, base, candidate);
    assert!(
        matches!(evaluation.changes.as_slice(), [Change::Renamed { .. }]),
        "{:?}",
        evaluation.changes
    );
    assert!(evaluation.output.diagnostics.is_empty());
    assert_eq!(evaluation.output.pairs.len(), 1);
}

#[test]
fn test_inserted_duplicate_is_reported_at_its_own_line() {
    let (_dir, repo) = common::init_repo();
    let base_text = class("Widget", &[method("run", &["a();", CATCH, "b();", CATCH])]);
    let candidate_text = class(
        "Widget",
        &[method(
            "run",
            &["z();", CATCH, "a();", CATCH, "b();", CATCH],
        )],
    );
    let base = commit(&repo, &[("Widget.java", base_text)]);
    let candidate = commit(&repo, &[("Widget.java", candidate_text.clone())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        v101_sites(&evaluation),
        vec![(
            rules::JAVA_EMPTY_CATCH,
            "Widget.java".to_string(),
            catch_lines(&candidate_text)[0]
        )]
    );
    assert_eq!(evaluation.output.pairs.len(), 2);
}

#[test]
fn test_removed_duplicate_raises_nothing() {
    let (_dir, repo) = common::init_repo();
    let base_text = class(
        "Widget",
        &[method(
            "run",
            &["a();", CATCH, "b();", CATCH, "c();", CATCH],
        )],
    );
    let candidate_text = class("Widget", &[method("run", &["a();", CATCH, "c();", CATCH])]);
    let base = commit(&repo, &[("Widget.java", base_text)]);
    let candidate = commit(&repo, &[("Widget.java", candidate_text)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert!(evaluation.output.diagnostics.is_empty());
    assert_eq!(evaluation.output.pairs.len(), 2);
}

#[test]
fn test_reformatted_finding_is_not_new() {
    let (_dir, repo) = common::init_repo();
    let reformatted = "try { work(); } catch /* why */ (Exception   e) {\n        }";
    let base = commit(
        &repo,
        &[(
            "Widget.java",
            class("Widget", &[method("run", &["work();", CATCH])]),
        )],
    );
    let candidate = commit(
        &repo,
        &[(
            "Widget.java",
            class("Widget", &[method("run", &["work();", reformatted])]),
        )],
    );

    let evaluation = evaluate(&repo, base, candidate);

    assert!(evaluation.output.diagnostics.is_empty());
    assert_eq!(evaluation.output.pairs.len(), 1);
}

#[test]
fn test_finding_moved_into_another_callable_is_new() {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[(
            "Widget.java",
            class(
                "Widget",
                &[
                    method("first", &["a();", CATCH]),
                    method("second", &["b();"]),
                ],
            ),
        )],
    );
    let candidate_text = class(
        "Widget",
        &[
            method("first", &["a();"]),
            method("second", &["b();", CATCH]),
        ],
    );
    let candidate = commit(&repo, &[("Widget.java", candidate_text.clone())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        v101_sites(&evaluation),
        vec![(
            rules::JAVA_EMPTY_CATCH,
            "Widget.java".to_string(),
            catch_lines(&candidate_text)[0]
        )]
    );
}

#[test]
fn test_finding_in_an_unmatched_callable_uses_file_context() {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[(
            "Widget.java",
            class("Widget", &[method("run", &["work();", CATCH])]),
        )],
    );
    let candidate = commit(
        &repo,
        &[(
            "Widget.java",
            class(
                "Widget",
                &[method("execute", &["work();", "extra();", CATCH])],
            ),
        )],
    );

    let evaluation = evaluate(&repo, base, candidate);

    assert!(
        evaluation.matched.matches.is_empty(),
        "{:?}",
        evaluation.matched
    );
    assert!(evaluation.output.diagnostics.is_empty());
    assert_eq!(evaluation.output.pairs.len(), 1);
}

#[test]
fn test_each_of_the_six_rules_raises_v101_when_new() {
    let scenarios: [(&str, &str, &str, &str); 6] = [
        (
            rules::JAVA_UNREACHABLE_AFTER_RETURN,
            "A.java",
            "class A {\n    void m() {\n        work();\n        return;\n    }\n}\n",
            "class A {\n    void m() {\n        work();\n        return;\n        work();\n    }\n}\n",
        ),
        (
            rules::JAVA_EMPTY_CATCH,
            "A.java",
            "class A {\n    void m() {\n        work();\n    }\n}\n",
            "class A {\n    void m() {\n        try { work(); } catch (Exception e) { }\n    }\n}\n",
        ),
        (
            rules::JAVA_REDUNDANT_ELSE_AFTER_RETURN,
            "A.java",
            "class A {\n    int m(int x) {\n        if (x > 0) {\n            return 1;\n        }\n        return 0;\n    }\n}\n",
            "class A {\n    int m(int x) {\n        if (x > 0) {\n            return 1;\n        } else {\n            return 0;\n        }\n    }\n}\n",
        ),
        (
            rules::JSTS_UNREACHABLE_AFTER_RETURN,
            "a.js",
            "function m() {\n    work();\n    return;\n}\n",
            "function m() {\n    work();\n    return;\n    work();\n}\n",
        ),
        (
            rules::JSTS_EMPTY_CATCH,
            "a.js",
            "function m() {\n    work();\n}\n",
            "function m() {\n    try {\n        work();\n    } catch (e) {\n    }\n}\n",
        ),
        (
            rules::JSTS_REDUNDANT_ELSE_AFTER_RETURN,
            "a.js",
            "function m(x) {\n    if (x > 0) {\n        return 1;\n    }\n    return 0;\n}\n",
            "function m(x) {\n    if (x > 0) {\n        return 1;\n    } else {\n        return 0;\n    }\n}\n",
        ),
    ];
    assert_eq!(scenarios.len(), rules::ALL_RULE_IDS.len());
    for (rule_id, file, base_text, candidate_text) in scenarios {
        let (_dir, repo) = common::init_repo();
        let base = commit(&repo, &[(file, base_text.to_string())]);
        let candidate = commit(&repo, &[(file, candidate_text.to_string())]);

        let evaluation = evaluate(&repo, base, candidate);

        let sites = v101_sites(&evaluation);
        assert_eq!(sites.len(), 1, "{rule_id}: {sites:?}");
        assert_eq!(sites[0].0, rule_id);
        assert_eq!(sites[0].1, file);
    }
}

#[test]
fn test_off_disables_v101() {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[(
            "Widget.java",
            class("Widget", &[method("run", &["work();"])]),
        )],
    );
    let candidate = commit(
        &repo,
        &[(
            "Widget.java",
            class("Widget", &[method("run", &["work();", CATCH])]),
        )],
    );

    let mut policy = default_policy();
    policy.nsd_v101 = Severity::Off;
    let off = evaluate_with(&repo, base, candidate, &policy, false);
    assert!(off.output.diagnostics.is_empty());
    assert_eq!(off.output.unmatched_candidates.len(), 1);

    policy.nsd_v101 = Severity::Warn;
    let warn = evaluate_with(&repo, base, candidate, &policy, false);
    assert_eq!(warn.output.diagnostics.len(), 1);
}

#[test]
fn test_matching_is_deterministic_under_input_order() {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            (
                "A.java",
                class("A", &[method("run", &["a();", CATCH, "b();", CATCH])]),
            ),
            ("B.java", class("B", &[method("run", &["work();", CATCH])])),
            ("C.java", class("C", &[method("run", &["work();"])])),
        ],
    );
    let candidate = commit(
        &repo,
        &[
            (
                "A.java",
                class(
                    "A",
                    &[method(
                        "run",
                        &["z();", CATCH, "a();", CATCH, "b();", CATCH],
                    )],
                ),
            ),
            (
                "B.java",
                class("B", &[method("run", &["pad();", "work();", CATCH])]),
            ),
            ("C.java", class("C", &[method("run", &["work();", CATCH])])),
        ],
    );

    let forward = evaluate_with(&repo, base, candidate, &default_policy(), false);
    let reversed = evaluate_with(&repo, base, candidate, &default_policy(), true);

    assert_eq!(forward.output.diagnostics.len(), 2);
    assert_eq!(forward.output.pairs.len(), 3);
    assert_eq!(forward.output, reversed.output);
}

#[test]
fn test_many_identical_findings_match_in_bounded_time() {
    let body = "try{}catch(E e){}\n".repeat(LARGE_GROUP);
    let text = format!("class Big {{\n    void run() {{\n{body}    }}\n}}\n");
    let shifted = format!("// shifted\n{text}");
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("Big.java", text)]);
    let candidate = commit(&repo, &[("Big.java", shifted)]);

    let changes = diff_commit_to_commit(&repo, Some(base), candidate).expect("diff commits");
    let (base_paths, candidate_paths) = changed_paths(&changes);
    let base_files = analyzed(
        &CommitSnapshot::at(&repo, base).expect("base snapshot"),
        &repo,
        &base_paths,
    );
    let candidate_files = analyzed(
        &CommitSnapshot::at(&repo, candidate).expect("candidate snapshot"),
        &repo,
        &candidate_paths,
    );
    let matched = match_callables(
        &callables_of(&base_files),
        &callables_of(&candidate_files),
        &changes,
    );

    let started = Instant::now();
    let output = match_findings(
        &finding_files(&base_files),
        &finding_files(&candidate_files),
        &matched,
        &changes,
        &default_policy(),
    )
    .expect("match findings");
    let elapsed = started.elapsed();

    assert!(elapsed < BOUND, "matching took {elapsed:?}");
    assert_eq!(output.pairs.len(), LARGE_GROUP);
    assert!(output.diagnostics.is_empty());
}

#[test]
fn test_pairs_name_the_base_and_candidate_file_across_a_rename() {
    let pads: Vec<String> = (0..PAD_METHODS)
        .map(|k| method(&format!("pad{k}"), &["work();"]))
        .collect();
    let mut methods = pads;
    methods.push(method("run", &["work();", CATCH]));
    let text = class("Widget", &methods);
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("Old.java", text.clone())]);
    let candidate = commit(&repo, &[("New.java", text)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.output.pairs.len(), 1);
    let pair = &evaluation.output.pairs[0];
    assert_eq!(pair.base.path, path("Old.java"));
    assert_eq!(pair.candidate.path, path("New.java"));
    assert_eq!(pair.base.index, pair.candidate.index);
}

#[test]
fn test_inserted_duplicate_in_a_renamed_file_is_reported_at_its_own_line() {
    let pads: Vec<String> = (0..PAD_METHODS)
        .map(|k| method(&format!("pad{k}"), &["work();"]))
        .collect();
    let with = |run: String| {
        let mut methods = pads.clone();
        methods.push(run);
        class("Widget", &methods)
    };
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[(
            "Old.java",
            with(method("run", &["a();", CATCH, "b();", CATCH])),
        )],
    );
    let candidate_text = with(method(
        "run",
        &["z();", CATCH, "a();", CATCH, "b();", CATCH],
    ));
    let candidate = commit(&repo, &[("New.java", candidate_text.clone())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert!(
        matches!(evaluation.changes.as_slice(), [Change::Renamed { .. }]),
        "{:?}",
        evaluation.changes
    );
    let first_catch = *catch_lines(&candidate_text).first().expect("a catch line");
    assert_eq!(
        v101_sites(&evaluation),
        vec![(rules::JAVA_EMPTY_CATCH, "New.java".to_string(), first_catch)]
    );
}

#[test]
fn test_output_is_sorted_by_candidate_path() {
    const FILES: usize = 12;
    let names: Vec<String> = (0..FILES).map(|k| format!("F{k:02}.java")).collect();
    let (_dir, repo) = common::init_repo();
    let base_files: Vec<(&str, String)> = names
        .iter()
        .map(|name| {
            (
                name.as_str(),
                class("C", &[method("run", &["work();", CATCH])]),
            )
        })
        .collect();
    let candidate_files: Vec<(&str, String)> = names
        .iter()
        .map(|name| {
            (
                name.as_str(),
                class("C", &[method("run", &["work();", CATCH, "more();", CATCH])]),
            )
        })
        .collect();
    let base = commit(&repo, &base_files);
    let candidate = commit(&repo, &candidate_files);

    let evaluation = evaluate(&repo, base, candidate);

    let diagnosed: Vec<String> = v101_sites(&evaluation)
        .into_iter()
        .map(|(_, file, _)| file)
        .collect();
    assert_eq!(diagnosed, names);
    let paired: Vec<&RepoPath> = evaluation
        .output
        .pairs
        .iter()
        .map(|pair| &pair.candidate.path)
        .collect();
    let expected: Vec<RepoPath> = names.iter().map(|name| path(name)).collect();
    assert_eq!(paired, expected.iter().collect::<Vec<_>>());
    let unmatched: Vec<&RepoPath> = evaluation
        .output
        .unmatched_candidates
        .iter()
        .map(|found| &found.path)
        .collect();
    assert_eq!(unmatched, expected.iter().collect::<Vec<_>>());
}

#[test]
fn test_rule_id_separates_findings_with_equal_syntax() {
    let (_dir, repo) = common::init_repo();
    let base_text = "class A {\n    int m(boolean x) {\n        if (x) {\n            return 1;\n        } else {\n            return 2;\n        }\n    }\n}\n".to_string();
    let candidate_text = "class A {\n    int m(boolean x) {\n        return 1;\n        {\n            return 2;\n        }\n    }\n}\n".to_string();
    let base = commit(&repo, &[("A.java", base_text)]);
    let candidate = commit(&repo, &[("A.java", candidate_text)]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(
        v101_sites(&evaluation),
        vec![(
            rules::JAVA_UNREACHABLE_AFTER_RETURN,
            "A.java".to_string(),
            4
        )]
    );
    assert!(evaluation.output.pairs.is_empty());
}

#[test]
fn test_replaced_finding_with_different_syntax_is_new() {
    let (_dir, repo) = common::init_repo();
    let base_text = class("Widget", &[method("run", &["work();", CATCH])]);
    let candidate_text = class(
        "Widget",
        &[method(
            "run",
            &["work();", "try { work(); } catch (Throwable t) { }"],
        )],
    );
    let base = commit(&repo, &[("Widget.java", base_text)]);
    let candidate = commit(&repo, &[("Widget.java", candidate_text.clone())]);

    let evaluation = evaluate(&repo, base, candidate);

    assert_eq!(evaluation.matched.matches.len(), 1);
    assert_eq!(
        v101_sites(&evaluation),
        vec![(
            rules::JAVA_EMPTY_CATCH,
            "Widget.java".to_string(),
            catch_lines(&candidate_text)[0]
        )]
    );
    assert!(evaluation.output.pairs.is_empty());
}

#[test]
fn test_many_identical_findings_without_line_mappings_match_in_bounded_time() {
    let line = "try{}catch(E e){}\n";
    let wrap = |body: &str| format!("class Big {{\n    void run() {{\n{body}    }}\n}}\n");
    let text = wrap(&line.repeat(LARGE_GROUP));
    let reindented = wrap(&format!("  {line}").repeat(LARGE_GROUP));
    let mapped = nsd::git::diff::map_lines(text.as_bytes(), reindented.as_bytes())
        .expect("map lines")
        .base_to_candidate;
    let first_body_line = 3;
    assert!(
        (first_body_line..first_body_line + LARGE_GROUP).all(|l| !mapped.contains_key(&l)),
        "a catch line still maps"
    );
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("Big.java", text)]);
    let candidate = commit(&repo, &[("Big.java", reindented)]);

    let changes = diff_commit_to_commit(&repo, Some(base), candidate).expect("diff commits");
    let (base_paths, candidate_paths) = changed_paths(&changes);
    let base_files = analyzed(
        &CommitSnapshot::at(&repo, base).expect("base snapshot"),
        &repo,
        &base_paths,
    );
    let candidate_files = analyzed(
        &CommitSnapshot::at(&repo, candidate).expect("candidate snapshot"),
        &repo,
        &candidate_paths,
    );
    let matched = match_callables(
        &callables_of(&base_files),
        &callables_of(&candidate_files),
        &changes,
    );

    let started = Instant::now();
    let output = match_findings(
        &finding_files(&base_files),
        &finding_files(&candidate_files),
        &matched,
        &changes,
        &default_policy(),
    )
    .expect("match findings");
    let elapsed = started.elapsed();

    assert!(elapsed < BOUND, "matching took {elapsed:?}");
    assert_eq!(output.pairs.len(), LARGE_GROUP);
    assert!(output.diagnostics.is_empty());
}
