//! WS-3's cross-language weight-parity suite (`nsd-plan-final.md`, *IR
//! conformance*): a corpus of matched constructs, written once in Java and
//! once in TS, must report identical `cc` and identical `sloc` per
//! construct -- the suite that makes `overall.erosion` defensible as a
//! single number across both language families.

use std::path::PathBuf;

use nsd::metrics;
use nsd::model::{Callable, DiscoveredFile, LanguageFamily};
use nsd::parse;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parity")
}

fn callables_for(relative_path: &str, language: LanguageFamily) -> Vec<Callable> {
    let root = fixture_root();
    let files = vec![DiscoveredFile {
        relative_path: PathBuf::from(relative_path),
        language,
    }];
    let (parsed, failures) = parse::parse_all(&root, &files);
    assert!(
        failures.is_empty(),
        "unexpected parse failures for {relative_path}: {failures:?}"
    );
    metrics::run(&parsed, false).callables
}

fn find_callable<'a>(callables: &'a [Callable], name: &str) -> &'a Callable {
    callables
        .iter()
        .find(|callable| callable.name == name)
        .unwrap_or_else(|| panic!("expected a callable named {name}, got {callables:?}"))
}

/// The eight matched decision constructs `nsd-plan-final.md`'s *IR
/// conformance* section names, one per callable, shared by both fixture
/// files under `tests/fixtures/parity/`.
const MATCHED_CONSTRUCTS: &[&str] = &[
    "ifConstruct",
    "forConstruct",
    "whileConstruct",
    "switchConstruct",
    "catchConstruct",
    "ternaryConstruct",
    "andConstruct",
    "orConstruct",
];

#[test]
fn test_matched_constructs_have_identical_cc_contribution() {
    let java = callables_for("Constructs.java", LanguageFamily::Java);
    let ts = callables_for("constructs.ts", LanguageFamily::JsTs);

    for name in MATCHED_CONSTRUCTS {
        let java_callable = find_callable(&java, name);
        let ts_callable = find_callable(&ts, name);
        assert_eq!(
            java_callable.cc, ts_callable.cc,
            "{name}: java cc {} != ts cc {}",
            java_callable.cc, ts_callable.cc
        );
    }
}

#[test]
fn test_matched_constructs_have_identical_executable_line_counts() {
    let java = callables_for("Constructs.java", LanguageFamily::Java);
    let ts = callables_for("constructs.ts", LanguageFamily::JsTs);

    for name in MATCHED_CONSTRUCTS {
        let java_callable = find_callable(&java, name);
        let ts_callable = find_callable(&ts, name);
        assert_eq!(
            java_callable.sloc, ts_callable.sloc,
            "{name}: java sloc {} != ts sloc {}",
            java_callable.sloc, ts_callable.sloc
        );
    }
}

/// `ifConstruct`'s `else`, `switchConstruct`'s `default` and
/// `catchConstruct`'s `finally` must not add to `cc` beyond the one counted
/// decision each callable actually has (the `if`, the one non-default
/// `case`, the one `catch`) -- in both lowerings.
#[test]
fn test_default_case_and_else_contribute_zero_in_both_languages() {
    let java = callables_for("Constructs.java", LanguageFamily::Java);
    let ts = callables_for("constructs.ts", LanguageFamily::JsTs);

    assert_eq!(find_callable(&java, "ifConstruct").cc, 2);
    assert_eq!(find_callable(&ts, "ifConstruct").cc, 2);

    assert_eq!(find_callable(&java, "switchConstruct").cc, 2);
    assert_eq!(find_callable(&ts, "switchConstruct").cc, 2);

    assert_eq!(find_callable(&java, "catchConstruct").cc, 2);
    assert_eq!(find_callable(&ts, "catchConstruct").cc, 2);
}
