//! Integration tests for M3-3's S101/S102 suppressions (`nsd-plan-final.md`
//! *Diagnostics*, *Stable data model*, A9). Every scenario commits real
//! sources to a scratch repository and goes through the real diff, analysis,
//! callable matcher and finding matcher.

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
use nsd::identity::matching::{match_callables, FileCallables};
use nsd::policy::diagnostics::{
    CODE_INVALID_SUPPRESSION, CODE_NEW_SUPPRESSION, CODE_UNMATCHED_FINDING,
};
use nsd::policy::findings::{match_findings, FindingFile};
use nsd::rules;
use nsd::suppress::{apply_suppressions, scan_suppressions, SuppressionOutput};

const MODE_REGULAR: i32 = 0o100644;
const PAD_METHODS: usize = 10;
const FILE: &str = "Widget.java";
const CATCH: &str = "try { work(); } catch (Exception e) { }";
const OTHER_CATCH: &str = "try { other(); } catch (Exception e) { }";
const SCALE_BOUND: Duration = Duration::from_secs(1);
const SCALE_PAIRS: usize = 14_000;
const SCALE_DIRECTIVE: &str = "// nsd-ignore[JAVA-EMPTY-CATCH]: x";
const SCALE_CATCH: &str = "try{a();}catch(Exception e){}";
const DIRECTIVE: &str = "// nsd-ignore[JAVA-EMPTY-CATCH]: legacy API";

// ---------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------

fn method(name: &str, body: &[&str]) -> String {
    let lines: String = body
        .iter()
        .map(|line| format!("        {line}\n"))
        .collect();
    format!("    void {name}() {{\n{lines}    }}\n")
}

fn class(methods: &[String]) -> String {
    format!("class Widget {{\n{}}}\n", methods.concat())
}

fn run_with(body: &[&str]) -> String {
    class(&[method("run", body)])
}

fn commit(repo: &Repository, text: &str) -> Oid {
    let entries = vec![(
        FILE.as_bytes().to_vec(),
        MODE_REGULAR,
        text.as_bytes().to_vec(),
    )];
    common::commit_entries(repo, &entries)
}

fn line_of(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
        .unwrap_or_else(|| panic!("no line contains {needle}"))
}

fn policy_with_s102(severity: Severity) -> PolicyConfig {
    PolicyConfig {
        nsd_s102: severity,
        ..Config::default().policy
    }
}

struct Analyzed {
    path: RepoPath,
    source: Vec<u8>,
    analysis: FileAnalysis,
}

fn analyze_snapshot(
    snapshot: &CommitSnapshot,
    repo: &Repository,
    wanted: &BTreeSet<RepoPath>,
) -> Vec<Analyzed> {
    let mut out = Vec::new();
    for entry in snapshot.entries.iter().filter(|e| wanted.contains(&e.path)) {
        let Some(bytes) = snapshot.read(repo, entry).expect("read blob") else {
            continue;
        };
        let analysis = analyze_file(Path::new(&entry.path.render()), &bytes).expect("analyze file");
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
) -> SuppressionOutput {
    let changes = diff_commit_to_commit(repo, Some(base), candidate).expect("diff commits");
    let (base_paths, candidate_paths) = changed_paths(&changes);
    let base_snapshot = CommitSnapshot::at(repo, base).expect("base snapshot");
    let candidate_snapshot = CommitSnapshot::at(repo, candidate).expect("candidate snapshot");
    let base_files = analyze_snapshot(&base_snapshot, repo, &base_paths);
    let candidate_files = analyze_snapshot(&candidate_snapshot, repo, &candidate_paths);
    let matched = match_callables(
        &callables_of(&base_files),
        &callables_of(&candidate_files),
        &changes,
    );
    let base_findings = finding_files(&base_files);
    let candidate_findings = finding_files(&candidate_files);
    let findings = match_findings(
        &base_findings,
        &candidate_findings,
        &matched,
        &changes,
        policy,
    )
    .expect("match findings");
    apply_suppressions(
        &base_findings,
        &candidate_findings,
        &findings,
        &changes,
        policy,
    )
    .expect("apply suppressions")
}

fn evaluate(repo: &Repository, base: Oid, candidate: Oid) -> SuppressionOutput {
    evaluate_with(repo, base, candidate, &Config::default().policy)
}

type Site = (&'static str, Option<&'static str>, usize);

fn sites(output: &SuppressionOutput) -> Vec<Site> {
    output
        .diagnostics
        .iter()
        .map(|d| {
            assert_eq!(d.candidate_path.render(), FILE);
            (d.code, d.rule_id, d.directive_line)
        })
        .collect()
}

fn v101_lines(output: &SuppressionOutput) -> Vec<usize> {
    output
        .findings
        .iter()
        .map(|d| {
            assert_eq!(d.code, CODE_UNMATCHED_FINDING);
            d.candidate_start_line
        })
        .collect()
}

/// Commits `base_body` and `candidate_body` as `run()` bodies and evaluates.
fn pair(base_body: &[&str], candidate_body: &[&str]) -> (SuppressionOutput, String) {
    let (_dir, repo) = common::init_repo();
    let base_text = run_with(base_body);
    let candidate_text = run_with(candidate_body);
    let base = commit(&repo, &base_text);
    let candidate = commit(&repo, &candidate_text);
    (evaluate(&repo, base, candidate), candidate_text)
}

fn scan(text: &str, policy: &PolicyConfig) -> Vec<Site> {
    let analysis = analyze_file(Path::new(FILE), text.as_bytes()).expect("analyze file");
    let files = [FindingFile {
        path: RepoPath::from_bytes(FILE.as_bytes().to_vec()),
        source: text.as_bytes(),
        analysis: &analysis,
    }];
    scan_suppressions(&files, policy)
        .iter()
        .map(|d| (d.code, d.rule_id, d.directive_line))
        .collect()
}

// ---------------------------------------------------------------------
// S101.
// ---------------------------------------------------------------------

#[test]
fn test_scratch_repo_new_suppression_raises_s101_not_v101() {
    let (output, text) = pair(&["work();"], &["work();", DIRECTIVE, CATCH]);

    assert_eq!(
        sites(&output),
        vec![(
            CODE_NEW_SUPPRESSION,
            Some(rules::JAVA_EMPTY_CATCH),
            line_of(&text, DIRECTIVE)
        )]
    );
    assert!(output.findings.is_empty());
}

#[test]
fn test_suppression_on_an_existing_unsuppressed_finding_raises_s101() {
    let (output, text) = pair(&[CATCH], &[DIRECTIVE, CATCH]);

    assert_eq!(
        sites(&output),
        vec![(
            CODE_NEW_SUPPRESSION,
            Some(rules::JAVA_EMPTY_CATCH),
            line_of(&text, DIRECTIVE)
        )]
    );
    assert!(output.findings.is_empty());
}

#[test]
fn test_moving_a_finding_with_its_directive_raises_nothing() {
    let (_dir, repo) = common::init_repo();
    let base_text = run_with(&[DIRECTIVE, CATCH]);
    let pad: Vec<String> = (0..PAD_METHODS)
        .map(|index| method(&format!("pad{index}"), &["work();", "work();"]))
        .collect();
    let mut moved = pad;
    moved.push(method("run", &[DIRECTIVE, CATCH]));
    let candidate_text = class(&moved);
    let base = commit(&repo, &base_text);
    let candidate = commit(&repo, &candidate_text);

    let output = evaluate(&repo, base, candidate);

    assert!(line_of(&candidate_text, CATCH) >= line_of(&base_text, CATCH) + 30);
    assert_eq!(sites(&output), vec![]);
    assert!(output.findings.is_empty());
}

#[test]
fn test_transferring_a_directive_to_an_unmatched_finding_raises_s101() {
    let (_dir, repo) = common::init_repo();
    let base_text = class(&[
        method("first", &[DIRECTIVE, CATCH]),
        method("second", &["work();"]),
    ]);
    let candidate_text = class(&[
        method("first", &["work();"]),
        method("second", &[DIRECTIVE, OTHER_CATCH]),
    ]);
    let base = commit(&repo, &base_text);
    let candidate = commit(&repo, &candidate_text);

    let output = evaluate(&repo, base, candidate);

    assert_eq!(
        sites(&output),
        vec![(
            CODE_NEW_SUPPRESSION,
            Some(rules::JAVA_EMPTY_CATCH),
            line_of(&candidate_text, DIRECTIVE)
        )]
    );
    assert!(output.findings.is_empty());
}

#[test]
fn test_editing_only_the_reason_raises_nothing() {
    let edited = "// nsd-ignore[JAVA-EMPTY-CATCH]: a different reason";
    let (output, _) = pair(&[DIRECTIVE, CATCH], &[edited, CATCH]);

    assert_eq!(sites(&output), vec![]);
    assert!(output.findings.is_empty());
}

// ---------------------------------------------------------------------
// S102 forms.
// ---------------------------------------------------------------------

#[test]
fn test_complexity_and_clone_codes_are_not_valid_targets() {
    for code in ["NSD-E101", "NSD-E102", "NSD-V102"] {
        let directive = format!("// nsd-ignore[{code}]: x");
        let (output, text) = pair(&["work();"], &[&directive, CATCH]);

        assert_eq!(
            sites(&output),
            vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, &directive))],
            "{code}"
        );
        assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)], "{code}");
    }
}

#[test]
fn test_block_comment_directive_is_invalid() {
    let directive = "/* nsd-ignore[JAVA-EMPTY-CATCH]: x */";
    let (output, text) = pair(&["work();"], &[directive, CATCH]);

    assert_eq!(
        sites(&output),
        vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, directive))]
    );
    assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)]);
}

#[test]
fn test_same_line_directive_is_invalid() {
    let trailing = format!("{CATCH} // nsd-ignore[JAVA-EMPTY-CATCH]: x");
    let (output, text) = pair(&["work();"], &["work();", &trailing]);

    assert_eq!(
        sites(&output),
        vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, &trailing))]
    );
    assert_eq!(v101_lines(&output), vec![line_of(&text, &trailing)]);
}

#[test]
fn test_missing_or_empty_reason_is_invalid() {
    for directive in [
        "// nsd-ignore[JAVA-EMPTY-CATCH]",
        "// nsd-ignore[JAVA-EMPTY-CATCH]:",
        "// nsd-ignore[JAVA-EMPTY-CATCH]:   ",
    ] {
        let (output, text) = pair(&["work();"], &[directive, CATCH]);

        assert_eq!(
            sites(&output),
            vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, "nsd-ignore"))],
            "{directive:?}"
        );
        assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)]);
    }
}

#[test]
fn test_unknown_rule_is_invalid() {
    for directive in [
        "// nsd-ignore[JAVA-NO-SUCH-RULE]: x",
        "// nsd-ignore[NSD-X999]: x",
        "// nsd-ignore[java-empty-catch]: x",
    ] {
        let (output, text) = pair(&["work();"], &[directive, CATCH]);

        assert_eq!(
            sites(&output),
            vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, "nsd-ignore"))],
            "{directive}"
        );
        assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)]);
    }
}

#[test]
fn test_new_unused_directive_raises_s102() {
    let (output, text) = pair(&["work();"], &[DIRECTIVE, "work();"]);

    assert_eq!(
        sites(&output),
        vec![(
            CODE_INVALID_SUPPRESSION,
            Some(rules::JAVA_EMPTY_CATCH),
            line_of(&text, DIRECTIVE)
        )]
    );
}

#[test]
fn test_directive_suppresses_only_the_next_line() {
    let (output, text) = pair(&["work();"], &[DIRECTIVE, "work();", CATCH]);

    assert_eq!(
        sites(&output),
        vec![(
            CODE_INVALID_SUPPRESSION,
            Some(rules::JAVA_EMPTY_CATCH),
            line_of(&text, DIRECTIVE)
        )]
    );
    assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)]);
}

// ---------------------------------------------------------------------
// S102 delta.
// ---------------------------------------------------------------------

#[test]
fn test_legacy_invalid_or_unused_directive_is_tolerated() {
    let (_dir, repo) = common::init_repo();
    let invalid = "// nsd-ignore[NSD-E101]: legacy";
    let base_text = class(&[method("run", &[invalid, "work();", DIRECTIVE, "work();"])]);
    let mut shifted = vec![method("added", &["work();"])];
    shifted.push(method("run", &[invalid, "work();", DIRECTIVE, "work();"]));
    let candidate_text = class(&shifted);
    let base = commit(&repo, &base_text);
    let candidate = commit(&repo, &candidate_text);

    let output = evaluate(&repo, base, candidate);

    assert_eq!(sites(&output), vec![]);

    let modified = "// nsd-ignore[NSD-E101]: legacy, reworded";
    let reworded = class(&[method("run", &[modified, "work();", DIRECTIVE, "work();"])]);
    let reworded_commit = commit(&repo, &reworded);
    let output = evaluate(&repo, base, reworded_commit);
    assert_eq!(
        sites(&output),
        vec![(CODE_INVALID_SUPPRESSION, None, line_of(&reworded, modified))]
    );
}

#[test]
fn test_directive_made_unused_by_the_change_raises_s102() {
    let (output, text) = pair(&[DIRECTIVE, CATCH], &[DIRECTIVE, "work();"]);

    assert_eq!(
        sites(&output),
        vec![(
            CODE_INVALID_SUPPRESSION,
            Some(rules::JAVA_EMPTY_CATCH),
            line_of(&text, DIRECTIVE)
        )]
    );
    assert!(output.findings.is_empty());
}

#[test]
fn test_off_disables_s102() {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &run_with(&["work();"]));
    let candidate_text = run_with(&["work();", "// nsd-ignore[NSD-E101]: x", DIRECTIVE, CATCH]);
    let candidate = commit(&repo, &candidate_text);

    let warn = evaluate_with(&repo, base, candidate, &policy_with_s102(Severity::Warn));
    let off = evaluate_with(&repo, base, candidate, &policy_with_s102(Severity::Off));

    let codes = |output: &SuppressionOutput| -> Vec<&'static str> {
        output.diagnostics.iter().map(|d| d.code).collect()
    };
    assert_eq!(
        codes(&warn),
        vec![CODE_INVALID_SUPPRESSION, CODE_NEW_SUPPRESSION]
    );
    assert_eq!(codes(&off), vec![CODE_NEW_SUPPRESSION]);
    assert!(scan(&candidate_text, &policy_with_s102(Severity::Off)).is_empty());

    let catch_only = run_with(&["work();", DIRECTIVE, CATCH]);
    let candidate = commit(&repo, &catch_only);
    let off = evaluate_with(&repo, base, candidate, &policy_with_s102(Severity::Off));
    assert_eq!(codes(&off), vec![CODE_NEW_SUPPRESSION]);
}

// ---------------------------------------------------------------------
// One snapshot, and string literals.
// ---------------------------------------------------------------------

#[test]
fn test_directive_text_inside_a_string_is_not_a_directive() {
    let literal = "String text = \"// nsd-ignore[JAVA-EMPTY-CATCH]: x\";";
    let text = run_with(&[literal, CATCH]);
    assert_eq!(scan(&text, &Config::default().policy), vec![]);

    let (output, text) = pair(&["work();"], &[literal, CATCH]);
    assert_eq!(sites(&output), vec![]);
    assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)]);
}

#[test]
fn test_single_snapshot_reports_every_invalid_and_unused_directive() {
    let trailing = "work(); // nsd-ignore[JAVA-EMPTY-CATCH]: x";
    let body = [
        DIRECTIVE,
        CATCH,
        "// nsd-ignore[NSD-E101]: x",
        "work();",
        "/* nsd-ignore[JAVA-EMPTY-CATCH]: x */",
        "work();",
        trailing,
        "// nsd-ignore[JAVA-EMPTY-CATCH]",
        "work();",
        DIRECTIVE,
        "work();",
    ];
    let text = run_with(&body);
    let lines: Vec<usize> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("nsd-ignore"))
        .map(|(index, _)| index + 1)
        .collect();
    assert_eq!(lines.len(), 6);

    let found = scan(&text, &Config::default().policy);

    let expected: Vec<Site> = vec![
        (CODE_INVALID_SUPPRESSION, None, lines[1]),
        (CODE_INVALID_SUPPRESSION, None, lines[2]),
        (CODE_INVALID_SUPPRESSION, None, lines[3]),
        (CODE_INVALID_SUPPRESSION, None, lines[4]),
        (
            CODE_INVALID_SUPPRESSION,
            Some(rules::JAVA_EMPTY_CATCH),
            lines[5],
        ),
    ];
    assert_eq!(found, expected);
}

#[test]
fn test_legacy_directives_in_a_renamed_file_are_tolerated() {
    let (_dir, repo) = common::init_repo();
    let text = run_with(&[
        "// nsd-ignore[NSD-E101]: legacy",
        "work();",
        DIRECTIVE,
        CATCH,
    ]);
    let entry = |name: &str| {
        vec![(
            name.as_bytes().to_vec(),
            MODE_REGULAR,
            text.clone().into_bytes(),
        )]
    };
    let base = common::commit_entries(&repo, &entry("Widget.java"));
    let candidate = common::commit_entries(&repo, &entry("Renamed.java"));

    let output = evaluate(&repo, base, candidate);

    assert_eq!(output.diagnostics, vec![]);
    assert!(output.findings.is_empty());
}

#[test]
fn test_partial_rule_or_missing_colon_is_invalid() {
    for directive in [
        "// nsd-ignore[EMPTY-CATCH]: x",
        "// nsd-ignore[JAVA-EMPTY-CATCH] legacy api",
    ] {
        let (output, text) = pair(&["work();"], &[directive, CATCH]);

        assert_eq!(
            sites(&output),
            vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, "nsd-ignore"))],
            "{directive}"
        );
        assert_eq!(
            v101_lines(&output),
            vec![line_of(&text, CATCH)],
            "{directive}"
        );
    }
}

#[test]
fn test_doc_comment_directive_is_invalid() {
    let directive = "/** nsd-ignore[JAVA-EMPTY-CATCH]: x */";
    let (output, text) = pair(&["work();"], &[directive, CATCH]);

    assert_eq!(
        sites(&output),
        vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, "nsd-ignore"))]
    );
    assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)]);
}

#[test]
fn test_two_directives_on_one_unchanged_line_raise_no_s102() {
    let (_dir, repo) = common::init_repo();
    let twin = "/* nsd-ignore[JAVA-EMPTY-CATCH]: a */ /* nsd-ignore[JAVA-EMPTY-CATCH]: b */";
    let body = [twin, CATCH];
    let base = commit(&repo, &class(&[method("run", &body)]));
    let candidate = commit(
        &repo,
        &class(&[method("added", &["work();"]), method("run", &body)]),
    );

    let output = evaluate(&repo, base, candidate);

    assert_eq!(sites(&output), vec![]);
}

#[test]
fn test_modified_multiline_block_directive_raises_s102() {
    let (_dir, repo) = common::init_repo();
    let base_text = run_with(&[
        "/* nsd-ignore[JAVA-EMPTY-CATCH]: a",
        "   first */",
        "work();",
    ]);
    let candidate_text = run_with(&[
        "/* nsd-ignore[JAVA-EMPTY-CATCH]: a",
        "   second */",
        "work();",
    ]);
    let base = commit(&repo, &base_text);
    let candidate = commit(&repo, &candidate_text);

    let output = evaluate(&repo, base, candidate);

    assert_eq!(
        sites(&output),
        vec![(
            CODE_INVALID_SUPPRESSION,
            None,
            line_of(&candidate_text, "nsd-ignore")
        )]
    );
}

#[test]
fn test_directive_on_a_later_javadoc_line_is_invalid() {
    let (output, text) = pair(
        &["work();"],
        &[
            "/**",
            " * nsd-ignore[JAVA-EMPTY-CATCH]: reason",
            " */",
            CATCH,
        ],
    );

    assert_eq!(
        sites(&output),
        vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, "nsd-ignore"))]
    );
    assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)]);
}

#[test]
fn test_directive_on_a_later_block_comment_line_is_invalid() {
    let (output, text) = pair(
        &["work();"],
        &["/*", "   nsd-ignore[JAVA-EMPTY-CATCH]: reason", "*/", CATCH],
    );

    assert_eq!(
        sites(&output),
        vec![(CODE_INVALID_SUPPRESSION, None, line_of(&text, "nsd-ignore"))]
    );
    assert_eq!(v101_lines(&output), vec![line_of(&text, CATCH)]);
}

#[test]
fn test_scan_with_many_directives_and_findings_is_not_quadratic() {
    let mut text = String::from("class W { void run() {\n");
    for _ in 0..SCALE_PAIRS {
        text.push_str(SCALE_DIRECTIVE);
        text.push('\n');
        text.push_str(SCALE_CATCH);
        text.push('\n');
    }
    text.push_str(SCALE_DIRECTIVE);
    text.push_str("\ntail();\n} }\n");
    let analysis = analyze_file(Path::new(FILE), text.as_bytes()).expect("analyze file");
    let files = [FindingFile {
        path: RepoPath::from_bytes(FILE.as_bytes().to_vec()),
        source: text.as_bytes(),
        analysis: &analysis,
    }];

    let started = Instant::now();
    let found = scan_suppressions(&files, &Config::default().policy);
    let elapsed = started.elapsed();

    assert!(elapsed < SCALE_BOUND, "scan took {elapsed:?}");
    let sites: Vec<(&str, usize)> = found.iter().map(|d| (d.code, d.directive_line)).collect();
    let unused = line_of(&text, "tail();") - 1;
    assert_eq!(sites, vec![(CODE_INVALID_SUPPRESSION, unused)]);
}

#[test]
fn test_directive_suppresses_every_finding_of_its_rule_on_the_next_line() {
    let two_catches = format!("{CATCH} {OTHER_CATCH}");
    let (output, text) = pair(&["work();"], &[DIRECTIVE, &two_catches]);

    let directive_line = line_of(&text, DIRECTIVE);
    assert_eq!(
        sites(&output),
        vec![
            (
                CODE_NEW_SUPPRESSION,
                Some("JAVA-EMPTY-CATCH"),
                directive_line
            ),
            (
                CODE_NEW_SUPPRESSION,
                Some("JAVA-EMPTY-CATCH"),
                directive_line
            ),
        ]
    );
    assert_eq!(v101_lines(&output), Vec::<usize>::new());
}

// ---------------------------------------------------------------------
// Suppression follow-ups: file-context carry-over and prose in comments.
// ---------------------------------------------------------------------

#[test]
fn test_directive_carried_into_an_unmatched_callable_raises_s101() {
    let (_dir, repo) = common::init_repo();
    let base_text = class(&[method("legacy", &[DIRECTIVE, CATCH])]);
    let candidate_text = class(&[method("fresh", &[DIRECTIVE, CATCH])]);
    let base = commit(&repo, &base_text);
    let candidate = commit(&repo, &candidate_text);

    let output = evaluate(&repo, base, candidate);

    assert_eq!(
        sites(&output),
        vec![(
            CODE_NEW_SUPPRESSION,
            Some(rules::JAVA_EMPTY_CATCH),
            line_of(&candidate_text, DIRECTIVE)
        )]
    );
    assert!(output.findings.is_empty());
}

#[test]
fn test_javadoc_prose_mentioning_nsd_ignore_is_not_a_directive() {
    let (output, text) = pair(
        &["work();"],
        &[
            "/**",
            " * nsd-ignore directives are documented here",
            " */",
            "work();",
            "/**",
            " * nsd-ignore[JAVA-EMPTY-CATCH]: reason",
            " */",
            "work();",
        ],
    );

    assert_eq!(
        sites(&output),
        vec![(
            CODE_INVALID_SUPPRESSION,
            None,
            line_of(&text, "nsd-ignore[")
        )]
    );
}
