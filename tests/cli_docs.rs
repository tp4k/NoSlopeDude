//! M7-2: the copyable pre-commit and CI commands in `docs/cli.md` parse
//! against the real command-line surface, and v1 ships no hook installer.

use std::fs;
use std::path::Path;

use clap::Parser;

use nsd::cli::{Cli, Command};

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
    for path in [
        ".github/workflows",
        ".gitlab-ci.yml",
        ".pre-commit-config.yaml",
    ] {
        assert!(!Path::new(path).exists(), "{path} must not exist in v1");
    }
    let parsed = Cli::try_parse_from(["nsd", "install-hook"]);
    assert!(
        parsed.is_err(),
        "nsd must have no hook installer subcommand"
    );
}
