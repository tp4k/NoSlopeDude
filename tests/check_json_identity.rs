//! `nsd check --format json` is byte-identical across checkout roots and
//! across rayon thread counts (M6-2 acceptance; D11: the thread count is set
//! on the child process, not by a flag).

mod common;

use std::path::Path;
use std::process::{Command, Output};

use git2::{IndexEntry, IndexTime, Oid, Repository};
use serde_json::Value;
use tempfile::TempDir;

const MODE_REGULAR: i32 = 0o100644;
const FILE_COUNT: usize = 12;

fn class_text(name: &str, extra: &str) -> String {
    format!(
        "package p;\n\nclass {name} {{\n    int f(int x) {{\n        int y = x + {extra}1;\n        return y;\n    }}\n\n    int g(int x) {{\n        if (x > 1) {{\n            return x;\n        }}\n        return 0;\n    }}\n}}\n"
    )
}

/// An origin with two commits: `File<i>.java` first, then edited copies and
/// one new file.
fn origin() -> (TempDir, Repository) {
    let (dir, repo) = common::init_repo();
    let first: Vec<(Vec<u8>, i32, Vec<u8>)> = (0..FILE_COUNT)
        .map(|index| {
            (
                format!("File{index}.java").into_bytes(),
                MODE_REGULAR,
                class_text(&format!("File{index}"), "").into_bytes(),
            )
        })
        .collect();
    common::commit_entries(&repo, &first);
    let mut second = first.clone();
    for (index, entry) in second.iter_mut().enumerate() {
        if index % 2 == 0 {
            entry.2 = class_text(&format!("File{index}"), "2 * ").into_bytes();
        }
    }
    second.push((
        b"Added.java".to_vec(),
        MODE_REGULAR,
        class_text("Added", "3 * ").into_bytes(),
    ));
    common::commit_entries(&repo, &second);
    (dir, repo)
}

fn stage(repo: &Repository, path: &str, bytes: &[u8]) {
    let mut index = repo.index().expect("open the index");
    let entry = IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: MODE_REGULAR as u32,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: Oid::ZERO_SHA1,
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    };
    index.add_frombuffer(&entry, bytes).expect("stage a buffer");
    index.write().expect("write the index");
}

fn run(root: &Path, args: &[&str], threads: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nsd"));
    command.args(args).current_dir(root);
    if let Some(threads) = threads {
        command.env("RAYON_NUM_THREADS", threads);
    }
    command.output().expect("run the nsd binary")
}

fn checked(output: &Output) -> &[u8] {
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("stdout is one document");
    assert!(
        parsed["entities"].as_array().is_some_and(|e| e.len() > 2),
        "the document must carry entities: {parsed}"
    );
    assert!(parsed["summaries"]["files"].as_u64().is_some_and(|n| n > 2));
    &output.stdout
}

#[test]
fn test_check_json_is_byte_identical_across_checkout_roots() {
    let (origin_dir, _origin) = origin();
    let first_root = TempDir::new().expect("create the first root");
    let second_parent = TempDir::new().expect("create the second root");
    let second_root = second_parent
        .path()
        .join("a-deeper-and-longer")
        .join("checkout-root");
    let origin_path = origin_dir.path().to_str().expect("UTF-8 path");
    let first = Repository::clone(origin_path, first_root.path()).expect("clone the first root");
    let second = Repository::clone(origin_path, &second_root).expect("clone the second root");
    for clone in [&first, &second] {
        stage(clone, "File1.java", class_text("File1", "4 * ").as_bytes());
    }
    // The staged leg needs more than two changed entities for `checked`;
    // File1 alone yields one. These two edits add to the fixture only.
    for clone in [&first, &second] {
        for name in ["File3", "File5"] {
            stage(
                clone,
                &format!("{name}.java"),
                class_text(name, "4 * ").as_bytes(),
            );
        }
    }

    let base = ["check", "--base", "HEAD~1", "--format", "json"];
    let staged = ["check", "--staged", "--format", "json"];
    let runs: [&[&str]; 2] = [&base, &staged];
    for args in runs {
        let one = run(first_root.path(), args, None);
        let two = run(&second_root, args, None);
        assert_eq!(checked(&one), checked(&two), "{args:?}");
        assert_eq!(one.status.code(), two.status.code());
    }
}

#[test]
fn test_check_json_is_byte_identical_across_thread_counts() {
    let (dir, repo) = origin();
    for index in 0..FILE_COUNT {
        stage(
            &repo,
            &format!("File{index}.java"),
            class_text(&format!("File{index}"), "5 * ").as_bytes(),
        );
    }
    let args = ["check", "--staged", "--format", "json"];

    let single = run(dir.path(), &args, Some("1"));
    let many = run(dir.path(), &args, Some("8"));

    assert_eq!(checked(&single), checked(&many));
    assert_eq!(single.status.code(), many.status.code());
}
