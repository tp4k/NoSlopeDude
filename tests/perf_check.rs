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
if [ -n "$FAKE_DIFFER" ]; then echo "{\"run\":$n}"; else echo '{"summaries":[]}'; fi
exit 0
"#;

struct Harness {
    _dir: tempfile::TempDir,
    fixture: std::path::PathBuf,
    measurements: std::path::PathBuf,
    state: std::path::PathBuf,
    fake: std::path::PathBuf,
}

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

fn run_main(h: &Harness, differ: bool, template: Option<&Path>) -> anyhow::Result<Output> {
    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(format!(
            "source \"{}\" && MEASUREMENTS_FILE='{}'; main '{}' '{}'",
            script_path(),
            h.measurements.display(),
            h.fixture.display(),
            h.fake.display()
        ))
        .env("FAKE_STATE", &h.state);
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

#[cfg(unix)]
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

#[cfg(unix)]
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
