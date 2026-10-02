//! M5-4 acceptance: one uncached `check` over scratch repositories. Staged
//! content lives only in the index, worktree content only on disk, so a
//! verdict that read the wrong side would change.

mod common;

use std::fs;
use std::path::Path;

use git2::{IndexEntry, IndexTime, Oid, Repository};
use tempfile::TempDir;

use nsd::check::{run_check, CheckDiagnostic, CheckMode, CheckOutcome, CheckRequest};
use nsd::git::snapshot::SOURCE_CEILING_BYTES;

const MODE_REGULAR: i32 = 0o100644;
const EXIT_PASS: u8 = 0;
const EXIT_REGRESSION: u8 = 1;
const EXIT_ERROR: u8 = 2;
const EXIT_BOTH: u8 = 3;
const E101: &str = "NSD-E101";
const E102: &str = "NSD-E102";
const V101: &str = "NSD-V101";
const V102: &str = "NSD-V102";
const S101: &str = "NSD-S101";
const A101: &str = "NSD-A101";
const A102: &str = "NSD-A102";
const C101: &str = "NSD-C101";
const C102: &str = "NSD-C102";
const G101: &str = "NSD-G101";
const LOW_CC_IFS: usize = 4;
const HIGH_CC_IFS: usize = 11;
const CLONE_BLOCK_LINES: usize = 12;
const SHIFT_LINES: usize = 20;
const GROWTH_FILLER_LINES: usize = 5;
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

impl Fixture {
    fn disk(&self, path: &str, bytes: &[u8]) {
        let target = self.dir.path().join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("create the parent directories");
        }
        fs::write(target, bytes).expect("write a worktree file");
    }

    /// Commits exactly `files` to `HEAD`, and mirrors them into the index and
    /// the worktree so nothing is pending afterwards.
    fn commit(&self, files: &[(&str, &[u8])]) -> Oid {
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
        for (path, bytes) in files {
            self.disk(path, bytes);
        }
        oid
    }

    /// Stages `bytes` at `path` without touching the worktree.
    fn stage(&self, path: &str, bytes: &[u8]) {
        let mut index = self.repo.index().expect("open the index");
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

    fn unstage(&self, path: &str) {
        let mut index = self.repo.index().expect("open the index");
        index
            .remove_path(Path::new(path))
            .expect("remove the path from the index");
        index.write().expect("write the index");
    }

    fn check(&self, mode: CheckMode) -> CheckOutcome {
        self.check_with(mode, None, false)
    }

    fn check_with(
        &self,
        mode: CheckMode,
        config_path: Option<&Path>,
        allow_new_suppressions: bool,
    ) -> CheckOutcome {
        run_check(&CheckRequest {
            repository: self.dir.path(),
            mode,
            config_path,
            allow_new_suppressions,
        })
    }

    fn staged(&self) -> CheckOutcome {
        self.check(CheckMode::Staged)
    }
}

fn base_mode(reference: &str, worktree: bool) -> CheckMode {
    CheckMode::Base {
        reference: reference.to_string(),
        worktree,
    }
}

fn codes(outcome: &CheckOutcome) -> Vec<&'static str> {
    outcome
        .diagnostics
        .iter()
        .map(CheckDiagnostic::code)
        .collect()
}

/// A method with `ifs` decision points (CC `ifs + 1`) and `filler` extra
/// statements; every identifier carries the class name so no two classes
/// look like clones of each other.
fn java_class(name: &str, ifs: usize, filler: usize) -> String {
    let mut text =
        format!("public class {name} {{\n    public int run(int a) {{\n        int r = 0;\n");
    for index in 0..ifs {
        text.push_str(&format!(
            "        if (a == {name}K{index}) {{ r += {name}V{index}; }}\n"
        ));
    }
    for index in 0..filler {
        text.push_str(&format!("        r += {name}F{index};\n"));
    }
    text.push_str("        return r;\n    }\n}\n");
    text
}

fn complexity_lines(outcome: &CheckOutcome) -> Vec<(String, usize, usize)> {
    outcome
        .diagnostics
        .iter()
        .filter_map(|found| match found {
            CheckDiagnostic::Complexity(found) => Some((
                found.candidate_path.render(),
                found.candidate_start_line,
                found.candidate_end_line,
            )),
            _ => None,
        })
        .collect()
}

fn coverage_reasons(outcome: &CheckOutcome) -> Vec<(String, &'static str)> {
    outcome
        .diagnostics
        .iter()
        .filter_map(|found| match found {
            CheckDiagnostic::Coverage(found) => Some((found.path.render(), found.reason.label())),
            _ => None,
        })
        .collect()
}

/// Valid Java of exactly `length` bytes: a class plus one padding comment.
fn java_of_length(length: usize) -> Vec<u8> {
    let mut bytes = b"class Big {\n}\n// ".to_vec();
    let pad = length - bytes.len() - 1;
    bytes.extend(std::iter::repeat_n(b'x', pad));
    bytes.push(b'\n');
    assert_eq!(bytes.len(), length);
    bytes
}

fn ceiling() -> usize {
    SOURCE_CEILING_BYTES as usize
}

const INVALID_UTF8_TS: &[u8] = b"export const x = '\xff\xfe';\n";

fn write_trusted_config(text: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("create a temp dir for the trusted config");
    let path = dir.path().join("trusted.yml");
    fs::write(&path, text).expect("write the trusted config");
    (dir, path)
}

// ---------------------------------------------------------------------
// Staged mode.
// ---------------------------------------------------------------------

#[test]
fn test_staged_complexity_regression_raises_e101_and_exits_1() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![E101]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
    let end_line = HIGH_CC_IFS + 5;
    assert_eq!(
        complexity_lines(&outcome),
        vec![("Foo.java".to_string(), 2, end_line)]
    );
}

#[test]
fn test_staged_clean_change_exits_0() {
    let fx = fixture();
    let base = java_class("Foo", LOW_CC_IFS, 0);
    fx.commit(&[("Foo.java", base.as_bytes())]);
    fx.stage("Foo.java", format!("// reviewed\n{base}").as_bytes());

    let outcome = fx.staged();

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_staged_verdict_ignores_unstaged_edits() {
    let fx = fixture();
    let low = java_class("Foo", LOW_CC_IFS, 0);
    let high = java_class("Foo", HIGH_CC_IFS, 0);
    fx.commit(&[("Foo.java", low.as_bytes())]);

    fx.stage("Foo.java", high.as_bytes());
    let hidden_baseline = fx.staged();
    fx.disk("Foo.java", low.as_bytes());
    let hidden = fx.staged();
    assert_eq!(codes(&hidden), vec![E101]);
    assert_eq!(hidden, hidden_baseline);
    assert_eq!(hidden.exit_status, EXIT_REGRESSION);

    fx.stage("Foo.java", format!("// reviewed\n{low}").as_bytes());
    fx.disk("Foo.java", high.as_bytes());
    let added = fx.staged();
    assert!(added.diagnostics.is_empty(), "{:?}", added.diagnostics);
    assert_eq!(added.exit_status, EXIT_PASS);
}

#[test]
fn test_unborn_repository_staged_check_uses_the_empty_tree() {
    let fx = fixture();
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![E101]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

// ---------------------------------------------------------------------
// Base and worktree modes.
// ---------------------------------------------------------------------

#[test]
fn test_base_mode_compares_head_with_the_merge_base() {
    let fx = fixture();
    let high = |name: &str| java_class(name, HIGH_CC_IFS, 0);
    let first = fx.commit(&[("Old.java", high("Old").as_bytes())]);
    let commit = fx.repo.find_commit(first).expect("find the first commit");
    fx.repo
        .branch("topic", &commit, false)
        .expect("branch at the merge base");
    fx.commit(&[
        ("Old.java", high("Old").as_bytes()),
        ("New.java", high("New").as_bytes()),
    ]);

    let outcome = fx.check(base_mode("topic", false));

    assert_eq!(codes(&outcome), vec![E101]);
    assert_eq!(
        complexity_lines(&outcome)[0].0,
        "New.java",
        "only the regression past the merge base is judged"
    );
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_worktree_mode_sees_unstaged_and_untracked_files() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.disk("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());
    fx.disk("Fresh.java", java_class("Fresh", HIGH_CC_IFS, 0).as_bytes());

    let committed = fx.check(base_mode("HEAD", false));
    let overlay = fx.check(base_mode("HEAD", true));

    assert!(
        committed.diagnostics.is_empty(),
        "{:?}",
        committed.diagnostics
    );
    let paths: Vec<String> = complexity_lines(&overlay)
        .into_iter()
        .map(|(path, _, _)| path)
        .collect();
    assert_eq!(paths, vec!["Foo.java", "Fresh.java"]);
    assert_eq!(overlay.exit_status, EXIT_REGRESSION);
}

// ---------------------------------------------------------------------
// Internal failures are diagnostics.
// ---------------------------------------------------------------------

#[test]
fn test_missing_base_ref_raises_g101_and_exits_2() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes())]);

    let outcome = fx.check(base_mode("no-such-ref", false));

    assert_eq!(codes(&outcome), vec![G101]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_non_repository_directory_raises_g101_and_exits_2() {
    let dir = TempDir::new().expect("create a plain directory");

    let outcome = run_check(&CheckRequest {
        repository: dir.path(),
        mode: CheckMode::Staged,
        config_path: None,
        allow_new_suppressions: false,
    });

    assert_eq!(codes(&outcome), vec![G101]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

/// Replaces a loose object with a well-formed one holding other bytes under
/// the same header, so the ODB header read passes and the content read does
/// not (a stored-block zlib stream, hash mismatch on read).
fn corrupt_loose_object(repo: &Repository, id: Oid, header_and_content: &[u8]) {
    let hex = id.to_string();
    let path = repo.path().join("objects").join(&hex[..2]).join(&hex[2..]);
    let length = header_and_content.len() as u16;
    let mut stream = ZLIB_STORED_HEADER.to_vec();
    stream.push(ZLIB_FINAL_STORED_BLOCK);
    stream.extend(length.to_le_bytes());
    stream.extend((!length).to_le_bytes());
    stream.extend(header_and_content);
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in header_and_content {
        a = (a + u32::from(byte)) % ADLER_MODULUS;
        b = (b + a) % ADLER_MODULUS;
    }
    stream.extend(((b << ADLER_SHIFT) | a).to_be_bytes());
    fs::remove_file(&path).expect("remove the loose object");
    fs::write(&path, stream).expect("write the corrupted loose object");
}

#[test]
fn test_unreadable_base_config_blob_raises_g101_not_c102() {
    let fx = fixture();
    let oid = fx.commit(&[
        ("nsd.yml", b"version: 1\n"),
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
    ]);
    let tree = fx
        .repo
        .find_commit(oid)
        .and_then(|c| c.tree())
        .expect("tree");
    let blob = tree.get_name("nsd.yml").expect("nsd.yml entry").id();
    corrupt_loose_object(&fx.repo, blob, b"blob 11\0version: 2\n");
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![G101]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

// ---------------------------------------------------------------------
// Evaluators.
// ---------------------------------------------------------------------

#[test]
fn test_e102_growth_past_threshold_exits_1() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes())]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 1).as_bytes());
    let within = fx.staged();
    fx.stage(
        "Foo.java",
        java_class("Foo", HIGH_CC_IFS, GROWTH_FILLER_LINES).as_bytes(),
    );

    let grown = fx.staged();

    assert!(within.diagnostics.is_empty(), "{:?}", within.diagnostics);
    assert_eq!(codes(&grown), vec![E102]);
    assert_eq!(grown.exit_status, EXIT_REGRESSION);
}

fn catching(extra: &str) -> String {
    format!("class Widget {{\n{extra}    void run() {{\n        try {{ work(); }} catch (Exception e) {{ }}\n    }}\n}}\n")
}

#[test]
fn test_new_rule_finding_raises_v101() {
    let fx = fixture();
    let quiet = "class Widget {\n    void run() {\n        work();\n    }\n}\n";
    fx.commit(&[("Widget.java", quiet.as_bytes())]);
    fx.stage("Widget.java", catching("").as_bytes());
    let added = fx.staged();

    fx.commit(&[("Widget.java", catching("").as_bytes())]);
    let shifted_text = format!("{}{}", "// pad\n".repeat(SHIFT_LINES), catching(""));
    fx.stage("Widget.java", shifted_text.as_bytes());
    let shifted = fx.staged();

    assert_eq!(codes(&added), vec![V101]);
    assert_eq!(added.exit_status, EXIT_REGRESSION);
    assert!(shifted.diagnostics.is_empty(), "{:?}", shifted.diagnostics);
}

fn clone_block(tag: &str) -> String {
    (0..CLONE_BLOCK_LINES)
        .map(|index| format!("        int {tag}{index} = {tag}Call{index}(x);\n"))
        .collect()
}

fn block_class(name: &str, body: &str) -> String {
    format!("class {name} {{\n    void run(int x) {{\n{body}    }}\n}}\n")
}

#[test]
fn test_cross_file_copy_raises_v102_and_move_passes() {
    let fx = fixture();
    fx.commit(&[("A.java", block_class("A", &clone_block("a")).as_bytes())]);
    fx.stage("New.java", block_class("New", &clone_block("a")).as_bytes());
    let copied = fx.staged();

    fx.stage("A.java", block_class("A", "").as_bytes());
    let moved = fx.staged();

    assert_eq!(codes(&copied), vec![V102]);
    assert_eq!(copied.exit_status, EXIT_REGRESSION);
    assert!(moved.diagnostics.is_empty(), "{:?}", moved.diagnostics);
}

#[test]
fn test_new_suppression_raises_s101_and_the_flag_makes_it_exit_neutral() {
    let fx = fixture();
    let quiet = "class Widget {\n    void run() {\n        work();\n    }\n}\n";
    fx.commit(&[("Widget.java", quiet.as_bytes())]);
    let suppressed = "class Widget {\n    void run() {\n        // nsd-ignore[JAVA-EMPTY-CATCH]: legacy\n        try { work(); } catch (Exception e) { }\n    }\n}\n";
    fx.stage("Widget.java", suppressed.as_bytes());

    let strict = fx.check_with(CheckMode::Staged, None, false);
    let allowed = fx.check_with(CheckMode::Staged, None, true);

    assert_eq!(codes(&strict), vec![S101]);
    assert_eq!(strict.exit_status, EXIT_REGRESSION);
    assert_eq!(codes(&allowed), vec![S101]);
    assert_eq!(allowed.exit_status, EXIT_PASS);
}

#[test]
fn test_new_parse_damage_raises_a101_and_exits_2() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    let broken = java_class("Foo", LOW_CC_IFS, 0).replace("a == FooK0", "a == ");
    fx.stage("Foo.java", broken.as_bytes());

    let outcome = fx.staged();

    assert!(codes(&outcome).contains(&A101), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

// ---------------------------------------------------------------------
// Coverage (A102).
// ---------------------------------------------------------------------

#[test]
fn test_changed_over_ceiling_file_raises_a102_too_large() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("Big.java", &java_of_length(ceiling() + 1));
    let over = fx.staged();
    fx.stage("Big.java", &java_of_length(ceiling()));

    let exact = fx.staged();

    assert_eq!(
        coverage_reasons(&over),
        vec![("Big.java".to_string(), "too_large")]
    );
    assert_eq!(codes(&over), vec![A102]);
    assert_eq!(over.exit_status, EXIT_ERROR);
    assert!(exact.diagnostics.is_empty(), "{:?}", exact.diagnostics);
    assert_eq!(exact.exit_status, EXIT_PASS);
}

#[test]
fn test_changed_invalid_encoding_file_raises_a102() {
    let fx = fixture();
    fx.commit(&[("util.ts", b"export const x = 1;\n")]);
    fx.stage("util.ts", INVALID_UTF8_TS);

    let outcome = fx.staged();

    assert_eq!(
        coverage_reasons(&outcome),
        vec![("util.ts".to_string(), "invalid_encoding")]
    );
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_unchanged_unanalyzable_file_raises_a102_only_while_v102_is_enabled() {
    let fx = fixture();
    fx.commit(&[
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
        ("Big.java", &java_of_length(ceiling() + 1)),
    ]);
    fx.stage(
        "Foo.java",
        format!("// reviewed\n{}", java_class("Foo", LOW_CC_IFS, 0)).as_bytes(),
    );
    let (_dir, off) = write_trusted_config("version: 1\npolicy:\n  NSD-V102: off\n");

    let enabled = fx.staged();
    let disabled = fx.check_with(CheckMode::Staged, Some(&off), false);

    assert_eq!(
        coverage_reasons(&enabled),
        vec![("Big.java".to_string(), "too_large")]
    );
    assert_eq!(enabled.exit_status, EXIT_ERROR);
    assert!(
        disabled.diagnostics.is_empty(),
        "{:?}",
        disabled.diagnostics
    );
    assert_eq!(disabled.exit_status, EXIT_PASS);
}

#[test]
fn test_regression_and_analysis_error_together_exit_3() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());
    fx.stage("bad.ts", INVALID_UTF8_TS);

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![E101, A102]);
    assert_eq!(outcome.exit_status, EXIT_BOTH);
}

// ---------------------------------------------------------------------
// Configuration trust.
// ---------------------------------------------------------------------

const E101_OFF: &str = "version: 1\npolicy:\n  NSD-E101: off\n";

#[test]
fn test_candidate_config_cannot_weaken_its_own_check() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());
    fx.stage("nsd.yml", E101_OFF.as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![C101, E101]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_c101_alone_exits_0() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("nsd.yml", E101_OFF.as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![C101]);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_invalid_base_config_raises_c102_and_exits_2() {
    let fx = fixture();
    fx.commit(&[
        ("nsd.yml", b"version: [\n"),
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
    ]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![C102]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_trusted_config_downgrade_to_warn_keeps_the_diagnostic_and_exits_0() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());
    let (_dir, warn) = write_trusted_config("version: 1\npolicy:\n  NSD-E101: warn\n");

    let outcome = fx.check_with(CheckMode::Staged, Some(&warn), false);

    assert_eq!(codes(&outcome), vec![E101]);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_base_config_exclude_removes_a_path_from_the_check() {
    let fx = fixture();
    fx.commit(&[
        ("nsd.yml", b"version: 1\nexclude:\n  - legacy/**\n"),
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
    ]);
    fx.stage(
        "legacy/Old.java",
        java_class("Old", HIGH_CC_IFS, 0).as_bytes(),
    );
    fx.stage("src/New.java", java_class("New", HIGH_CC_IFS, 0).as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![E101]);
    assert_eq!(complexity_lines(&outcome)[0].0, "src/New.java");
}

#[test]
fn test_check_has_no_default_test_exclusion() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage(
        "src/test/FooTest.java",
        java_class("FooTest", HIGH_CC_IFS, 0).as_bytes(),
    );

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![E101]);
    assert_eq!(complexity_lines(&outcome)[0].0, "src/test/FooTest.java");
}

// ---------------------------------------------------------------------
// Determinism.
// ---------------------------------------------------------------------

#[test]
fn test_check_result_is_deterministic() {
    let unrelated_a = ("A.java", block_class("A", &clone_block("a")));
    let unrelated_b = ("B.java", block_class("B", &clone_block("b")));
    let build = |first: &(&str, String), second: &(&str, String)| {
        let fx = fixture();
        fx.commit(&[(first.0, first.1.as_bytes())]);
        fx.commit(&[
            (first.0, first.1.as_bytes()),
            (second.0, second.1.as_bytes()),
        ]);
        for name in ["C", "D", "E"] {
            fx.stage(
                &format!("{name}.java"),
                java_class(name, HIGH_CC_IFS, 0).as_bytes(),
            );
        }
        fx.stage(
            "Copy.java",
            block_class("Copy", &clone_block("a")).as_bytes(),
        );
        fx.stage("Bad.ts", INVALID_UTF8_TS);
        fx.stage("Widget.java", catching("").as_bytes());
        fx
    };
    let forward = build(&unrelated_a, &unrelated_b);
    let reversed = build(&unrelated_b, &unrelated_a);

    let first = forward.staged();
    let second = forward.staged();
    let other = reversed.staged();

    assert!(first.diagnostics.len() >= 6, "{:?}", first.diagnostics);
    assert_eq!(first, second);
    assert_eq!(first, other);
}

#[test]
fn test_clone_moved_by_deleting_its_source_file_passes() {
    let fx = fixture();
    fx.commit(&[
        ("A.java", block_class("A", &clone_block("a")).as_bytes()),
        ("B.java", block_class("B", &clone_block("a")).as_bytes()),
    ]);
    fx.unstage("A.java");
    // Enough unique code around the block keeps rename detection from pairing
    // the two files, so the source really is a deletion.
    let body = format!(
        "{}{}{}{}",
        clone_block("a"),
        clone_block("b"),
        clone_block("c"),
        clone_block("d")
    );
    fx.stage("New.java", block_class("New", &body).as_bytes());

    let outcome = fx.staged();

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_base_mode_judges_a_file_changed_after_the_merge_base() {
    let fx = fixture();
    let first = fx.commit(&[("Mod.java", java_class("Mod", LOW_CC_IFS, 0).as_bytes())]);
    let commit = fx.repo.find_commit(first).expect("find the first commit");
    fx.repo
        .branch("topic", &commit, false)
        .expect("branch at the merge base");
    fx.commit(&[("Mod.java", java_class("Mod", HIGH_CC_IFS, 0).as_bytes())]);

    let outcome = fx.check(base_mode("topic", false));

    assert_eq!(codes(&outcome), vec![E101]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}
