use std::path::PathBuf;

use nsd::model::{DiscoveredFile, LanguageFamily};
use nsd::parse::{self, ParsedFile};
use nsd::rules::{self, ALL_RULE_IDS};

const JAVA: LanguageFamily = LanguageFamily::Java;
const JS_TS: LanguageFamily = LanguageFamily::JsTs;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules")
}

fn parsed_files(paths: &[(&str, LanguageFamily)]) -> Vec<ParsedFile> {
    let root = fixture_root();
    let files: Vec<DiscoveredFile> = paths
        .iter()
        .map(|(path, language)| DiscoveredFile {
            relative_path: PathBuf::from(*path),
            language: *language,
        })
        .collect();
    let (parsed, failures) = parse::parse_all(&root, &files);
    assert!(
        failures.is_empty(),
        "unexpected parse failures: {failures:?}"
    );
    parsed
}

#[test]
fn test_each_java_rule_fires_once_on_its_fixture() {
    let files = parsed_files(&[("__tests__/JavaRulesFixture.java", JAVA)]);
    let findings = rules::find_findings(&files);

    // Exact (start_line, end_line, flagged_lines) per rule id, hand-counted
    // against __tests__/JavaRulesFixture.java: dropping the D11 filter, or
    // moving a finding to the wrong line, must fail one of these.
    let expected: [(&'static str, usize, usize, Vec<usize>); 3] = [
        (rules::JAVA_UNREACHABLE_AFTER_RETURN, 8, 9, vec![8, 9]),
        (rules::JAVA_EMPTY_CATCH, 15, 16, vec![15]),
        (rules::JAVA_REDUNDANT_ELSE_AFTER_RETURN, 22, 24, vec![23]),
    ];
    assert_eq!(findings.len(), expected.len(), "{findings:?}");
    for (rule_id, start_line, end_line, flagged_lines) in expected {
        let hits: Vec<_> = findings.iter().filter(|f| f.rule_id == rule_id).collect();
        assert_eq!(
            hits.len(),
            1,
            "{rule_id} should fire exactly once: {findings:?}"
        );
        let hit = hits[0];
        assert_eq!(hit.language, LanguageFamily::Java);
        assert!(hit.start_line > 0);
        assert!(hit.end_line >= hit.start_line);
        assert_eq!(
            hit.relative_path,
            PathBuf::from("__tests__/JavaRulesFixture.java")
        );
        assert_eq!(hit.start_line, start_line, "{rule_id}: {hit:?}");
        assert_eq!(hit.end_line, end_line, "{rule_id}: {hit:?}");
        assert_eq!(hit.flagged_lines, flagged_lines, "{rule_id}: {hit:?}");
    }
}

#[test]
fn test_each_jsts_rule_fires_once_on_its_fixture() {
    let files = parsed_files(&[("__tests__/JsRulesFixture.js", JS_TS)]);
    let findings = rules::find_findings(&files);

    // Exact (start_line, end_line, flagged_lines) per rule id, hand-counted
    // against __tests__/JsRulesFixture.js.
    let expected: [(&'static str, usize, usize, Vec<usize>); 3] = [
        (rules::JSTS_UNREACHABLE_AFTER_RETURN, 4, 5, vec![4, 5]),
        (rules::JSTS_EMPTY_CATCH, 11, 12, vec![11]),
        (rules::JSTS_REDUNDANT_ELSE_AFTER_RETURN, 18, 20, vec![19]),
    ];
    // JSTS_UNREACHABLE_AFTER_RETURN's second hit: `mixedExemptAndDead` mixes
    // an exempt hoisted `helper` declaration (line 27) with two genuinely
    // dead statements (lines 28-29). This pins the exemption's scope — it
    // drops only the exempt statements from the finding, not the whole
    // finding whenever any exempt statement is present; an over-broad
    // mutant doing the latter drops this hit entirely and fails the
    // assertions below.
    let second_unreachable_hit: (&'static str, usize, usize, Vec<usize>) =
        (rules::JSTS_UNREACHABLE_AFTER_RETURN, 28, 29, vec![28, 29]);

    assert_eq!(findings.len(), expected.len() + 1, "{findings:?}");
    for (rule_id, start_line, end_line, flagged_lines) in expected {
        let hits: Vec<_> = findings.iter().filter(|f| f.rule_id == rule_id).collect();
        let expected_hit_count = if rule_id == rules::JSTS_UNREACHABLE_AFTER_RETURN {
            2
        } else {
            1
        };
        assert_eq!(
            hits.len(),
            expected_hit_count,
            "{rule_id} should fire {expected_hit_count} time(s): {findings:?}"
        );
        let hit = hits[0];
        assert_eq!(hit.language, LanguageFamily::JsTs);
        assert_eq!(hit.start_line, start_line, "{rule_id}: {hit:?}");
        assert_eq!(hit.end_line, end_line, "{rule_id}: {hit:?}");
        assert_eq!(hit.flagged_lines, flagged_lines, "{rule_id}: {hit:?}");
    }

    let (rule_id, start_line, end_line, flagged_lines) = second_unreachable_hit;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == rule_id && f.start_line == start_line)
        .unwrap_or_else(|| {
            panic!("expected a second {rule_id} hit at line {start_line}: {findings:?}")
        });
    assert_eq!(hit.language, LanguageFamily::JsTs);
    assert_eq!(hit.end_line, end_line, "{rule_id}: {hit:?}");
    assert_eq!(hit.flagged_lines, flagged_lines, "{rule_id}: {hit:?}");
    assert!(
        !hit.flagged_lines.contains(&27),
        "the second unreachable-after-return hit must exclude the exempt helper declaration's line: {hit:?}"
    );
}

#[test]
fn test_clean_fixture_produces_no_findings() {
    let files = parsed_files(&[
        ("__tests__/CleanJava.java", JAVA),
        ("__tests__/CleanJs.js", JS_TS),
        ("__tests__/CleanTs.ts", JS_TS),
    ]);
    let findings = rules::find_findings(&files);
    assert!(findings.is_empty(), "{findings:?}");
}

/// Every `JAVA-*`/`JSTS-*` token in `text`, treating any character that is
/// not an uppercase ASCII letter or `-` as a separator — so a token
/// embedded in backticks, headings or prose (e.g. `` `JAVA-EMPTY-CATCH` ``)
/// is extracted whole and nothing else is mistaken for a rule id.
fn extract_rule_id_like_tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut token_start: Option<usize> = None;
    let mut push_if_rule_id = |start: usize, end: usize| {
        let token = &text[start..end];
        if token.starts_with("JAVA-") || token.starts_with("JSTS-") {
            tokens.push(token);
        }
    };
    for (index, ch) in text.char_indices() {
        let is_token_char = ch.is_ascii_uppercase() || ch == '-';
        match (is_token_char, token_start) {
            (true, None) => token_start = Some(index),
            (false, Some(start)) => {
                push_if_rule_id(start, index);
                token_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = token_start {
        push_if_rule_id(start, text.len());
    }
    tokens
}

#[test]
fn test_every_rule_id_is_documented() {
    let docs_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/wasteful-rules.md");
    let docs = std::fs::read_to_string(&docs_path).expect("docs/wasteful-rules.md must exist");
    for rule_id in ALL_RULE_IDS {
        assert!(
            docs.contains(rule_id),
            "docs/wasteful-rules.md must document {rule_id}"
        );
    }
    // Reverse direction: a documented-but-unimplemented or typo'd id would
    // pass the forward loop above (`docs.contains` only checks the six real
    // ids are present) but must fail here.
    for token in extract_rule_id_like_tokens(&docs) {
        assert!(
            ALL_RULE_IDS.contains(&token),
            "docs/wasteful-rules.md mentions {token}, which is not in ALL_RULE_IDS {ALL_RULE_IDS:?}"
        );
    }
}
