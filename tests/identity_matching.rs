//! Integration tests for M2-3's callable matching
//! (`nsd-plan-final.md` *Stable data model*, `docs/implementation-status.md`
//! row M2-3): tiers 1-3 in order, plus ambiguity.
//!
//! Every source here is inline (`parse_inline`) or an in-memory `git2`
//! repository (`tests/common::{init_repo,commit_entries}`), never a file
//! under `tests/fixtures/`: `tests/neutrality.rs::all_fixture_relative_paths`
//! walks the *whole* `tests/fixtures/` tree as its clean corpus and compares
//! the rendered `report.json` byte-for-byte against a committed baseline
//! (`tests/golden/neutrality/clean.report.json`) -- adding a file there would
//! grow that corpus and move the baseline, which this stream's scope forbids
//! touching. `tests/identity.rs`'s own doc comment states the same rule; its
//! `parse_inline`/`lower_java`/`lower_ts`/`tree_sitter_language` helpers are
//! duplicated here rather than imported, since each `tests/*.rs` file is its
//! own compiled binary.

mod common;

use std::path::PathBuf;

use nsd::git::diff::{self, Change};
use nsd::git::path::RepoPath;
use nsd::identity::matching::{match_callables, CallableRef, FileCallables, MatchTier};
use nsd::identity::{self, CallableIdentity};
use nsd::ir::CallableKind;
use nsd::lower::{self, IrFile};
use nsd::model::{Grammar, LanguageFamily};
use nsd::parse::ParsedFile;

const JAVA: LanguageFamily = LanguageFamily::Java;
const JS_TS: LanguageFamily = LanguageFamily::JsTs;
const MODE_REGULAR: i32 = 0o100644;

// ---------------------------------------------------------------------
// Shared helpers: real lowering (mirrors `tests/identity.rs`).
// ---------------------------------------------------------------------

fn parse_inline(source: &str, grammar: Grammar, language: LanguageFamily) -> ParsedFile {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_language(grammar))
        .expect("set_language");
    let tree = parser.parse(source, None).expect("parse");
    ParsedFile {
        relative_path: PathBuf::from("inline"),
        language,
        source: source.to_string(),
        tree,
    }
}

fn tree_sitter_language(grammar: Grammar) -> tree_sitter::Language {
    match grammar {
        Grammar::Java => tree_sitter_java_orchard::LANGUAGE.into(),
        Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

fn lower_java(source: &str) -> (IrFile, String) {
    let parsed = parse_inline(source, Grammar::Java, JAVA);
    (lower::lower_file(&parsed), parsed.source)
}

fn lower_ts(source: &str) -> (IrFile, String) {
    let parsed = parse_inline(source, Grammar::TypeScript, JS_TS);
    (lower::lower_file(&parsed), parsed.source)
}

/// Builds a `FileCallables` at `path` from a lowered file and its own
/// source, in `IrFile.callables`' own document order -- the caller-side
/// bridge from WS-1's identity API to this module's input shape.
fn file_callables(path: &str, ir_file: &IrFile, source: &str) -> FileCallables {
    FileCallables {
        path: RepoPath::from_bytes(path.as_bytes().to_vec()),
        callables: ir_file
            .callables
            .iter()
            .map(|callable| {
                (
                    identity::callable_identity(ir_file, callable),
                    identity::body_fingerprint(ir_file, callable, source),
                )
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------
// Shared helpers: synthetic identities, for algorithm-focused tests that
// do not need a real parse.
// ---------------------------------------------------------------------

/// A minimal, path-free `CallableIdentity` distinguished only by `name` and
/// `signature` -- enough to drive tier 1's same-key grouping without a real
/// lowering.
fn synth_identity(name: &str, signature: &[&str]) -> CallableIdentity {
    CallableIdentity {
        owner_chain: Vec::new(),
        kind: CallableKind::JavaMethod,
        name: name.to_string(),
        signature: signature.iter().map(|part| part.to_string()).collect(),
    }
}

/// One file's callables, built directly from `(identity, fingerprint)`
/// pairs in the given (document) order -- for tests that assert on exact
/// pairing behaviour rather than on a real parse.
fn synth_file(path: &str, callables: &[(CallableIdentity, &str)]) -> FileCallables {
    FileCallables {
        path: RepoPath::from_bytes(path.as_bytes().to_vec()),
        callables: callables
            .iter()
            .cloned()
            .map(|(identity, fingerprint)| (identity, fingerprint.to_string()))
            .collect(),
    }
}

fn structural_matches(matches: &[nsd::identity::matching::CallableMatch]) -> usize {
    matches
        .iter()
        .filter(|m| m.tier == MatchTier::Structural)
        .count()
}

fn callable_ref(path: &str, index: usize) -> CallableRef {
    CallableRef {
        path: RepoPath::from_bytes(path.as_bytes().to_vec()),
        index,
    }
}

// ---------------------------------------------------------------------
// Tier 1: same path, equal identity.
// ---------------------------------------------------------------------

/// Observable acceptance: base `A.java` has a lambda, and the candidate
/// inserts lines above it -- one tier-1 match, no unmatched callables.
#[test]
fn test_line_only_change_matches_at_tier_1() {
    let base_java = "\
class C {
    Runnable r = () -> {
        System.out.println(\"hi\");
    };
}
";
    let candidate_java = "\
class C {


    // a comment, to move the lambda's line without touching its identity

    Runnable r = () -> {
        System.out.println(\"hi\");
    };
}
";
    let (base_ir, base_source) = lower_java(base_java);
    let (candidate_ir, candidate_source) = lower_java(candidate_java);
    assert_eq!(base_ir.callables.len(), 1, "fixture must have one callable");
    assert_eq!(
        candidate_ir.callables.len(),
        1,
        "fixture must have one callable"
    );

    let base = vec![file_callables("A.java", &base_ir, &base_source)];
    let candidate = vec![file_callables("A.java", &candidate_ir, &candidate_source)];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(output.matches.len(), 1, "{output:#?}");
    assert_eq!(output.matches[0].tier, MatchTier::Structural);
    assert_eq!(output.matches[0].base, callable_ref("A.java", 0));
    assert_eq!(output.matches[0].candidate, callable_ref("A.java", 0));
    assert!(output.ambiguities.is_empty());
}

/// A named method inserted above a JS callback, and the callback still
/// matches at tier 1.
#[test]
fn test_anonymous_callback_after_inserted_code_matches() {
    let base_js = "\
const list = [];
list.forEach(function () {
  console.log(\"hi\");
});
";
    let candidate_js = "\
function named() {}
const list = [];
list.forEach(function () {
  console.log(\"hi\");
});
";
    let (base_ir, base_source) = lower_ts(base_js);
    let (candidate_ir, candidate_source) = lower_ts(candidate_js);
    assert_eq!(base_ir.callables.len(), 1, "fixture must have one callable");
    assert_eq!(
        candidate_ir.callables.len(),
        2,
        "fixture must have named() plus the callback"
    );

    let base = vec![file_callables("a.js", &base_ir, &base_source)];
    let candidate = vec![file_callables("a.js", &candidate_ir, &candidate_source)];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(output.matches.len(), 1, "{output:#?}");
    assert_eq!(output.matches[0].tier, MatchTier::Structural);
    assert_eq!(output.matches[0].base, callable_ref("a.js", 0));
    assert!(output.ambiguities.is_empty());
}

/// Three same-key callbacks with distinct bodies, the third inserted before
/// the two existing ones: each original pairs at tier 1 with its own body,
/// and only the inserted one is unmatched.
#[test]
fn test_inserted_same_key_sibling_does_not_shift_pairs() {
    let identity = synth_identity("<anonymous>", &[]);
    let base = vec![synth_file(
        "a.js",
        &[
            (identity.clone(), "blake3:aaaa"),
            (identity.clone(), "blake3:bbbb"),
        ],
    )];
    let candidate = vec![synth_file(
        "a.js",
        &[
            (identity.clone(), "blake3:cccc"),
            (identity.clone(), "blake3:aaaa"),
            (identity.clone(), "blake3:bbbb"),
        ],
    )];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(structural_matches(&output.matches), 2, "{output:#?}");
    assert!(output
        .matches
        .iter()
        .any(|m| m.base == callable_ref("a.js", 0) && m.candidate == callable_ref("a.js", 1)));
    assert!(output
        .matches
        .iter()
        .any(|m| m.base == callable_ref("a.js", 1) && m.candidate == callable_ref("a.js", 2)));
    assert!(
        !output
            .matches
            .iter()
            .any(|m| m.candidate == callable_ref("a.js", 0)),
        "the inserted sibling must stay unmatched: {output:#?}"
    );
}

/// Two same-key callbacks whose bodies both change: 1st-with-1st,
/// 2nd-with-2nd, by order-preserving greedy matching in source order.
#[test]
fn test_same_key_group_falls_back_to_source_order() {
    let identity = synth_identity("<anonymous>", &[]);
    let base = vec![synth_file(
        "a.js",
        &[
            (identity.clone(), "blake3:a1"),
            (identity.clone(), "blake3:b1"),
        ],
    )];
    let candidate = vec![synth_file(
        "a.js",
        &[
            (identity.clone(), "blake3:a2"),
            (identity.clone(), "blake3:b2"),
        ],
    )];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(structural_matches(&output.matches), 2, "{output:#?}");
    assert!(output
        .matches
        .iter()
        .any(|m| m.base == callable_ref("a.js", 0) && m.candidate == callable_ref("a.js", 0)));
    assert!(output
        .matches
        .iter()
        .any(|m| m.base == callable_ref("a.js", 1) && m.candidate == callable_ref("a.js", 1)));
}

/// `f(int)`/`f(String)` swapped in place: each pairs with its own base
/// counterpart, never with the other overload.
#[test]
fn test_overload_reorder_keeps_pairs() {
    let f_int = synth_identity("f", &["int"]);
    let f_string = synth_identity("f", &["String"]);
    let base = vec![synth_file(
        "A.java",
        &[
            (f_int.clone(), "blake3:int-body"),
            (f_string.clone(), "blake3:string-body"),
        ],
    )];
    let candidate = vec![synth_file(
        "A.java",
        &[
            (f_string.clone(), "blake3:string-body"),
            (f_int.clone(), "blake3:int-body"),
        ],
    )];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(structural_matches(&output.matches), 2, "{output:#?}");
    assert!(output
        .matches
        .iter()
        .any(|m| m.base == callable_ref("A.java", 0) && m.candidate == callable_ref("A.java", 1)));
    assert!(output
        .matches
        .iter()
        .any(|m| m.base == callable_ref("A.java", 1) && m.candidate == callable_ref("A.java", 0)));
}

// ---------------------------------------------------------------------
// Tier 2: a Git rename, equal identity.
// ---------------------------------------------------------------------

/// Thirty near-identical padding lines around one `run()` method, so a
/// rename plus a one-line body edit still scores well above
/// `diff::RENAME_THRESHOLD`.
fn padded_java_class(body_line: &str) -> String {
    let mut source = String::new();
    for i in 0..30 {
        source.push_str(&format!("// pad {i}\n"));
    }
    source.push_str("class Widget {\n");
    source.push_str("    void run() {\n");
    source.push_str(&format!("        System.out.println(\"{body_line}\");\n"));
    source.push_str("    }\n");
    source.push_str("}\n");
    source
}

/// A real `git2` rename via `diff_commit_to_commit`, with the body edited,
/// still matches at tier 2.
#[test]
fn test_renamed_file_matches_at_tier_2() {
    let base_source = padded_java_class("original");
    let candidate_source = padded_java_class("changed");

    let (_dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(
            b"src/Old.java".to_vec(),
            MODE_REGULAR,
            base_source.clone().into_bytes(),
        )],
    );
    let candidate_oid = common::commit_entries(
        &repo,
        &[(
            b"src/New.java".to_vec(),
            MODE_REGULAR,
            candidate_source.clone().into_bytes(),
        )],
    );

    let changes = diff::diff_commit_to_commit(&repo, Some(base_oid), candidate_oid)
        .expect("diff commit to commit");
    assert_eq!(changes.len(), 1, "{changes:?}");
    match &changes[0] {
        Change::Renamed { from, to, .. } => {
            assert_eq!(from.as_bytes(), b"src/Old.java");
            assert_eq!(to.as_bytes(), b"src/New.java");
        }
        other => panic!("expected a Renamed change, got {other:?}"),
    }

    let (base_ir, _) = lower_java(&base_source);
    let (candidate_ir, _) = lower_java(&candidate_source);
    assert_eq!(base_ir.callables.len(), 1, "fixture must have one callable");
    assert_eq!(
        candidate_ir.callables.len(),
        1,
        "fixture must have one callable"
    );

    let base = vec![file_callables("src/Old.java", &base_ir, &base_source)];
    let candidate = vec![file_callables(
        "src/New.java",
        &candidate_ir,
        &candidate_source,
    )];

    let output = match_callables(&base, &candidate, &changes);

    assert_eq!(output.matches.len(), 1, "{output:#?}");
    assert_eq!(output.matches[0].tier, MatchTier::Rename);
    assert_eq!(output.matches[0].base, callable_ref("src/Old.java", 0));
    assert_eq!(output.matches[0].candidate, callable_ref("src/New.java", 0));
    assert!(output.ambiguities.is_empty());
}
