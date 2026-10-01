use std::fs;
use std::path::{Path, PathBuf};

use nsd::analysis::{analyze_file, FileAnalysis, UnanalyzableReason};
use nsd::identity;
use nsd::metrics;
use nsd::model::{DiscoveredFile, Grammar, LanguageFamily};
use nsd::parse::{self, ParsedFile};
use nsd::rules;

const MEBIBYTE: usize = 1_048_576;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .expect("read fixture dir")
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn extension_of(path: &Path) -> &str {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
}

fn analyze_text(path: &str, source: &str) -> FileAnalysis {
    match analyze_file(Path::new(path), source.as_bytes()) {
        Ok(analysis) => analysis,
        Err(reason) => panic!("{path} should be analyzable, got {reason:?}"),
    }
}

fn parse_with_scan_pipeline(root: &Path, relative: &Path) -> ParsedFile {
    let language = LanguageFamily::from_extension(extension_of(relative)).expect("language");
    let files = vec![DiscoveredFile {
        relative_path: relative.to_path_buf(),
        language,
    }];
    let (parsed, _failures) = parse::parse_all(root, &files);
    parsed.into_iter().next().expect("parsed file")
}

type CallableRow = (String, usize, usize, u32, usize);
type FindingRow = (&'static str, usize, usize, Vec<usize>);

#[test]
fn test_analysis_matches_the_scan_pipeline_on_every_fixture_file() {
    let mut checked = 0usize;
    for tree in ["metrics", "rules"] {
        let root = fixtures_root().join(tree);
        let mut files = Vec::new();
        collect_files(&root, &mut files);
        for file in files {
            if Grammar::for_extension(extension_of(&file)).is_none() {
                continue;
            }
            let relative = file.strip_prefix(&root).expect("under root").to_path_buf();
            let bytes = fs::read(&file).expect("read fixture");
            let analysis = match analyze_file(&relative, &bytes) {
                Ok(analysis) => analysis,
                Err(reason) => panic!("{relative:?} should be analyzable, got {reason:?}"),
            };

            let parsed = parse_with_scan_pipeline(&root, &relative);
            let scan_metrics = metrics::run(std::slice::from_ref(&parsed), false);
            let mut expected_callables: Vec<CallableRow> = scan_metrics
                .callables
                .iter()
                .map(|c| (c.name.clone(), c.start_line, c.end_line, c.cc, c.sloc))
                .collect();
            let mut actual_callables: Vec<CallableRow> = analysis
                .callables
                .iter()
                .map(|c| {
                    let m = &c.metrics;
                    (m.name.clone(), m.start_line, m.end_line, m.cc, m.sloc)
                })
                .collect();
            expected_callables.sort();
            actual_callables.sort();
            assert_eq!(actual_callables, expected_callables, "{relative:?}");

            let mut expected_findings: Vec<FindingRow> = rules::find_findings(&[parsed])
                .iter()
                .map(|f| (f.rule_id, f.start_line, f.end_line, f.flagged_lines.clone()))
                .collect();
            let mut actual_findings: Vec<FindingRow> = analysis
                .findings
                .iter()
                .map(|f| {
                    let f = &f.finding;
                    (f.rule_id, f.start_line, f.end_line, f.flagged_lines.clone())
                })
                .collect();
            expected_findings.sort();
            actual_findings.sort();
            assert_eq!(actual_findings, expected_findings, "{relative:?}");
            checked += 1;
        }
    }
    assert!(checked >= 20, "only {checked} fixture files were compared");
}

fn padded_java(total_bytes: usize) -> Vec<u8> {
    let mut source = b"class Padded { void m() { return; } }".to_vec();
    source.resize(total_bytes, b' ');
    source
}

#[test]
fn test_exactly_one_mebibyte_is_analyzed() {
    let bytes = padded_java(MEBIBYTE);
    assert_eq!(bytes.len(), 1_048_576);
    let analysis = analyze_file(Path::new("Padded.java"), &bytes).expect("1 MiB is accepted");
    assert_eq!(analysis.callables.len(), 1);
}

#[test]
fn test_one_byte_over_a_mebibyte_is_too_large() {
    let bytes = padded_java(MEBIBYTE + 1);
    assert_eq!(
        analyze_file(Path::new("Padded.java"), &bytes).err(),
        Some(UnanalyzableReason::TooLarge)
    );
}

#[test]
fn test_invalid_utf8_source_is_an_encoding_failure() {
    let bytes = b"class A { String s = \"\xff\xfe\"; }";
    assert_eq!(
        analyze_file(Path::new("A.java"), bytes).err(),
        Some(UnanalyzableReason::InvalidEncoding)
    );
}

#[cfg(unix)]
#[test]
fn test_non_utf8_path_is_unanalyzable() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let path = Path::new(OsStr::from_bytes(b"src/bad\xff.java"));
    assert_eq!(
        analyze_file(path, b"class A {}").err(),
        Some(UnanalyzableReason::NonUtf8Path)
    );
}

#[cfg(unix)]
#[test]
fn test_non_utf8_path_with_an_unsupported_extension_is_unsupported() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let path = Path::new(OsStr::from_bytes(b"img/bad\xff.png"));
    assert_eq!(
        analyze_file(path, b"\x89PNG\r\n").err(),
        Some(UnanalyzableReason::UnsupportedExtension)
    );
}

#[test]
fn test_unsupported_extension_is_unanalyzable() {
    assert_eq!(
        analyze_file(Path::new("tool.py"), b"def f():\n    return 1\n").err(),
        Some(UnanalyzableReason::UnsupportedExtension)
    );
}

#[test]
fn test_salvaged_file_is_analyzed_with_its_damage() {
    let path = fixtures_root().join("salvage/Mixed.java");
    let bytes = fs::read(path).expect("read fixture");
    let analysis = analyze_file(Path::new("Mixed.java"), &bytes).expect("salvaged file analyzed");
    let names: Vec<&str> = analysis
        .callables
        .iter()
        .map(|c| c.metrics.name.as_str())
        .collect();
    assert_eq!(names, vec!["safe"], "damaged callable must not appear");
    assert!(!analysis.ir.damage.is_empty(), "damage spans are exposed");
}

const EMPTY_CATCH_COMPACT: &str =
    "class C {\n    void m() {\n        try { f(); } catch (Exception e) { }\n    }\n}\n";

const EMPTY_CATCH_REFORMATTED: &str = "class C {\n    void m() {\n        try {\n            f();\n        }\n        catch (   Exception    e   ) /* why */ // nothing\n        {\n\n        }\n    }\n}\n";

const EMPTY_CATCH_EDITED: &str =
    "class C {\n    void m() {\n        try { f(); } catch (RuntimeException e) { }\n    }\n}\n";

fn only_finding_digest(source: &str) -> String {
    let analysis = analyze_text("C.java", source);
    assert_eq!(analysis.findings.len(), 1, "{source}");
    analysis.findings[0].syntax_digest.clone()
}

#[test]
fn test_finding_syntax_ignores_whitespace_and_comments() {
    assert_eq!(
        only_finding_digest(EMPTY_CATCH_COMPACT),
        only_finding_digest(EMPTY_CATCH_REFORMATTED)
    );
}

#[test]
fn test_finding_syntax_changes_with_a_token_edit() {
    assert_ne!(
        only_finding_digest(EMPTY_CATCH_COMPACT),
        only_finding_digest(EMPTY_CATCH_EDITED)
    );
}

#[test]
fn test_finding_records_its_innermost_enclosing_callable() {
    let source = "\
try { a(); } catch (e) {}
function outer() {
  function inner() {
    try { b(); } catch (e) {}
  }
  try { c(); } catch (e) {}
}
";
    let analysis = analyze_text("nested.js", source);
    let mut by_line: Vec<(usize, Option<&str>)> = analysis
        .findings
        .iter()
        .map(|f| {
            (
                f.finding.start_line,
                f.enclosing_callable
                    .map(|index| analysis.callables[index].metrics.name.as_str()),
            )
        })
        .collect();
    by_line.sort();
    assert_eq!(
        by_line,
        vec![(1, None), (4, Some("inner")), (6, Some("outer"))]
    );
}

#[test]
fn test_callable_identities_and_fingerprints_match_the_identity_module() {
    let relative = Path::new("__tests__/NestedCallable.js");
    let path = fixtures_root().join("metrics").join(relative);
    let source = fs::read_to_string(path).expect("read fixture");
    let analysis = analyze_text("__tests__/NestedCallable.js", &source);
    let identities = identity::identities(&analysis.ir);
    assert!(analysis.callables.len() >= 2);
    assert_eq!(analysis.callables.len(), identities.len());
    for (index, callable) in analysis.callables.iter().enumerate() {
        assert_eq!(callable.identity, identities[index]);
        assert_eq!(
            callable.body_fingerprint,
            identity::body_fingerprint(&analysis.ir, &analysis.ir.callables[index], &source)
        );
    }
}

fn fingerprint_with_body_span(
    analysis: &FileAnalysis,
    source: &str,
    body_span: nsd::ir::Span,
) -> String {
    let mut callable = analysis.ir.callables[0].clone();
    callable.body_span = body_span;
    identity::body_fingerprint(&analysis.ir, &callable, source)
}

fn span(start_byte: u32, end_byte: u32) -> nsd::ir::Span {
    nsd::ir::Span {
        start_byte,
        end_byte,
        start_line: 1,
        end_line: 1,
    }
}

#[test]
fn test_bodies_without_an_ir_subtree_share_one_fingerprint() {
    let source = "class C { int m() { return 1; } }";
    let analysis = analyze_text("C.java", source);
    let past_source = fingerprint_with_body_span(&analysis, source, span(9_000, 9_010));
    let further_past = fingerprint_with_body_span(&analysis, source, span(70_000, 70_001));
    assert_eq!(past_source, further_past);
    assert_ne!(past_source, analysis.callables[0].body_fingerprint);
}

fn unreachable_method(second_statement: &str) -> String {
    format!(
        "class C {{\n    int m() {{\n        return 1;\n        a();\n        {second_statement}\n    }}\n}}\n"
    )
}

#[test]
fn test_finding_syntax_covers_every_flagged_statement() {
    let original = only_finding_digest(&unreachable_method("b();"));
    let edited = only_finding_digest(&unreachable_method("c();"));
    assert_ne!(original, edited);
}

#[test]
fn test_enclosing_callable_after_a_closed_callable_is_the_outer_one_or_none() {
    let source = "\
function first() {
  function deep() {
    function deepest() {}
    try { a(); } catch (e) {}
  }
  try { b(); } catch (e) {}
}
try { c(); } catch (e) {}
function second() {
  try { d(); } catch (e) {}
}
try { e(); } catch (e) {}
";
    let analysis = analyze_text("siblings.js", source);
    let mut by_line: Vec<(usize, Option<&str>)> = analysis
        .findings
        .iter()
        .map(|f| {
            (
                f.finding.start_line,
                f.enclosing_callable
                    .map(|index| analysis.callables[index].metrics.name.as_str()),
            )
        })
        .collect();
    by_line.sort();
    assert_eq!(
        by_line,
        vec![
            (4, Some("deep")),
            (6, Some("first")),
            (8, None),
            (10, Some("second")),
            (12, None)
        ]
    );
}

#[test]
fn test_unanalyzable_reason_labels_are_distinct_and_stable() {
    let labels: Vec<&str> = [
        UnanalyzableReason::NonUtf8Path,
        UnanalyzableReason::UnsupportedExtension,
        UnanalyzableReason::TooLarge,
        UnanalyzableReason::InvalidEncoding,
        UnanalyzableReason::ParserUnavailable,
    ]
    .into_iter()
    .map(UnanalyzableReason::label)
    .collect();
    assert_eq!(
        labels,
        vec![
            "non_utf8_path",
            "unsupported_extension",
            "too_large",
            "invalid_encoding",
            "parser_unavailable"
        ]
    );
}
