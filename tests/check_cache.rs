//! M5-1 acceptance: `nsd check` over scratch repositories with the per-blob
//! analysis cache wired in. Every test compares what a caller observes (the
//! outcome, the process output, the entries on disk) across cache states.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use git2::{IndexEntry, IndexTime, ObjectType, Oid, Repository};
use serde_json::Value;
use tempfile::TempDir;

use nsd::cache::{Cache, CacheKey, CachedAnalysis};
use nsd::check::{run_check, CheckDiagnostic, CheckMode, CheckOutcome, CheckRequest};
use nsd::model::{Grammar, DEFAULT_MIN_CLONE_LINES};

const MODE_REGULAR: i32 = 0o100644;
const HIGH_CC_IFS: usize = 11;
const CLONE_BLOCK_LINES: usize = 12;
const SHORT_BLOCK_LINES: usize = 7;
const LOWERED_MIN_CLONE_LINES: u32 = 5;
const SCENARIO_CODES: [&str; 6] = [
    "NSD-E101", "NSD-E101", "NSD-E101", "NSD-V101", "NSD-A102", "NSD-V102",
];
const INVALID_UTF8_TS: &[u8] = b"export const x = '\xff\xfe';\n";
const G101: &str = "NSD-G101";
const V102: &str = "NSD-V102";
const A102: &str = "NSD-A102";
const FOREIGN_KEY: &str = "0123456789abcdef0123456789abcdef";
const SOURCE_LINES_FIELD: usize = 5;
const CACHE_PARTS: [&str; 3] = ["nsd", "cache", "v1"];
const ZLIB_STORED_HEADER: [u8; 2] = [0x78, 0x01];
const ZLIB_FINAL_STORED_BLOCK: u8 = 0x01;
const ADLER_MODULUS: u32 = 65_521;
const ADLER_SHIFT: u32 = 16;

struct Fixture {
    dir: TempDir,
    repo: Repository,
}

fn fixture() -> Fixture {
    let (dir, repo) = common::init_repo();
    Fixture { dir, repo }
}

fn stage_in(repo: &Repository, path: &str, bytes: &[u8]) {
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

impl Fixture {
    fn commit(&self, files: &[(&str, &[u8])]) {
        let entries: Vec<(Vec<u8>, i32, Vec<u8>)> = files
            .iter()
            .map(|(path, bytes)| (path.as_bytes().to_vec(), MODE_REGULAR, bytes.to_vec()))
            .collect();
        let oid = common::commit_entries(&self.repo, &entries);
        let tree = self
            .repo
            .find_commit(oid)
            .and_then(|commit| commit.tree())
            .expect("read the committed tree");
        let mut index = self.repo.index().expect("open the index");
        index
            .read_tree(&tree)
            .expect("mirror the tree into the index");
        index.write().expect("write the index");
    }

    fn stage(&self, path: &str, bytes: &[u8]) {
        stage_in(&self.repo, path, bytes);
    }

    /// E101 x3, V101, A102 and V102 staged over a two-file base.
    fn scenario() -> Fixture {
        let fx = fixture();
        let first = block_class("A", &clone_block("a", CLONE_BLOCK_LINES));
        let second = block_class("B", &clone_block("b", CLONE_BLOCK_LINES));
        fx.commit(&[("A.java", first.as_bytes())]);
        fx.commit(&[("A.java", first.as_bytes()), ("B.java", second.as_bytes())]);
        for name in ["C", "D", "E"] {
            fx.stage(
                &format!("{name}.java"),
                java_class(name, HIGH_CC_IFS).as_bytes(),
            );
        }
        fx.stage(
            "Copy.java",
            block_class("Copy", &clone_block("a", CLONE_BLOCK_LINES)).as_bytes(),
        );
        fx.stage("Bad.ts", INVALID_UTF8_TS);
        fx.stage("Widget.java", catching().as_bytes());
        fx
    }

    /// An unchanged `A.java` holding a clone block and a staged copy of it.
    fn block_and_copy(lines: usize) -> Fixture {
        let fx = fixture();
        fx.commit(&[(
            "A.java",
            block_class("A", &clone_block("a", lines)).as_bytes(),
        )]);
        fx.stage(
            "Copy.java",
            block_class("Copy", &clone_block("a", lines)).as_bytes(),
        );
        fx
    }

    fn staged(&self) -> CheckOutcome {
        self.check(None)
    }

    fn check(&self, config_path: Option<&Path>) -> CheckOutcome {
        check_at(self.dir.path(), config_path)
    }

    fn nsd(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_nsd"))
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .expect("run the nsd binary")
    }

    fn blob(&self, path: &str) -> Oid {
        self.repo
            .revparse_single(&format!("HEAD:{path}"))
            .expect("resolve the committed blob")
            .id()
    }

    fn cache_root(&self) -> PathBuf {
        CACHE_PARTS
            .iter()
            .fold(self.repo.commondir().to_path_buf(), |path, part| {
                path.join(part)
            })
    }

    fn cache(&self) -> Cache {
        Cache::open(&self.repo).expect("open the cache")
    }

    fn entry(&self, blob: Oid, grammar: Grammar, min_clone_lines: u32) -> Option<CachedAnalysis> {
        self.cache()
            .get(&CacheKey::new(blob, grammar, min_clone_lines))
            .expect("read the entry")
    }

    fn entry_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let Ok(fan_outs) = fs::read_dir(self.cache_root()) else {
            return files;
        };
        let fan_outs = fan_outs.filter(|fan_out| fan_out.as_ref().is_ok_and(|e| e.path().is_dir()));
        for fan_out in fan_outs {
            for entry in fs::read_dir(fan_out.expect("read a fan-out").path()).expect("list it") {
                files.push(entry.expect("read an entry").path());
            }
        }
        files.sort();
        files
    }

    fn truncate_every_entry(&self) {
        for file in self.entry_files() {
            let bytes = fs::read(&file).expect("read an entry");
            fs::write(&file, &bytes[..bytes.len() / 2]).expect("truncate an entry");
        }
    }

    /// Replaces `$GIT_COMMON_DIR/nsd` by a regular file.
    fn block_cache_dir(&self) {
        let nsd_dir = self.repo.commondir().join("nsd");
        if nsd_dir.exists() {
            fs::remove_dir_all(&nsd_dir).expect("remove the cache directory");
        }
        fs::write(&nsd_dir, b"not a directory").expect("write a regular file in its place");
    }
}

fn check_at(repository: &Path, config_path: Option<&Path>) -> CheckOutcome {
    run_check(&CheckRequest {
        repository,
        mode: CheckMode::Staged,
        config_path,
        allow_new_suppressions: false,
    })
}

fn java_class(name: &str, ifs: usize) -> String {
    let mut text =
        format!("public class {name} {{\n    public int run(int a) {{\n        int r = 0;\n");
    for index in 0..ifs {
        text.push_str(&format!(
            "        if (a == {name}K{index}) {{ r += {name}V{index}; }}\n"
        ));
    }
    text.push_str("        return r;\n    }\n}\n");
    text
}

fn clone_block(tag: &str, lines: usize) -> String {
    (0..lines)
        .map(|index| format!("        int {tag}{index} = {tag}Call{index}(x);\n"))
        .collect()
}

fn block_class(name: &str, body: &str) -> String {
    format!("class {name} {{\n    void run(int x) {{\n{body}    }}\n}}\n")
}

fn catching() -> String {
    "class Widget {\n    void run() {\n        try { work(); } catch (Exception e) { }\n    }\n}\n"
        .to_string()
}

fn codes(outcome: &CheckOutcome) -> Vec<&'static str> {
    outcome
        .diagnostics
        .iter()
        .map(CheckDiagnostic::code)
        .collect()
}

/// Replaces a loose object with a well-formed one holding other bytes under
/// the same header, so the ODB header read passes and the content read does
/// not (a stored-block zlib stream, hash mismatch on read).
fn corrupt_loose_object(repo: &Repository, id: Oid) {
    let size = repo.find_blob(id).expect("find the blob").content().len();
    let mut header_and_content = format!("blob {size}\0").into_bytes();
    header_and_content.extend(std::iter::repeat_n(b'#', size));
    let hex = id.to_string();
    let path = repo.path().join("objects").join(&hex[..2]).join(&hex[2..]);
    let length = header_and_content.len() as u16;
    let mut stream = ZLIB_STORED_HEADER.to_vec();
    stream.push(ZLIB_FINAL_STORED_BLOCK);
    stream.extend(length.to_le_bytes());
    stream.extend((!length).to_le_bytes());
    stream.extend(&header_and_content);
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &header_and_content {
        a = (a + u32::from(byte)) % ADLER_MODULUS;
        b = (b + a) % ADLER_MODULUS;
    }
    stream.extend(((b << ADLER_SHIFT) | a).to_be_bytes());
    fs::remove_file(&path).expect("remove the loose object");
    fs::write(&path, stream).expect("write the corrupted loose object");
}

fn write_min_clone_lines_config(lines: u32) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("create a temp dir for the trusted config");
    let path = dir.path().join("trusted.yml");
    fs::write(
        &path,
        format!("version: 1\nmeasurement:\n  min_clone_lines: {lines}\n"),
    )
    .expect("write the trusted config");
    (dir, path)
}

#[test]
fn test_warm_check_equals_cold_check() {
    let fx = Fixture::scenario();

    let cold = fx.staged();
    let unchanged_entries = [
        fx.entry(fx.blob("A.java"), Grammar::Java, DEFAULT_MIN_CLONE_LINES),
        fx.entry(fx.blob("B.java"), Grammar::Java, DEFAULT_MIN_CLONE_LINES),
    ];
    let warm = fx.staged();

    assert_eq!(codes(&cold), SCENARIO_CODES);
    assert!(
        unchanged_entries.iter().all(Option::is_some),
        "the cold run caches the unchanged blobs"
    );
    assert_eq!(warm.diagnostics, cold.diagnostics);
    assert_eq!(warm.exit_status, cold.exit_status);
}

#[test]
fn test_corrupt_cache_entries_recompute_the_same_outcome() {
    let fx = Fixture::scenario();
    let cold = fx.staged();
    assert!(fx.entry_files().len() >= 2, "the cold run wrote entries");

    fx.truncate_every_entry();
    let truncated = fx.staged();
    fx.block_cache_dir();
    let blocked = fx.staged();

    assert_eq!(truncated.diagnostics, cold.diagnostics);
    assert_eq!(truncated.exit_status, cold.exit_status);
    assert_eq!(blocked.diagnostics, cold.diagnostics);
    assert_eq!(blocked.exit_status, cold.exit_status);
}

#[test]
fn test_warm_hit_reads_no_unchanged_blob() {
    let fx = Fixture::scenario();
    let healthy = fx.staged();
    corrupt_loose_object(&fx.repo, fx.blob("B.java"));

    let warm = fx.staged();

    assert!(!codes(&warm).contains(&G101), "{:?}", warm.diagnostics);
    assert_eq!(warm.diagnostics, healthy.diagnostics);
    assert_eq!(warm.exit_status, healthy.exit_status);
}

#[test]
fn test_changed_min_clone_lines_misses_the_warm_cache() {
    let (_config_dir, lowered) = write_min_clone_lines_config(LOWERED_MIN_CLONE_LINES);
    let cold_lowered = Fixture::block_and_copy(SHORT_BLOCK_LINES).check(Some(&lowered));
    let fx = Fixture::block_and_copy(SHORT_BLOCK_LINES);
    let default_run = fx.staged();

    let warm_lowered = fx.check(Some(&lowered));

    assert_eq!(codes(&cold_lowered), [V102]);
    assert!(default_run.diagnostics.is_empty());
    assert_eq!(warm_lowered.diagnostics, cold_lowered.diagnostics);
    let blob = fx.blob("A.java");
    assert!(fx
        .entry(blob, Grammar::Java, DEFAULT_MIN_CLONE_LINES)
        .is_some());
    assert!(fx
        .entry(blob, Grammar::Java, LOWERED_MIN_CLONE_LINES)
        .is_some());
}

#[test]
fn test_same_blob_under_two_languages_keeps_its_verdict_warm() {
    let build = || {
        let fx = fixture();
        let same = block_class("Same", &clone_block("a", CLONE_BLOCK_LINES));
        fx.commit(&[("Same.java", same.as_bytes()), ("same.js", same.as_bytes())]);
        fx.stage(
            "Copy.java",
            block_class("Copy", &clone_block("a", CLONE_BLOCK_LINES)).as_bytes(),
        );
        fx
    };
    let cold = build().staged();
    let fx = build();
    fx.staged();

    let warm = fx.staged();

    let blob = fx.blob("Same.java");
    assert_eq!(blob, fx.blob("same.js"));
    assert!(fx
        .entry(blob, Grammar::Java, DEFAULT_MIN_CLONE_LINES)
        .is_some());
    assert!(fx
        .entry(blob, Grammar::JavaScript, DEFAULT_MIN_CLONE_LINES)
        .is_some());
    assert_eq!(warm.diagnostics, cold.diagnostics);
    assert_eq!(warm.exit_status, cold.exit_status);
}

/// Rewrites the clone key of the longest candidate in `file`'s payload.
fn tamper_longest_clone_key(file: &Path) {
    let bytes = fs::read(file).expect("read the entry");
    let split = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("the entry has a header line");
    let mut payload: Value = serde_json::from_slice(&bytes[split + 1..]).expect("parse payload");
    let candidates = payload["analyzed"]["clone_candidates"]
        .as_array_mut()
        .expect("an analyzed payload lists clone candidates");
    let longest = candidates
        .iter_mut()
        .max_by_key(|candidate| candidate[SOURCE_LINES_FIELD].as_u64())
        .expect("the block has candidates");
    longest[0] = Value::String(FOREIGN_KEY.to_string());
    let mut rewritten = bytes[..=split].to_vec();
    rewritten.extend(serde_json::to_vec(&payload).expect("serialize payload"));
    fs::write(file, rewritten).expect("write the tampered entry");
}

#[test]
fn test_tampered_cache_payload_recomputes_the_same_diagnostics() {
    let fx = Fixture::block_and_copy(CLONE_BLOCK_LINES);
    let healthy = fx.staged();
    let key = CacheKey::new(fx.blob("A.java"), Grammar::Java, DEFAULT_MIN_CLONE_LINES);
    let entry = fx.cache().entry_path(&key);
    assert!(entry.exists(), "the healthy run caches the unchanged blob");
    tamper_longest_clone_key(&entry);

    let after = fx.staged();

    assert_eq!(codes(&healthy), [V102]);
    assert_eq!(after.diagnostics, healthy.diagnostics);
    assert_eq!(after.exit_status, healthy.exit_status);
}

#[test]
fn test_linked_worktree_check_hits_the_shared_cache() {
    let fx = Fixture::block_and_copy(CLONE_BLOCK_LINES);
    let healthy = fx.staged();
    let holder = TempDir::new().expect("create a directory for the linked worktree");
    let linked_path = holder.path().join("linked");
    fx.repo
        .worktree("linked", &linked_path, None)
        .expect("add a linked worktree");
    let linked = Repository::open(&linked_path).expect("open the linked worktree");
    corrupt_loose_object(&fx.repo, fx.blob("A.java"));
    stage_in(
        &linked,
        "Copy.java",
        block_class("Copy", &clone_block("a", CLONE_BLOCK_LINES)).as_bytes(),
    );

    let outcome = check_at(&linked_path, None);

    assert_eq!(codes(&healthy), [V102]);
    assert!(
        !codes(&outcome).contains(&G101),
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(outcome.diagnostics, healthy.diagnostics);
}

#[test]
fn test_staged_entry_is_keyed_by_the_staged_blob() {
    let fx = fixture();
    let committed = java_class("Foo", 4);
    let staged = java_class("Foo", HIGH_CC_IFS);
    let dirty = java_class("Foo", HIGH_CC_IFS + 3);
    fx.commit(&[("Foo.java", committed.as_bytes())]);
    fx.stage("Foo.java", staged.as_bytes());
    fs::write(fx.dir.path().join("Foo.java"), dirty.as_bytes()).expect("dirty the worktree");

    let outcome = fx.staged();

    let staged_blob = Oid::hash_object(ObjectType::Blob, staged.as_bytes()).expect("hash");
    let dirty_blob = Oid::hash_object(ObjectType::Blob, dirty.as_bytes()).expect("hash");
    assert_eq!(codes(&outcome), ["NSD-E101"]);
    assert!(fx
        .entry(staged_blob, Grammar::Java, DEFAULT_MIN_CLONE_LINES)
        .is_some());
    assert!(fx
        .entry(dirty_blob, Grammar::Java, DEFAULT_MIN_CLONE_LINES)
        .is_none());
}

#[test]
fn test_unwritable_cache_warns_once_and_keeps_the_verdict() {
    let healthy = Fixture::scenario().staged();
    let fx = Fixture::scenario();
    fx.block_cache_dir();

    let outcome = fx.staged();

    assert_eq!(outcome.diagnostics, healthy.diagnostics);
    assert_eq!(outcome.exit_status, healthy.exit_status);
    assert!(healthy.warnings.is_empty(), "{:?}", healthy.warnings);
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Cold, warm, truncated and blocked runs of `nsd check --staged` with
/// `extra` arguments.
fn four_states(extra: &[&str]) -> Vec<Output> {
    let fx = Fixture::scenario();
    let mut args = vec!["check", "--staged"];
    args.extend(extra);
    let cold = fx.nsd(&args);
    assert!(!fx.entry_files().is_empty(), "the cold run wrote entries");
    let warm = fx.nsd(&args);
    fx.truncate_every_entry();
    let truncated = fx.nsd(&args);
    fx.block_cache_dir();
    let blocked = fx.nsd(&args);
    vec![cold, warm, truncated, blocked]
}

#[test]
fn test_binary_json_is_byte_stable_across_cache_states() {
    let runs = four_states(&["--format", "json"]);

    assert!(!runs[0].stdout.is_empty());
    for run in &runs {
        assert_eq!(run.stdout, runs[0].stdout);
        assert_eq!(run.status.code(), runs[0].status.code());
    }
}

#[test]
fn test_binary_terminal_output_is_byte_stable_across_cache_states() {
    let runs = four_states(&[]);

    assert!(!runs[0].stdout.is_empty());
    for run in &runs {
        assert_eq!(run.stdout, runs[0].stdout);
        assert_eq!(run.status.code(), runs[0].status.code());
    }
}

#[test]
fn test_binary_prints_the_cache_warning_on_stderr_without_a_path() {
    let fx = Fixture::scenario();
    fx.block_cache_dir();

    let output = fx.nsd(&["check", "--staged"]);

    let text = stderr(&output);
    let warnings: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("warning:"))
        .collect();
    assert_eq!(warnings.len(), 1, "{text}");
    assert!(
        !text.contains(fx.dir.path().to_str().expect("utf-8 tempdir")),
        "{text}"
    );
    let healthy = Fixture::scenario().nsd(&["check", "--staged"]);
    assert!(!stderr(&healthy).contains("warning:"));
}

#[test]
fn test_concurrent_checks_agree() {
    let fx = Fixture::scenario();
    let path = fx.dir.path().to_path_buf();

    let (first, second) = std::thread::scope(|scope| {
        let a = scope.spawn(|| check_at(&path, None));
        let b = scope.spawn(|| check_at(&path, None));
        (
            a.join().expect("first check"),
            b.join().expect("second check"),
        )
    });

    assert_eq!(first, second);
    let files = fx.entry_files();
    assert!(!files.is_empty(), "the checks wrote entries");
    for file in &files {
        assert_eq!(file.extension().and_then(|e| e.to_str()), Some("json"));
    }
    for name in ["A.java", "B.java"] {
        assert!(
            fx.entry(fx.blob(name), Grammar::Java, DEFAULT_MIN_CLONE_LINES)
                .is_some(),
            "{name} has a whole entry"
        );
    }
}

#[test]
fn test_unchanged_cached_unanalyzable_file_still_raises_a102() {
    let fx = fixture();
    fx.commit(&[("Bad.ts", INVALID_UTF8_TS)]);
    fx.stage("Other.java", java_class("Other", 1).as_bytes());
    let cold = fx.staged();

    let warm = fx.staged();

    let cached = fx.entry(
        fx.blob("Bad.ts"),
        Grammar::TypeScript,
        DEFAULT_MIN_CLONE_LINES,
    );
    assert!(matches!(cached, Some(CachedAnalysis::Unanalyzable(_))));
    assert_eq!(codes(&cold), [A102]);
    assert_eq!(warm.diagnostics, cold.diagnostics);
    assert_eq!(warm.exit_status, cold.exit_status);
}

fn modified_at(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .expect("read the entry's modification time")
}

#[test]
fn test_worktree_check_hits_an_unchanged_file_by_the_bytes_it_read() {
    let fx = fixture();
    let block = block_class("A", &clone_block("a", CLONE_BLOCK_LINES));
    fx.commit(&[("A.java", block.as_bytes())]);
    fs::write(fx.dir.path().join("A.java"), block.as_bytes()).expect("mirror the file");
    fs::write(
        fx.dir.path().join("Copy.java"),
        block_class("Copy", &clone_block("a", CLONE_BLOCK_LINES)).as_bytes(),
    )
    .expect("add an untracked copy");
    let mode = || CheckMode::Base {
        reference: "HEAD".to_string(),
        worktree: true,
    };
    let run = || {
        run_check(&CheckRequest {
            repository: fx.dir.path(),
            mode: mode(),
            config_path: None,
            allow_new_suppressions: false,
        })
    };
    let cold = run();
    let key = CacheKey::new(fx.blob("A.java"), Grammar::Java, DEFAULT_MIN_CLONE_LINES);
    let entry = fx.cache().entry_path(&key);
    let written = modified_at(&entry);
    std::thread::sleep(std::time::Duration::from_millis(50));

    let warm = run();

    assert_eq!(codes(&cold), [V102]);
    assert_eq!(warm.diagnostics, cold.diagnostics);
    assert_eq!(modified_at(&entry), written, "a hit does not rewrite");
}

#[test]
fn test_base_side_blob_never_enters_the_cache() {
    let fx = fixture();
    let committed = java_class("Foo", 4);
    let staged = java_class("Foo", HIGH_CC_IFS);
    fx.commit(&[("Foo.java", committed.as_bytes())]);
    fx.stage("Foo.java", staged.as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), ["NSD-E101"]);
    assert!(fx
        .entry(fx.blob("Foo.java"), Grammar::Java, DEFAULT_MIN_CLONE_LINES)
        .is_none());
}

#[cfg(unix)]
#[test]
fn test_failed_entry_write_warns_once_and_keeps_the_verdict() {
    use std::os::unix::fs::PermissionsExt;

    let healthy = Fixture::scenario().staged();
    let fx = Fixture::scenario();
    let root = fx.cache_root();
    fx.cache();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).expect("make the root read-only");

    let outcome = fx.staged();

    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).expect("restore the root");
    assert_eq!(outcome.diagnostics, healthy.diagnostics);
    assert_eq!(outcome.exit_status, healthy.exit_status);
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
}
