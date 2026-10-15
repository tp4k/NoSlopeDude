//! `nsd scan` reads `measurement.min_clone_lines` from the scanned root's
//! `nsd.yml` (amendment A2); `--min-clone-lines` overrides it.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nsd::git::snapshot::SOURCE_CEILING_BYTES;
use nsd::profile::measurement_fingerprint;

const DEFAULT_THRESHOLD: u32 = 10;
const FILE_THRESHOLD: u32 = 5;
const FLAG_THRESHOLD: u32 = 12;
const DUPLICATED_BLOCK_LINES: u64 = 6;
const NSD_YML: &str = "nsd.yml";
const INVALID_CODE: &str = "NSD-C102";

fn config_with(min_clone_lines: u32) -> String {
    format!("version: 1\nmeasurement:\n  min_clone_lines: {min_clone_lines}\n")
}

fn class_source(name: &str) -> String {
    format!(
        "class {name} {{\n    int run(int x) {{\n        int a = x + 1;\n        int b = a * 2;\n        \
         int c = b - 3;\n        int d = c / 4;\n        int e = d % 5;\n        return e;\n    }}\n}}\n"
    )
}

/// Two Java files sharing a six-line duplicated block.
fn write_tree(root: &Path) {
    for name in ["A", "B"] {
        fs::write(root.join(format!("{name}.java")), class_source(name)).expect("write source");
    }
}

fn scan(root: &Path, extra: &[&str]) -> (Output, tempfile::TempDir) {
    let output_dir = tempfile::tempdir().expect("output dir");
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .arg("scan")
        .arg(root)
        .arg("--output")
        .arg(output_dir.path())
        .args(extra)
        .output()
        .expect("run nsd scan");
    (output, output_dir)
}

fn report(output_dir: &tempfile::TempDir) -> serde_json::Value {
    let text = fs::read_to_string(output_dir.path().join("report.json")).expect("report.json");
    serde_json::from_str(&text).expect("report.json parses")
}

fn measurement_of(report: &serde_json::Value) -> &str {
    report["fingerprints"]["measurement"]
        .as_str()
        .expect("measurement fingerprint")
}

fn redundant_lines(report: &serde_json::Value) -> Vec<u64> {
    report["duplicates"]
        .as_array()
        .expect("duplicates array")
        .iter()
        .map(|group| group["redundant_lines"].as_u64().expect("redundant_lines"))
        .collect()
}

fn assert_refused(output: &Output, output_dir: &tempfile::TempDir) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "scan must fail: {stderr}");
    assert!(stderr.starts_with("error: "), "stderr was: {stderr}");
    assert!(stderr.contains(INVALID_CODE), "stderr was: {stderr}");
    assert!(
        !output_dir.path().join("report.json").exists(),
        "no report.json may be written"
    );
}

#[test]
fn test_scan_uses_min_clone_lines_from_root_nsd_yml() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    fs::write(dir.path().join(NSD_YML), config_with(FILE_THRESHOLD)).expect("write nsd.yml");
    let (output, output_dir) = scan(dir.path(), &[]);
    assert!(output.status.success(), "{:?}", output);
    let report = report(&output_dir);
    assert_eq!(
        measurement_of(&report),
        measurement_fingerprint(FILE_THRESHOLD)
    );
    assert_eq!(redundant_lines(&report), vec![DUPLICATED_BLOCK_LINES]);
}

#[test]
fn test_scan_without_nsd_yml_uses_ten() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    let (output, output_dir) = scan(dir.path(), &[]);
    assert!(output.status.success(), "{:?}", output);
    let report = report(&output_dir);
    assert_eq!(
        measurement_of(&report),
        measurement_fingerprint(DEFAULT_THRESHOLD)
    );
    assert!(redundant_lines(&report).is_empty());
}

#[test]
fn test_min_clone_lines_flag_overrides_nsd_yml() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    fs::write(dir.path().join(NSD_YML), config_with(FILE_THRESHOLD)).expect("write nsd.yml");
    let (output, output_dir) = scan(dir.path(), &["--min-clone-lines", "12"]);
    assert!(output.status.success(), "{:?}", output);
    let report = report(&output_dir);
    assert_eq!(
        measurement_of(&report),
        measurement_fingerprint(FLAG_THRESHOLD)
    );
    assert!(redundant_lines(&report).is_empty());
}

#[test]
fn test_flag_equal_to_the_default_still_overrides_nsd_yml() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    fs::write(dir.path().join(NSD_YML), config_with(FILE_THRESHOLD)).expect("write nsd.yml");
    let (output, output_dir) = scan(dir.path(), &["--min-clone-lines", "10"]);
    assert!(output.status.success(), "{:?}", output);
    let report = report(&output_dir);
    assert_eq!(
        measurement_of(&report),
        measurement_fingerprint(DEFAULT_THRESHOLD)
    );
    assert!(redundant_lines(&report).is_empty());
}

#[test]
fn test_invalid_nsd_yml_fails_the_scan_with_c102() {
    for invalid in [
        config_with(0),
        "version: 1\nunknown_key: true\n".to_string(),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        write_tree(dir.path());
        fs::write(dir.path().join(NSD_YML), &invalid).expect("write nsd.yml");
        let (output, output_dir) = scan(dir.path(), &[]);
        assert_refused(&output, &output_dir);
    }
}

#[test]
fn test_flag_given_does_not_read_nsd_yml() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    fs::write(dir.path().join(NSD_YML), config_with(0)).expect("write nsd.yml");
    let (output, output_dir) = scan(dir.path(), &["--min-clone-lines", "7"]);
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        measurement_of(&report(&output_dir)),
        measurement_fingerprint(7)
    );
}

#[cfg(unix)]
#[test]
fn test_symlinked_nsd_yml_is_refused() {
    let outside = tempfile::tempdir().expect("outside dir");
    let target = outside.path().join("elsewhere.yml");
    fs::write(&target, config_with(FILE_THRESHOLD)).expect("write link target");
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    std::os::unix::fs::symlink(&target, dir.path().join(NSD_YML)).expect("symlink");
    let (output, output_dir) = scan(dir.path(), &[]);
    assert_refused(&output, &output_dir);
}

#[test]
fn test_nested_nsd_yml_is_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    let nested = dir.path().join("sub");
    fs::create_dir(&nested).expect("subdirectory");
    fs::write(nested.join(NSD_YML), config_with(FILE_THRESHOLD)).expect("write nsd.yml");
    let (output, output_dir) = scan(dir.path(), &[]);
    assert!(output.status.success(), "{:?}", output);
    let report = report(&output_dir);
    assert_eq!(
        measurement_of(&report),
        measurement_fingerprint(DEFAULT_THRESHOLD)
    );
    assert!(redundant_lines(&report).is_empty());
}

#[test]
fn test_parent_nsd_yml_is_ignored() {
    let parent = tempfile::tempdir().expect("tempdir");
    fs::write(parent.path().join(NSD_YML), config_with(FILE_THRESHOLD)).expect("write nsd.yml");
    let target = parent.path().join("project");
    fs::create_dir(&target).expect("target dir");
    write_tree(&target);
    let (output, output_dir) = scan(&target, &[]);
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        measurement_of(&report(&output_dir)),
        measurement_fingerprint(DEFAULT_THRESHOLD)
    );
}

#[test]
fn test_oversized_nsd_yml_is_refused_and_exactly_the_ceiling_is_accepted() {
    let padded = |total: u64| {
        let mut text = config_with(FILE_THRESHOLD);
        let padding = total as usize - text.len();
        text.push_str(&"#".repeat(padding - 1));
        text.push('\n');
        text
    };

    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    fs::write(dir.path().join(NSD_YML), padded(SOURCE_CEILING_BYTES + 1)).expect("write nsd.yml");
    let (output, output_dir) = scan(dir.path(), &[]);
    assert_refused(&output, &output_dir);

    fs::write(dir.path().join(NSD_YML), padded(SOURCE_CEILING_BYTES)).expect("write nsd.yml");
    let (output, output_dir) = scan(dir.path(), &[]);
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        measurement_of(&report(&output_dir)),
        measurement_fingerprint(FILE_THRESHOLD)
    );
}

#[test]
fn test_nsd_yml_exclude_does_not_narrow_the_scan() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    fs::write(
        dir.path().join(NSD_YML),
        "version: 1\nexclude:\n  - \"**/*.java\"\nmeasurement:\n  min_clone_lines: 5\n",
    )
    .expect("write nsd.yml");
    let (output, output_dir) = scan(dir.path(), &[]);
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        redundant_lines(&report(&output_dir)),
        vec![DUPLICATED_BLOCK_LINES]
    );
}
