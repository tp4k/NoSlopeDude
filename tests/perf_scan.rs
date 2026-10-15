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
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(output)
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

#[cfg(unix)]
#[test]
fn test_row_marks_a_dirty_cargo_manifest() -> anyhow::Result<()> {
    // Same as test_row_marks_a_dirty_src_tree, but the untracked change is
    // to Cargo.toml alone (no src/): B12's pathspec is `-- src Cargo.toml
    // Cargo.lock`, and narrowing it to `-- src` alone still passes every
    // other test, so this is the only assertion that exercises the
    // Cargo.toml/Cargo.lock half of that pathspec.
    let dir = tempfile::tempdir()?;
    let repo = dir.path();
    let run_git = |args: &[&str]| -> anyhow::Result<Output> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(output)
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

    std::fs::write(repo.join("Cargo.toml"), "")?;

    let output = run_sourced(&format!("REPO_ROOT='{}'; date_cell", repo.display()))?;
    assert!(
        output.status.success(),
        "date_cell failed on an untracked Cargo.toml-only tree: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim_end().ends_with("+dirty)"),
        "expected an untracked Cargo.toml to mark the row +dirty, got: {stdout}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_date_cell_fails_outside_a_git_repo() -> anyhow::Result<()> {
    // A REPO_ROOT that is not inside any git repository: `git rev-parse
    // HEAD` and `git status --porcelain` both fail there. Command
    // substitution clears `set -e`, so without an explicit `|| return 1`
    // on each of those calls, date_cell prints `(nsd@)` and exits 0
    // instead of failing closed.
    let dir = tempfile::tempdir()?;
    let output = run_sourced(&format!("REPO_ROOT='{}'; date_cell", dir.path().display()))?;
    assert!(
        !output.status.success(),
        "expected date_cell to fail outside a git repo, got success with stdout {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    // The same failure, exercised through the command-substitution shape
    // `main` actually uses (`date_cell_value="$(date_cell)"`, :182), not
    // just a direct call: the :135 guard is the one that only matters in
    // this shape, since a direct call's own exit status already propagates
    // without it.
    let sub_output = run_sourced(&format!(
        "REPO_ROOT='{}'; cell=\"$(date_cell)\"",
        dir.path().display()
    ))?;
    assert!(
        !sub_output.status.success(),
        "expected date_cell to fail outside a git repo via command substitution, got success with stdout {:?}",
        String::from_utf8_lossy(&sub_output.stdout)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_date_cell_fails_on_an_unborn_head() -> anyhow::Result<()> {
    // A repo with no commits yet: `git rev-parse HEAD` fails ("unknown
    // revision or path not in the working tree"), but `git status
    // --porcelain` still succeeds. Without the :116 `|| return 1` guard,
    // head_sha_annotation keeps going with an empty/garbage sha instead of
    // failing closed.
    let dir = tempfile::tempdir()?;
    let repo = dir.path();
    let init = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("init")
        .output()?;
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let output = run_sourced(&format!("REPO_ROOT='{}'; date_cell", repo.display()))?;
    assert!(
        !output.status.success(),
        "expected date_cell to fail on an unborn HEAD, got success with stdout {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_date_cell_fails_when_git_status_fails() -> anyhow::Result<()> {
    // A repo with a real commit but a corrupted `.git/index`: `git
    // rev-parse HEAD` succeeds, but `git status --porcelain` fails
    // ("index file smaller than expected"). Without the :117 `|| return 1`
    // guard, head_sha_annotation would print a clean-looking `(nsd@<sha>)`
    // even though the dirty check never ran.
    let dir = tempfile::tempdir()?;
    let repo = dir.path();
    let run_git = |args: &[&str]| -> anyhow::Result<Output> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(output)
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
    std::fs::write(repo.join(".git/index"), "garbage")?;

    let output = run_sourced(&format!("REPO_ROOT='{}'; date_cell", repo.display()))?;
    assert!(
        !output.status.success(),
        "expected date_cell to fail when git status fails on a corrupt index, got success with stdout {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    // The `ann=` guard in `date_cell` only matters under a command
    // substitution, where errexit is cleared: without it the call prints
    // `<day> ` and exits 0.
    let sub_output = run_sourced(&format!(
        "REPO_ROOT='{}'; cell=\"$(date_cell)\"",
        repo.display()
    ))?;
    assert!(
        !sub_output.status.success(),
        "expected date_cell to fail on a corrupt index via command substitution, got success with stdout {:?}",
        String::from_utf8_lossy(&sub_output.stdout)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_incomplete_survives_a_skipped_path_named_incomplete() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let report = dir.path().join("report.json");
    // The skipped entry's path value is the string "incomplete" and is
    // listed before the top-level key: an unanchored first-match grep reads
    // that entry's line, which carries no boolean.
    std::fs::write(
        &report,
        r#"{
  "overall": { "scanned_lines": 42 },
  "skipped_files": [
    {
      "relative_path": "incomplete",
      "reason": "parse_SyntaxError",
      "detail": null
    }
  ],
  "incomplete": true
}
"#,
    )?;

    let output = run_sourced(&format!("extract_incomplete '{}'", report.display()))?;

    assert!(
        output.status.success(),
        "extract_incomplete failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_date_cell_fails_when_date_fails() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    let shim = dir.path().join("shim");
    std::fs::create_dir_all(&repo)?;
    std::fs::create_dir_all(&shim)?;
    let run_git = |args: &[&str]| -> anyhow::Result<()> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
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
    let date_shim = shim.join("date");
    std::fs::write(&date_shim, "#!/bin/sh\nexit 1\n")?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&date_shim, std::fs::Permissions::from_mode(0o755))?;
    }

    // The sha annotation succeeds (real git), so only `date -u` can fail.
    let output = run_sourced(&format!(
        "REPO_ROOT='{}'; PATH='{}':\"$PATH\"; date_cell",
        repo.display(),
        shim.display()
    ))?;
    assert!(
        !output.status.success(),
        "expected date_cell to fail when date fails, got success with stdout {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stdout.is_empty(),
        "expected no cell on failure, got {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    // Mirrors main's `date_cell_value="$(date_cell)"`: errexit does not
    // apply inside the command substitution, so only date_cell's own
    // `|| return 1` makes the assignment fail.
    let output = run_sourced(&format!(
        "REPO_ROOT='{}'; PATH='{}':\"$PATH\"; v=\"$(date_cell)\"; echo reached",
        repo.display(),
        shim.display()
    ))?;
    assert!(
        !output.status.success(),
        "expected the command-substitution call to fail, got success with stdout {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("reached"),
        "script continued past a failed date_cell, stdout {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_build_plain_tree_drops_each_clones_git_entry_and_keeps_everything_else(
) -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("fixture");
    let dest = dir.path().join("plain");
    std::fs::create_dir_all(&dest)?;
    for clone in ["spring-framework", "angular"] {
        let base = root.join(clone);
        std::fs::create_dir_all(base.join(".git/objects"))?;
        std::fs::write(base.join(".git/HEAD"), "ref: refs/heads/main\n")?;
        std::fs::create_dir_all(base.join("src/sub"))?;
        std::fs::write(base.join("src/sub/A.java"), "class A {}\n")?;
        std::fs::write(base.join(".gitignore"), "target/\n")?;
        // `.git` below the clone's top level (e.g. a vendored directory)
        // is ordinary content of the working tree and must survive.
        std::fs::create_dir_all(base.join("vendor/.git"))?;
        std::fs::write(base.join("vendor/.git/keep"), "x")?;
    }

    let output = run_sourced(&format!(
        "build_plain_tree '{}' '{}'",
        root.display(),
        dest.display()
    ))?;
    assert!(
        output.status.success(),
        "build_plain_tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for clone in ["spring-framework", "angular"] {
        let base = dest.join(clone);
        assert!(
            !base.join(".git").exists(),
            "{clone}: top-level .git must not be copied"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("src/sub/A.java"))?,
            "class A {}\n",
            "{clone}: working-tree file missing"
        );
        assert!(
            base.join(".gitignore").is_file(),
            "{clone}: dotfile .gitignore missing"
        );
        assert!(
            base.join("vendor/.git/keep").is_file(),
            "{clone}: a non-top-level .git must survive"
        );
        assert!(
            root.join(clone).join(".git/HEAD").is_file(),
            "{clone}: the source clone must be untouched"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_row_is_inserted_at_the_end_of_the_scan_table() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("measurements.md");
    // A prose section and a second table follow the scan table, so EOF is
    // not where the new row belongs.
    let before = "# Title\n\n## Rows\n\nlegend\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n\n## Check rows\n\n| c |\n| --- |\n| 9 |\n\nclosing prose\n";
    std::fs::write(&file, before)?;

    let output = run_sourced(&format!(
        "insert_table_row '{}' '## Rows' '| 5 | 6 |'",
        file.display()
    ))?;
    assert!(
        output.status.success(),
        "insert_table_row failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = before.replace("| 3 | 4 |\n", "| 3 | 4 |\n| 5 | 6 |\n");
    assert_eq!(std::fs::read_to_string(&file)?, expected);

    // A missing heading must fail closed, not fall back to appending at EOF.
    let missing = dir.path().join("missing.md");
    std::fs::write(&missing, "# Title\n\n| a |\n| --- |\n| 1 |\n")?;
    let output = run_sourced(&format!(
        "insert_table_row '{}' '## Rows' '| 5 |'",
        missing.display()
    ))?;
    assert!(
        !output.status.success(),
        "expected a nonzero exit when the heading is absent"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("error:"),
        "expected an `error:` line, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&missing)?,
        "# Title\n\n| a |\n| --- |\n| 1 |\n",
        "a refused insert must leave the file unchanged"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_failed_plain_dir_mktemp_leaks_no_temp_files() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let shim = dir.path().join("shim");
    let log = dir.path().join("created");
    std::fs::create_dir_all(&shim)?;
    // Fails only the third mktemp (the plain tree's, named by its template);
    // the other two run the real tool and record what they created.
    let mktemp = shim.join("mktemp");
    std::fs::write(
        &mktemp,
        "#!/bin/sh\nif [ -n \"$FAIL_PLAIN\" ]; then case \"$*\" in *nsd-perf-plain*) exit 1;; esac; fi\np=$(/usr/bin/mktemp \"$@\") || exit 1\necho \"$p\" >>\"$MKTEMP_LOG\"\necho \"$p\"\n",
    )?;
    std::fs::set_permissions(&mktemp, std::fs::Permissions::from_mode(0o755))?;
    let run = |fail: &str| -> anyhow::Result<Output> {
        std::fs::write(&log, "")?;
        Ok(Command::new("bash")
            .arg("-c")
            .arg(format!(
                "source \"{}\" && PATH='{}':\"$PATH\"; make_temp_paths",
                script_path(),
                shim.display()
            ))
            .env("MKTEMP_LOG", &log)
            .env("FAIL_PLAIN", fail)
            .output()?)
    };
    let created = || -> anyhow::Result<Vec<String>> {
        Ok(std::fs::read_to_string(&log)?
            .lines()
            .map(str::to_owned)
            .collect())
    };

    let output = run("1")?;
    assert!(
        !output.status.success(),
        "expected make_temp_paths to fail when the plain_dir mktemp fails"
    );
    let leaked: Vec<String> = created()?
        .into_iter()
        .filter(|p| std::path::Path::new(p).exists())
        .collect();
    assert_eq!(
        leaked,
        Vec::<String>::new(),
        "a failed third mktemp leaked output_dir/time_log"
    );

    // Success path: the EXIT trap must remove all three, plain_dir included.
    let output = run("")?;
    assert!(
        output.status.success(),
        "make_temp_paths failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let created = created()?;
    assert_eq!(
        created.len(),
        3,
        "expected three temp paths, got {created:?}"
    );
    let left: Vec<&String> = created
        .iter()
        .filter(|p| std::path::Path::new(p).exists())
        .collect();
    assert!(left.is_empty(), "the EXIT trap left {left:?} behind");
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_insert_refuses_a_heading_whose_section_has_no_table() -> anyhow::Result<()> {
    // `## Rows` holds no table; the next section's table must not receive
    // the row just because it is the first table after the heading.
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("measurements.md");
    let before = "## Rows\n\nprose only\n\n## Check rows\n\n| c |\n| --- |\n| 9 |\n";
    std::fs::write(&file, before)?;

    let output = run_sourced(&format!(
        "insert_table_row '{}' '## Rows' '| 5 |'",
        file.display()
    ))?;
    assert!(
        !output.status.success(),
        "expected a nonzero exit for a section without a table"
    );
    assert_eq!(std::fs::read_to_string(&file)?, before);
    Ok(())
}
