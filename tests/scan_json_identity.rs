//! `nsd scan`'s `report.json` is byte-identical across repeated runs,
//! checkout roots and rayon thread counts (M6-1 acceptance; D11: the thread
//! count is set on the child process, not by a flag).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

/// Copies every file under `from` into `to`, keeping the directory shape.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create the destination");
    for entry in fs::read_dir(from).expect("read the source directory") {
        let entry = entry.expect("read an entry");
        let destination = to.join(entry.file_name());
        let kind = entry.file_type().expect("read the entry type");
        if kind.is_dir() {
            copy_tree(&entry.path(), &destination);
        } else if kind.is_file() {
            fs::copy(entry.path(), &destination).expect("copy a file");
        }
    }
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=fixture@example.invalid"])
        .args(["-c", "user.name=nsd test fixture"])
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// Scans `target` with the binary and returns `report.json`'s bytes. The
/// document must be the canonical scan one with duplicates and ties to
/// order, or the comparison below would compare nothing.
fn scan(target: &Path, threads: Option<&str>) -> Vec<u8> {
    let output = TempDir::new().expect("create the output directory");
    let mut command = Command::new(env!("CARGO_BIN_EXE_nsd"));
    command
        .arg("scan")
        .arg(target)
        .arg("--output")
        .arg(output.path())
        .arg("--include-tests");
    if let Some(threads) = threads {
        command.env("RAYON_NUM_THREADS", threads);
    }
    let status = command.output().expect("run the nsd binary").status;
    assert!(status.success(), "scan of {} failed", target.display());
    let bytes = fs::read(output.path().join("report.json")).expect("read report.json");
    let document: Value = serde_json::from_slice(&bytes).expect("report.json is JSON");
    assert_eq!(document["result_scope"], "scan");
    assert!(document["callables"]
        .as_array()
        .is_some_and(|c| c.len() > 20));
    assert!(document["duplicates"]
        .as_array()
        .is_some_and(|d| !d.is_empty()));
    bytes
}

fn corpus_at(root: &Path) {
    copy_tree(&fixtures(), root);
}

#[test]
fn test_scan_report_is_byte_identical_across_repeated_runs() {
    let corpus = TempDir::new().expect("create the corpus");
    corpus_at(corpus.path());

    let first = scan(corpus.path(), None);
    let second = scan(corpus.path(), None);

    assert_eq!(first, second);
}

#[test]
fn test_scan_report_is_byte_identical_across_checkout_roots() {
    let first_root = TempDir::new().expect("create the first root");
    let second_parent = TempDir::new().expect("create the second root");
    let second_root = second_parent
        .path()
        .join("a-deeper-and-longer")
        .join("root");
    corpus_at(first_root.path());
    corpus_at(&second_root);
    assert_eq!(
        scan(first_root.path(), None),
        scan(&second_root, None),
        "plain directories"
    );

    // Two clones of one repository at different roots also carry a snapshot
    // ID, which must agree too.
    let origin = TempDir::new().expect("create the origin");
    corpus_at(origin.path());
    git(origin.path(), &["init", "-q"]);
    git(origin.path(), &["add", "-A"]);
    git(origin.path(), &["commit", "-q", "-m", "corpus"]);
    let clone_parent = TempDir::new().expect("create the clone parent");
    let near = clone_parent.path().join("near");
    let far = clone_parent
        .path()
        .join("a")
        .join("much")
        .join("deeper")
        .join("far");
    for clone in [&near, &far] {
        fs::create_dir_all(clone.parent().expect("a parent")).expect("create the parent");
        let origin_path = origin.path().to_str().expect("UTF-8 path");
        let status = Command::new("git")
            .args(["clone", "-q", origin_path])
            .arg(clone)
            .status()
            .expect("spawn git clone");
        assert!(status.success(), "clone failed");
    }
    let one = scan(&near, None);
    let two = scan(&far, None);
    assert_eq!(one, two, "git clones");
    let document: Value = serde_json::from_slice(&one).expect("JSON");
    assert!(
        document["snapshots"]["scan"].is_string(),
        "a clone root carries a snapshot ID: {document}"
    );
}

#[test]
fn test_scan_report_is_byte_identical_across_thread_counts() {
    let corpus = TempDir::new().expect("create the corpus");
    corpus_at(corpus.path());

    let one = scan(corpus.path(), Some("1"));
    let eight = scan(corpus.path(), Some("8"));

    assert_eq!(one, eight);
}
