//! WS-3: `scripts/perf_check.sh` measures cold and warm `nsd check` time and
//! the cache size. Like `perf_scan.rs`, these tests source the script and
//! call its functions; the end-to-end ones drive `main` with a fake `nsd`
//! binary so no real fixture, network or cargo build is needed.

use std::path::Path;
use std::process::{Command, Output};

fn script_path() -> String {
    format!("{}/scripts/perf_check.sh", env!("CARGO_MANIFEST_DIR"))
}

fn run_sourced(call: &str) -> anyhow::Result<Output> {
    Ok(Command::new("bash")
        .arg("-c")
        .arg(format!("source \"{}\" && {}", script_path(), call))
        .output()?)
}

fn git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(["-c", "maintenance.auto=false", "-c", "gc.auto=0"])
        .args(args)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Two real git clones with one commit each, as `fetch_perf_fixture.sh`
/// would leave them.
fn make_fixture(root: &Path) -> anyhow::Result<()> {
    for clone in ["spring-framework", "angular"] {
        let base = root.join(clone);
        std::fs::create_dir_all(base.join("src/sub"))?;
        std::fs::write(base.join("src/sub/A.java"), "class A {}\n")?;
        git(&base, &["init", "--quiet"])?;
        git(&base, &["add", "-A"])?;
        git(&base, &["commit", "--quiet", "-m", "base"])?;
    }
    Ok(())
}

fn snapshot(root: &Path) -> anyhow::Result<String> {
    let output = Command::new("find").arg(root).output()?;
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    lines.sort();
    Ok(lines.join("\n"))
}

const MEASUREMENTS: &str = "# M\n\n## Rows\n\n| a |\n| --- |\n| 1 |\n\n## Check rows\n\nlegend\n\n| date | machine |\n| --- | --- |\n| old | old |\n\ntrailing prose\n";

// The end-to-end tests drive `main`, which times each run with macOS
// `/usr/bin/time -l` (GNU time has no `-l`), so they and this harness are
// macOS-only; the function-level tests run on any unix.
#[cfg(target_os = "macos")]
const FAKE_NSD: &str = r#"#!/bin/sh
git_dir="$(git rev-parse --absolute-git-dir)"
n=$(cat "$FAKE_STATE/count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" >"$FAKE_STATE/count"
{
  echo "call $n pwd=$(pwd) args=$*"
  echo "stale=$([ -e "$git_dir/nsd/cache/stale" ] && echo yes || echo no) blob=$([ -e "$git_dir/nsd/cache/v1/blob" ] && echo yes || echo no)"
  echo "changed=$(git diff --name-only HEAD~1 HEAD | tr '\n' ' ')"
} >>"$FAKE_STATE/calls"
mkdir -p "$git_dir/nsd/cache/v1"
head -c 1048576 /dev/zero >"$git_dir/nsd/cache/v1/blob"
status=${FAKE_EXIT:-0}
doc_status=${FAKE_DOC_STATUS:-$status}
doc_head="\"diagnostics\":[],\"exit_status\":$doc_status,\"result_scope\":\"check\""
if [ -n "${FAKE_BODY+x}" ]; then
  printf '%s' "$FAKE_BODY"
elif [ -n "$FAKE_DIFFER" ]; then
  echo "{$doc_head,\"run\":$n,\"schema_version\":1}"
else
  echo "{$doc_head,\"schema_version\":1}"
fi
if [ -n "${FAKE_STDERR+x}" ]; then printf '%s\n' "$FAKE_STDERR" >&2; fi
if [ "$n" = 2 ] && [ -n "${FAKE_WARM_EXIT+x}" ]; then status=$FAKE_WARM_EXIT; fi
if [ "$n" = 1 ] && [ -n "${FAKE_COLD_EXIT+x}" ]; then status=$FAKE_COLD_EXIT; fi
exit "$status"
"#;

#[cfg(target_os = "macos")]
struct Harness {
    _dir: tempfile::TempDir,
    fixture: std::path::PathBuf,
    measurements: std::path::PathBuf,
    state: std::path::PathBuf,
    fake: std::path::PathBuf,
}

#[cfg(target_os = "macos")]
fn harness() -> anyhow::Result<Harness> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let fixture = dir.path().join("fixture");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&fixture)?;
    std::fs::create_dir_all(&state)?;
    make_fixture(&fixture)?;
    let measurements = dir.path().join("measurements.md");
    std::fs::write(&measurements, MEASUREMENTS)?;
    let fake = dir.path().join("fake-nsd");
    std::fs::write(&fake, FAKE_NSD)?;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))?;
    Ok(Harness {
        _dir: dir,
        fixture,
        measurements,
        state,
        fake,
    })
}

#[cfg(target_os = "macos")]
fn run_main(h: &Harness, differ: bool, template: Option<&Path>) -> anyhow::Result<Output> {
    run_main_env(h, differ, template, &[])
}

#[cfg(target_os = "macos")]
fn run_main_env(
    h: &Harness,
    differ: bool,
    template: Option<&Path>,
    extra_env: &[(&str, &str)],
) -> anyhow::Result<Output> {
    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(format!(
            "source \"{}\" && MEASUREMENTS_FILE='{}'; main '{}' '{}'",
            script_path(),
            h.measurements.display(),
            h.fixture.display(),
            h.fake.display()
        ))
        .env("FAKE_STATE", &h.state)
        .envs(extra_env.iter().copied());
    if differ {
        cmd.env("FAKE_DIFFER", "1");
    }
    if let Some(template) = template {
        cmd.env("GIT_TEMPLATE_DIR", template);
    }
    Ok(cmd.output()?)
}

#[cfg(unix)]
#[test]
fn test_check_row_is_inserted_at_the_end_of_the_check_table() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("measurements.md");
    std::fs::write(&file, MEASUREMENTS)?;

    let output = run_sourced(&format!(
        "insert_table_row '{}' '## Check rows' '| new | new |'",
        file.display()
    ))?;
    assert!(
        output.status.success(),
        "insert_table_row failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&file)?,
        MEASUREMENTS.replace("| old | old |\n", "| old | old |\n| new | new |\n"),
        "the row must land after the last check-table row and leave the scan table alone"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_missing_fixture_exits_1_and_changes_nothing() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let measurements = format!("{}/docs/measurements.md", env!("CARGO_MANIFEST_DIR"));
    let before = std::fs::read(&measurements)?;
    let output = Command::new("bash")
        .arg(script_path())
        .arg(dir.path().join("no-such-fixture"))
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error:") && stderr.contains("scripts/fetch_perf_fixture.sh"),
        "expected an `error:` line naming scripts/fetch_perf_fixture.sh, got: {stderr}"
    );
    assert_eq!(
        std::fs::read(&measurements)?,
        before,
        "a refused run must not touch docs/measurements.md"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_differing_cold_and_warm_output_refuses_the_row() -> anyhow::Result<()> {
    let h = harness()?;
    let output = run_main(&h, true, None)?;
    assert!(
        !output.status.success(),
        "expected a nonzero exit when cold and warm output differ, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error:") && stderr.contains("differ"),
        "expected an `error:` line saying the outputs differ, got: {stderr}"
    );
    let calls = std::fs::read_to_string(h.state.join("calls"))?;
    assert_eq!(
        calls.lines().filter(|l| l.starts_with("call ")).count(),
        2,
        "both runs must complete before the comparison refuses, got: {calls}"
    );
    assert_eq!(
        std::fs::read_to_string(&h.measurements)?,
        MEASUREMENTS,
        "a refused run must not append a row"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_scratch_repo_has_a_base_and_a_one_file_candidate() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let fixture = dir.path().join("fixture");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&fixture)?;
    make_fixture(&fixture)?;
    let before = snapshot(&fixture)?;

    let output = run_sourced(&format!(
        "build_check_repo '{}' '{}'",
        fixture.display(),
        repo.display()
    ))?;
    assert!(
        output.status.success(),
        "build_check_repo failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(git(&repo, &["rev-list", "--count", "HEAD"])?.trim(), "2");
    let diff = git(&repo, &["diff", "--name-status", "HEAD~1", "HEAD"])?;
    let lines: Vec<&str> = diff.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly one candidate file, got: {diff}"
    );
    assert!(
        lines[0].starts_with("A\t") && lines[0].ends_with(".java"),
        "expected one added .java file, got: {diff}"
    );
    let base_files = git(&repo, &["ls-tree", "-r", "--name-only", "HEAD~1"])?;
    for clone in ["spring-framework", "angular"] {
        assert!(
            base_files.contains(&format!("{clone}/src/sub/A.java")),
            "base commit lacks {clone}'s files: {base_files}"
        );
    }
    assert_eq!(
        snapshot(&fixture)?,
        before,
        "the source fixture must be unchanged"
    );
    for clone in ["spring-framework", "angular"] {
        assert_eq!(git(&fixture.join(clone), &["status", "--porcelain"])?, "");
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_cache_size_is_read_from_the_git_dir_cache() -> anyhow::Result<()> {
    const MIB: usize = 1024 * 1024;
    let dir = tempfile::tempdir()?;
    let repo = dir.path();
    git(repo, &["init", "--quiet"])?;
    std::fs::create_dir_all(repo.join(".git/nsd/cache/v1"))?;
    std::fs::write(repo.join(".git/nsd/cache/v1/blob"), vec![1u8; 4 * MIB])?;
    // Outside `<git dir>/nsd/cache`: must not be counted.
    std::fs::write(repo.join(".git/nsd/other"), vec![1u8; 4 * MIB])?;

    let output = run_sourced(&format!("cache_size_mb '{}'", repo.display()))?;
    assert!(
        output.status.success(),
        "cache_size_mb failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mb: f64 = stdout.trim().parse()?;
    assert!(
        (4.0..4.1).contains(&mb),
        "expected about 4 MB for a 4 MiB cache, got {stdout}"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_main_runs_a_cold_then_a_warm_check_in_the_scratch_repo() -> anyhow::Result<()> {
    let h = harness()?;
    // A stale cache planted into every fresh `.git` by git's own template
    // mechanism: the cold run must not see it.
    let template = h.state.join("template");
    std::fs::create_dir_all(template.join("nsd/cache"))?;
    std::fs::write(template.join("nsd/cache/stale"), "x")?;
    let fixture_before = snapshot(&h.fixture)?;

    let output = run_main(&h, false, Some(&template))?;
    assert!(
        output.status.success(),
        "main failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = std::fs::read_to_string(h.state.join("calls"))?;
    let expected_args = "args=check --base HEAD~1 --format json";
    let call_lines: Vec<&str> = calls.lines().filter(|l| l.starts_with("call ")).collect();
    assert_eq!(
        call_lines.len(),
        2,
        "expected a cold and a warm run, got: {calls}"
    );
    for line in &call_lines {
        assert!(
            line.contains(expected_args),
            "unexpected check command: {line}"
        );
        assert!(
            !line.contains(h.fixture.to_string_lossy().as_ref()),
            "the check must run in the scratch repo, not the fixture: {line}"
        );
    }
    let cache_lines: Vec<&str> = calls.lines().filter(|l| l.starts_with("stale=")).collect();
    assert_eq!(
        cache_lines,
        ["stale=no blob=no", "stale=no blob=yes"],
        "cold must start from an empty cache and warm must reuse the cold run's"
    );
    for changed in calls.lines().filter(|l| l.starts_with("changed=")) {
        assert!(
            changed.trim_end().ends_with(".java") && changed.matches(".java").count() == 1,
            "expected the candidate to change exactly one .java file, got: {changed}"
        );
    }
    assert_eq!(
        snapshot(&h.fixture)?,
        fixture_before,
        "the fixture must be unchanged"
    );
    let scratch = call_lines[0]
        .split_once("pwd=")
        .and_then(|(_, rest)| rest.split(" args=").next())
        .ok_or_else(|| anyhow::anyhow!("no pwd= in call line: {}", call_lines[0]))?;
    assert!(
        !Path::new(scratch).exists(),
        "the scratch repo must be removed on exit"
    );

    let measurements = std::fs::read_to_string(&h.measurements)?;
    let new_rows: Vec<&str> = measurements
        .lines()
        .filter(|l| l.starts_with('|') && !MEASUREMENTS.lines().any(|m| m == *l))
        .collect();
    assert_eq!(
        new_rows.len(),
        1,
        "expected exactly one new row, got: {new_rows:?}"
    );
    assert!(
        measurements.contains(&format!("| old | old |\n{}\n\ntrailing prose", new_rows[0])),
        "the row must follow the last row of the check table"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(new_rows[0]),
        "the row must also be printed"
    );
    let cells: Vec<&str> = new_rows[0].split('|').map(str::trim).collect();
    let cache_mb: f64 = cells[10].trim_end_matches(" MB").parse()?;
    assert!(
        (1.0..1.1).contains(&cache_mb),
        "expected about 1 MB of cache, got cell {:?}",
        cells[10]
    );
    assert_eq!(cells[11], "0", "the check's exit status column");
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_time_cells_reads_wall_seconds_and_rss_bytes() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("check.time");
    std::fs::write(
        &log,
        "        3.37 real         2.50 user         0.40 sys\n  978321408  maximum resident set size\n",
    )?;
    let output = run_sourced(&format!("time_cells '{}'", log.display()))?;
    assert!(
        output.status.success(),
        "time_cells failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3.37 933.00");
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_row_records_a_nonzero_check_exit_status() -> anyhow::Result<()> {
    let h = harness()?;
    let output = run_main_env(&h, false, None, &[("FAKE_EXIT", "2")])?;
    assert!(
        output.status.success(),
        "a failing check is still a measured run, main failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let measurements = std::fs::read_to_string(&h.measurements)?;
    let new_rows: Vec<&str> = measurements
        .lines()
        .filter(|l| l.starts_with('|') && !MEASUREMENTS.lines().any(|m| m == *l))
        .collect();
    assert_eq!(
        new_rows.len(),
        1,
        "expected exactly one new row, got: {new_rows:?}"
    );
    let cells: Vec<&str> = new_rows[0].split('|').map(str::trim).collect();
    assert_eq!(cells[11], "2", "the check's exit status column");
    Ok(())
}

/// Runs `main` with the given fake-`nsd` env and asserts the row is refused:
/// nonzero exit, an `error:` line, and the measurements file untouched.
#[cfg(target_os = "macos")]
fn assert_row_refused(env: &[(&str, &str)]) -> anyhow::Result<String> {
    let h = harness()?;
    let output = run_main_env(&h, false, None, env)?;
    assert!(
        !output.status.success(),
        "expected a nonzero exit for {env:?}, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error:"),
        "expected an `error:` line for {env:?}, got: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&h.measurements)?,
        MEASUREMENTS,
        "a refused run must not append a row for {env:?}"
    );
    Ok(stderr.into_owned())
}

#[cfg(target_os = "macos")]
#[test]
fn test_crashed_check_with_empty_output_refuses_the_row() -> anyhow::Result<()> {
    assert_row_refused(&[("FAKE_EXIT", "101"), ("FAKE_BODY", "")])?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_status_outside_zero_to_three_refuses_the_row() -> anyhow::Result<()> {
    // The document agrees with the process, so only the range can refuse.
    assert_row_refused(&[("FAKE_EXIT", "125")])?;
    assert_row_refused(&[("FAKE_EXIT", "4")])?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_output_that_is_not_a_check_document_refuses_the_row() -> anyhow::Result<()> {
    assert_row_refused(&[("FAKE_BODY", "{\"summaries\":[]}\n")])?;
    assert_row_refused(&[(
        "FAKE_BODY",
        "{\"diagnostics\":[],\"exit_status\":0,\"result_scope\":\"scan\",\"schema_version\":1}\n",
    )])?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_json_exit_status_disagreeing_with_the_process_refuses_the_row() -> anyhow::Result<()> {
    assert_row_refused(&[("FAKE_EXIT", "2"), ("FAKE_DOC_STATUS", "0")])?;
    assert_row_refused(&[("FAKE_EXIT", "1"), ("FAKE_DOC_STATUS", "3")])?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_warm_run_status_disagreeing_with_its_output_refuses_the_row() -> anyhow::Result<()> {
    let stderr = assert_row_refused(&[("FAKE_WARM_EXIT", "101")])?;
    assert!(
        stderr.contains("error: warm check exited 101"),
        "the error must name the warm run and its status, got: {stderr}"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_exit_status_3_is_recorded() -> anyhow::Result<()> {
    let h = harness()?;
    let output = run_main_env(&h, false, None, &[("FAKE_EXIT", "3")])?;
    assert!(
        output.status.success(),
        "status 3 is a legal check status, main failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let measurements = std::fs::read_to_string(&h.measurements)?;
    let new_rows: Vec<&str> = measurements
        .lines()
        .filter(|l| l.starts_with('|') && !MEASUREMENTS.lines().any(|m| m == *l))
        .collect();
    assert_eq!(
        new_rows.len(),
        1,
        "expected exactly one new row, got: {new_rows:?}"
    );
    let cells: Vec<&str> = new_rows[0].split('|').map(str::trim).collect();
    assert_eq!(cells[11], "3", "the check's exit status column");
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_cold_run_status_disagreeing_with_its_output_refuses_the_row() -> anyhow::Result<()> {
    let stderr = assert_row_refused(&[("FAKE_COLD_EXIT", "101")])?;
    assert!(
        stderr.contains("error: cold check exited 101"),
        "the error must name the cold run and its status, got: {stderr}"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_empty_or_unsupported_documents_refuse_the_row_at_a_legal_status() -> anyhow::Result<()> {
    let doc =
        "{\"diagnostics\":[],\"exit_status\":0,\"result_scope\":\"check\",\"schema_version\":";
    assert_row_refused(&[("FAKE_BODY", "")])?;
    assert_row_refused(&[("FAKE_EXIT", "2"), ("FAKE_BODY", "")])?;
    assert_row_refused(&[("FAKE_BODY", &format!("{doc}2}}\n"))])?;
    assert_row_refused(&[("FAKE_BODY", &format!("{doc}1}}\n{doc}1}}\n"))])?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_legal_warm_status_disagreeing_with_its_output_refuses_the_row() -> anyhow::Result<()> {
    // Status 2 is legal and both documents report 0, so only the warm
    // exit_status comparison refuses; the outputs match each other.
    let stderr = assert_row_refused(&[("FAKE_WARM_EXIT", "2")])?;
    assert!(
        stderr.contains("error: warm check output") && stderr.contains("exit_status 2"),
        "the error must name the warm run and status 2, got: {stderr}"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_legal_cold_status_disagreeing_with_its_output_refuses_the_row() -> anyhow::Result<()> {
    let stderr = assert_row_refused(&[("FAKE_COLD_EXIT", "2")])?;
    assert!(
        stderr.contains("error: cold check output") && stderr.contains("exit_status 2"),
        "the error must name the cold run and status 2, got: {stderr}"
    );
    Ok(())
}

const TIME_LOOKALIKE_STDERR: &str = "9.99 real\n123 maximum resident set size";

#[cfg(target_os = "macos")]
#[test]
fn test_nsd_stderr_cannot_alter_wall_and_rss_cells() -> anyhow::Result<()> {
    let h = harness()?;
    let output = run_main_env(&h, false, None, &[("FAKE_STDERR", TIME_LOOKALIKE_STDERR)])?;
    assert!(
        output.status.success(),
        "main failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let measurements = std::fs::read_to_string(&h.measurements)?;
    let new_rows: Vec<&str> = measurements
        .lines()
        .filter(|l| l.starts_with('|') && !MEASUREMENTS.lines().any(|m| m == *l))
        .collect();
    assert_eq!(new_rows.len(), 1, "expected one intact row: {new_rows:?}");
    let cells: Vec<&str> = new_rows[0].split('|').map(str::trim).collect();
    for (wall, rss) in [(cells[6], cells[7]), (cells[8], cells[9])] {
        assert_ne!(
            wall, "9.99s",
            "wall cell taken from nsd stderr: {new_rows:?}"
        );
        assert!(wall.ends_with('s'), "wall cell: {wall}");
        assert_ne!(
            rss, "0.00 MB",
            "RSS cell taken from nsd stderr: {new_rows:?}"
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_run_timed_check_keeps_nsd_stderr_in_its_own_file() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let fake = dir.path().join("fake-nsd");
    std::fs::write(
        &fake,
        "#!/bin/sh\necho '{}'\nprintf '9.99 real\\n123 maximum resident set size\\n' >&2\n",
    )?;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))?;
    let (json, time_log, nsd_err) = (
        dir.path().join("out.json"),
        dir.path().join("out.time"),
        dir.path().join("out.nsd-stderr"),
    );
    let output = run_sourced(&format!(
        "run_timed_check '{}' '{}' '{}' '{}' '{}'",
        fake.display(),
        dir.path().display(),
        json.display(),
        time_log.display(),
        nsd_err.display()
    ))?;
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0");
    assert!(nsd_err.exists(), "no separate nsd stderr file was written");
    let kept = std::fs::read_to_string(&nsd_err)?;
    assert!(
        kept.contains("9.99 real") && kept.contains("123 maximum resident set size"),
        "nsd stderr must be kept for diagnosis, got: {kept:?}"
    );
    let timing = std::fs::read_to_string(&time_log)?;
    assert!(
        !timing.contains("9.99 real") && !timing.contains("123 maximum"),
        "time output must not hold nsd stderr, got: {timing:?}"
    );
    assert!(timing.contains(" real"), "time output missing: {timing:?}");
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn test_long_nsd_stderr_cannot_alter_wall_and_rss_cells() -> anyhow::Result<()> {
    const PADDING_LINES: usize = 200;
    let padding = "x".repeat(40);
    let long_stderr = format!(
        "{}\n{TIME_LOOKALIKE_STDERR}",
        vec![padding; PADDING_LINES].join("\n")
    );
    assert!(long_stderr.len() >= 4096, "padding must exceed 4 KB");
    let h = harness()?;
    let output = run_main_env(&h, false, None, &[("FAKE_STDERR", &long_stderr)])?;
    assert!(
        output.status.success(),
        "main failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let measurements = std::fs::read_to_string(&h.measurements)?;
    let new_rows: Vec<&str> = measurements
        .lines()
        .filter(|l| l.starts_with('|') && !MEASUREMENTS.lines().any(|m| m == *l))
        .collect();
    assert_eq!(new_rows.len(), 1, "expected one intact row: {new_rows:?}");
    let cells: Vec<&str> = new_rows[0].split('|').map(str::trim).collect();
    for (wall, rss) in [(cells[6], cells[7]), (cells[8], cells[9])] {
        assert_ne!(
            wall, "9.99s",
            "wall cell taken from nsd stderr: {new_rows:?}"
        );
        assert!(wall.ends_with('s'), "wall cell: {wall}");
        assert_ne!(
            rss, "0.00 MB",
            "RSS cell taken from nsd stderr: {new_rows:?}"
        );
    }
    Ok(())
}
