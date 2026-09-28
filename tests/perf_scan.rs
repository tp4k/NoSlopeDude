//! WS-8/B10-B12: `scripts/perf_scan.sh`'s `report.json` parsing lives in
//! plain bash functions behind a `[[ "${BASH_SOURCE[0]}" == "$0" ]]` main
//! guard specifically so this suite can `source` the script and call them
//! directly against synthetic `report.json` text, without a real scan.

use std::process::{Command, Output};

fn script_path() -> String {
    format!("{}/scripts/perf_scan.sh", env!("CARGO_MANIFEST_DIR"))
}

/// Sources `scripts/perf_scan.sh` (a no-op past its main guard when not
/// invoked as `$0`) and runs `call` in the same shell, returning its
/// output.
fn run_sourced(call: &str) -> anyhow::Result<Output> {
    Ok(Command::new("bash")
        .arg("-c")
        .arg(format!("source \"{}\" && {}", script_path(), call))
        .output()?)
}

#[cfg(unix)]
#[test]
fn test_missing_overall_anchor_exits_1() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let report = dir.path().join("report.json");
    // "overall" renamed to "totals" (e.g. a key rename): a plain top-level
    // grep for "scanned_lines" still finds the per-family value below and
    // would wrongly succeed with it.
    std::fs::write(
        &report,
        r#"{
  "totals": { "scanned_lines": 42 },
  "java": { "scanned_lines": 999 },
  "incomplete": false,
  "skipped_files": []
}
"#,
    )?;

    let output = run_sourced(&format!("extract_scanned_lines '{}'", report.display()))?;

    assert!(
        !output.status.success(),
        "expected a nonzero exit when \"overall\": {{ is absent, got success with stdout {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error:"),
        "expected an `error:` line on stderr, got: {stderr}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_skipped_count_survives_an_unbalanced_bracket_in_a_path() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let report = dir.path().join("report.json");
    // The first skipped entry's own path contains a literal ']', which a
    // naive per-line bracket-depth count (without blanking string
    // contents first) mistakes for the array's own closing bracket.
    std::fs::write(
        &report,
        r#"{
  "overall": { "scanned_lines": 42 },
  "incomplete": true,
  "skipped_files": [
    {"relative_path": "src/weird]name.js", "reason": "parse_SyntaxError", "detail": null},
    {"relative_path": "src/other.java", "reason": "parse_SyntaxError", "detail": null}
  ]
}
"#,
    )?;

    let output = run_sourced(&format!("extract_skipped_count '{}'", report.display()))?;

    assert!(
        output.status.success(),
        "extract_skipped_count failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "2",
        "expected both skipped entries to survive the unbalanced ']' in the first path, got: {stdout}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_skipped_count_survives_an_escaped_quote_before_a_bracket() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let report = dir.path().join("report.json");
    // The first skipped entry's own path contains an escaped '"' right
    // before a ']' (`\"]`); a blank_strings that does not skip the
    // character after a backslash toggles `in_str` on that escaped quote
    // and reads the following ']' as real array structure, closing the
    // capture early. The second entry repeats the same `\"]` inside
    // `detail` instead of the path, to confirm the escape is honoured
    // wherever it appears, not just in `relative_path`.
    std::fs::write(
        &report,
        r#"{
  "overall": { "scanned_lines": 42 },
  "incomplete": true,
  "skipped_files": [
    {"relative_path": "src/a\"]b.js", "reason": "parse_SyntaxError", "detail": null},
    {"relative_path": "src/second.js", "reason": "parse_SyntaxError", "detail": "note: a\"]b"},
    {"relative_path": "src/other.java", "reason": "parse_SyntaxError", "detail": null}
  ]
}
"#,
    )?;

    let output = run_sourced(&format!("extract_skipped_count '{}'", report.display()))?;

    assert!(
        output.status.success(),
        "extract_skipped_count failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "3",
        "expected all three skipped entries to survive the escaped '\"]' in a path and a detail, got: {stdout}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_row_carries_the_head_sha() -> anyhow::Result<()> {
    let output = run_sourced("date_cell")?;
    assert!(
        output.status.success(),
        "date_cell failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let after = stdout
        .split("nsd@")
        .nth(1)
        .unwrap_or_else(|| panic!("expected the date cell to carry `nsd@<sha>`, got: {stdout}"));
    let hex: String = after
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    assert_eq!(
        hex.len(),
        40,
        "expected a 40-hex HEAD sha right after `nsd@`, got: {stdout}"
    );
    let head = Command::new("git")
        .arg("-C")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .args(["rev-parse", "HEAD"])
        .output()?;
    assert!(
        head.status.success(),
        "git rev-parse HEAD failed: {}",
        String::from_utf8_lossy(&head.stderr)
    );
    let expected_head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    assert_eq!(
        hex, expected_head,
        "expected the date cell's sha to be this checkout's own HEAD, got: {stdout}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_row_marks_a_dirty_src_tree() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path();
    let run_git = |args: &[&str]| -> anyhow::Result<Output> {
        Ok(Command::new("git").arg("-C").arg(repo).args(args).output()?)
    };
    run_git(&["init"])?;
    run_git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--allow-empty",
        "-m",
        "x",
    ])?;

    let clean = run_sourced(&format!("REPO_ROOT='{}'; date_cell", repo.display()))?;
    assert!(
        clean.status.success(),
        "date_cell failed on a clean tree: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let clean_stdout = String::from_utf8_lossy(&clean.stdout);
    assert!(
        !clean_stdout.contains("+dirty"),
        "expected a clean src tree to carry no +dirty, got: {clean_stdout}"
    );

    std::fs::create_dir_all(repo.join("src"))?;
    std::fs::write(repo.join("src/x.rs"), "")?;

    let dirty = run_sourced(&format!("REPO_ROOT='{}'; date_cell", repo.display()))?;
    assert!(
        dirty.status.success(),
        "date_cell failed on a dirty tree: {}",
        String::from_utf8_lossy(&dirty.stderr)
    );
    let dirty_stdout = String::from_utf8_lossy(&dirty.stdout);
    assert!(
        dirty_stdout.trim_end().ends_with("+dirty)"),
        "expected an untracked src/ file to mark the row +dirty, got: {dirty_stdout}"
    );
    Ok(())
}
