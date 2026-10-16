//! M7-2: the copyable pre-commit and CI commands in `docs/cli.md` parse
//! against the real command-line surface, and v1 ships no hook installer.

use std::fs;
use std::path::Path;

use clap::Parser;

use nsd::cli::{Cli, Command};
use nsd::config::Config;
use serde_yaml_ng::Value;

const CLI_DOC: &str = "docs/cli.md";
const SECTION_HEADING: &str = "## Pre-commit and CI";
const COMMAND_PREFIX: &str = "nsd ";
const MIN_DOCUMENTED_COMMANDS: usize = 3;

fn section(doc: &str) -> String {
    let mut lines = doc.lines().skip_while(|line| *line != SECTION_HEADING);
    assert!(lines.next().is_some(), "{CLI_DOC} has no {SECTION_HEADING}");
    lines
        .take_while(|line| !line.starts_with("## "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn documented_commands(section: &str) -> Vec<&str> {
    let mut in_fence = false;
    let mut commands = Vec::new();
    for line in section.lines() {
        if line.starts_with("```") {
            in_fence = !in_fence;
        } else if in_fence && line.starts_with(COMMAND_PREFIX) {
            commands.push(line.trim());
        }
    }
    commands
}

#[test]
fn test_documented_pre_commit_and_ci_commands_parse() {
    let doc = fs::read_to_string(CLI_DOC).expect("read docs/cli.md");
    let section = section(&doc);

    let commands = documented_commands(&section);

    assert!(commands.len() >= MIN_DOCUMENTED_COMMANDS, "{commands:?}");
    let mut staged = false;
    let mut base = false;
    let mut allowed = false;
    for command in &commands {
        let cli = Cli::try_parse_from(command.split_whitespace())
            .unwrap_or_else(|error| panic!("`{command}` does not parse: {error}"));
        let Command::Check(args) = cli.command else {
            panic!("`{command}` is not a check command");
        };
        staged |= args.staged;
        base |= args.base.is_some();
        allowed |= args.allow_new_suppressions;
    }
    assert!(staged, "no pre-commit (--staged) command in {commands:?}");
    assert!(base, "no CI (--base) command in {commands:?}");
    assert!(allowed, "--allow-new-suppressions is not documented");
}

#[test]
fn test_no_hook_installer_or_provider_workflow_ships() {
    for path in [".gitlab-ci.yml", ".pre-commit-config.yaml"] {
        assert!(!Path::new(path).exists(), "{path} must not exist in v1");
    }
    let parsed = Cli::try_parse_from(["nsd", "install-hook"]);
    assert!(
        parsed.is_err(),
        "nsd must have no hook installer subcommand"
    );
}

const WORKFLOW_DIR: &str = ".github/workflows";
const WORKFLOW: &str = ".github/workflows/self-check.yml";
const ROOT_CONFIG: &str = "nsd.yml";
const BASE_SCRIPT: &str = "self_check_base.sh";
const TEST_COMMAND: &str = "cargo test --locked";
const NSD_CHECK: &str = "nsd check";
const RESOLVED_BASE_SENTINEL: &str = "resolved-base-sentinel";
const FIXTURE_GLOBS: [&str; 2] = ["tests/fixtures/**", "tests/golden/**"];
const EVENT_EXPRESSIONS: [&str; 3] = [
    "github.event_name",
    "github.event.before",
    "github.base_ref",
];

fn workflow() -> Value {
    let text = fs::read_to_string(WORKFLOW).expect("read the self-check workflow");
    serde_yaml_ng::from_str(&text).expect("the workflow is valid YAML")
}

fn steps(workflow: &Value) -> Vec<Value> {
    let jobs = workflow["jobs"].as_mapping().expect("a jobs mapping");
    assert_eq!(jobs.len(), 1, "the self-check has one job");
    let job = jobs.values().next().expect("one job");
    job["steps"].as_sequence().expect("job steps").clone()
}

fn run_text(step: &Value) -> &str {
    step["run"].as_str().unwrap_or("")
}

fn step_text(step: &Value) -> String {
    serde_yaml_ng::to_string(step).expect("serialise a step")
}

fn without_expressions(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${{") {
        kept.push_str(&rest[..start]);
        rest = rest[start..]
            .split_once("}}")
            .map_or("", |(_, after)| after);
    }
    kept.push_str(rest);
    kept
}

fn step_index(steps: &[Value], needle: &str) -> usize {
    steps
        .iter()
        .position(|step| run_text(step).contains(needle))
        .unwrap_or_else(|| panic!("no step runs `{needle}`"))
}

#[test]
fn test_self_check_workflow_runs_a_base_check_with_full_history() {
    let workflow = workflow();
    let steps = steps(&workflow);

    let checkout = steps
        .iter()
        .find(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("actions/checkout"))
        })
        .expect("a checkout step");
    assert_eq!(checkout["with"]["fetch-depth"].as_u64(), Some(0));
    let tests = step_index(&steps, TEST_COMMAND);
    let check = step_index(&steps, NSD_CHECK);
    assert!(tests < check, "cargo test must run before nsd check");
    let build = step_index(&steps, "cargo build --locked --release");
    assert!(build < tests, "the release build must precede cargo test");
    assert!(workflow["on"].get("pull_request").is_some());
    let push_branches: Vec<&str> = workflow["on"]["push"]["branches"]
        .as_sequence()
        .expect("push branches")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(push_branches, ["main"]);
    let line = run_text(&steps[check])
        .lines()
        .find(|line| line.contains(NSD_CHECK))
        .expect("the nsd check line");
    let arguments: Vec<String> = line
        .replace("./target/release/nsd", "nsd")
        .replace("\"$BASE\"", "origin/main")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let cli = Cli::try_parse_from(&arguments).unwrap_or_else(|error| panic!("`{line}`: {error}"));
    let Command::Check(args) = cli.command else {
        panic!("`{line}` is not a check command");
    };
    assert!(args.base.is_some(), "`{line}` has no --base");
    assert!(!args.staged, "`{line}` checks the (empty) index");
}

#[test]
fn test_self_check_workflow_takes_its_base_from_the_event() {
    let workflow = workflow();
    let steps = steps(&workflow);

    let resolve = &steps[step_index(&steps, BASE_SCRIPT)];
    let text = step_text(resolve);
    for expression in EVENT_EXPRESSIONS {
        assert!(text.contains(expression), "{expression} missing in {text}");
    }
    for step in &steps {
        assert!(
            !run_text(step).contains("origin/main"),
            "a literal origin/main in {}",
            run_text(step)
        );
        assert!(
            !without_expressions(&step_text(step)).contains("origin/main"),
            "a literal origin/main in {}",
            step_text(step)
        );
    }
}

#[test]
fn test_self_check_workflow_checks_the_resolved_base() {
    let workflow = workflow();
    let steps = steps(&workflow);

    let resolve = &steps[step_index(&steps, BASE_SCRIPT)];
    let id = resolve["id"].as_str().expect("the resolve step has an id");
    let run = run_text(resolve);
    assert!(
        run.contains("base=") && run.contains(">> \"$GITHUB_OUTPUT\""),
        "the resolved base is not written to GITHUB_OUTPUT: {run}"
    );
    let output = format!("steps.{id}.outputs.base");
    let check = &steps[step_index(&steps, NSD_CHECK)];
    assert_eq!(
        check["env"]["BASE"].as_str(),
        Some(format!("${{{{ {output} }}}}").as_str())
    );
    assert_eq!(
        check["if"].as_str(),
        Some(format!("{output} != ''").as_str())
    );

    let expression = format!("${{{{ {output} }}}}");
    let carriers: Vec<&str> = check["env"]
        .as_mapping()
        .expect("the nsd check step has an env mapping")
        .iter()
        .filter(|(_, value)| value.as_str() == Some(expression.as_str()))
        .filter_map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(carriers.len(), 1, "one env var carries {output}");
    let variable = format!("\"${}\"", carriers[0]);
    let line = run_text(check)
        .lines()
        .find(|line| line.contains(NSD_CHECK))
        .expect("the nsd check line");
    assert_eq!(
        run_text(check).trim(),
        line.trim(),
        "the nsd check step runs only the check command"
    );
    let arguments: Vec<String> = line
        .replace("./target/release/nsd", "nsd")
        .replace(&variable, RESOLVED_BASE_SENTINEL)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let cli = Cli::try_parse_from(&arguments).unwrap_or_else(|error| panic!("`{line}`: {error}"));
    let Command::Check(args) = cli.command else {
        panic!("`{line}` is not a check command");
    };
    assert_eq!(
        args.base.as_deref(),
        Some(RESOLVED_BASE_SENTINEL),
        "`{line}` does not pass {variable} as --base"
    );
}

#[test]
fn test_self_check_workflow_is_unprivileged_and_runs_on_macos() {
    let text = fs::read_to_string(WORKFLOW).expect("read the self-check workflow");
    let workflow = workflow();

    assert!(!text.contains("pull_request_target"));
    assert!(!text.contains("secrets."));
    let permissions = workflow["permissions"].as_mapping().expect("permissions");
    assert_eq!(permissions.len(), 1);
    assert_eq!(workflow["permissions"]["contents"].as_str(), Some("read"));
    let job = workflow["jobs"].as_mapping().expect("jobs").values().next();
    let runner = job.and_then(|job| job["runs-on"].as_str());
    assert_eq!(runner, Some("macos-latest"));
}

#[test]
fn test_only_the_self_check_workflow_ships() {
    let mut names: Vec<String> = fs::read_dir(WORKFLOW_DIR)
        .expect("read .github/workflows")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();

    assert_eq!(names, ["self-check.yml"]);
}

#[test]
fn test_root_nsd_yml_is_valid_config() {
    let bytes = fs::read(ROOT_CONFIG).expect("read the root nsd.yml");

    let config = Config::parse(&bytes).expect("the root nsd.yml parses");

    assert_eq!(config.exclude, FIXTURE_GLOBS);
}
