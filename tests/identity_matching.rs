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
use nsd::identity::matching::{
    match_callables, CallableRef, FileCallables, MatchPairing, MatchTier, PositionalRemainder,
};
use nsd::identity::{self, CallableIdentity, OwnerDigest};
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
        owner_digest: OwnerDigest::default(),
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

// ---------------------------------------------------------------------
// Tier 3: pooled, exact 1:1 leftover body fingerprint.
// ---------------------------------------------------------------------

/// A method moved from `A.java` to `B.java` with an unchanged body matches
/// at tier 3 -- no `Change::Renamed` is supplied, so tiers 1-2 cannot find
/// it; only the pooled leftover fingerprint does.
#[test]
fn test_cross_file_move_matches_at_tier_3() {
    let moved = synth_identity("run", &[]);
    let base = vec![synth_file("A.java", &[(moved.clone(), "blake3:unchanged")])];
    let candidate = vec![synth_file("B.java", &[(moved.clone(), "blake3:unchanged")])];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(output.matches.len(), 1, "{output:#?}");
    assert_eq!(output.matches[0].tier, MatchTier::BodyFingerprint);
    assert_eq!(output.matches[0].base, callable_ref("A.java", 0));
    assert_eq!(output.matches[0].candidate, callable_ref("B.java", 0));
    assert!(output.ambiguities.is_empty());
}

/// A method renamed in the same file with an unchanged body pairs at tier 3
/// -- same path on both sides, but the name change gives it a different
/// `CallableIdentity`, so tier 1 cannot pair it either.
#[test]
fn test_renamed_method_with_unchanged_body_matches_at_tier_3() {
    let old_name = synth_identity("oldName", &[]);
    let new_name = synth_identity("newName", &[]);
    let base = vec![synth_file(
        "A.java",
        &[(old_name.clone(), "blake3:unchanged")],
    )];
    let candidate = vec![synth_file(
        "A.java",
        &[(new_name.clone(), "blake3:unchanged")],
    )];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(output.matches.len(), 1, "{output:#?}");
    assert_eq!(output.matches[0].tier, MatchTier::BodyFingerprint);
    assert_eq!(output.matches[0].base, callable_ref("A.java", 0));
    assert_eq!(output.matches[0].candidate, callable_ref("A.java", 0));
    assert!(output.ambiguities.is_empty());
}

/// A callable that tier 1 would match is never taken by tier 3, even when
/// another file has an identical body: base has `A.java::x` (fingerprint
/// "shared") matching the candidate's `A.java::x` at tier 1, and also
/// `B.java::y` with the very same fingerprint "shared" but no counterpart on
/// the candidate side. `y` must not be spuriously paired against `x`'s
/// candidate, which tier 1 already consumed.
#[test]
fn test_tier_order_prefers_structural_identity() {
    let x = synth_identity("x", &[]);
    let y = synth_identity("y", &[]);
    let base = vec![
        synth_file("A.java", &[(x.clone(), "blake3:shared")]),
        synth_file("B.java", &[(y.clone(), "blake3:shared")]),
    ];
    let candidate = vec![synth_file("A.java", &[(x.clone(), "blake3:shared")])];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(output.matches.len(), 1, "{output:#?}");
    assert_eq!(output.matches[0].tier, MatchTier::Structural);
    assert_eq!(output.matches[0].base, callable_ref("A.java", 0));
    assert_eq!(output.matches[0].candidate, callable_ref("A.java", 0));
    assert!(
        !output
            .matches
            .iter()
            .any(|m| m.base == callable_ref("B.java", 0)),
        "B.java's y must stay unmatched, not steal x's candidate: {output:#?}"
    );
    assert!(output.ambiguities.is_empty());
}

// ---------------------------------------------------------------------
// Ambiguity: a tier-3 fingerprint bucket with more than one leftover on
// either side.
// ---------------------------------------------------------------------

/// Two pairs of identical getters, one on each side: 2 base and 2 candidate
/// leftovers sharing one fingerprint give one `Ambiguity` and zero matches
/// for that fingerprint. All four identities are kept distinct so tiers 1-2
/// cannot place any of them first.
#[test]
fn test_exact_body_ambiguity_stays_unmatched() {
    let base = vec![synth_file(
        "A.java",
        &[
            (synth_identity("a1", &[]), "blake3:dup"),
            (synth_identity("a2", &[]), "blake3:dup"),
        ],
    )];
    let candidate = vec![synth_file(
        "A.java",
        &[
            (synth_identity("c1", &[]), "blake3:dup"),
            (synth_identity("c2", &[]), "blake3:dup"),
        ],
    )];

    let output = match_callables(&base, &candidate, &[]);

    assert!(output.matches.is_empty(), "{output:#?}");
    assert_eq!(output.ambiguities.len(), 1, "{output:#?}");
    let ambiguity = &output.ambiguities[0];
    assert_eq!(ambiguity.fingerprint, "blake3:dup");
    assert_eq!(
        ambiguity.base,
        vec![callable_ref("A.java", 0), callable_ref("A.java", 1)]
    );
    assert_eq!(
        ambiguity.candidate,
        vec![callable_ref("A.java", 0), callable_ref("A.java", 1)]
    );
}

/// Two identical getters added where one existed: a 1-base/2-candidate
/// fingerprint bucket gives one `Ambiguity` and zero matches, not a spurious
/// 1:1 pair with one candidate left over. The mirrored 2-base/1-candidate
/// case (one of two identical getters deleted) is asserted the same way.
#[test]
fn test_one_to_many_body_ambiguity_stays_unmatched() {
    let base = vec![synth_file(
        "A.java",
        &[(synth_identity("a1", &[]), "blake3:dup")],
    )];
    let candidate = vec![synth_file(
        "A.java",
        &[
            (synth_identity("c1", &[]), "blake3:dup"),
            (synth_identity("c2", &[]), "blake3:dup"),
        ],
    )];

    let output = match_callables(&base, &candidate, &[]);

    assert!(output.matches.is_empty(), "{output:#?}");
    assert_eq!(output.ambiguities.len(), 1, "{output:#?}");
    let ambiguity = &output.ambiguities[0];
    assert_eq!(ambiguity.fingerprint, "blake3:dup");
    assert_eq!(ambiguity.base, vec![callable_ref("A.java", 0)]);
    assert_eq!(
        ambiguity.candidate,
        vec![callable_ref("A.java", 0), callable_ref("A.java", 1)]
    );

    // Mirrored: 2 base, 1 candidate.
    let base = vec![synth_file(
        "A.java",
        &[
            (synth_identity("a1", &[]), "blake3:dup"),
            (synth_identity("a2", &[]), "blake3:dup"),
        ],
    )];
    let candidate = vec![synth_file(
        "A.java",
        &[(synth_identity("c1", &[]), "blake3:dup")],
    )];

    let output = match_callables(&base, &candidate, &[]);

    assert!(output.matches.is_empty(), "{output:#?}");
    assert_eq!(output.ambiguities.len(), 1, "{output:#?}");
    let ambiguity = &output.ambiguities[0];
    assert_eq!(ambiguity.fingerprint, "blake3:dup");
    assert_eq!(
        ambiguity.base,
        vec![callable_ref("A.java", 0), callable_ref("A.java", 1)]
    );
    assert_eq!(ambiguity.candidate, vec![callable_ref("A.java", 0)]);
}

/// 2,000 identical leftovers (1,000 base, 1,000 candidate, one distinct
/// identity per callable so tiers 1-2 place none of them) give one
/// `Ambiguity` group containing all of them, built from hash-keyed pools
/// rather than an O(n^2) comparison.
#[test]
fn test_many_identical_bodies_form_one_ambiguity_group() {
    const N: usize = 1000;
    let base: Vec<FileCallables> = (0..N)
        .map(|i| {
            synth_file(
                &format!("base{i}.java"),
                &[(synth_identity(&format!("base{i}"), &[]), "blake3:dup")],
            )
        })
        .collect();
    let candidate: Vec<FileCallables> = (0..N)
        .map(|i| {
            synth_file(
                &format!("candidate{i}.java"),
                &[(synth_identity(&format!("candidate{i}"), &[]), "blake3:dup")],
            )
        })
        .collect();

    let output = match_callables(&base, &candidate, &[]);

    assert!(output.matches.is_empty(), "{}", output.matches.len());
    assert_eq!(output.ambiguities.len(), 1, "{}", output.ambiguities.len());
    let ambiguity = &output.ambiguities[0];
    assert_eq!(ambiguity.fingerprint, "blake3:dup");
    assert_eq!(ambiguity.base.len(), N);
    assert_eq!(ambiguity.candidate.len(), N);
}

/// A deleted callable and an unrelated added callable both stay unmatched:
/// distinct identities and distinct fingerprints give tiers 1-3 nothing to
/// pair them on.
#[test]
fn test_deleted_and_added_callables_are_unmatched() {
    let base = vec![synth_file(
        "A.java",
        &[(synth_identity("deletedFn", &[]), "blake3:deleted-body")],
    )];
    let candidate = vec![synth_file(
        "A.java",
        &[(synth_identity("addedFn", &[]), "blake3:added-body")],
    )];

    let output = match_callables(&base, &candidate, &[]);

    assert!(output.matches.is_empty(), "{output:#?}");
    assert!(output.ambiguities.is_empty(), "{output:#?}");
}

/// Shuffled file order (base and candidate both reversed) gives an
/// identical `MatchOutput`: the spec's own sort by (path bytes, callable
/// index) makes the result independent of input order, across a structural
/// match, a cross-file tier-3 move, an ambiguity group, and a deletion all
/// at once.
#[test]
fn test_matching_is_deterministic_under_input_order() {
    let s = synth_identity("s", &[]);
    let m1 = synth_identity("m1", &[]);
    let m2 = synth_identity("m2", &[]);
    let q1 = synth_identity("q1", &[]);
    let q2 = synth_identity("q2", &[]);
    let q3 = synth_identity("q3", &[]);
    let q4 = synth_identity("q4", &[]);
    let d = synth_identity("d", &[]);

    let base = vec![
        synth_file("S.java", &[(s.clone(), "blake3:s")]),
        synth_file("M1.java", &[(m1.clone(), "blake3:move")]),
        synth_file(
            "Q.java",
            &[(q1.clone(), "blake3:dup"), (q2.clone(), "blake3:dup")],
        ),
        synth_file("D.java", &[(d.clone(), "blake3:deleted")]),
    ];
    let candidate = vec![
        synth_file("S.java", &[(s.clone(), "blake3:s")]),
        synth_file("M2.java", &[(m2.clone(), "blake3:move")]),
        synth_file(
            "Q.java",
            &[(q3.clone(), "blake3:dup"), (q4.clone(), "blake3:dup")],
        ),
    ];

    let forward = match_callables(&base, &candidate, &[]);

    let mut base_reversed = base.clone();
    base_reversed.reverse();
    let mut candidate_reversed = candidate.clone();
    candidate_reversed.reverse();
    let reversed = match_callables(&base_reversed, &candidate_reversed, &[]);

    assert_eq!(forward, reversed);
    // A sanity check that this fixture actually exercises every code path,
    // so the equality above is not vacuously true.
    assert_eq!(forward.matches.len(), 2, "{forward:#?}");
    assert_eq!(forward.ambiguities.len(), 1, "{forward:#?}");
    // The Output bullet's own sort by (path bytes, callable index): unsorted
    // insertion order for these matches is [S.java, M1.java] (S is matched in
    // tier 1's loop before the tier-3 pool sees M1's move), but "M1.java" <
    // "S.java" byte-wise.
    assert_eq!(
        forward
            .matches
            .iter()
            .map(|m| m.base.clone())
            .collect::<Vec<_>>(),
        vec![callable_ref("M1.java", 0), callable_ref("S.java", 0)]
    );
}

/// Two files' ambiguity lists are sorted by (path bytes, callable index),
/// not by insertion order: `Z.java` is matched against `A.java` with `Z`
/// inserted first on both sides.
#[test]
fn test_ambiguity_lists_are_sorted_by_path_and_index() {
    let base = vec![
        synth_file("Z.java", &[(synth_identity("z", &[]), "blake3:dup")]),
        synth_file("A.java", &[(synth_identity("a", &[]), "blake3:dup")]),
    ];
    let candidate = vec![
        synth_file("Z.java", &[(synth_identity("z2", &[]), "blake3:dup")]),
        synth_file("A.java", &[(synth_identity("a2", &[]), "blake3:dup")]),
    ];

    let output = match_callables(&base, &candidate, &[]);

    assert!(output.matches.is_empty(), "{output:#?}");
    assert_eq!(output.ambiguities.len(), 1, "{output:#?}");
    let ambiguity = &output.ambiguities[0];
    assert_eq!(
        ambiguity.base,
        vec![callable_ref("A.java", 0), callable_ref("Z.java", 0)]
    );
    assert_eq!(
        ambiguity.candidate,
        vec![callable_ref("A.java", 0), callable_ref("Z.java", 0)]
    );
}

// ---------------------------------------------------------------------
// A6: `match_callables` pins its own "paths are unique per side" assumption
// with a `debug_assert!` on the `HashMap` insert result.
// ---------------------------------------------------------------------

/// Two `FileCallables` entries at the same path on the base side trip the
/// `debug_assert!` on `base_by_path`'s own `insert` call -- only reachable
/// in a debug build (`debug_assert!` compiles away entirely in release).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "duplicate base path")]
fn test_duplicate_path_on_one_side_panics_in_debug() {
    let base = vec![
        synth_file("A.java", &[(synth_identity("a", &[]), "blake3:a")]),
        synth_file("A.java", &[(synth_identity("b", &[]), "blake3:b")]),
    ];
    let candidate = vec![synth_file(
        "A.java",
        &[(synth_identity("a", &[]), "blake3:a")],
    )];

    let _ = match_callables(&base, &candidate, &[]);
}

/// The candidate-side twin of the test above: nothing else in
/// `match_callables` reaches `candidate_by_path`'s own `insert` before tier
/// 1's path lookups, so without this test a duplicate candidate path (with
/// a unique base side) leaves the suite green even if the candidate-side
/// `debug_assert!` were deleted entirely.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "duplicate candidate path")]
fn test_duplicate_candidate_path_panics_in_debug() {
    let base = vec![synth_file(
        "A.java",
        &[(synth_identity("a", &[]), "blake3:a")],
    )];
    let candidate = vec![
        synth_file("A.java", &[(synth_identity("a", &[]), "blake3:a")]),
        synth_file("A.java", &[(synth_identity("b", &[]), "blake3:b")]),
    ];

    let _ = match_callables(&base, &candidate, &[]);
}

/// Within one same-key group, the fingerprint-equal pair and the pairs made
/// by the positional fallback carry different provenance.
#[test]
fn test_fingerprint_exact_and_positional_pairs_are_distinguished() {
    let callback = synth_identity("cb", &[]);
    let base = vec![synth_file(
        "A.java",
        &[
            (callback.clone(), "blake3:a"),
            (callback.clone(), "blake3:b"),
            (callback.clone(), "blake3:c"),
        ],
    )];
    let candidate = vec![synth_file(
        "A.java",
        &[
            (callback.clone(), "blake3:c"),
            (callback.clone(), "blake3:x"),
            (callback.clone(), "blake3:y"),
        ],
    )];

    let output = match_callables(&base, &candidate, &[]);

    let pairing_of = |base_index: usize| {
        output
            .matches
            .iter()
            .find(|m| m.base == callable_ref("A.java", base_index))
            .map(|m| m.pairing)
    };
    assert_eq!(output.matches.len(), 3, "{output:#?}");
    assert_eq!(pairing_of(2), Some(MatchPairing::FingerprintExact));
    assert_eq!(pairing_of(0), Some(MatchPairing::Positional));
    assert_eq!(pairing_of(1), Some(MatchPairing::Positional));
}

/// The callables a same-key group could not pair by fingerprint are exposed,
/// one remainder per group, sorted by (path bytes, index) of the first base
/// member; a group that fingerprint-paired completely has none.
#[test]
fn test_positional_remainders_are_reported() {
    let callback = synth_identity("cb", &[]);
    let exact = synth_identity("exact", &[]);
    let base = vec![
        synth_file(
            "Z.java",
            &[(callback.clone(), "blake3:z1"), (exact.clone(), "blake3:e")],
        ),
        synth_file(
            "A.java",
            &[
                (callback.clone(), "blake3:a1"),
                (callback.clone(), "blake3:a2"),
                (callback.clone(), "blake3:keep"),
            ],
        ),
    ];
    let candidate = vec![
        synth_file(
            "Z.java",
            &[(callback.clone(), "blake3:z2"), (exact.clone(), "blake3:e")],
        ),
        synth_file(
            "A.java",
            &[
                (callback.clone(), "blake3:keep"),
                (callback.clone(), "blake3:a3"),
                (callback.clone(), "blake3:a4"),
            ],
        ),
    ];

    let output = match_callables(&base, &candidate, &[]);

    assert_eq!(
        output.positional_remainders,
        vec![
            PositionalRemainder {
                base: vec![callable_ref("A.java", 0), callable_ref("A.java", 1)],
                candidate: vec![callable_ref("A.java", 1), callable_ref("A.java", 2)],
            },
            PositionalRemainder {
                base: vec![callable_ref("Z.java", 0)],
                candidate: vec![callable_ref("Z.java", 0)],
            },
        ]
    );
}

/// (base ref, candidate fingerprint) for every match, in output order.
fn match_summary(
    output: &nsd::identity::matching::MatchOutput,
    candidate: &[FileCallables],
) -> Vec<(CallableRef, String, MatchTier)> {
    output
        .matches
        .iter()
        .map(|m| {
            let file = candidate
                .iter()
                .find(|file| file.path == m.candidate.path)
                .expect("candidate file");
            (
                m.base.clone(),
                file.callables[m.candidate.index].1.clone(),
                m.tier,
            )
        })
        .collect()
}

/// A candidate positionally paired in a non-1:1 same-key group, whose body
/// equals a base left unmatched elsewhere, takes that tier-3 pair instead; the
/// result does not depend on the candidate's source order.
#[test]
fn test_positional_candidate_takes_its_tier3_base_in_either_order() {
    let callback = synth_identity("cb", &[]);
    let moved = synth_identity("moved", &[]);
    let base = vec![
        synth_file("A.java", &[(callback.clone(), "blake3:x")]),
        synth_file("B.java", &[(moved.clone(), "blake3:y")]),
    ];
    let x_first = vec![synth_file(
        "A.java",
        &[
            (callback.clone(), "blake3:x2"),
            (callback.clone(), "blake3:y"),
        ],
    )];
    let y_first = vec![synth_file(
        "A.java",
        &[
            (callback.clone(), "blake3:y"),
            (callback.clone(), "blake3:x2"),
        ],
    )];

    let from_x_first = match_callables(&base, &x_first, &[]);
    let from_y_first = match_callables(&base, &y_first, &[]);

    let expected = vec![
        (
            callable_ref("A.java", 0),
            "blake3:x2".to_string(),
            MatchTier::Structural,
        ),
        (
            callable_ref("B.java", 0),
            "blake3:y".to_string(),
            MatchTier::BodyFingerprint,
        ),
    ];
    assert_eq!(match_summary(&from_x_first, &x_first), expected);
    assert_eq!(match_summary(&from_y_first, &y_first), expected);
    assert!(from_y_first.ambiguities.is_empty(), "{from_y_first:#?}");
}

/// Tier 1 pairs that are not positional fallback are never displaced by a
/// same-fingerprint base elsewhere: a fingerprint-exact same-key pair, and a
/// 1:1 same-key edit whose new body equals another base's body.
#[test]
fn test_key_pairs_are_never_displaced_by_a_tier3_fingerprint() {
    let keyed = synth_identity("m", &[]);
    let other = synth_identity("n", &[]);
    let base = vec![
        synth_file("A.java", &[(keyed.clone(), "blake3:f")]),
        synth_file("B.java", &[(other.clone(), "blake3:f")]),
    ];
    let exact = vec![synth_file("A.java", &[(keyed.clone(), "blake3:f")])];
    let edit_base = vec![
        synth_file("A.java", &[(keyed.clone(), "blake3:old")]),
        synth_file("B.java", &[(other.clone(), "blake3:f")]),
    ];
    let edited = vec![synth_file("A.java", &[(keyed.clone(), "blake3:f")])];

    let exact_output = match_callables(&base, &exact, &[]);
    let edited_output = match_callables(&edit_base, &edited, &[]);

    assert_eq!(
        match_summary(&exact_output, &exact),
        vec![(
            callable_ref("A.java", 0),
            "blake3:f".to_string(),
            MatchTier::Structural
        )]
    );
    assert_eq!(
        match_summary(&edited_output, &edited),
        vec![(
            callable_ref("A.java", 0),
            "blake3:f".to_string(),
            MatchTier::Structural
        )]
    );
}
