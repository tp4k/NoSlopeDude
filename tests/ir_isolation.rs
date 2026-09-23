//! WS-6: the lowering-isolation freeze (`nsd-plan-final.md`'s *IR
//! conformance* -> *Lowering isolation* bullet): "no symbol under `src/ir/`
//! or any analyzer names a grammar node kind (enforced by a source-level
//! test)". A leaked node-kind string would silently re-couple a metric to
//! the grammar and quietly restore the re-baseline cost D-IR exists to
//! remove. `src/lower/` (`java.rs`/`jsts.rs`) is the one exempt directory --
//! it is the sole place a `node.kind()` match is allowed to live.
//!
//! The vocabulary this scan checks against is not hand-copied: it is read
//! straight off the three pinned grammars via `tree_sitter::Language`'s own
//! `node_kind_for_id`, so it can never drift stale against the real
//! lowering (D20's four grammars: Java, JavaScript, TypeScript, TSX).
//!
//! Only actual Rust string-literal tokens are checked, not doc-comment
//! prose that happens to mention a node-kind word in quotes (several `///`
//! comments in the scanned files do exactly that, e.g.
//! `src/rules/mod.rs`'s own doc comment naming `"block"`/`"program"`) --
//! a full lexer is overkill here, so this scan uses the one heuristic that
//! holds across every file it scans: a line whose first non-whitespace
//! characters are `//` is comment-only and contributes no code.

use std::fs;
use std::path::{Path, PathBuf};

use tree_sitter::Language;

/// The four grammars D20 pins, mirroring `src/parse/mod.rs::language_for`.
fn grammar_languages() -> Vec<Language> {
    vec![
        tree_sitter_java::LANGUAGE.into(),
        tree_sitter_javascript::LANGUAGE.into(),
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        tree_sitter_typescript::LANGUAGE_TSX.into(),
    ]
}

/// Every node-kind string any of the four grammars actually defines,
/// harvested straight from the grammars via `Language::node_kind_for_id`
/// rather than hand-copied.
fn known_grammar_node_kinds() -> Vec<String> {
    let mut kinds = Vec::new();
    for language in grammar_languages() {
        for id in 0..language.node_kind_count() as u16 {
            if let Some(kind) = language.node_kind_for_id(id) {
                if !kind.is_empty() {
                    kinds.push(kind.to_string());
                }
            }
        }
    }
    kinds.sort();
    kinds.dedup();
    kinds
}

/// Every double-quoted Rust string-literal token in `text`, skipping any
/// line that is comment-only (see the module doc comment above). Good
/// enough for this crate's own formatting (`cargo fmt` never places a
/// string literal on a line whose only other content is a `//` comment
/// opener), without pulling in a full Rust lexer.
fn string_literals(text: &str) -> Vec<String> {
    let mut literals = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '"' {
                continue;
            }
            let mut literal = String::new();
            let mut closed = false;
            while let Some(next) = chars.next() {
                if next == '\\' {
                    // Skip one escaped character (e.g. `\"`); irrelevant
                    // for matching plain snake_case node-kind literals,
                    // which never contain an escape.
                    chars.next();
                    continue;
                }
                if next == '"' {
                    closed = true;
                    break;
                }
                literal.push(next);
            }
            if closed {
                literals.push(literal);
            }
        }
    }
    literals
}

/// Every `.rs` file directly inside `dir` (none of the four scanned
/// directories currently nest submodules beyond their own `mod.rs`, but a
/// walk keeps this from silently going blind if one gains a sibling file).
fn rust_files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = fs::read_dir(&current)
            .unwrap_or_else(|error| panic!("read_dir {}: {error}", current.display()));
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files
}

/// Every string literal, from every scanned file, that exactly matches a
/// known grammar node kind -- the isolation violation this freeze forbids.
fn violations_in(dir: &Path, vocabulary: &[String]) -> Vec<(PathBuf, String)> {
    let mut violations = Vec::new();
    for file in rust_files_under(dir) {
        let text = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        for literal in string_literals(&text) {
            if vocabulary.iter().any(|kind| kind == &literal) {
                violations.push((file.clone(), literal));
            }
        }
    }
    violations
}

/// The freeze itself: none of the four analyzer-facing directories name a
/// real grammar node kind as a string literal. `src/lower/` is exempt --
/// it is the one place these literals are allowed to live.
#[test]
fn test_no_analyzer_names_a_grammar_node_kind() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vocabulary = known_grammar_node_kinds();
    assert!(
        !vocabulary.is_empty(),
        "the harvested grammar vocabulary should not be empty"
    );

    let mut violations = Vec::new();
    for scanned in ["src/ir", "src/metrics", "src/clones", "src/rules"] {
        violations.extend(violations_in(&root.join(scanned), &vocabulary));
    }

    assert!(
        violations.is_empty(),
        "grammar node-kind literal(s) leaked outside src/lower/: {violations:?}"
    );
}

/// The scan above is discriminating, not vacuously green: a synthetic
/// source string carrying a real grammar node-kind literal in actual code
/// (not a comment) is detected, and the same word inside a comment-only
/// line is correctly ignored.
#[test]
fn test_the_scan_catches_a_planted_grammar_string() {
    let vocabulary = known_grammar_node_kinds();
    assert!(
        vocabulary.iter().any(|kind| kind == "if_statement"),
        "`if_statement` is expected to be a real node kind in at least one pinned grammar"
    );

    let planted_in_code = "fn planted(kind: &str) -> bool {\n    kind == \"if_statement\"\n}\n";
    let code_literals = string_literals(planted_in_code);
    assert!(
        code_literals
            .iter()
            .any(|literal| vocabulary.contains(literal)),
        "a grammar node-kind literal planted in real code should be caught: {code_literals:?}"
    );

    let planted_in_comment =
        "/// mentions \"if_statement\" only in prose, never in code\nfn clean() {}\n";
    let comment_literals = string_literals(planted_in_comment);
    assert!(
        !comment_literals
            .iter()
            .any(|literal| vocabulary.contains(literal)),
        "a node-kind word inside a comment-only line must not be flagged: {comment_literals:?}"
    );

    // Positive control on the real file walk itself: `rust_files_under`
    // returning `Vec::new()` unconditionally (e.g. a broken directory-walk
    // refactor) would leave `test_no_analyzer_names_a_grammar_node_kind`
    // vacuously green, since `violations_in` would have nothing to scan.
    // Pin that every one of the four analyzer-facing directories actually
    // yields files, and that `src/lower/` -- the one directory these
    // literals are allowed to live in -- actually contains at least one
    // real grammar node-kind literal, so the walk is exercising real
    // production code, not an empty directory.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for scanned in ["src/ir", "src/metrics", "src/clones", "src/rules"] {
        let files = rust_files_under(&root.join(scanned));
        assert!(
            !files.is_empty(),
            "rust_files_under({scanned:?}) found no files -- the walk itself is broken"
        );
    }
    let lower_violations = violations_in(&root.join("src/lower"), &vocabulary);
    assert!(
        !lower_violations.is_empty(),
        "src/lower/ is expected to contain at least one real grammar node-kind literal \
         (it is the one exempt directory) -- finding none means violations_in itself is \
         broken, not that src/lower/ became clean"
    );
}
