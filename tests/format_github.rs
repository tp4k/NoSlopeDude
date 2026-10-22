//! `nsd check --format github` on hand-built diagnostics: one `::notice`
//! workflow command per diagnostic shape, the E102 metrics join, the
//! violations-first order and cap, and hostile PR values read back through a
//! re-implementation of the runner's command parser.

use std::collections::BTreeMap;

use nsd::analysis::UnanalyzableReason;
use nsd::check::{CheckDetails, CheckDiagnostic, Entity, FamilySummary, Summaries};
use nsd::config::{Config, PolicyConfig, Severity, CODE_CONFIG_CHANGED, CODE_INVALID_CONFIG};
use nsd::format::{escape_github_data, escape_github_property, render_github};
use nsd::git::path::RepoPath;
use nsd::policy::diagnostics::{
    BaseCallable, CloneDiagnostic, CoverageDiagnostic, DamageDiagnostic, FindingDiagnostic,
    PolicyDiagnostic, SuppressionDiagnostic, CODE_ANALYSIS_UNAVAILABLE, CODE_CLONE_REGRESSION,
    CODE_COMPLEXITY_ABOVE_THRESHOLD, CODE_COMPLEXITY_INCREASED, CODE_INVALID_SUPPRESSION,
    CODE_PARSE_DAMAGE, CODE_UNMATCHED_FINDING,
};
use nsd::policy::Diagnostic as ConfigNote;
use nsd::report::published_name;

const G101: &str = "NSD-G101";
const DEFAULT_CAP: u32 = 10;
const NOTICE_PREFIX: &str = "::notice ";
const OMITTED_PREFIX: &str = "omitted diagnostics:";
const PROPERTY_KEYS: [&str; 4] = ["title", "file", "line", "endLine"];

/// The probe set R2 ran against the runner's parser: a line break followed by
/// a command, a bare CR, an injected property list, a literal escape, NEL,
/// LINE SEPARATOR, ESC, NUL, a stop-commands token and an RLO override.
const PROBES: [&str; 10] = [
    "a\n::error::pwned",
    "a\rb",
    ",line=999,title=FAKE:",
    "%0A",
    "a\u{85}b",
    "a\u{2028}b",
    "a\u{1b}[31mb",
    "a\0b",
    "::stop-commands::tok",
    "a\u{202e}b",
];

/// One workflow command as the runner reads it.
#[derive(Debug)]
struct Command {
    name: String,
    properties: Vec<(String, String)>,
    data: String,
}

/// The runner's `ActionCommand.TryParseV2`: a line that starts with `::`,
/// a command name up to the first space, `,`-separated `key=value`
/// properties up to the next `::`, then the data. Property values and data
/// are unescaped after the split, so an encoded value cannot move it.
fn parse_command(line: &str) -> Option<Command> {
    let rest = line.strip_prefix("::")?;
    let end = rest.find("::")?;
    let (info, data) = (&rest[..end], &rest[end + 2..]);
    let (name, properties) = info.split_once(' ').unwrap_or((info, ""));
    let properties = properties
        .split(',')
        .filter(|property| !property.is_empty())
        .filter_map(|property| property.split_once('='))
        .map(|(key, value)| (key.to_string(), unescape_property(value)))
        .collect();
    Some(Command {
        name: name.to_string(),
        properties,
        data: unescape_data(data),
    })
}

fn unescape_data(text: &str) -> String {
    text.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%25", "%")
}

fn unescape_property(text: &str) -> String {
    text.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%3A", ":")
        .replace("%2C", ",")
        .replace("%25", "%")
}

/// The runner's line split: LF, CR or CRLF end a line, nothing else does.
fn runner_lines(output: &str) -> Vec<&str> {
    output
        .split('\n')
        .flat_map(|line| line.split('\r'))
        .filter(|line| !line.is_empty())
        .collect()
}

/// What a value reads as once the runner has decoded it: CR and LF survive,
/// every other control or bidi character shows as `\u{..}`.
fn as_displayed(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\r' | '\n' => character.to_string(),
            other if other.is_control() || is_bidi_control(other) => {
                format!("\\u{{{:x}}}", u32::from(other))
            }
            other => other.to_string(),
        })
        .collect()
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn property<'a>(command: &'a Command, key: &str) -> Option<&'a str> {
    command
        .properties
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn policy() -> PolicyConfig {
    Config::default().policy
}

fn render(diagnostics: &[CheckDiagnostic], details: Option<&CheckDetails>) -> String {
    render_github(diagnostics, details, &policy(), false, DEFAULT_CAP)
}

/// The single command `render` prints for `diagnostic`.
fn single(diagnostic: CheckDiagnostic, details: Option<&CheckDetails>) -> Command {
    let output = render(&[diagnostic], details);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 1, "{output}");
    assert!(lines[0].starts_with(NOTICE_PREFIX), "{output}");
    let command = parse_command(lines[0]).expect("a well-formed workflow command");
    assert_eq!(command.name, "notice", "{output}");
    assert!(!command.data.is_empty(), "{output}");
    command
}

fn keys(command: &Command) -> Vec<&str> {
    command
        .properties
        .iter()
        .map(|(key, _)| key.as_str())
        .collect()
}

fn complexity(
    code: &'static str,
    path: &str,
    name: &str,
    start: usize,
    base: Option<BaseCallable>,
) -> CheckDiagnostic {
    CheckDiagnostic::Complexity(Box::new(PolicyDiagnostic {
        code,
        candidate_path: RepoPath::from_bytes(path),
        candidate_name: name.to_string(),
        candidate_start_line: start,
        candidate_end_line: start + 20,
        base,
    }))
}

fn base_callable(cc: u32, sloc: usize) -> BaseCallable {
    BaseCallable {
        path: RepoPath::from_bytes("src/A.java"),
        start_line: 3,
        end_line: 22,
        cc,
        sloc,
    }
}

fn finding(path: &str) -> CheckDiagnostic {
    CheckDiagnostic::Finding(FindingDiagnostic {
        code: CODE_UNMATCHED_FINDING,
        rule_id: "JAVA-EMPTY-CATCH",
        candidate_path: RepoPath::from_bytes(path),
        candidate_start_line: 4,
        candidate_end_line: 6,
    })
}

fn suppression(path: &str, rule_id: Option<&'static str>) -> CheckDiagnostic {
    CheckDiagnostic::Suppression(SuppressionDiagnostic {
        code: CODE_INVALID_SUPPRESSION,
        rule_id,
        candidate_path: RepoPath::from_bytes(path),
        directive_line: 7,
    })
}

fn family() -> FamilySummary {
    FamilySummary {
        erosion: 0.0,
        flagged_lines: 0,
        scanned_lines: 0,
        unanalyzed_lines: 0,
        complete: true,
        ratio: 0.0,
    }
}

fn details(entities: Vec<Entity>) -> CheckDetails {
    CheckDetails {
        base_snapshot: String::new(),
        candidate_snapshot: String::new(),
        measurement_fingerprint: String::new(),
        configuration_fingerprint: String::new(),
        skipped: BTreeMap::new(),
        entities,
        summaries: Summaries {
            scope: "changed",
            files: 1,
            overall: family(),
            java: family(),
            js_ts: family(),
            rust: family(),
        },
        coverage: Vec::new(),
    }
}

fn entity(path: &str, name: &str, start_line: usize, cc: u32, sloc: usize) -> Entity {
    Entity {
        path: path.to_string(),
        name: name.to_string(),
        start_line,
        end_line: start_line + 20,
        cc,
        sloc,
        base: None,
    }
}

#[test]
fn test_complexity_is_one_notice_on_the_callable_span() {
    let command = single(
        complexity(
            CODE_COMPLEXITY_ABOVE_THRESHOLD,
            "src/A.java",
            "run",
            3,
            None,
        ),
        None,
    );

    assert_eq!(keys(&command), PROPERTY_KEYS);
    assert_eq!(property(&command, "title"), Some("nsd NSD-E101"));
    assert_eq!(property(&command, "file"), Some("src/A.java"));
    assert_eq!(property(&command, "line"), Some("3"));
    assert_eq!(property(&command, "endLine"), Some("23"));
    assert_eq!(command.data, "run");
}

#[test]
fn test_finding_names_its_rule() {
    let command = single(finding("src/A.java"), None);

    assert_eq!(keys(&command), PROPERTY_KEYS);
    assert_eq!(property(&command, "title"), Some("nsd NSD-V101"));
    assert_eq!(property(&command, "line"), Some("4"));
    assert_eq!(property(&command, "endLine"), Some("6"));
    assert_eq!(command.data, "JAVA-EMPTY-CATCH");
}

#[test]
fn test_suppression_points_at_its_directive_line() {
    let command = single(suppression("src/A.java", Some("JAVA-EMPTY-CATCH")), None);

    assert_eq!(keys(&command), PROPERTY_KEYS);
    assert_eq!(property(&command, "title"), Some("nsd NSD-S102"));
    assert_eq!(property(&command, "line"), Some("7"));
    assert_eq!(property(&command, "endLine"), Some("7"));
    assert_eq!(command.data, "JAVA-EMPTY-CATCH");
}

#[test]
fn test_suppression_without_a_valid_rule_keeps_a_message() {
    let command = single(suppression("src/A.java", None), None);

    assert_eq!(command.data, "suppression directive");
}

#[test]
fn test_damage_points_at_the_json_report() {
    let command = single(
        CheckDiagnostic::Damage(DamageDiagnostic {
            code: CODE_PARSE_DAMAGE,
            candidate_path: RepoPath::from_bytes("src/A.ts"),
            candidate_start_line: 2,
            candidate_end_line: 9,
        }),
        None,
    );

    assert_eq!(keys(&command), PROPERTY_KEYS);
    assert_eq!(property(&command, "title"), Some("nsd NSD-A101"));
    assert_eq!(property(&command, "line"), Some("2"));
    assert_eq!(property(&command, "endLine"), Some("9"));
    assert_eq!(command.data, "damaged span; see the nsd JSON report");
}

#[test]
fn test_coverage_names_the_file_and_its_reason() {
    let command = single(
        CheckDiagnostic::Coverage(CoverageDiagnostic {
            code: CODE_ANALYSIS_UNAVAILABLE,
            path: RepoPath::from_bytes("src/big.ts"),
            reason: UnanalyzableReason::TooLarge,
        }),
        None,
    );

    assert_eq!(keys(&command), ["title", "file"]);
    assert_eq!(property(&command, "title"), Some("nsd NSD-A102"));
    assert_eq!(property(&command, "file"), Some("src/big.ts"));
    assert_eq!(command.data, "too_large");
}

#[test]
fn test_clone_names_its_match() {
    let command = single(
        CheckDiagnostic::Clone(Box::new(CloneDiagnostic {
            code: CODE_CLONE_REGRESSION,
            candidate_path: RepoPath::from_bytes("src/B.java"),
            candidate_start_line: 10,
            candidate_end_line: 24,
            base_lines: None,
            added_lines: 15,
            matched_path: RepoPath::from_bytes("src/A.java"),
            matched_start_line: 30,
            matched_end_line: 44,
        })),
        None,
    );

    assert_eq!(keys(&command), PROPERTY_KEYS);
    assert_eq!(property(&command, "title"), Some("nsd NSD-V102"));
    assert_eq!(property(&command, "file"), Some("src/B.java"));
    assert_eq!(property(&command, "line"), Some("10"));
    assert_eq!(property(&command, "endLine"), Some("24"));
    assert_eq!(command.data, "clone of src/A.java:30-44 (+15 lines)");
}

#[test]
fn test_config_change_and_invalid_config_name_nsd_yml() {
    let changed = single(
        CheckDiagnostic::Config(ConfigNote {
            code: CODE_CONFIG_CHANGED,
        }),
        None,
    );
    let invalid = single(
        CheckDiagnostic::Config(ConfigNote {
            code: CODE_INVALID_CONFIG,
        }),
        None,
    );

    assert_eq!(keys(&changed), ["title", "file"]);
    assert_eq!(property(&changed, "title"), Some("nsd NSD-C101"));
    assert_eq!(property(&changed, "file"), Some("nsd.yml"));
    assert_eq!(changed.data, "nsd.yml changed");
    assert_eq!(keys(&invalid), ["title", "file"]);
    assert_eq!(property(&invalid, "title"), Some("nsd NSD-C102"));
    assert_eq!(invalid.data, "nsd.yml is invalid");
}

#[test]
fn test_failure_has_a_title_and_its_message_only() {
    let generic = single(
        CheckDiagnostic::Failure {
            code: G101,
            message: "cannot read HEAD".to_string(),
        },
        None,
    );
    let refused = single(
        CheckDiagnostic::Failure {
            code: CODE_INVALID_CONFIG,
            message: "trusted config is inside the candidate checkout".to_string(),
        },
        None,
    );

    assert_eq!(keys(&generic), ["title"]);
    assert_eq!(property(&generic, "title"), Some("nsd NSD-G101"));
    assert_eq!(generic.data, "cannot read HEAD");
    assert_eq!(keys(&refused), ["title"]);
    assert_eq!(property(&refused, "title"), Some("nsd NSD-C102"));
    assert_eq!(
        refused.data,
        "trusted config is inside the candidate checkout"
    );
}

#[test]
fn test_e102_joins_the_matching_candidate_entity() {
    let diagnostic = complexity(
        CODE_COMPLEXITY_INCREASED,
        "src/A.java",
        "run",
        3,
        Some(base_callable(11, 20)),
    );
    let found = details(vec![
        entity("src/A.java", "other", 3, 99, 99),
        entity("src/A.java", "run", 3, 14, 21),
    ]);

    let command = single(diagnostic, Some(&found));

    assert_eq!(command.data, "run | CC 11 -> 14, SLOC 20 -> 21");
}

#[test]
fn test_computed_callable_shows_its_published_name_and_still_joins() {
    let raw = "[\"secret literal\"]";
    let diagnostic = complexity(
        CODE_COMPLEXITY_INCREASED,
        "src/A.ts",
        raw,
        3,
        Some(base_callable(11, 20)),
    );
    let found = details(vec![entity("src/A.ts", raw, 3, 14, 21)]);

    let command = single(diagnostic, Some(&found));

    assert_eq!(command.data, "<computed>@3 | CC 11 -> 14, SLOC 20 -> 21");
}

#[test]
fn test_e102_without_a_matching_entity_falls_back_to_the_base() {
    let diagnostic = || {
        complexity(
            CODE_COMPLEXITY_INCREASED,
            "src/A.java",
            "run",
            3,
            Some(base_callable(11, 20)),
        )
    };
    let other_path = details(vec![entity("src/B.java", "run", 3, 14, 21)]);
    let other_line = details(vec![entity("src/A.java", "run", 4, 14, 21)]);
    let same_line_overloads = details(vec![
        entity("src/A.java", "run", 3, 14, 21),
        entity("src/A.java", "run", 3, 30, 40),
    ]);
    let fallback = "run | base CC 11, SLOC 20";

    assert_eq!(single(diagnostic(), Some(&other_path)).data, fallback);
    assert_eq!(single(diagnostic(), Some(&other_line)).data, fallback);
    assert_eq!(
        single(diagnostic(), Some(&same_line_overloads)).data,
        fallback
    );
    assert_eq!(single(diagnostic(), None).data, fallback);
}

#[test]
fn test_complexity_without_a_base_is_the_callable_alone() {
    let found = details(vec![entity("src/A.java", "run", 3, 14, 21)]);

    let command = single(
        complexity(CODE_COMPLEXITY_INCREASED, "src/A.java", "run", 3, None),
        Some(&found),
    );

    assert_eq!(command.data, "run");
}

#[test]
fn test_violations_come_first_then_the_cap_names_what_was_omitted() {
    let violations = [1, 4, 5, 8, 11];
    let diagnostics: Vec<CheckDiagnostic> = (0..12)
        .map(|index| {
            let path = format!("f{index:02}.java");
            if violations.contains(&index) {
                finding(&path)
            } else {
                suppression(&path, Some("JAVA-EMPTY-CATCH"))
            }
        })
        .collect();

    let output = render(&diagnostics, None);

    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 11, "{output}");
    let files: Vec<String> = lines[..10]
        .iter()
        .map(|line| {
            let command = parse_command(line).expect("a workflow command");
            property(&command, "file").expect("a file").to_string()
        })
        .collect();
    let expected: Vec<String> = [1, 4, 5, 8, 11, 0, 2, 3, 6, 7]
        .iter()
        .map(|index| format!("f{index:02}.java"))
        .collect();
    assert_eq!(files, expected, "{output}");
    assert_eq!(lines[10], "omitted diagnostics: 2 of 12");
}

#[test]
fn test_a_code_turned_off_sorts_with_the_non_violations() {
    let diagnostics = [
        suppression("a.java", Some("JAVA-EMPTY-CATCH")),
        finding("b.java"),
        complexity(CODE_COMPLEXITY_ABOVE_THRESHOLD, "c.java", "run", 3, None),
    ];
    let mut off = policy();
    off.nsd_v101 = Severity::Off;

    let output = render_github(&diagnostics, None, &off, false, DEFAULT_CAP);

    let titles: Vec<String> = output
        .lines()
        .map(|line| {
            let command = parse_command(line).expect("a workflow command");
            property(&command, "title").expect("a title").to_string()
        })
        .collect();
    assert_eq!(titles, ["nsd NSD-E101", "nsd NSD-S102", "nsd NSD-V101"]);
}

#[test]
fn test_a_cap_at_least_the_total_omits_nothing() {
    let diagnostics = [finding("a.java"), finding("b.java")];

    let output = render_github(&diagnostics, None, &policy(), false, 2);

    assert_eq!(output.lines().count(), 2, "{output}");
    assert!(!output.contains(OMITTED_PREFIX), "{output}");
}

#[test]
fn test_escape_github_data_encodes_in_the_runner_order() {
    assert_eq!(escape_github_data("100%"), "100%25");
    assert_eq!(escape_github_data("%0A"), "%250A");
    assert_eq!(escape_github_data("a\r\nb"), "a%0D%0Ab");
    assert_eq!(escape_github_data("a\u{1b}b\0c"), "a\\u{1b}b\\u{0}c");
    assert_eq!(escape_github_data("a\u{85}b"), "a\\u{85}b");
    assert_eq!(escape_github_data("a\u{202e}b"), "a\\u{202e}b");
    assert_eq!(escape_github_data("a:b,c"), "a:b,c");
}

#[test]
fn test_escape_github_property_also_encodes_colon_and_comma() {
    assert_eq!(
        escape_github_property(",line=999,title=FAKE:"),
        "%2Cline=999%2Ctitle=FAKE%3A"
    );
    assert_eq!(
        escape_github_property("a\n::error::x"),
        "a%0A%3A%3Aerror%3A%3Ax"
    );
    assert_eq!(escape_github_property("%2C"), "%252C");
}

#[test]
fn test_hostile_property_values_decode_to_themselves() {
    for probe in PROBES {
        let line = format!(
            "::notice title={},file={}::{}",
            escape_github_property(probe),
            escape_github_property(probe),
            escape_github_data(probe)
        );

        assert_eq!(runner_lines(&line).len(), 1, "{probe:?}: {line}");
        let command = parse_command(&line).expect("a workflow command");
        assert_eq!(command.name, "notice", "{probe:?}");
        assert_eq!(keys(&command), ["title", "file"], "{probe:?}: {line}");
        assert_eq!(
            property(&command, "title"),
            Some(as_displayed(probe).as_str())
        );
        assert_eq!(
            property(&command, "file"),
            Some(as_displayed(probe).as_str())
        );
        assert_eq!(command.data, as_displayed(probe), "{probe:?}");
    }
}

#[test]
fn test_hostile_paths_and_names_stay_one_annotation_each() {
    let diagnostics: Vec<CheckDiagnostic> = PROBES
        .iter()
        .flat_map(|probe| {
            [
                complexity(
                    CODE_COMPLEXITY_ABOVE_THRESHOLD,
                    &format!("src/{probe}.java"),
                    probe,
                    3,
                    None,
                ),
                CheckDiagnostic::Failure {
                    code: G101,
                    message: (*probe).to_string(),
                },
            ]
        })
        .collect();

    let output = render_github(&diagnostics, None, &policy(), false, u32::MAX);

    let lines = runner_lines(&output);
    assert_eq!(lines.len(), diagnostics.len(), "{output}");
    for (line, diagnostic) in lines.iter().zip(&diagnostics) {
        assert!(line.starts_with(NOTICE_PREFIX), "{line}");
        let command = parse_command(line).expect("a workflow command");
        assert_eq!(command.name, "notice", "{line}");
        let found = keys(&command);
        assert!(
            found.iter().all(|key| PROPERTY_KEYS.contains(key)),
            "{line}"
        );
        let mut unique = found.clone();
        unique.dedup();
        assert_eq!(unique, found, "{line}");
        match diagnostic {
            CheckDiagnostic::Complexity(found) => {
                assert_eq!(
                    property(&command, "file"),
                    Some(as_displayed(&found.candidate_path.render()).as_str()),
                    "{line}"
                );
                assert_eq!(property(&command, "line"), Some("3"), "{line}");
                assert_eq!(
                    command.data,
                    published_name(&found.candidate_name, found.candidate_start_line)
                );
            }
            CheckDiagnostic::Failure { message, .. } => {
                assert_eq!(found, ["title"], "{line}");
                assert_eq!(command.data, as_displayed(message));
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn test_every_line_is_a_notice_or_the_omitted_count() {
    let diagnostics: Vec<CheckDiagnostic> = PROBES
        .iter()
        .map(|probe| finding(&format!("src/{probe}.java")))
        .collect();

    let output = render(&diagnostics, None);

    assert!(
        runner_lines(&output)
            .iter()
            .all(|line| line.starts_with(NOTICE_PREFIX) || line.starts_with(OMITTED_PREFIX)),
        "{output}"
    );
}
