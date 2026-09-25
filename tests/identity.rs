//! Integration tests for M1-7's line-independent callable identity
//! (`nsd-plan-final.md` *M1-M2*, `docs/implementation-status.md` row M1-7).
//!
//! Every source here is inline (`parse_inline`, mirroring
//! `tests/ir_lowering.rs`'s own helper of the same name), never a file under
//! `tests/fixtures/`: `tests/neutrality.rs::all_fixture_relative_paths` walks
//! the *whole* `tests/fixtures/` tree as its clean corpus and compares the
//! rendered `report.json` byte-for-byte against a committed baseline
//! (`tests/golden/neutrality/clean.report.json`) -- adding a parseable
//! source file there would grow that corpus and move the baseline, which
//! this stream's scope forbids touching. `tests/ir_lowering.rs::parse_inline`
//! predates this file for exactly this reason (see its own doc comment).

use std::collections::HashSet;
use std::path::PathBuf;

use nsd::identity::{self, CallableIdentity};
use nsd::ir::{CallableKind, IrCallable, OwnerKind};
use nsd::lower::{self, IrFile};
use nsd::model::{Grammar, LanguageFamily};
use nsd::parse::ParsedFile;

const JAVA: LanguageFamily = LanguageFamily::Java;
const JS_TS: LanguageFamily = LanguageFamily::JsTs;

/// Parses `source` directly with tree-sitter under `grammar`, bypassing
/// `parse::parse_all` and any file on disk -- see this file's own doc
/// comment for why every test here uses an inline source.
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

fn lower_java(source: &str) -> IrFile {
    let parsed = parse_inline(source, Grammar::Java, JAVA);
    lower::lower_file(&parsed)
}

fn lower_ts(source: &str) -> IrFile {
    let parsed = parse_inline(source, Grammar::TypeScript, JS_TS);
    lower::lower_file(&parsed)
}

fn callable_at_line(ir_file: &IrFile, start_line: u32) -> &IrCallable {
    ir_file
        .callables
        .iter()
        .find(|callable| callable.span.start_line == start_line)
        .unwrap_or_else(|| panic!("no callable at line {start_line}: {:#?}", ir_file.callables))
}

/// M1-7's own regression: inserting a named callable and blank lines above
/// an anonymous callable must move its display name (still line-embedded)
/// but leave its `CallableIdentity` and body fingerprint untouched --
/// the exact case `<anonymous>@<line>` alone gets wrong (`nsd-plan-final.md`
/// *M1-M2*: "the current form makes every insertion above an anonymous
/// callable look like delete+add").
#[test]
fn test_insertion_above_an_anonymous_callable_keeps_its_identity() {
    let java_before = "\
class C {
    void run() {
        java.util.List<Runnable> list = new java.util.ArrayList<>();
        list.add(() -> {
            System.out.println(\"hi\");
        });
    }
}
";
    let java_after = "\
class C {
    void named() {}



    void run() {
        java.util.List<Runnable> list = new java.util.ArrayList<>();
        list.add(() -> {
            System.out.println(\"hi\");
        });
    }
}
";
    let before_parsed = parse_inline(java_before, Grammar::Java, JAVA);
    let before = lower::lower_file(&before_parsed);
    let after_parsed = parse_inline(java_after, Grammar::Java, JAVA);
    let after = lower::lower_file(&after_parsed);

    let before_lambda = callable_at_line(&before, 4);
    let after_lambda = callable_at_line(&after, 8);
    assert_eq!(before_lambda.name, "<anonymous>@4");
    assert_eq!(after_lambda.name, "<anonymous>@8");
    assert_eq!(
        identity::callable_identity(&before, before_lambda),
        identity::callable_identity(&after, after_lambda)
    );
    assert_eq!(
        identity::body_fingerprint(&before, before_lambda, &before_parsed.source),
        identity::body_fingerprint(&after, after_lambda, &after_parsed.source)
    );

    let js_before = "\
function run() {
    var list = [];
    (function () {
        console.log(\"hi\");
    })();
}
";
    let js_after = "\
function named() {}



function run() {
    var list = [];
    (function () {
        console.log(\"hi\");
    })();
}
";
    let before_parsed = parse_inline(js_before, Grammar::JavaScript, JS_TS);
    let before = lower::lower_file(&before_parsed);
    let after_parsed = parse_inline(js_after, Grammar::JavaScript, JS_TS);
    let after = lower::lower_file(&after_parsed);

    let before_iife = callable_at_line(&before, 3);
    let after_iife = callable_at_line(&after, 7);
    assert_eq!(before_iife.name, "<anonymous>@3");
    assert_eq!(after_iife.name, "<anonymous>@7");
    assert_eq!(
        identity::callable_identity(&before, before_iife),
        identity::callable_identity(&after, after_iife)
    );
    assert_eq!(
        identity::body_fingerprint(&before, before_iife, &before_parsed.source),
        identity::body_fingerprint(&after, after_iife, &after_parsed.source)
    );
}

/// Plan decision 4: same-key siblings share an identity with no ordinal, but
/// two overloads (differing signatures) must not -- and each overload keeps
/// its own identity regardless of declaration order.
#[test]
fn test_overloads_get_distinct_identities() {
    let int_first = "\
class Overloads {
    void f(int a) {
        System.out.println(a);
    }

    void f(String a) {
        System.out.println(a);
    }
}
";
    let ir_file = lower_java(int_first);
    let f_int = callable_at_line(&ir_file, 2);
    let f_string = callable_at_line(&ir_file, 6);
    let identity_int = identity::callable_identity(&ir_file, f_int);
    let identity_string = identity::callable_identity(&ir_file, f_string);
    assert_ne!(identity_int, identity_string);
    assert_eq!(identity_int.signature, vec!["int".to_string()]);
    assert_eq!(identity_string.signature, vec!["String".to_string()]);

    let string_first = "\
class Overloads {
    void f(String a) {
        System.out.println(a);
    }

    void f(int a) {
        System.out.println(a);
    }
}
";
    let swapped = lower_java(string_first);
    let swapped_string = callable_at_line(&swapped, 2);
    let swapped_int = callable_at_line(&swapped, 6);
    assert_eq!(
        identity::callable_identity(&swapped, swapped_int),
        identity_int
    );
    assert_eq!(
        identity::callable_identity(&swapped, swapped_string),
        identity_string
    );

    // WS-1 triage row 4 (round 2): `f(int)` and a varargs overload
    // `f(int... a)` must not share a signature -- the `...` used to be
    // dropped from a `spread_parameter`'s own `type` field text.
    let varargs_source = "\
class Overloads {
    void h(int a) {
        System.out.println(a);
    }

    void h(int... a) {
        System.out.println(a.length);
    }
}
";
    let varargs_ir = lower_java(varargs_source);
    let h_fixed = callable_at_line(&varargs_ir, 2);
    let h_varargs = callable_at_line(&varargs_ir, 6);
    let h_fixed_identity = identity::callable_identity(&varargs_ir, h_fixed);
    let h_varargs_identity = identity::callable_identity(&varargs_ir, h_varargs);
    assert_ne!(h_fixed_identity, h_varargs_identity);
    assert_eq!(h_fixed_identity.signature, vec!["int".to_string()]);
    assert_eq!(h_varargs_identity.signature, vec!["int...".to_string()]);

    // The C-style array-parameter spelling `int x[]` must give the same
    // signature shape as `int[] x` -- the `dimensions` field used to be
    // dropped entirely.
    let array_dims_source = "\
class Overloads {
    void g(int x[]) {
        System.out.println(x.length);
    }
}
";
    let array_dims_ir = lower_java(array_dims_source);
    let g = callable_at_line(&array_dims_ir, 2);
    let g_identity = identity::callable_identity(&array_dims_ir, g);
    assert_eq!(g_identity.signature, vec!["int[]".to_string()]);
}

/// A method in an inner class and a lambda inside a method each carry
/// distinct owner segments, and the same method name in two sibling classes
/// gives two distinct identities (owner chain, not just kind/name/signature,
/// separates them).
#[test]
fn test_nested_callables_carry_their_owner_chain() {
    let source = "\
class Outer {
    class Inner {
        void method() {
            System.out.println(\"inner\");
        }
    }

    void method() {
        java.util.List<Runnable> list = new java.util.ArrayList<>();
        list.add(() -> {
            System.out.println(\"lambda\");
        });
    }
}

class Sibling {
    void method() {
        System.out.println(\"sibling\");
    }
}
";
    let ir_file = lower_java(source);
    let inner_method = callable_at_line(&ir_file, 3);
    let outer_method = callable_at_line(&ir_file, 8);
    let lambda = callable_at_line(&ir_file, 10);
    let sibling_method = callable_at_line(&ir_file, 17);

    let inner_identity = identity::callable_identity(&ir_file, inner_method);
    let outer_identity = identity::callable_identity(&ir_file, outer_method);
    let lambda_identity = identity::callable_identity(&ir_file, lambda);
    let sibling_identity = identity::callable_identity(&ir_file, sibling_method);

    assert_eq!(
        inner_identity.owner_chain,
        vec![owner_named_type("Outer"), owner_named_type("Inner"),]
    );
    assert_eq!(outer_identity.owner_chain, vec![owner_named_type("Outer")]);
    assert_eq!(
        lambda_identity.owner_chain,
        vec![owner_named_type("Outer"), owner_callable("method")]
    );
    assert_eq!(
        sibling_identity.owner_chain,
        vec![owner_named_type("Sibling")]
    );

    assert_ne!(inner_identity, outer_identity, "distinct owner segments");
    assert_ne!(inner_identity, lambda_identity, "distinct owner segments");
    assert_ne!(
        outer_identity, sibling_identity,
        "same kind/name/signature, distinct owner chain"
    );
}

fn owner_named_type(name: &str) -> nsd::ir::OwnerSegment {
    nsd::ir::OwnerSegment {
        kind: OwnerKind::NamedType,
        name: Some(name.to_string()),
    }
}

fn owner_callable(name: &str) -> nsd::ir::OwnerSegment {
    nsd::ir::OwnerSegment {
        kind: OwnerKind::Callable,
        name: Some(name.to_string()),
    }
}

/// Two anonymous callbacks in one owner share one `CallableIdentity` (no
/// ordinal), with different body fingerprints since their bodies differ.
/// Inserting a third same-key callback before both leaves both unchanged.
#[test]
fn test_same_key_siblings_share_an_identity_without_an_ordinal() {
    let two = "\
class Callbacks {
    void run() {
        java.util.List<Runnable> list = new java.util.ArrayList<>();
        list.add(() -> {
            System.out.println(\"first\");
        });
        list.add(() -> {
            System.out.println(\"second\");
        });
    }
}
";
    let two_parsed = parse_inline(two, Grammar::Java, JAVA);
    let two_ir = lower::lower_file(&two_parsed);
    let first = callable_at_line(&two_ir, 4);
    let second = callable_at_line(&two_ir, 7);
    let first_identity = identity::callable_identity(&two_ir, first);
    let second_identity = identity::callable_identity(&two_ir, second);
    assert_eq!(first_identity, second_identity);
    let first_fingerprint = identity::body_fingerprint(&two_ir, first, &two_parsed.source);
    let second_fingerprint = identity::body_fingerprint(&two_ir, second, &two_parsed.source);
    assert_ne!(first_fingerprint, second_fingerprint);

    let three = "\
class Callbacks {
    void run() {
        java.util.List<Runnable> list = new java.util.ArrayList<>();
        list.add(() -> {
            System.out.println(\"zero\");
        });
        list.add(() -> {
            System.out.println(\"first\");
        });
        list.add(() -> {
            System.out.println(\"second\");
        });
    }
}
";
    let three_parsed = parse_inline(three, Grammar::Java, JAVA);
    let three_ir = lower::lower_file(&three_parsed);
    let zero = callable_at_line(&three_ir, 4);
    let first_after = callable_at_line(&three_ir, 7);
    let second_after = callable_at_line(&three_ir, 10);

    for callable in [zero, first_after, second_after] {
        assert_eq!(
            identity::callable_identity(&three_ir, callable),
            first_identity
        );
    }
    assert_eq!(
        identity::body_fingerprint(&three_ir, first_after, &three_parsed.source),
        first_fingerprint,
        "the 'first' callback's own fingerprint is unaffected by the insertion before it"
    );
    assert_eq!(
        identity::body_fingerprint(&three_ir, second_after, &three_parsed.source),
        second_fingerprint,
        "the 'second' callback's own fingerprint is unaffected by the insertion before it"
    );
}

/// Every Java and JS/TS callable-kind grammar node `src/lower/java.rs`/
/// `jsts.rs` recognize maps to a distinct `CallableKind` -- TS covered via a
/// TypeScript-grammar inline source (`parse_inline(.., Grammar::TypeScript,
/// ..)`), not a `.ts` file under `tests/fixtures/` (see this file's own doc
/// comment for why).
#[test]
fn test_every_callable_kind_is_classified() {
    let java_source = "\
class AllKinds {
    static {
        int a = 1;
    }

    AllKinds() {
        int a = 1;
    }

    void method() {
        java.util.List<Runnable> list = new java.util.ArrayList<>();
        list.add(() -> {
            int b = 1;
        });
    }
}

record Point(int x, int y) {
    Point {
        if (x < 0) {
            x = 0;
        }
    }
}
";
    let java_ir = lower_java(java_source);
    let java_kinds: HashSet<CallableKind> = java_ir
        .callables
        .iter()
        .map(|callable| callable.kind)
        .collect();
    let expected_java: HashSet<CallableKind> = [
        CallableKind::JavaMethod,
        CallableKind::JavaConstructor,
        CallableKind::JavaCompactConstructor,
        CallableKind::JavaStaticInitializer,
        CallableKind::JavaLambda,
    ]
    .into_iter()
    .collect();
    assert_eq!(java_kinds, expected_java, "{:#?}", java_ir.callables);

    // WS-1 triage row 6 (round 2): the set-equality assert above would still
    // pass if two grammar nodes swapped kinds -- pin each callable's kind to
    // its own declaration line too.
    assert_eq!(
        callable_at_line(&java_ir, 2).kind,
        CallableKind::JavaStaticInitializer
    );
    assert_eq!(
        callable_at_line(&java_ir, 6).kind,
        CallableKind::JavaConstructor
    );
    assert_eq!(
        callable_at_line(&java_ir, 10).kind,
        CallableKind::JavaMethod
    );
    assert_eq!(
        callable_at_line(&java_ir, 12).kind,
        CallableKind::JavaLambda
    );
    assert_eq!(
        callable_at_line(&java_ir, 19).kind,
        CallableKind::JavaCompactConstructor
    );

    let ts_source = "\
function plain() {
    return 1;
}

function* gen() {
    yield 1;
}

const expr = function () {
    return 2;
};

const arrow = () => 3;

class C {
    method() {
        return 4;
    }
}
";
    let ts_ir = lower_ts(ts_source);
    let ts_kinds: HashSet<CallableKind> = ts_ir
        .callables
        .iter()
        .map(|callable| callable.kind)
        .collect();
    let expected_ts: HashSet<CallableKind> = [
        CallableKind::JsFunctionDeclaration,
        CallableKind::JsGeneratorFunctionDeclaration,
        CallableKind::JsFunctionExpression,
        CallableKind::JsArrowFunction,
        CallableKind::JsMethodDefinition,
    ]
    .into_iter()
    .collect();
    assert_eq!(ts_kinds, expected_ts, "{:#?}", ts_ir.callables);

    // Same per-line pinning as the Java half above.
    assert_eq!(
        callable_at_line(&ts_ir, 1).kind,
        CallableKind::JsFunctionDeclaration
    );
    assert_eq!(
        callable_at_line(&ts_ir, 5).kind,
        CallableKind::JsGeneratorFunctionDeclaration
    );
    assert_eq!(
        callable_at_line(&ts_ir, 9).kind,
        CallableKind::JsFunctionExpression
    );
    assert_eq!(
        callable_at_line(&ts_ir, 13).kind,
        CallableKind::JsArrowFunction
    );
    assert_eq!(
        callable_at_line(&ts_ir, 16).kind,
        CallableKind::JsMethodDefinition
    );
}

fn is_well_formed_blake3(fingerprint: &str) -> bool {
    let Some(hex) = fingerprint.strip_prefix("blake3:") else {
        return false;
    };
    hex.len() == 32
        && hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// The body fingerprint normalizes the same way `clones::ir_statement_tokens`
/// does (plan decision 5): reformatting, commenting and moving a body all
/// leave its fingerprint unchanged; one changed token changes it. The format
/// is `blake3:<32 lowercase hex>`.
#[test]
fn test_body_fingerprint_ignores_whitespace_comments_and_position() {
    let original = "\
class A {
    void m() {
        int x = 1;
        return;
    }
}
";
    let reformatted_commented_and_moved = "\
class Pad {
    void pad() {}

    void m() {
        // a comment that was not here before
        int   x   =   1;

        return;
    }
}
";
    let changed_token = "\
class A {
    void m() {
        int x = 2;
        return;
    }
}
";

    let original_parsed = parse_inline(original, Grammar::Java, JAVA);
    let original_ir = lower::lower_file(&original_parsed);
    let original_callable = callable_at_line(&original_ir, 2);
    let original_fingerprint =
        identity::body_fingerprint(&original_ir, original_callable, &original_parsed.source);
    assert!(
        is_well_formed_blake3(&original_fingerprint),
        "{original_fingerprint}"
    );

    let moved_parsed = parse_inline(reformatted_commented_and_moved, Grammar::Java, JAVA);
    let moved_ir = lower::lower_file(&moved_parsed);
    let moved_callable = callable_at_line(&moved_ir, 4);
    let moved_fingerprint =
        identity::body_fingerprint(&moved_ir, moved_callable, &moved_parsed.source);
    assert_eq!(original_fingerprint, moved_fingerprint);

    let changed_parsed = parse_inline(changed_token, Grammar::Java, JAVA);
    let changed_ir = lower::lower_file(&changed_parsed);
    let changed_callable = callable_at_line(&changed_ir, 2);
    let changed_fingerprint =
        identity::body_fingerprint(&changed_ir, changed_callable, &changed_parsed.source);
    assert_ne!(original_fingerprint, changed_fingerprint);
}

/// Plan decision 5's own family-prefix half: an identical body text in a
/// Java file and a JS file must not collide -- the fingerprint is hashed
/// under a language-specific family prefix, not just the raw token text.
#[test]
fn test_body_fingerprint_separates_languages() {
    let java_source = "class A { int m() { return 1; } }";
    let java_parsed = parse_inline(java_source, Grammar::Java, JAVA);
    let java_ir = lower::lower_file(&java_parsed);
    let java_callable = &java_ir.callables[0];
    let java_fingerprint = identity::body_fingerprint(&java_ir, java_callable, &java_parsed.source);

    let js_source = "function m() { return 1; }";
    let js_parsed = parse_inline(js_source, Grammar::JavaScript, JS_TS);
    let js_ir = lower::lower_file(&js_parsed);
    let js_callable = &js_ir.callables[0];
    let js_fingerprint = identity::body_fingerprint(&js_ir, js_callable, &js_parsed.source);

    assert_ne!(java_fingerprint, js_fingerprint);
}

/// `identity::identities` is a pure function of an already-lowered
/// `&IrFile`: two independent lowerings of the same source produce identical
/// identity vectors.
#[test]
fn test_identity_is_deterministic_across_runs() {
    let source = "\
class Determinism {
    void a() {
        System.out.println(\"a\");
    }

    void b(int x) {
        System.out.println(x);
    }
}
";
    let first_parsed = parse_inline(source, Grammar::Java, JAVA);
    let first_ir = lower::lower_file(&first_parsed);
    let second_parsed = parse_inline(source, Grammar::Java, JAVA);
    let second_ir = lower::lower_file(&second_parsed);

    let first_identities: Vec<CallableIdentity> = identity::identities(&first_ir);
    let second_identities: Vec<CallableIdentity> = identity::identities(&second_ir);
    assert_eq!(first_identities, second_identities);
    assert_eq!(first_identities.len(), 2, "{first_identities:#?}");
}

/// WS-1 triage row 1 (round 2, security+perf HIGH): the owner table stores
/// one `OwnerEntry` per nesting level, not a deep-copied `Vec<OwnerSegment>`
/// per callable -- a 15,000-level nested `()=>` chain (the same depth
/// `tests/metrics.rs::DEEP_NESTING_LEVELS` already exercises for the pre-M1-7
/// tree build) must give exactly one owner entry per callable, not one entry
/// times its own nesting depth.
#[test]
fn test_deeply_nested_callables_store_owners_linearly() {
    const DEPTH: usize = 15_000;
    let source = format!("const f = {}0;", "()=>".repeat(DEPTH));
    let parsed = parse_inline(&source, Grammar::TypeScript, JS_TS);
    let ir_file = lower::lower_file(&parsed);
    assert_eq!(ir_file.callables.len(), DEPTH);
    assert_eq!(ir_file.owners.len(), DEPTH);
}

/// WS-1 triage row 1 (round 2): a giant declared name shared by many sibling
/// callables must be stored exactly once in the owner table, not once per
/// sibling -- the concrete DoS shape the row's own memory estimate names (a
/// large owner name times many sibling callables). 2,000 anonymous arrow
/// statements share one 10,000-character enclosing function name; the owner
/// table holds exactly 1 (the function) + 2,000 (each arrow's own segment,
/// since every callable is itself a potential owner) entries -- never a
/// multiple of the name's own length -- and every arrow's materialized
/// `owner_chain` points back to that one shared entry.
#[test]
fn test_owner_segments_are_stored_once_per_owner() {
    const NAME_LEN: usize = 10_000;
    const ARROW_COUNT: usize = 2_000;
    let name = "A".repeat(NAME_LEN);
    let mut source = format!("function {name}() {{\n");
    for _ in 0..ARROW_COUNT {
        source.push_str("()=>0;\n");
    }
    source.push_str("}\n");

    let parsed = parse_inline(&source, Grammar::JavaScript, JS_TS);
    let ir_file = lower::lower_file(&parsed);

    assert_eq!(
        ir_file.owners.len(),
        ARROW_COUNT + 1,
        "{}",
        ir_file.owners.len()
    );

    let arrows: Vec<&IrCallable> = ir_file
        .callables
        .iter()
        .filter(|callable| callable.kind == CallableKind::JsArrowFunction)
        .collect();
    assert_eq!(arrows.len(), ARROW_COUNT);
    let expected_owner_chain = vec![owner_callable(&name)];
    for arrow in arrows {
        assert_eq!(
            identity::callable_identity(&ir_file, arrow).owner_chain,
            expected_owner_chain
        );
    }
}

fn owner_namespace(name: &str) -> nsd::ir::OwnerSegment {
    nsd::ir::OwnerSegment {
        kind: OwnerKind::Namespace,
        name: Some(name.to_string()),
    }
}

/// WS-1 triage row 2: `abstract class` and `module`/`namespace` owner
/// segments -- both used to fall through `owner_segment_for_type` entirely
/// and give an empty owner chain, so two abstract classes' same-named method
/// collided on identity, and `module M {}` gave no `Namespace` segment at
/// all.
#[test]
fn test_ts_abstract_class_and_namespace_owner_segments() {
    let source = "\
abstract class A {
    m() {
        return 1;
    }
}

abstract class B {
    m() {
        return 2;
    }
}

module M {
    function f() {
        return 3;
    }
}

namespace N {
    function f() {
        return 4;
    }
}

function f() {
    return 5;
}
";
    let ir_file = lower_ts(source);
    let a_m = callable_at_line(&ir_file, 2);
    let b_m = callable_at_line(&ir_file, 8);
    let module_f = callable_at_line(&ir_file, 14);
    let namespace_f = callable_at_line(&ir_file, 20);
    let top_level_f = callable_at_line(&ir_file, 25);

    let a_identity = identity::callable_identity(&ir_file, a_m);
    let b_identity = identity::callable_identity(&ir_file, b_m);
    assert_eq!(a_identity.owner_chain, vec![owner_named_type("A")]);
    assert_eq!(b_identity.owner_chain, vec![owner_named_type("B")]);
    assert_ne!(
        a_identity, b_identity,
        "distinct abstract-class owner chains"
    );

    let module_identity = identity::callable_identity(&ir_file, module_f);
    let namespace_identity = identity::callable_identity(&ir_file, namespace_f);
    let top_level_identity = identity::callable_identity(&ir_file, top_level_f);
    assert_eq!(module_identity.owner_chain, vec![owner_namespace("M")]);
    assert_eq!(namespace_identity.owner_chain, vec![owner_namespace("N")]);
    assert_eq!(top_level_identity.owner_chain, Vec::new());
    assert_ne!(module_identity, namespace_identity);
    assert_ne!(module_identity, top_level_identity);
    assert_ne!(namespace_identity, top_level_identity);

    // WS-1 triage row 1 (round 3): `owner_segment_for_type` must not run on
    // the anonymous `class`/`module` keyword leaves that share their node
    // kind string with the named `class_declaration`/`module` arms -- each
    // owner table below holds exactly one type/namespace entry plus one
    // callable entry, never a phantom third entry for the keyword leaf.
    assert_eq!(
        lower_ts("class A {\n    m() {\n        return 1;\n    }\n}\n")
            .owners
            .len(),
        2
    );
    assert_eq!(
        lower_ts("module M {\n    function f() {\n        return 3;\n    }\n}\n")
            .owners
            .len(),
        2
    );
}

fn owner_anonymous_class_body() -> nsd::ir::OwnerSegment {
    nsd::ir::OwnerSegment {
        kind: OwnerKind::AnonymousClassBody,
        name: None,
    }
}

/// WS-1 triage row 3: an enum constant's own body (`PLUS { … }`) is an
/// anonymous class body (JLS §8.9.1), not a member of the enum's own
/// `NamedType` -- `PLUS`'s overriding `apply` must carry a distinct owner
/// chain from the enum's own shared `apply`. A method nested inside an
/// `@interface` confirms `annotation_type_declaration` itself contributes a
/// `NamedType` segment (it used to be entirely absent from the NamedType
/// arm).
#[test]
fn test_java_owner_segments_cover_anonymous_bodies_and_annotation_types() {
    let enum_source = "\
enum Op {
    PLUS {
        int apply() {
            return 1;
        }
    },
    MINUS {
        int apply() {
            return 2;
        }
    };

    int apply() {
        return 0;
    }
}
";
    let enum_ir = lower_java(enum_source);
    let plus_apply = callable_at_line(&enum_ir, 3);
    let minus_apply = callable_at_line(&enum_ir, 8);
    let shared_apply = callable_at_line(&enum_ir, 13);

    let plus_identity = identity::callable_identity(&enum_ir, plus_apply);
    let minus_identity = identity::callable_identity(&enum_ir, minus_apply);
    let shared_identity = identity::callable_identity(&enum_ir, shared_apply);
    assert_eq!(
        plus_identity.owner_chain,
        vec![owner_named_type("Op"), owner_anonymous_class_body()]
    );
    assert_eq!(
        minus_identity.owner_chain,
        vec![owner_named_type("Op"), owner_anonymous_class_body()]
    );
    assert_eq!(shared_identity.owner_chain, vec![owner_named_type("Op")]);
    assert_ne!(
        plus_identity, shared_identity,
        "an enum constant's own body is an anonymous class body, not the enum's NamedType"
    );

    let anonymous_class_body_source = "\
class A {
    void m() {
        Runnable r = new Runnable() {
            public void run() {
            }
        };
    }
}
";
    let anonymous_ir = lower_java(anonymous_class_body_source);
    let run_method = callable_at_line(&anonymous_ir, 4);
    let run_identity = identity::callable_identity(&anonymous_ir, run_method);
    assert_eq!(
        run_identity.owner_chain,
        vec![
            owner_named_type("A"),
            owner_callable("m"),
            owner_anonymous_class_body(),
        ]
    );

    let annotation_source = "\
@interface Config {
    record Loader() {
        void run() {
        }
    }
}
";
    let annotation_ir = lower_java(annotation_source);
    let loader_run = callable_at_line(&annotation_ir, 3);
    let loader_run_identity = identity::callable_identity(&annotation_ir, loader_run);
    assert_eq!(
        loader_run_identity.owner_chain,
        vec![owner_named_type("Config"), owner_named_type("Loader")]
    );
}
