//! PV-1: the Rust lowering (`docs/rust-lowering.md`). Every test feeds inline
//! Rust source through the production analysis or scan entry point and reads
//! what a caller observes: cyclomatic complexity, callable identity, executable
//! lines, clone groups and the scan report's Rust family.

use std::fs;
use std::path::Path;

use nsd::analysis::{analyze_file, FileAnalysis};
use nsd::ir::CallableKind;
use nsd::model::{LanguageFamily, ScanSettings, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline;

fn analyze(source: &str) -> FileAnalysis {
    match analyze_file(Path::new("probe.rs"), source.as_bytes()) {
        Ok(analysis) => analysis,
        Err(reason) => panic!("a Rust file should be analyzable, got {reason:?}"),
    }
}

/// The CC of the one callable named `probe` in `source`.
fn cc_of_probe(source: &str) -> u32 {
    let analysis = analyze(source);
    let probe: Vec<u32> = analysis
        .callables
        .iter()
        .filter(|callable| callable.metrics.name == "probe")
        .map(|callable| callable.metrics.cc)
        .collect();
    assert_eq!(probe.len(), 1, "exactly one `probe` callable in {source}");
    probe[0]
}

#[test]
fn test_rust_decision_kinds_match_the_mapping() {
    let cases: [(&str, &str, u32); 15] = [
        ("straight line", "fn probe() -> i32 { 1 }", 1),
        (
            "if",
            "fn probe(x: i32) -> i32 {\n    if x > 0 { 1 } else { 2 }\n}\n",
            2,
        ),
        (
            "match with a wildcard arm",
            "fn probe(x: i32) -> i32 {\n    match x {\n        1 => 10,\n        2 => 20,\n        _ => 0,\n    }\n}\n",
            3,
        ),
        (
            "match with a binding arm",
            "fn probe(x: i32) -> i32 {\n    match x {\n        1 => 10,\n        2 => 20,\n        n => n,\n        _ => 0,\n    }\n}\n",
            4,
        ),
        (
            "match with a guarded arm",
            "fn probe(x: i32) -> i32 {\n    match x {\n        1 => 10,\n        2 => 20,\n        n if n > 0 => n,\n        _ => 0,\n    }\n}\n",
            5,
        ),
        (
            "guarded wildcard",
            "fn probe(x: i32) -> i32 {\n    match x {\n        1 => 10,\n        _ if x > 5 => 20,\n        _ => 0,\n    }\n}\n",
            3,
        ),
        (
            "and / or",
            "fn probe(a: bool, b: bool, c: bool) -> bool {\n    a && b || c\n}\n",
            3,
        ),
        (
            "loop is not a decision",
            "fn probe() {\n    loop {\n        break;\n    }\n}\n",
            1,
        ),
        (
            "while",
            "fn probe(mut x: i32) {\n    while x > 0 {\n        x -= 1;\n    }\n}\n",
            2,
        ),
        (
            "for",
            "fn probe(v: Vec<i32>) {\n    for i in v {\n        drop(i);\n    }\n}\n",
            2,
        ),
        (
            "if let",
            "fn probe(o: Option<i32>) -> i32 {\n    if let Some(v) = o { v } else { 0 }\n}\n",
            2,
        ),
        (
            "while let",
            "fn probe(mut o: Option<i32>) {\n    while let Some(v) = o {\n        o = Some(v - 1).filter(|n| *n > 0);\n    }\n}\n",
            2,
        ),
        (
            "let-else",
            "fn probe(o: Option<i32>) -> i32 {\n    let Some(v) = o else {\n        return 0;\n    };\n    v\n}\n",
            2,
        ),
        (
            "the ? operator is not a decision",
            "fn probe(s: &str) -> Result<i32, std::num::ParseIntError> {\n    let n = s.parse::<i32>()?;\n    Ok(n)\n}\n",
            1,
        ),
        (
            "a macro invocation is opaque",
            "fn probe(x: i32) {\n    println!(\"{}\", if x > 0 { 1 } else { 2 });\n}\n",
            1,
        ),
    ];
    for (label, source, expected) in cases {
        assert_eq!(cc_of_probe(source), expected, "{label}");
    }
}

#[test]
fn test_rust_callables_closures_and_impl_owners() {
    let source = "use std::fmt;\n\
struct T;\n\
impl fmt::Display for T {\n    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {\n        Ok(())\n    }\n}\n\
impl fmt::Debug for T {\n    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {\n        Ok(())\n    }\n}\n\
trait Sig {\n    fn only_signature(&self);\n}\n\
fn host() -> i32 {\n    let f = |x: i32| x + 1;\n    if f(1) > 1 { 1 } else { 0 }\n}\n";
    let analysis = analyze(source);

    let names: Vec<&str> = analysis
        .callables
        .iter()
        .map(|callable| callable.metrics.name.as_str())
        .collect();
    assert!(
        !names.contains(&"only_signature"),
        "a trait signature without a body is not a callable: {names:?}"
    );
    assert_eq!(
        names.iter().filter(|name| **name == "fmt").count(),
        2,
        "{names:?}"
    );

    let closure = analysis
        .callables
        .iter()
        .find(|callable| callable.metrics.name == "f")
        .expect("`let f = |x| ...` is a callable named `f`");
    assert_eq!(closure.identity.kind, CallableKind::RustClosure);
    let host = analysis
        .callables
        .iter()
        .find(|callable| callable.metrics.name == "host")
        .expect("`host` is a callable");
    assert_eq!(host.identity.kind, CallableKind::RustFunction);
    assert_eq!(host.metrics.cc, 2, "the closure's body is not host's own");

    let fmts: Vec<_> = analysis
        .callables
        .iter()
        .filter(|callable| callable.metrics.name == "fmt")
        .collect();
    assert_ne!(
        fmts[0].identity, fmts[1].identity,
        "`impl Display for T` and `impl Debug for T` must not collide"
    );
    assert_ne!(fmts[0].identity.owner_digest, fmts[1].identity.owner_digest);
}

#[test]
fn test_rust_comments_and_attributes_are_not_executable() {
    let source = "#![allow(dead_code)]\n\
/// docs\n\
#[inline]\n\
fn probe(x: i32) -> i32 {\n\
    // note\n\
    /* block\n       comment */\n\
    let y = x + 1;\n\
    y\n\
}\n";
    let analysis = analyze(source);
    let lines = &analysis.executable_lines;
    for (line, label) in [
        (1, "inner attribute"),
        (2, "doc comment"),
        (3, "outer attribute"),
        (5, "line comment"),
        (6, "block comment start"),
        (7, "block comment end"),
    ] {
        assert!(!lines.contains(&line), "{label} line {line}: {lines:?}");
    }
    for line in [8, 9] {
        assert!(lines.contains(&line), "statement line {line}: {lines:?}");
    }
}

fn clone_body(tag: &str, comment: bool) -> String {
    let mut body = String::new();
    for index in 0..12 {
        if comment && index == 4 {
            body.push_str("    // a reviewer note, not code\n");
            body.push_str("    /// stray doc comment\n");
        }
        body.push_str(&format!("    let {tag}{index} = call{index}(x);\n"));
    }
    body
}

fn scan(root: &Path) -> (tempfile::TempDir, pipeline::PipelineOutput) {
    let output_dir = tempfile::tempdir().expect("output tempdir");
    let settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: DEFAULT_MIN_CLONE_LINES,
    };
    let target = root.to_str().expect("a UTF-8 path").to_string();
    let output = pipeline::run(&target, settings).expect("the scan succeeds");
    (output_dir, output)
}

#[test]
fn test_rust_copy_pasted_blocks_share_clone_tokens() {
    let dir = tempfile::tempdir().expect("fixture tempdir");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).expect("create src");
    fs::write(
        src.join("a.rs"),
        format!("fn first(x: i32) {{\n{}}}\n", clone_body("v", false)),
    )
    .expect("write a.rs");
    // Same tokens, other function name, extra comments between statements.
    fs::write(
        src.join("b.rs"),
        format!("fn second(x: i32) {{\n{}}}\n", clone_body("v", true)),
    )
    .expect("write b.rs");
    // Same shape but other identifiers: not a clone of the first two.
    fs::write(
        src.join("c.rs"),
        format!("fn third(x: i32) {{\n{}}}\n", clone_body("w", false)),
    )
    .expect("write c.rs");

    let (_output_dir, output) = scan(dir.path());

    assert_eq!(output.clones.groups.len(), 1, "{:?}", output.clones.groups);
    let group = &output.clones.groups[0];
    assert_eq!(group.language, LanguageFamily::Rust);
    let mut files: Vec<String> = group
        .locations
        .iter()
        .map(|location| location.relative_path.display().to_string())
        .collect();
    files.sort();
    assert_eq!(files, ["src/a.rs", "src/b.rs"]);
}

#[test]
fn test_scan_reports_a_rust_family() {
    let dir = tempfile::tempdir().expect("fixture tempdir");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).expect("create src");
    fs::write(
        src.join("lib.rs"),
        "pub fn grade(x: i32) -> i32 {\n    if x > 0 {\n        1\n    } else {\n        2\n    }\n}\n",
    )
    .expect("write lib.rs");

    let (output_dir, output) = scan(dir.path());

    assert_eq!(output.discover.discovered.len(), 1);
    assert_eq!(output.discover.discovered[0].language, LanguageFamily::Rust);
    let text = fs::read_to_string(output_dir.path().join("report.json")).expect("report.json");
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    let grade = value["top25"]
        .as_array()
        .expect("top25 array")
        .iter()
        .find(|callable| callable["name"] == "grade")
        .expect("the Rust callable is reported");
    assert_eq!(grade["cc"].as_u64(), Some(2));
    let scanned = value["scores"]["rust"]["verbosity"]["scanned_lines"]
        .as_u64()
        .expect("scores.rust.verbosity.scanned_lines is a number");
    assert!(scanned > 0, "the Rust family scanned lines: {scanned}");
    assert_eq!(
        value["scores"]["java"]["verbosity"]["scanned_lines"].as_u64(),
        Some(0),
        "no Java in this repo"
    );
}

/// A block doc comment holds named leaves of its own (the doc marker and the
/// `doc_comment` text), so only dropping the whole `block_comment` subtree
/// keeps its lines out of the executable set.
#[test]
fn test_rust_block_doc_comments_are_not_executable() {
    let source = "/** outer\n * block doc\n */\n\
fn probe(x: i32) -> i32 {\n\
    /*! inner block doc */\n\
    x + 1\n\
}\n";
    let analysis = analyze(source);
    let lines = &analysis.executable_lines;
    for line in [1, 2, 3, 5] {
        assert!(!lines.contains(&line), "doc comment line {line}: {lines:?}");
    }
    assert!(lines.contains(&6), "statement line 6: {lines:?}");
}
