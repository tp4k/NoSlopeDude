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
use nsd::policy::diagnostics::{
    BaseCallable, CloneDiagnostic, CoverageDiagnostic, DamageDiagnostic, FindingDiagnostic,
    PolicyDiagnostic, SuppressionDiagnostic, CODE_ANALYSIS_UNAVAILABLE, CODE_CLONE_REGRESSION,
    CODE_COMPLEXITY_ABOVE_THRESHOLD, CODE_COMPLEXITY_INCREASED, CODE_INVALID_SUPPRESSION,
    CODE_MATCH_AMBIGUITY, CODE_NEW_SUPPRESSION, CODE_PARSE_DAMAGE, CODE_UNMATCHED_FINDING,
};
use nsd::policy::Diagnostic;

const MODE_REGULAR: i32 = 0o100644;
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

fn assert_integers_only(value: &Value) {
    match value {
        Value::Number(number) => assert!(!number.is_f64(), "{number}"),
        Value::Array(items) => items.iter().for_each(assert_integers_only),
        Value::Object(fields) => fields.values().for_each(assert_integers_only),
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
    assert_integers_only(&document(&output));
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
    assert_eq!(parsed["diagnostics"][0]["callable"], json!(hostile));
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
