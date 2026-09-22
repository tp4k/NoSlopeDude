use std::path::PathBuf;
use std::process::Command;

use clap::Parser;

use agent_slope::cli::{Cli, Command as CliCommand};
use agent_slope::model::Target;
use agent_slope::target::classify;

#[test]
fn test_min_clone_lines_defaults_to_10() -> anyhow::Result<()> {
    let cli = Cli::try_parse_from(["agent_slope", "scan", "some/path", "--output", "out"])?;
    let CliCommand::Scan(args) = cli.command;
    assert_eq!(args.min_clone_lines, 10);
    Ok(())
}

#[test]
fn test_output_directory_is_required() {
    let result = Cli::try_parse_from(["agent_slope", "scan", "some/path"]);
    assert!(result.is_err());
}

#[test]
fn test_github_url_is_recognised_as_remote_target() {
    let remote = classify("https://github.com/owner/repo");
    assert!(matches!(remote, Target::Remote(_)));
    if let Target::Remote(remote) = remote {
        assert_eq!(remote.url, "https://github.com/owner/repo");
    }

    let local = classify("some/local/path");
    assert!(matches!(local, Target::Local(_)));
    if let Target::Local(path) = local {
        assert_eq!(path, PathBuf::from("some/local/path"));
    }
}

#[test]
fn test_missing_local_path_is_fatal_nonzero() -> anyhow::Result<()> {
    let output_dir = tempfile::tempdir()?;
    let status = Command::new(env!("CARGO_BIN_EXE_agent_slope"))
        .args([
            "scan",
            "/definitely/does/not/exist/agent-slope-fixture",
            "--output",
        ])
        .arg(output_dir.path())
        .status()?;
    assert!(!status.success());
    Ok(())
}

#[test]
fn test_binary_name_is_nsd() -> anyhow::Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .arg("--help")
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("nsd"),
        "expected --help output to report the binary name nsd, got: {stdout}"
    );
    Ok(())
}
