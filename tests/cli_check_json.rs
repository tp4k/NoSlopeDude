//! `nsd check --format json`: the minimal check document (a subset of
//! M6-1/M6-2) over scratch repositories, and its renderer on hand-built
//! diagnostics.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use git2::{IndexEntry, IndexTime, Oid, Repository};
use serde_json::{json, Value};
use tempfile::TempDir;

use nsd::analysis::UnanalyzableReason;
use nsd::check::CheckDiagnostic;
use nsd::config::{CODE_CONFIG_CHANGED, CODE_INVALID_CONFIG};
use nsd::format::render_json;
use nsd::git::path::RepoPath;
use nsd::git::snapshot::{CommitSnapshot, IndexSnapshot};
use nsd::git::snapshot_id::SnapshotId;
use nsd::policy::diagnostics::{
    BaseCallable, CloneDiagnostic, CoverageDiagnostic, DamageDiagnostic, FindingDiagnostic,
    PolicyDiagnostic, SuppressionDiagnostic, CODE_ANALYSIS_UNAVAILABLE, CODE_CLONE_REGRESSION,
    CODE_COMPLEXITY_ABOVE_THRESHOLD, CODE_COMPLEXITY_INCREASED, CODE_INVALID_SUPPRESSION,
    CODE_MATCH_AMBIGUITY, CODE_NEW_SUPPRESSION, CODE_PARSE_DAMAGE, CODE_UNMATCHED_FINDING,
};
use nsd::policy::Diagnostic;
use nsd::profile::{fingerprint, MeasurementProfileInputs};

const MODE_REGULAR: i32 = 0o100644;
const MODE_SYMLINK: i32 = 0o120000;
const MODE_SUBMODULE: i32 = 0o160000;
const V102_OFF: &str = "version: 1\npolicy:\n  NSD-V102: off\n";
const SKIP_KEYS: [&str; 13] = [
    "builtin_exclusion",
    "config_exclude",
    "invalid_encoding",
    "nested_checkout",
    "non_utf8_path",
    "outside_include",
    "parse_syntax_error",
    "parser_unavailable",
    "special_file",
    "submodule",
    "symlink",
    "too_large",
    "unsupported_extension",
];
const RENDERED_STATUS: u8 = 1;
const EXIT_ERROR: i32 = 2;
const EXIT_BOTH: i32 = 3;
const HIGH_CC_IFS: usize = 11;
const CLONE_BLOCK_LINES: usize = 12;
const SCHEMA_VERSION: u64 = 1;
const SCENARIO_CODES: [&str; 6] = [
    "NSD-E101", "NSD-E101", "NSD-E101", "NSD-V101", "NSD-A102", "NSD-V102",
];
const INVALID_UTF8_TS: &[u8] = b"export const x = '\xff\xfe';\n";
const REFUSAL: &str = "trusted config is inside the candidate checkout";

struct Fixture {
    dir: TempDir,
    repo: Repository,
}

fn fixture() -> Fixture {
    let (dir, repo) = common::init_repo();
    Fixture { dir, repo }
}

impl Fixture {
    fn commit(&self, files: &[(&str, &[u8])]) {
        let entries: Vec<(Vec<u8>, i32, Vec<u8>)> = files
            .iter()
            .map(|(path, bytes)| (path.as_bytes().to_vec(), MODE_REGULAR, bytes.to_vec()))
            .collect();
        let oid = common::commit_entries(&self.repo, &entries);
        let tree = self
            .repo
            .find_commit(oid)
            .and_then(|commit| commit.tree())
            .expect("read the committed tree");
        let mut index = self.repo.index().expect("open the index");
        index
            .read_tree(&tree)
            .expect("mirror the tree into the index");
        index.write().expect("write the index");
    }

    fn commit_modes(&self, files: &[(&str, i32, &[u8])]) {
        let entries: Vec<(Vec<u8>, i32, Vec<u8>)> = files
            .iter()
            .map(|(path, mode, bytes)| (path.as_bytes().to_vec(), *mode, bytes.to_vec()))
            .collect();
        let oid = common::commit_entries(&self.repo, &entries);
        let tree = self
            .repo
            .find_commit(oid)
            .and_then(|commit| commit.tree())
            .expect("read the committed tree");
        let mut index = self.repo.index().expect("open the index");
        index
            .read_tree(&tree)
            .expect("mirror the tree into the index");
        index.write().expect("write the index");
    }

    fn staged_json(&self) -> Value {
        document(&self.nsd(&["check", "--staged", "--format", "json"]))
    }

    fn staged_json_with(&self, config: &Path) -> Value {
        let config = config.to_str().expect("UTF-8 path");
        document(&self.nsd(&["check", "--staged", "--format", "json", "--config", config]))
    }

    fn stage(&self, path: &str, bytes: &[u8]) {
        let mut index = self.repo.index().expect("open the index");
        let entry = IndexEntry {
            ctime: IndexTime::new(0, 0),
            mtime: IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: MODE_REGULAR as u32,
            uid: 0,
            gid: 0,
            file_size: 0,
            id: Oid::ZERO_SHA1,
            flags: 0,
            flags_extended: 0,
            path: path.as_bytes().to_vec(),
        };
        index.add_frombuffer(&entry, bytes).expect("stage a buffer");
        index.write().expect("write the index");
    }

    /// E101 x3, V101, A102 and V102 staged over a two-file base.
    fn scenario() -> Fixture {
        let fx = fixture();
        let first = block_class("A", &clone_block("a"));
        let second = block_class("B", &clone_block("b"));
        fx.commit(&[("A.java", first.as_bytes())]);
        fx.commit(&[("A.java", first.as_bytes()), ("B.java", second.as_bytes())]);
        for name in ["C", "D", "E"] {
            fx.stage(
                &format!("{name}.java"),
                java_class(name, HIGH_CC_IFS).as_bytes(),
            );
        }
        fx.stage(
            "Copy.java",
            block_class("Copy", &clone_block("a")).as_bytes(),
        );
        fx.stage("Bad.ts", INVALID_UTF8_TS);
        fx.stage("Widget.java", catching().as_bytes());
        fx
    }

    fn nsd(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_nsd"))
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .expect("run the nsd binary")
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }
}

fn java_class(name: &str, ifs: usize) -> String {
    let mut text =
        format!("public class {name} {{\n    public int run(int a) {{\n        int r = 0;\n");
    for index in 0..ifs {
        text.push_str(&format!(
            "        if (a == {name}K{index}) {{ r += {name}V{index}; }}\n"
        ));
    }
    text.push_str("        return r;\n    }\n}\n");
    text
}

fn clone_block(tag: &str) -> String {
    (0..CLONE_BLOCK_LINES)
        .map(|index| format!("        int {tag}{index} = {tag}Call{index}(x);\n"))
        .collect()
}

fn block_class(name: &str, body: &str) -> String {
    format!("class {name} {{\n    void run(int x) {{\n{body}    }}\n}}\n")
}

fn catching() -> String {
    "class Widget {\n    void run() {\n        try { work(); } catch (Exception e) { }\n    }\n}\n"
        .to_string()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON document")
}

fn entries(document: &Value) -> &Vec<Value> {
    document["diagnostics"]
        .as_array()
        .expect("diagnostics is an array")
}

fn path(text: &str) -> RepoPath {
    RepoPath::from_bytes(text)
}

/// The keys of every object in `text`, in the order written, one list per object.
fn key_orders(text: &str) -> Vec<Vec<String>> {
    let mut finished = Vec::new();
    let mut open: Vec<Vec<String>> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '{' => open.push(Vec::new()),
            '}' => finished.extend(open.pop()),
            '"' => {
                let mut literal = String::new();
                while let Some(next) = chars.next() {
                    match next {
                        '\\' => {
                            literal.push(next);
                            literal.extend(chars.next());
                        }
                        '"' => break,
                        other => literal.push(other),
                    }
                }
                if chars.peek() == Some(&':') {
                    open.last_mut()
                        .expect("a key sits in an object")
                        .push(literal);
                }
            }
            _ => {}
        }
    }
    finished
}

/// D6: every number is an integer except the `erosion` and `ratio` scores,
/// which are finite shortest round-trip floats.
fn assert_integers_except_ratios(value: &Value, key: &str) {
    match value {
        Value::Number(number) if matches!(key, "ratio" | "erosion") => {
            let float = number.as_f64().expect("a float score");
            assert!(float.is_finite() && !(float == 0.0 && float.is_sign_negative()));
        }
        Value::Null if matches!(key, "ratio" | "erosion") => panic!("{key} is not a number"),
        Value::Number(number) => assert!(!number.is_f64(), "{key}: {number}"),
        Value::Array(items) => items
            .iter()
            .for_each(|item| assert_integers_except_ratios(item, key)),
        Value::Object(fields) => fields
            .iter()
            .for_each(|(name, item)| assert_integers_except_ratios(item, name)),
        _ => {}
    }
}

#[test]
fn test_check_format_json_prints_one_canonical_document() {
    let fx = Fixture::scenario();

    let output = fx.nsd(&["check", "--staged", "--format", "json"]);

    assert_eq!(output.status.code(), Some(EXIT_BOTH));
    let text = stdout(&output);
    assert!(
        text.ends_with("}\n") && text.matches('\n').count() == 1,
        "{text:?}"
    );
    let parsed = document(&output);
    assert_eq!(parsed["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(parsed["result_scope"], json!("check"));
    assert_eq!(parsed["exit_status"], json!(EXIT_BOTH));
    let codes: Vec<&str> = entries(&parsed)
        .iter()
        .map(|entry| entry["code"].as_str().expect("a code"))
        .collect();
    assert_eq!(codes, SCENARIO_CODES);
    let keys: Vec<&str> = parsed
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "coverage",
            "diagnostics",
            "entities",
            "exit_status",
            "fingerprints",
            "result_scope",
            "schema_version",
            "skipped",
            "snapshots",
            "summaries"
        ]
    );
    for id in [
        &parsed["snapshots"]["base"],
        &parsed["snapshots"]["candidate"],
        &parsed["fingerprints"]["measurement"],
        &parsed["fingerprints"]["configuration"],
    ] {
        let text = id.as_str().expect("a hash string");
        let hex = text.strip_prefix("blake3:").expect("a blake3 prefix");
        assert!(
            hex.len() >= 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{text}"
        );
    }
    assert!(parsed["entities"].is_array());
    assert_eq!(parsed["summaries"]["scope"], json!("full"));
    assert!(parsed["coverage"].is_array());
    let terminal = fx.nsd(&["check", "--staged"]);
    let terminal_codes: Vec<String> = stdout(&terminal)
        .lines()
        .map(|line| line.split(' ').next().unwrap_or_default().to_string())
        .collect();
    assert_eq!(codes, terminal_codes);
}

fn span_of(text: &str) -> (String, u64, u64) {
    let (file, lines) = text.rsplit_once(':').expect("a path:start-end span");
    let (start, end) = lines.split_once('-').expect("a start-end range");
    (
        file.to_string(),
        start.parse().expect("a start line"),
        end.parse().expect("an end line"),
    )
}

#[test]
fn test_check_format_json_entries_carry_each_terminal_field() {
    let fx = Fixture::scenario();

    let terminal = stdout(&fx.nsd(&["check", "--staged"]));
    let parsed = document(&fx.nsd(&["check", "--staged", "--format", "json"]));

    let lines: Vec<&str> = terminal.lines().collect();
    assert_eq!(lines.len(), entries(&parsed).len(), "{terminal}");
    for (line, entry) in lines.iter().zip(entries(&parsed)) {
        let words: Vec<&str> = line.split(' ').collect();
        assert_eq!(entry["code"], json!(words[0]), "{line}");
        match words[0] {
            "NSD-E101" => {
                let (file, start, end) = span_of(words[1]);
                assert_eq!(entry["path"], json!(file), "{line}");
                assert_eq!(entry["start_line"], json!(start), "{line}");
                assert_eq!(entry["end_line"], json!(end), "{line}");
                assert_eq!(entry["callable"], json!(words[2]), "{line}");
                assert_eq!(entry["base"], Value::Null, "{line}");
            }
            "NSD-V101" => {
                let (file, start, end) = span_of(words[1]);
                assert_eq!(entry["path"], json!(file), "{line}");
                assert_eq!(entry["start_line"], json!(start), "{line}");
                assert_eq!(entry["end_line"], json!(end), "{line}");
                assert_eq!(entry["rule_id"], json!(words[2]), "{line}");
            }
            "NSD-A102" => {
                assert_eq!(entry["path"], json!(words[1]), "{line}");
                assert_eq!(entry["reason"], json!(words[2]), "{line}");
            }
            "NSD-V102" => {
                let (file, start, end) = span_of(words[1]);
                let added = words[2]
                    .trim_start_matches("(+")
                    .parse::<u64>()
                    .expect("added");
                let (matched, matched_start, matched_end) = span_of(words[5]);
                assert_eq!(entry["path"], json!(file), "{line}");
                assert_eq!(entry["start_line"], json!(start), "{line}");
                assert_eq!(entry["end_line"], json!(end), "{line}");
                assert_eq!(entry["added_lines"], json!(added), "{line}");
                assert_eq!(
                    entry["matched"],
                    json!({"path": matched, "start_line": matched_start, "end_line": matched_end}),
                    "{line}"
                );
            }
            other => panic!("unexpected code {other} in {line}"),
        }
    }
}

#[test]
fn test_render_json_covers_every_diagnostic_kind() {
    let base = BaseCallable {
        path: path("old/Foo.java"),
        start_line: 3,
        end_line: 30,
        cc: 12,
        sloc: 25,
    };
    let policy = |code, base| {
        CheckDiagnostic::Complexity(Box::new(PolicyDiagnostic {
            code,
            candidate_path: path("src/Foo.java"),
            candidate_name: "run".to_string(),
            candidate_start_line: 4,
            candidate_end_line: 40,
            base,
        }))
    };
    let diagnostics = vec![
        CheckDiagnostic::Config(Diagnostic {
            code: CODE_CONFIG_CHANGED,
        }),
        CheckDiagnostic::Config(Diagnostic {
            code: CODE_INVALID_CONFIG,
        }),
        CheckDiagnostic::Failure {
            code: "NSD-G101",
            message: "cannot read the index".to_string(),
        },
        policy(CODE_COMPLEXITY_ABOVE_THRESHOLD, None),
        policy(CODE_COMPLEXITY_INCREASED, Some(base)),
        policy(CODE_MATCH_AMBIGUITY, None),
        CheckDiagnostic::Finding(FindingDiagnostic {
            code: CODE_UNMATCHED_FINDING,
            rule_id: "JAVA-EMPTY-CATCH",
            candidate_path: path("W.java"),
            candidate_start_line: 5,
            candidate_end_line: 6,
        }),
        CheckDiagnostic::Suppression(SuppressionDiagnostic {
            code: CODE_NEW_SUPPRESSION,
            rule_id: Some("JAVA-EMPTY-CATCH"),
            candidate_path: path("W.java"),
            directive_line: 4,
        }),
        CheckDiagnostic::Suppression(SuppressionDiagnostic {
            code: CODE_INVALID_SUPPRESSION,
            rule_id: None,
            candidate_path: path("W.java"),
            directive_line: 9,
        }),
        CheckDiagnostic::Damage(DamageDiagnostic {
            code: CODE_PARSE_DAMAGE,
            candidate_path: path("D.java"),
            candidate_start_line: 7,
            candidate_end_line: 8,
        }),
        CheckDiagnostic::Coverage(CoverageDiagnostic {
            code: CODE_ANALYSIS_UNAVAILABLE,
            path: RepoPath::from_bytes(b"src/100%\xff.java".to_vec()),
            reason: UnanalyzableReason::NonUtf8Path,
        }),
        CheckDiagnostic::Clone(Box::new(CloneDiagnostic {
            code: CODE_CLONE_REGRESSION,
            candidate_path: path("Copy.java"),
            candidate_start_line: 2,
            candidate_end_line: 13,
            base_lines: Some(3),
            added_lines: 12,
            matched_path: path("A.java"),
            matched_start_line: 20,
            matched_end_line: 31,
        })),
    ];

    let rendered = render_json(&diagnostics, 3);

    let expected = json!({
        "schema_version": 1,
        "result_scope": "check",
        "exit_status": 3,
        "diagnostics": [
            {"code": "NSD-C101", "path": "nsd.yml"},
            {"code": "NSD-C102", "path": "nsd.yml"},
            {"code": "NSD-G101", "message": "cannot read the index"},
            {"code": "NSD-E101", "path": "src/Foo.java", "start_line": 4, "end_line": 40,
             "callable": "run", "base": null},
            {"code": "NSD-E102", "path": "src/Foo.java", "start_line": 4, "end_line": 40,
             "callable": "run",
             "base": {"path": "old/Foo.java", "start_line": 3, "end_line": 30, "cc": 12, "sloc": 25}},
            {"code": "NSD-G102", "path": "src/Foo.java", "start_line": 4, "end_line": 40,
             "callable": "run", "base": null},
            {"code": "NSD-V101", "path": "W.java", "start_line": 5, "end_line": 6,
             "rule_id": "JAVA-EMPTY-CATCH"},
            {"code": "NSD-S101", "path": "W.java", "directive_line": 4,
             "rule_id": "JAVA-EMPTY-CATCH"},
            {"code": "NSD-S102", "path": "W.java", "directive_line": 9, "rule_id": null},
            {"code": "NSD-A101", "path": "D.java", "start_line": 7, "end_line": 8},
            {"code": "NSD-A102", "path": "src/100%25%FF.java", "reason": "non_utf8_path"},
            {"code": "NSD-V102", "path": "Copy.java", "start_line": 2, "end_line": 13,
             "added_lines": 12,
             "matched": {"path": "A.java", "start_line": 20, "end_line": 31}},
        ],
    });
    let parsed: Value = serde_json::from_str(&rendered).expect("valid JSON");
    assert_eq!(parsed, expected);
    assert_eq!(rendered, format!("{expected}\n"));
}

#[test]
fn test_check_format_json_is_byte_identical_across_repeated_runs() {
    let first = Fixture::scenario();
    let other_checkout = Fixture::scenario();
    let args = ["check", "--staged", "--format", "json"];

    let once = first.nsd(&args);
    let twice = first.nsd(&args);
    let elsewhere = other_checkout.nsd(&args);

    assert!(!once.stdout.is_empty());
    assert_eq!(once.stdout, twice.stdout);
    assert_eq!(once.stdout, elsewhere.stdout);
}

#[test]
fn test_check_format_json_keys_are_sorted_and_numbers_are_integers() {
    let fx = Fixture::scenario();

    let output = fx.nsd(&["check", "--staged", "--format", "json"]);

    let text = stdout(&output);
    let orders = key_orders(&text);
    assert!(orders.len() > SCENARIO_CODES.len(), "{text}");
    for keys in &orders {
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, &sorted, "{text}");
    }
    assert_integers_except_ratios(&document(&output), "");
    assert!(!text.contains("-0.0") && !text.contains("NaN"), "{text}");
}

#[test]
fn test_check_format_json_holds_no_checkout_path() {
    let fx = Fixture::scenario();
    let canonical = fx.root().canonicalize().expect("canonicalize the checkout");

    let output = fx.nsd(&["check", "--staged", "--format", "json"]);

    let text = stdout(&output);
    assert!(!text.is_empty());
    assert!(
        !text.contains(fx.root().to_str().expect("UTF-8 path")),
        "{text}"
    );
    assert!(
        !text.contains(canonical.to_str().expect("UTF-8 path")),
        "{text}"
    );
}

#[test]
fn test_check_format_json_escapes_control_characters() {
    let hostile = "run\u{1b}[31m\u{7}\u{7f}\u{9b}\n\"NSD-FAKE\\";
    let diagnostics = [CheckDiagnostic::Complexity(Box::new(PolicyDiagnostic {
        code: CODE_COMPLEXITY_ABOVE_THRESHOLD,
        candidate_path: path("src/Foo.java"),
        candidate_name: hostile.to_string(),
        candidate_start_line: 1,
        candidate_end_line: 2,
        base: None,
    }))];

    let rendered = render_json(&diagnostics, RENDERED_STATUS);

    assert_eq!(rendered.matches('\n').count(), 1, "{rendered:?}");
    assert!(!rendered.bytes().any(|byte| byte < 0x20 && byte != b'\n'));
    let parsed: Value = serde_json::from_str(&rendered).expect("valid JSON");
    assert_eq!(parsed["diagnostics"][0]["callable"], json!("<computed>@1"));
}

#[test]
fn test_check_default_format_is_terminal() {
    let fx = Fixture::scenario();

    let bare = fx.nsd(&["check", "--staged"]);
    let named = fx.nsd(&["check", "--staged", "--format", "terminal"]);

    assert_eq!(bare.status.code(), Some(EXIT_BOTH));
    assert_eq!(named.status.code(), Some(EXIT_BOTH));
    assert!(stdout(&bare).starts_with("NSD-E101 "), "{}", stdout(&bare));
    assert_eq!(bare.stdout, named.stdout);
}

#[test]
fn test_check_format_agent_is_a_usage_error() {
    let fx = Fixture::scenario();

    let output = fx.nsd(&["check", "--staged", "--format", "agent"]);

    assert_eq!(output.status.code(), Some(EXIT_ERROR));
    assert_eq!(stdout(&output), "");
}

#[test]
fn test_refused_config_renders_as_json() {
    let fx = Fixture::scenario();
    let inside = fx.root().join("trusted.yml");
    std::fs::write(&inside, "version: 1\n").expect("write the config");

    let output = fx.nsd(&[
        "check",
        "--staged",
        "--format",
        "json",
        "--config",
        inside.to_str().expect("UTF-8 path"),
    ]);

    assert_eq!(output.status.code(), Some(EXIT_ERROR));
    let parsed = document(&output);
    assert_eq!(parsed["exit_status"], json!(EXIT_ERROR));
    assert_eq!(
        parsed["diagnostics"],
        json!([{"code": CODE_INVALID_CONFIG, "message": REFUSAL}])
    );
}

const MIXED_JAVA: &str = include_str!("fixtures/salvage/Mixed.java");

fn simple_class(name: &str, extra: &str) -> String {
    format!(
        "package p;\n\nclass {name} {{\n    int f(int x) {{\n        int y = x + 1;\n{extra}        return y;\n    }}\n}}\n"
    )
}

fn trusted_config(text: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("create a temp dir for the trusted config");
    let path = dir.path().join("trusted.yml");
    std::fs::write(&path, text).expect("write the trusted config");
    (dir, path)
}

fn text_of<'a>(value: &'a Value, what: &str) -> &'a str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a string"))
}

fn range(start: u64, end: u64) -> Value {
    json!({"start_line": start, "end_line": end})
}

#[test]
fn test_check_json_carries_snapshot_ids_and_fingerprints() {
    let fx = fixture();
    fx.commit(&[("A.java", simple_class("A", "").as_bytes())]);
    fx.stage(
        "A.java",
        simple_class("A", "        y = y * 2;\n").as_bytes(),
    );
    let base_id = SnapshotId::of_commit(&CommitSnapshot::head_or_empty(&fx.repo).expect("head"));
    let candidate_id = SnapshotId::of_index(&IndexSnapshot::open(&fx.repo).expect("index"));
    let measured = |lines: u32| {
        fingerprint(&MeasurementProfileInputs {
            min_clone_lines: lines,
            ..MeasurementProfileInputs::current()
        })
    };
    let ten = "version: 1\nmeasurement:\n  min_clone_lines: 10\npolicy:\n  NSD-V102: warn\n";
    let ten_reformatted =
        "# a comment\npolicy: {NSD-V102: warn}\n\n\nmeasurement: {min_clone_lines: 10}\nversion: 1\n";
    let twenty = "version: 1\nmeasurement:\n  min_clone_lines: 20\npolicy:\n  NSD-V102: warn\n";
    let stricter = "version: 1\nmeasurement:\n  min_clone_lines: 10\npolicy:\n  NSD-V102: deny\n";
    let (_keep_a, config_ten) = trusted_config(ten);
    let (_keep_b, config_ten_elsewhere) = trusted_config(ten_reformatted);
    let (_keep_c, config_twenty) = trusted_config(twenty);
    let (_keep_d, config_stricter) = trusted_config(stricter);

    let with_ten = fx.staged_json_with(&config_ten);
    let reformatted = fx.staged_json_with(&config_ten_elsewhere);
    let with_twenty = fx.staged_json_with(&config_twenty);
    let with_stricter = fx.staged_json_with(&config_stricter);

    assert_eq!(with_ten["snapshots"]["base"], json!(base_id.to_string()));
    assert_eq!(
        with_ten["snapshots"]["candidate"],
        json!(candidate_id.to_string())
    );
    assert_ne!(
        with_ten["snapshots"]["base"],
        with_ten["snapshots"]["candidate"]
    );
    assert_eq!(with_ten["fingerprints"]["measurement"], json!(measured(10)));
    assert_eq!(
        with_twenty["fingerprints"]["measurement"],
        json!(measured(20))
    );
    assert_ne!(measured(10), measured(20));
    let configuration = |document: &Value| document["fingerprints"]["configuration"].clone();
    assert!(configuration(&with_ten)
        .as_str()
        .is_some_and(|text| text.starts_with("blake3:")));
    assert_eq!(configuration(&with_ten), configuration(&reformatted));
    assert_ne!(configuration(&with_ten), configuration(&with_twenty));
    assert_ne!(configuration(&with_ten), configuration(&with_stricter));
    assert_eq!(
        with_ten["fingerprints"]["measurement"],
        with_stricter["fingerprints"]["measurement"]
    );
}

#[test]
fn test_check_json_lists_changed_callables_with_base_matches() {
    let fx = fixture();
    let base = "class A {\n    int g(int x) {\n        return x;\n    }\n\n    int f(int x) {\n        int y = x + 1;\n        return y;\n    }\n}\n";
    let other = "class Z {\n    int z(int x) {\n        return x;\n    }\n}\n";
    fx.commit(&[("A.java", base.as_bytes()), ("Z.java", other.as_bytes())]);
    let edited = "class A {\n    int g(int x) {\n        return x;\n    }\n\n    int f(int x) {\n        int y = x + 1;\n        y = y * 2;\n        return y;\n    }\n\n    int h(int x) {\n        return x * 3;\n    }\n}\n";
    fx.stage("A.java", edited.as_bytes());
    fx.stage(
        "B.java",
        "class B {\n    int b() {\n        return 1;\n    }\n}\n".as_bytes(),
    );

    let parsed = fx.staged_json();

    let entities = parsed["entities"].as_array().expect("an entities array");
    let summary: Vec<(String, String, u64)> = entities
        .iter()
        .map(|entity| {
            (
                text_of(&entity["path"], "path").to_string(),
                text_of(&entity["name"], "name").to_string(),
                entity["start_line"].as_u64().expect("start_line"),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("A.java".to_string(), "f".to_string(), 6),
            ("A.java".to_string(), "h".to_string(), 12),
            ("B.java".to_string(), "b".to_string(), 2),
        ],
        "{entities:?}"
    );
    assert_eq!(entities[0]["end_line"], json!(10));
    assert_eq!(entities[0]["cc"], json!(1));
    assert!(entities[0]["sloc"].as_u64().is_some_and(|sloc| sloc > 0));
    let matched = &entities[0]["base"];
    assert_eq!(matched["path"], json!("A.java"));
    assert_eq!(matched["start_line"], json!(6));
    assert_eq!(matched["end_line"], json!(9));
    assert_eq!(matched["cc"], json!(1));
    assert!(matched["sloc"].as_u64().is_some());
    assert_eq!(entities[1]["base"], Value::Null);
    assert_eq!(entities[2]["base"], Value::Null);
}

#[test]
fn test_check_json_counts_skips_per_reason() {
    let fx = fixture();
    fx.commit_modes(&[
        ("A.java", MODE_REGULAR, simple_class("A", "").as_bytes()),
        (
            "legacy/G.java",
            MODE_REGULAR,
            simple_class("G", "").as_bytes(),
        ),
        ("link.java", MODE_SYMLINK, b"A.java"),
        ("node_modules/n.js", MODE_REGULAR, b"let n = 1;\n"),
        ("vendor/lib", MODE_SUBMODULE, &[0xCCu8; 20]),
    ]);
    fx.stage(
        "A.java",
        simple_class("A", "        y = y * 2;\n").as_bytes(),
    );
    let (_keep, config) = trusted_config("version: 1\nexclude:\n  - \"legacy/**\"\n");

    let parsed = fx.staged_json_with(&config);

    let skipped = parsed["skipped"].as_object().expect("a skipped object");
    let keys: Vec<&str> = skipped.keys().map(String::as_str).collect();
    assert_eq!(keys, SKIP_KEYS, "{skipped:?}");
    for key in SKIP_KEYS {
        let expected = match key {
            "symlink" | "submodule" | "config_exclude" | "builtin_exclusion" => 1,
            _ => 0,
        };
        assert_eq!(skipped[key], json!(expected), "{key}: {skipped:?}");
    }
}

#[test]
fn test_check_json_summaries_follow_the_settled_scope() {
    let fx = fixture();
    fx.commit(&[
        ("A.java", simple_class("A", "").as_bytes()),
        ("B.java", simple_class("B", "").as_bytes()),
    ]);
    fx.stage(
        "A.java",
        simple_class("A", "        y = y * 2;\n").as_bytes(),
    );
    let (_keep, off) = trusted_config(V102_OFF);

    let changed_only = fx.staged_json_with(&off);
    let full = fx.staged_json();

    assert_eq!(changed_only["summaries"]["scope"], json!("changed"));
    assert_eq!(changed_only["summaries"]["files"], json!(1));
    assert_eq!(full["summaries"]["scope"], json!("full"));
    assert_eq!(full["summaries"]["files"], json!(2));
    let lines = |document: &Value, family: &str| document["summaries"][family]["verbosity"].clone();
    for (document, scanned) in [(&changed_only, 6), (&full, 11)] {
        let overall = lines(document, "overall");
        assert_eq!(overall["scanned_lines"], json!(scanned), "{overall}");
        assert_eq!(overall["unanalyzed_lines"], json!(0));
        assert_eq!(overall["complete"], json!(true));
        assert_eq!(overall["flagged_lines"], json!(0));
        assert_eq!(overall["ratio"], json!(0.0));
        assert_eq!(lines(document, "java")["scanned_lines"], json!(scanned));
        assert_eq!(lines(document, "js_ts")["scanned_lines"], json!(0));
        assert_eq!(lines(document, "js_ts")["complete"], json!(true));
        assert!(document["summaries"]["overall"]["erosion"].is_number());
    }
    let only_b = TempDir::new().expect("create a scan target");
    std::fs::write(only_b.path().join("B.java"), simple_class("B", "")).expect("write B.java");
    let output = TempDir::new().expect("create a scan output directory");
    let scan = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .arg("scan")
        .arg(only_b.path())
        .arg("--output")
        .arg(output.path())
        .output()
        .expect("run nsd scan");
    assert!(
        scan.status.success(),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let report: Value = serde_json::from_slice(
        &std::fs::read(output.path().join("report.json")).expect("read report.json"),
    )
    .expect("report.json is JSON");
    let b_alone = report["scores"]["overall"]["verbosity"]["scanned_lines"]
        .as_u64()
        .expect("scanned_lines");
    assert_eq!(b_alone, 5, "B.java alone must scan to the pinned 5 lines");
    assert_eq!(11 - 6, b_alone);
}

#[test]
fn test_check_json_surfaces_tolerated_parser_gaps() {
    let fx = fixture();
    fx.commit(&[("Mixed.java", MIXED_JAVA.as_bytes())]);
    let edited = MIXED_JAVA.replace("return 1;", "return 2;");
    fx.stage("Mixed.java", edited.as_bytes());

    let output = fx.nsd(&["check", "--staged", "--format", "json"]);

    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let parsed = document(&output);
    assert_eq!(parsed["diagnostics"], json!([]));
    assert_eq!(parsed["skipped"]["parse_syntax_error"], json!(1));
    let coverage = parsed["coverage"].as_array().expect("a coverage array");
    assert_eq!(coverage.len(), 1, "{coverage:?}");
    assert_eq!(coverage[0]["path"], json!("Mixed.java"));
    assert_eq!(coverage[0]["complete"], json!(false));
    assert!(coverage[0]["unanalyzed_lines"]
        .as_u64()
        .is_some_and(|n| n > 0));
    assert!(coverage[0]["analyzed_lines"]
        .as_u64()
        .is_some_and(|n| n > 0));
    assert_eq!(
        coverage[0]["gaps"],
        json!([{"base": range(12, 12), "candidate": range(12, 12), "tolerated": true}])
    );
    assert_eq!(
        parsed["summaries"]["overall"]["verbosity"]["complete"],
        json!(false)
    );

    let shifted = edited.replacen(
        "    }\n\n    void broken",
        "    }\n    // one\n    // two\n\n    void broken",
        1,
    );
    assert_ne!(shifted, edited);
    fx.stage("Mixed.java", shifted.as_bytes());

    let output = fx.nsd(&["check", "--staged", "--format", "json"]);

    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let parsed = document(&output);
    assert_eq!(parsed["diagnostics"], json!([]));
    assert_eq!(
        parsed["coverage"][0]["gaps"],
        json!([{"base": range(12, 12), "candidate": range(14, 14), "tolerated": true}])
    );
}

#[test]
fn test_failure_messages_hold_no_absolute_path() {
    let outside = TempDir::new().expect("create a directory that is no repository");
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(["check", "--staged", "--format", "json"])
        .current_dir(outside.path())
        .output()
        .expect("run the nsd binary");
    let text = stdout(&output);
    assert_eq!(output.status.code(), Some(EXIT_ERROR), "{text}");
    assert_eq!(
        document(&output)["diagnostics"][0]["code"],
        json!("NSD-G101")
    );
    let canonical = outside.path().canonicalize().expect("canonicalize");
    for form in [outside.path(), canonical.as_path()] {
        assert!(!text.contains(form.to_str().expect("UTF-8 path")), "{text}");
    }

    let fx = Fixture::scenario();
    let raw = fx.root().to_str().expect("UTF-8 path").to_string();
    let canonical = fx
        .root()
        .canonicalize()
        .expect("canonicalize")
        .to_str()
        .expect("UTF-8 path")
        .to_string();
    let config_dir = TempDir::new().expect("create a config directory");
    let missing = config_dir.path().join("missing-trusted.yml");
    let inside = fx.root().join("trusted.yml");
    std::fs::write(&inside, "version: 1\n").expect("write the config");
    for config in [&missing, &inside] {
        let output = fx.nsd(&[
            "check",
            "--staged",
            "--format",
            "json",
            "--config",
            config.to_str().expect("UTF-8 path"),
        ]);
        let text = stdout(&output);
        assert!(!text.is_empty());
        assert!(!text.contains(&raw) && !text.contains(&canonical), "{text}");
        let config_text = config.to_str().expect("UTF-8 path");
        let config_canonical = config
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .zip(config.file_name())
            .map(|(parent, name)| parent.join(name).to_string_lossy().into_owned());
        assert!(!text.contains(config_text), "{text}");
        if let Some(form) = config_canonical {
            assert!(!text.contains(&form), "{text}");
        }
    }
    let output = fx.nsd(&["check", "--base", "no-such-ref", "--format", "json"]);
    let text = stdout(&output);
    assert_eq!(
        document(&output)["diagnostics"][0]["code"],
        json!("NSD-G101")
    );
    assert!(!text.contains(&raw) && !text.contains(&canonical), "{text}");
}

/// Replaces a committed blob's loose object with one whose header (and so
/// its listed size) still reads, but whose contents end short of that size,
/// so only the full read fails. The zlib stream is one stored block.
fn corrupt_loose_blob(fx: &Fixture, bytes: &[u8]) {
    let oid = Oid::hash_object(git2::ObjectType::Blob, bytes).expect("hash the blob");
    let hex = oid.to_string();
    let object = fx
        .repo
        .path()
        .join("objects")
        .join(&hex[..2])
        .join(&hex[2..]);
    let mut raw = format!("blob {}\0", bytes.len()).into_bytes();
    raw.extend_from_slice(&bytes[..bytes.len() / 2]);
    let len = u16::try_from(raw.len()).expect("a short object");
    let mut stream = vec![0x78, 0x01, 0x01];
    stream.extend_from_slice(&len.to_le_bytes());
    stream.extend_from_slice(&(!len).to_le_bytes());
    stream.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    stream.extend_from_slice(&((b << 16) | a).to_be_bytes());
    std::fs::remove_file(&object).expect("remove the loose object");
    std::fs::write(&object, stream).expect("write the corrupt object");
}

#[test]
fn test_failed_candidate_read_makes_its_family_incomplete() {
    let fx = fixture();
    let unchanged = simple_class("B", "");
    fx.commit(&[
        ("A.java", simple_class("A", "").as_bytes()),
        ("B.java", unchanged.as_bytes()),
    ]);
    fx.stage(
        "A.java",
        simple_class("A", "        y = y * 2;\n").as_bytes(),
    );
    corrupt_loose_blob(&fx, unchanged.as_bytes());

    let parsed = fx.staged_json();

    assert!(
        entries(&parsed)
            .iter()
            .any(|entry| entry["code"] == json!("NSD-G101")),
        "{parsed}"
    );
    for family in ["overall", "java"] {
        assert_eq!(
            parsed["summaries"][family]["verbosity"]["complete"],
            json!(false),
            "{family}: {parsed}"
        );
    }
    assert_eq!(
        parsed["summaries"]["js_ts"]["verbosity"]["complete"],
        json!(true),
        "{parsed}"
    );
}

#[test]
fn test_check_json_fingerprint_covers_scope_and_every_severity() {
    let fx = fixture();
    fx.commit(&[("A.java", simple_class("A", "").as_bytes())]);
    fx.stage(
        "A.java",
        simple_class("A", "        y = y * 2;\n").as_bytes(),
    );
    let (_keep_base, base_config) = trusted_config("version: 1\n");
    let base = fx.staged_json_with(&base_config);
    let variants = [
        "version: 1\nexclude:\n  - \"legacy/**\"\n",
        "version: 1\ninclude:\n  - \"**/*.java\"\n",
        "version: 1\npolicy:\n  NSD-E101: off\n",
        "version: 1\npolicy:\n  NSD-E102: off\n",
        "version: 1\npolicy:\n  NSD-V101: off\n",
        "version: 1\npolicy:\n  NSD-S102: off\n",
    ];
    for text in variants {
        let (_keep, config) = trusted_config(text);
        let varied = fx.staged_json_with(&config);
        assert_ne!(
            varied["fingerprints"]["configuration"], base["fingerprints"]["configuration"],
            "{text}"
        );
        assert_eq!(
            varied["fingerprints"]["measurement"], base["fingerprints"]["measurement"],
            "{text}"
        );
    }
}

#[test]
fn test_check_json_orders_entities_by_name_on_one_line() {
    let fx = fixture();
    fx.commit(&[("A.java", b"class A {\n}\n")]);
    fx.stage(
        "A.java",
        b"class A {\n    void b() { work(); } void a() { work(); }\n}\n",
    );

    let parsed = fx.staged_json();

    let entities = parsed["entities"].as_array().expect("an entities array");
    let names: Vec<&str> = entities
        .iter()
        .map(|entity| text_of(&entity["name"], "name"))
        .collect();
    assert_eq!(names, ["a", "b"], "{entities:?}");
    assert_eq!(entities[0]["start_line"], entities[1]["start_line"]);
}

#[test]
fn test_check_json_lists_a_resigned_callable_with_its_base() {
    let fx = fixture();
    fx.commit(&[(
        "A.java",
        b"class A {\n    int g(int x) {\n        return x;\n    }\n}\n",
    )]);
    fx.stage(
        "A.java",
        b"class A {\n    int g(long x) {\n        return x;\n    }\n}\n",
    );

    let parsed = fx.staged_json();

    let entities = parsed["entities"].as_array().expect("an entities array");
    assert_eq!(entities.len(), 1, "{entities:?}");
    assert_eq!(entities[0]["path"], json!("A.java"));
    assert_eq!(entities[0]["start_line"], json!(2));
    assert_eq!(entities[0]["base"]["path"], json!("A.java"));
    assert_eq!(entities[0]["base"]["start_line"], json!(2));
}

#[test]
fn test_check_json_reports_an_unanalyzable_changed_file() {
    let fx = fixture();
    fx.commit(&[("A.java", simple_class("A", "").as_bytes())]);
    fx.stage(
        "A.java",
        simple_class("A", "        y = y * 2;\n").as_bytes(),
    );
    fx.stage("Bad.ts", INVALID_UTF8_TS);

    let parsed = fx.staged_json();

    assert_eq!(parsed["skipped"]["invalid_encoding"], json!(1));
    let coverage = parsed["coverage"].as_array().expect("a coverage array");
    let paths: Vec<&str> = coverage
        .iter()
        .map(|entry| text_of(&entry["path"], "path"))
        .collect();
    assert_eq!(paths, ["A.java", "Bad.ts"], "{coverage:?}");
    assert_eq!(
        coverage[1],
        json!({
            "path": "Bad.ts",
            "analyzed_lines": 0,
            "unanalyzed_lines": 0,
            "complete": false,
            "gaps": [],
        })
    );
    let summaries = &parsed["summaries"];
    assert_eq!(summaries["js_ts"]["verbosity"]["complete"], json!(false));
    assert_eq!(summaries["overall"]["verbosity"]["complete"], json!(false));
    assert_eq!(summaries["java"]["verbosity"]["complete"], json!(true));
}

#[test]
fn test_check_json_lists_a_raised_gap_as_not_tolerated() {
    let fx = fixture();
    fx.commit(&[("B.java", simple_class("B", "").as_bytes())]);
    fx.stage("A.java", b"class A {\n    int x = ;\n}\n");

    let output = fx.nsd(&["check", "--staged", "--format", "json"]);

    assert_eq!(
        output.status.code(),
        Some(EXIT_ERROR),
        "{}",
        stdout(&output)
    );
    let parsed = document(&output);
    let codes: Vec<&str> = entries(&parsed)
        .iter()
        .map(|entry| text_of(&entry["code"], "code"))
        .collect();
    assert_eq!(codes, ["NSD-A101"], "{parsed}");
    let coverage = &parsed["coverage"][0];
    assert_eq!(coverage["path"], json!("A.java"));
    assert_eq!(
        coverage["gaps"],
        json!([{"base": Value::Null, "candidate": range(2, 2), "tolerated": false}])
    );
    assert_eq!(coverage["unanalyzed_lines"], json!(0));
    assert_eq!(coverage["complete"], json!(false));
}

#[test]
fn test_check_json_publishes_no_source_text_in_entity_names() {
    let fx = fixture();
    fx.commit(&[("Base.java", b"class Base {\n}\n".as_slice())]);
    fx.stage(
        "reg.js",
        "registry[(function () {\n  const apiKey = \"SECRET-TOKEN-1234\";\n  return apiKey;\n})()] =\nfunction (x) {\n  if (x) { return 1; }\n  return 2;\n};\n"
            .as_bytes(),
    );

    let output = fx.nsd(&["check", "--staged", "--format", "json"]);

    let text = stdout(&output);
    assert!(!text.contains("SECRET-TOKEN-1234"), "{text}");
    assert!(text.contains("<computed>@5"), "{text}");
}

#[test]
fn test_check_json_publishes_no_source_text_in_complexity_entries() {
    let fx = fixture();
    fx.commit(&[("Base.java", b"class Base {\n}\n".as_slice())]);
    let branches = "  if (x === 1) { return 1; }\n".repeat(11);
    fx.stage(
        "reg.js",
        format!(
            "registry[(function () {{\n  const apiKey = \"SECRET-TOKEN-1234\";\n  return apiKey;\n}})()] =\nfunction (x) {{\n{branches}  return 2;\n}};\n"
        )
        .as_bytes(),
    );

    let output = fx.nsd(&["check", "--staged", "--format", "json"]);

    let text = stdout(&output);
    assert!(!text.contains("SECRET-TOKEN-1234"), "{text}");
    let parsed = document(&output);
    let complexity = entries(&parsed)
        .iter()
        .find(|entry| entry["code"] == json!("NSD-E101"))
        .unwrap_or_else(|| panic!("an NSD-E101 entry: {text}"));
    assert_eq!(complexity["callable"], json!("<computed>@5"), "{text}");
}

const DUP_A: &str = include_str!("fixtures/clones/src/main/java/DupA.java");
const DUP_B: &str = include_str!("fixtures/clones/src/main/java/DupB.java");
const DUP_A_PATH: &str = "app/DupA.java";
const DUP_B_PATH: &str = "app/DupB.java";
const DUP_FLAGGED_LINES: u64 = 12;
const DUP_SCANNED_LINES: u64 = 30;
const DUP_B_SCANNED_LINES: u64 = 15;
const DUP_BLOCK_LINES: u64 = 12;

fn overall_verbosity(document: &Value) -> Value {
    document["summaries"]["overall"]["verbosity"].clone()
}

/// `scores.overall.verbosity` of `nsd scan` over a directory holding the two
/// duplicate fixtures at the same relative paths.
fn scan_overall_verbosity() -> Value {
    let target = TempDir::new().expect("create a scan target");
    std::fs::create_dir(target.path().join("app")).expect("create the app directory");
    std::fs::write(target.path().join(DUP_A_PATH), DUP_A).expect("write DupA.java");
    std::fs::write(target.path().join(DUP_B_PATH), DUP_B).expect("write DupB.java");
    let output = TempDir::new().expect("create a scan output directory");
    let scan = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .arg("scan")
        .arg(target.path())
        .arg("--output")
        .arg(output.path())
        .output()
        .expect("run nsd scan");
    assert!(
        scan.status.success(),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let report: Value = serde_json::from_slice(
        &std::fs::read(output.path().join("report.json")).expect("read report.json"),
    )
    .expect("report.json is JSON");
    report["scores"]["overall"]["verbosity"].clone()
}

fn committed_duplicates() -> Fixture {
    let fx = fixture();
    fx.commit(&[
        (DUP_A_PATH, DUP_A.as_bytes()),
        (DUP_B_PATH, DUP_B.as_bytes()),
    ]);
    fx
}

fn assert_dup_counts(verbosity: &Value, flagged: u64) {
    assert_eq!(verbosity["flagged_lines"], json!(flagged), "{verbosity}");
    assert_eq!(
        verbosity["scanned_lines"],
        json!(DUP_SCANNED_LINES),
        "{verbosity}"
    );
}

#[test]
fn test_check_summary_ratio_counts_clone_lines_like_scan() {
    let fx = committed_duplicates();
    let scan = scan_overall_verbosity();
    assert_dup_counts(&scan, DUP_FLAGGED_LINES);

    let check = overall_verbosity(&fx.staged_json());

    assert_dup_counts(&check, DUP_FLAGGED_LINES);
    assert_eq!(check["ratio"], scan["ratio"], "check {check} scan {scan}");
    assert_eq!(check["ratio"], json!(0.4));
}

#[test]
fn test_check_summary_clone_lines_use_the_trusted_min_clone_lines() {
    let (_keep, above_block) = trusted_config(&format!(
        "version: 1\nmeasurement:\n  min_clone_lines: {}\n",
        DUP_BLOCK_LINES + 1
    ));
    for fx in [committed_duplicates(), {
        let staged = fixture();
        staged.commit(&[("Base.java", b"class Base {\n}\n".as_slice())]);
        staged.stage(DUP_A_PATH, DUP_A.as_bytes());
        staged.stage(DUP_B_PATH, DUP_B.as_bytes());
        staged
    }] {
        let verbosity = overall_verbosity(&fx.staged_json_with(&above_block));

        assert_eq!(verbosity["flagged_lines"], json!(0), "{verbosity}");
    }
}

#[test]
fn test_check_summary_clone_lines_survive_a_warm_cache() {
    let fx = committed_duplicates();

    let cold = fx.staged_json();
    let warm = fx.staged_json();

    assert_dup_counts(&overall_verbosity(&warm), DUP_FLAGGED_LINES);
    assert_eq!(cold["summaries"], warm["summaries"]);
}

#[test]
fn test_check_summary_clone_lines_follow_the_changed_scope() {
    let (_keep, off) = trusted_config(V102_OFF);

    let both = fixture();
    both.commit(&[("Base.java", b"class Base {\n}\n".as_slice())]);
    both.stage(DUP_A_PATH, DUP_A.as_bytes());
    both.stage(DUP_B_PATH, DUP_B.as_bytes());
    let both = both.staged_json_with(&off);
    assert_eq!(both["summaries"]["scope"], json!("changed"));
    let verbosity = overall_verbosity(&both);
    assert_eq!(verbosity["flagged_lines"], json!(0), "{verbosity}");

    let one = fixture();
    one.commit(&[
        ("Base.java", b"class Base {\n}\n".as_slice()),
        (DUP_B_PATH, DUP_B.as_bytes()),
    ]);
    one.stage(DUP_A_PATH, DUP_A.as_bytes());
    let one = one.staged_json_with(&off);
    assert_eq!(one["summaries"]["scope"], json!("changed"));
    let verbosity = overall_verbosity(&one);
    assert_eq!(verbosity["flagged_lines"], json!(0), "{verbosity}");
    assert_eq!(
        verbosity["scanned_lines"],
        json!(DUP_B_SCANNED_LINES),
        "{verbosity}"
    );
}

#[test]
fn test_check_summary_counts_staged_clone_lines_like_scan() {
    let scan = scan_overall_verbosity();
    assert_dup_counts(&scan, DUP_FLAGGED_LINES);

    let both = fixture();
    both.commit(&[("Base.java", b"class Base {\n}\n".as_slice())]);
    both.stage(DUP_A_PATH, DUP_A.as_bytes());
    both.stage(DUP_B_PATH, DUP_B.as_bytes());
    let both = both.staged_json();
    assert_eq!(both["summaries"]["scope"], json!("full"));
    assert_eq!(
        overall_verbosity(&both)["flagged_lines"],
        scan["flagged_lines"],
        "{both}"
    );

    let one = fixture();
    one.commit(&[
        ("Base.java", b"class Base {\n}\n".as_slice()),
        (DUP_B_PATH, DUP_B.as_bytes()),
    ]);
    one.stage(DUP_A_PATH, DUP_A.as_bytes());
    let one = one.staged_json();
    assert_eq!(one["summaries"]["scope"], json!("full"));
    assert_eq!(
        overall_verbosity(&one)["flagged_lines"],
        scan["flagged_lines"],
        "{one}"
    );
}
