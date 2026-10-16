use std::path::PathBuf;
use std::process::Command;

use clap::{CommandFactory, Parser};

use nsd::cli::{Cli, Command as CliCommand};
use nsd::model::Target;
use nsd::target::classify;

#[test]
fn test_min_clone_lines_defaults_to_10() -> anyhow::Result<()> {
    let cli = Cli::try_parse_from(["nsd", "scan", "some/path", "--output", "out"])?;
    let CliCommand::Scan(args) = cli.command else {
        anyhow::bail!("expected the scan subcommand");
    };
    assert_eq!(args.min_clone_lines, None);
    Ok(())
}

#[test]
fn test_output_directory_is_required() {
    let result = Cli::try_parse_from(["nsd", "scan", "some/path"]);
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
    let status = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(["scan", "/definitely/does/not/exist/nsd-fixture", "--output"])
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
    assert_eq!(
        Cli::command().get_name(),
        "nsd",
        "the clap command's own name must be nsd, not just the --help output"
    );
    Ok(())
}

#[cfg(unix)]
const ESC_BIDI_NAME: &str = "dir\u{1b}[31m\u{202e}name";

#[cfg(unix)]
#[test]
fn test_scan_summary_escapes_control_and_bidi_characters_in_the_target() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let target = root.path().join(ESC_BIDI_NAME);
    std::fs::create_dir(&target)?;
    let output_dir = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .arg("scan")
        .arg(&target)
        .arg("--output")
        .arg(output_dir.path())
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(!stdout.contains('\u{1b}'), "raw ESC in: {stdout:?}");
    assert!(!stdout.contains('\u{202e}'), "raw U+202E in: {stdout:?}");
    let target_line = stdout
        .lines()
        .find(|line| line.starts_with("  target: "))
        .ok_or_else(|| anyhow::anyhow!("no target line in: {stdout:?}"))?;
    assert!(target_line.contains("\\u{1b}"), "got: {target_line:?}");
    assert!(target_line.contains("\\u{202e}"), "got: {target_line:?}");
    assert!(stdout.ends_with('\n'));
    assert!(stdout.starts_with("Scan settings:\n  target: "));
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_scan_summary_escapes_a_newline_in_the_target() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let target = root.path().join("dir\nNSD-FAKE");
    std::fs::create_dir(&target)?;
    let output_dir = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .arg("scan")
        .arg(&target)
        .arg("--output")
        .arg(output_dir.path())
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    let target_line = stdout
        .lines()
        .find(|line| line.starts_with("  target: "))
        .ok_or_else(|| anyhow::anyhow!("no target line in: {stdout:?}"))?;
    assert!(
        target_line.contains("\\u{a}NSD-FAKE"),
        "got: {target_line:?}"
    );
    assert!(
        stdout.lines().all(|line| !line.starts_with("NSD-FAKE")),
        "injected line in: {stdout:?}"
    );
    Ok(())
}

#[test]
fn test_scan_error_escapes_control_and_bidi_characters() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let target = root.path().join("src-dir");
    std::fs::create_dir(&target)?;
    let blocker = root.path().join("blocker");
    std::fs::write(&blocker, "not a directory")?;
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .arg("scan")
        .arg(&target)
        .arg("--output")
        .arg(blocker.join("out\u{1b}[31m\u{202e}"))
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.starts_with("error: "), "got: {stderr:?}");
    assert!(stderr.contains("\\u{1b}"), "got: {stderr:?}");
    assert!(stderr.contains("\\u{202e}"), "got: {stderr:?}");
    assert!(!stderr.contains('\u{1b}'), "raw ESC in: {stderr:?}");
    assert!(!stderr.contains('\u{202e}'), "raw U+202E in: {stderr:?}");
    Ok(())
}

#[test]
fn test_scan_error_escapes_control_and_bidi_characters_in_the_cause_chain() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let target = root.path().join("src-dir");
    std::fs::create_dir(&target)?;
    let output_dir = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .arg("scan")
        .arg(&target)
        .arg("--output")
        .arg(output_dir.path())
        .arg("--exclude")
        .arg("a[\u{1b}[31m\u{202e}")
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("error: invalid exclusion glob"),
        "got: {stderr:?}"
    );
    let (_, cause) = stderr
        .split_once(": error parsing glob")
        .ok_or_else(|| anyhow::anyhow!("no cause line in: {stderr:?}"))?;
    assert!(cause.contains("\\u{1b}"), "got: {cause:?}");
    assert!(cause.contains("\\u{202e}"), "got: {cause:?}");
    assert!(!stderr.contains('\u{1b}'), "raw ESC in: {stderr:?}");
    assert!(!stderr.contains('\u{202e}'), "raw U+202E in: {stderr:?}");
    Ok(())
}

/// Runs the binary on `args` and returns its exit code, stdout and stderr.
fn run_nsd(args: &[&str]) -> anyhow::Result<(Option<i32>, String, String)> {
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(args)
        .output()?;
    Ok((
        output.status.code(),
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
    ))
}

/// What clap itself renders for `args`, uncoloured: the bytes a parse error
/// printed before nsd escaped them.
fn clap_rendering(args: &[&str]) -> anyhow::Result<String> {
    let argv = std::iter::once("nsd").chain(args.iter().copied());
    match Cli::try_parse_from(argv) {
        Ok(_) => anyhow::bail!("{args:?} parsed"),
        Err(error) => Ok(error.render().to_string()),
    }
}

#[test]
fn test_argument_error_escapes_control_and_bidi_characters_in_an_invalid_value(
) -> anyhow::Result<()> {
    let (code, stdout, stderr) =
        run_nsd(&["check", "--staged", "--format", "json\u{1b}[31m\u{202e}x"])?;
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(stdout.is_empty(), "got: {stdout:?}");
    assert!(
        stderr.starts_with(
            "error: invalid value 'json\\u{1b}[31m\\u{202e}x' for '--format <FORMAT>'\n"
        ),
        "got: {stderr:?}"
    );
    assert!(!stderr.contains('\u{1b}'), "raw ESC in: {stderr:?}");
    assert!(!stderr.contains('\u{202e}'), "raw U+202E in: {stderr:?}");
    Ok(())
}

#[test]
fn test_argument_error_escapes_a_newline_inside_an_invalid_value() -> anyhow::Result<()> {
    let (code, _, stderr) = run_nsd(&["check", "--staged", "--format", "json\nNSD-FAKE"])?;
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(
        stderr.starts_with("error: invalid value 'json\\u{a}NSD-FAKE' for '--format <FORMAT>'\n"),
        "got: {stderr:?}"
    );
    assert!(
        stderr.lines().all(|line| !line.starts_with("NSD-FAKE")),
        "injected line in: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_argument_error_escapes_a_newline_inside_a_numeric_value() -> anyhow::Result<()> {
    let (code, _, stderr) = run_nsd(&[
        "scan",
        ".",
        "--output",
        "out",
        "--min-clone-lines",
        "1\nNSD-FAKE\u{1b}[2J",
    ])?;
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(
        stderr
            .starts_with("error: invalid value '1\\u{a}NSD-FAKE\\u{1b}[2J' for '--min-clone-lines"),
        "got: {stderr:?}"
    );
    assert!(!stderr.contains('\u{1b}'), "raw ESC in: {stderr:?}");
    assert!(
        stderr.lines().all(|line| !line.starts_with("NSD-FAKE")),
        "injected line in: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_argument_error_escapes_an_unexpected_argument_everywhere_it_is_quoted() -> anyhow::Result<()>
{
    let hostile = "--bogus\u{1b}[31m\u{202e}\nNSD-FAKE";
    let (code, _, stderr) = run_nsd(&["scan", hostile])?;
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    let escaped = "--bogus\\u{1b}[31m\\u{202e}\\u{a}NSD-FAKE";
    assert!(
        stderr.starts_with(&format!("error: unexpected argument '{escaped}' found\n")),
        "got: {stderr:?}"
    );
    assert!(
        stderr.contains(&format!("use '-- {escaped}'")),
        "the tip quotes the argument too: {stderr:?}"
    );
    assert!(!stderr.contains('\u{1b}'), "raw ESC in: {stderr:?}");
    assert!(!stderr.contains('\u{202e}'), "raw U+202E in: {stderr:?}");
    assert!(
        stderr.lines().all(|line| !line.starts_with("NSD-FAKE")),
        "injected line in: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_argument_error_escapes_an_unrecognized_subcommand() -> anyhow::Result<()> {
    let (code, _, stderr) = run_nsd(&["sc\u{1b}[31m\u{202e}an\nNSD-FAKE"])?;
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(
        stderr.starts_with(
            "error: unrecognized subcommand 'sc\\u{1b}[31m\\u{202e}an\\u{a}NSD-FAKE'\n"
        ),
        "got: {stderr:?}"
    );
    assert!(!stderr.contains('\u{1b}'), "raw ESC in: {stderr:?}");
    assert!(!stderr.contains('\u{202e}'), "raw U+202E in: {stderr:?}");
    assert!(
        stderr.lines().all(|line| !line.starts_with("NSD-FAKE")),
        "injected line in: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_argument_error_value_equal_to_a_clap_style_code_leaves_clap_text_alone(
) -> anyhow::Result<()> {
    let (code, _, stderr) = run_nsd(&["scan", "--", ".", "\u{1b}[1m"])?;
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(
        stderr.starts_with("error: unexpected argument '\\u{1b}[1m' found\n"),
        "got: {stderr:?}"
    );
    assert!(
        stderr
            .lines()
            .any(|line| line.starts_with("Usage: nsd scan [OPTIONS] --output <OUTPUT>")),
        "the usage line reads as clap renders it: {stderr:?}"
    );
    assert_eq!(
        stderr.matches("\\u{1b}").count(),
        1,
        "only the value is escaped: {stderr:?}"
    );
    assert!(!stderr.contains('\u{1b}'), "raw ESC in: {stderr:?}");
    Ok(())
}

#[test]
fn test_argument_error_without_hostile_characters_prints_clap_rendering_unchanged(
) -> anyhow::Result<()> {
    let cases: [&[&str]; 6] = [
        &["check", "--staged", "--format", "xml"],
        &["scan", "--bogus"],
        &["scan", "--outpt", "x"],
        &["frobnicate"],
        &["scan", ".", "--output", "out", "--min-clone-lines", "abc"],
        &["check"],
    ];
    for args in cases {
        let (code, stdout, stderr) = run_nsd(args)?;
        assert_eq!(code, Some(2), "{args:?}: {stderr:?}");
        assert!(stdout.is_empty(), "{args:?}: {stdout:?}");
        assert_eq!(stderr, clap_rendering(args)?, "{args:?}");
    }
    Ok(())
}

#[test]
fn test_help_prints_as_clap_does() -> anyhow::Result<()> {
    for args in [&["--help"][..], &["check", "--help"][..]] {
        let (code, stdout, stderr) = run_nsd(args)?;
        assert_eq!(code, Some(0), "{args:?}: {stderr:?}");
        assert!(stderr.is_empty(), "{args:?}: {stderr:?}");
        assert_eq!(stdout, clap_rendering(args)?, "{args:?}");
        assert!(stdout.contains("Usage: nsd"), "{args:?}: {stdout:?}");
    }
    Ok(())
}
