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
// Base-ignore filter for `--worktree` (an untracked entry is left out only
// when the base snapshot's `.gitignore` rules AND the candidate's own
// `.gitignore` files both ignore it).
// ---------------------------------------------------------------------

fn violating(name: &str) -> String {
    java_class(name, HIGH_CC_IFS, 0)
}

fn complexity_paths(outcome: &CheckOutcome) -> Vec<String> {
    let mut paths: Vec<String> = complexity_lines(outcome)
        .into_iter()
        .map(|(path, _, _)| path)
        .collect();
    paths.sort();
    paths
}

/// Restores a directory's mode on drop so a failing assertion cannot leave
/// an undeletable directory behind in the temp tree.
#[cfg(unix)]
struct Unlock(std::path::PathBuf);

#[cfg(unix)]
impl Drop for Unlock {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

#[cfg(unix)]
#[test]
fn test_worktree_skips_untracked_files_the_base_gitignore_ignores() {
    use std::os::unix::fs::PermissionsExt;

    let fx = fixture();
    fx.commit(&[
        (".gitignore", b"playwright-report/\n"),
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
    ]);
    fx.disk(
        "playwright-report/trace/assets/codeMirrorModule-x.java",
        violating("Minified").as_bytes(),
    );
    let locked = fx.dir.path().join("playwright-report/locked");
    fs::create_dir_all(&locked).expect("create the locked directory");
    let _unlock = Unlock(locked.clone());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock the directory");

    let outcome = fx.check(base_mode("HEAD", true));

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_candidate_gitignore_addition_hides_nothing() {
    let fx = fixture();
    fx.commit(&[(".gitignore", b"build/\n")]);
    fx.disk(".gitignore", b"build/\nhidden/\n");
    fx.disk("hidden/X.java", violating("X").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["hidden/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_tracked_file_under_a_base_ignored_directory_is_still_checked() {
    let fx = fixture();
    fx.commit(&[
        (".gitignore", b"codegen/\n"),
        ("codegen/T.java", java_class("T", LOW_CC_IFS, 0).as_bytes()),
    ]);
    fx.disk("codegen/T.java", violating("T").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["codegen/T.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_base_gitignore_negation_reincludes_an_untracked_file() {
    let fx = fixture();
    fx.commit(&[(".gitignore", b"emitted/*\n!emitted/Keep.java\n")]);
    fx.disk("emitted/Keep.java", violating("Keep").as_bytes());
    fx.disk("emitted/Other.java", violating("Other").as_bytes());
    assert!(!fx.repo.is_path_ignored("emitted/Keep.java").expect("git"));
    assert!(fx.repo.is_path_ignored("emitted/Other.java").expect("git"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["emitted/Keep.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);

    // Git: a parent-directory exclusion cannot be re-included.
    let excluded = fixture();
    excluded.commit(&[(".gitignore", b"emitted/\n!emitted/Keep.java\n")]);
    excluded.disk("emitted/Keep.java", violating("Keep").as_bytes());
    assert!(excluded
        .repo
        .is_path_ignored("emitted/Keep.java")
        .expect("git"));

    let skipped = excluded.check(base_mode("HEAD", true));

    assert!(skipped.diagnostics.is_empty(), "{:?}", skipped.diagnostics);
    assert_eq!(skipped.exit_status, EXIT_PASS);
}

#[test]
fn test_nested_base_gitignore_applies_only_below_its_directory() {
    let fx = fixture();
    fx.commit(&[("sub/.gitignore", b"gen.java\n")]);
    fx.disk("sub/gen.java", violating("SubGen").as_bytes());
    fx.disk("sub/deep/gen.java", violating("DeepGen").as_bytes());
    fx.disk("gen.java", violating("RootGen").as_bytes());
    fx.disk("other/gen.java", violating("OtherGen").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(
        complexity_paths(&outcome),
        vec!["gen.java", "other/gen.java"]
    );
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_candidate_deleting_a_base_ignore_line_exposes_the_files() {
    let fx = fixture();
    fx.commit(&[(".gitignore", b"tmp/\n")]);
    fx.disk(".gitignore", b"");
    fx.disk("tmp/X.java", violating("X").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["tmp/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_candidate_ignored_ancestor_still_skips_a_base_ignored_directory() {
    let fx = fixture();
    fx.commit(&[(".gitignore", b"a/b/\n")]);
    fx.disk(".gitignore", b"a/\n");
    fx.disk("a/b/X.java", violating("X").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert!(fx.repo.is_path_ignored("a/b/X.java").expect("git"));
    assert!(complexity_paths(&outcome).is_empty());
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_deeper_base_gitignore_negation_overrides_a_parent_rule() {
    let fx = fixture();
    fx.commit(&[
        (".gitignore", b"*.gen.java\n"),
        ("sub/.gitignore", b"!keep.gen.java\n"),
    ]);
    fx.disk("sub/keep.gen.java", violating("KeepGen").as_bytes());
    fx.disk("sub/x.gen.java", violating("XGen").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["sub/keep.gen.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_directory_only_base_pattern_does_not_skip_a_file_of_that_name() {
    let fx = fixture();
    fx.commit(&[(".gitignore", b"Gen.java/\n")]);
    fx.disk("Gen.java", violating("Gen").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["Gen.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_info_exclude_and_core_excludes_file_do_not_skip_in_check() {
    let fx = fixture();
    fx.commit(&[(".gitignore", b"build/\n")]);
    let info = fx.dir.path().join(".git/info");
    fs::create_dir_all(&info).expect("create .git/info");
    fs::write(info.join("exclude"), b"Excluded.java\n").expect("write info/exclude");
    let global = fx.dir.path().join(".git/global-excludes");
    fs::write(&global, b"Global.java\n").expect("write the excludes file");
    fx.repo
        .config()
        .and_then(|mut config| config.set_str("core.excludesFile", &global.display().to_string()))
        .expect("set core.excludesFile");
    fx.disk("Excluded.java", violating("Excluded").as_bytes());
    fx.disk("Global.java", violating("Global").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(
        complexity_paths(&outcome),
        vec!["Excluded.java", "Global.java"]
    );
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_an_anchored_nested_base_pattern_matches_only_in_its_own_directory() {
    let fx = fixture();
    fx.commit(&[("sub/.gitignore", b"/anchored.java\n")]);
    fx.disk("sub/anchored.java", violating("Anchored").as_bytes());
    fx.disk(
        "sub/deep/anchored.java",
        violating("DeepAnchored").as_bytes(),
    );

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["sub/deep/anchored.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_base_ignored_parent_directory_cannot_be_reincluded_by_a_child_negation() {
    let fx = fixture();
    fx.commit(&[(".gitignore", b"emitted/\n!emitted/Keep.java\n")]);
    fx.disk(".gitignore", b"Keep.java\n");
    fx.disk("emitted/Keep.java", violating("Keep").as_bytes());

    let outcome = fx.check(base_mode("HEAD", true));

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[cfg(unix)]
#[test]
fn test_a_symlinked_gitignore_contributes_no_rules_on_either_side() {
    let base_link = fixture();
    let blob: Vec<(Vec<u8>, i32, Vec<u8>)> =
        vec![(b".gitignore".to_vec(), 0o120000, b"Linked.java".to_vec())];
    common::commit_entries(&base_link.repo, &blob);
    base_link.disk(".gitignore", b"Linked.java\n");
    base_link.disk("Linked.java", violating("Linked").as_bytes());

    let outcome = base_link.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["Linked.java"]);

    let candidate_link = fixture();
    candidate_link.commit(&[(".gitignore", b"Linked.java\n")]);
    fs::remove_file(candidate_link.dir.path().join(".gitignore")).expect("remove .gitignore");
    fs::write(
        candidate_link.dir.path().join("rules.txt"),
        b"Linked.java\n",
    )
    .expect("rules");
    std::os::unix::fs::symlink("rules.txt", candidate_link.dir.path().join(".gitignore"))
        .expect("symlink .gitignore");
    candidate_link.disk("Linked.java", violating("Linked").as_bytes());

    let outcome = candidate_link.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["Linked.java"]);
}

#[test]
fn test_an_over_ceiling_gitignore_contributes_no_rules_on_either_side() {
    let mut oversized = b"big/\n".to_vec();
    oversized.resize(ceiling() + 1, b'#');

    let base_big = fixture();
    base_big.commit(&[(".gitignore", oversized.as_slice())]);
    base_big.disk(".gitignore", b"big/\n");
    base_big.disk("big/X.java", violating("X").as_bytes());
    let outcome = base_big.check(base_mode("HEAD", true));
    assert_eq!(complexity_paths(&outcome), vec!["big/X.java"]);

    let candidate_big = fixture();
    candidate_big.commit(&[(".gitignore", b"big/\n")]);
    candidate_big.disk(".gitignore", oversized.as_slice());
    candidate_big.disk("big/X.java", violating("X").as_bytes());
    let outcome = candidate_big.check(base_mode("HEAD", true));
    assert_eq!(complexity_paths(&outcome), vec!["big/X.java"]);
}

#[test]
fn test_a_bom_prefixed_base_gitignore_still_ignores_its_first_pattern() {
    let bom_rules: &[u8] = b"\xEF\xBB\xBFpw-report/\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", bom_rules)]);
    fx.disk(".gitignore", bom_rules);
    fx.disk("pw-report/X.java", violating("X").as_bytes());
    assert!(fx.repo.is_path_ignored("pw-report/X.java").expect("git"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

/// Git's own verdict (the CLI, not libgit2) on whether `path` is ignored.
fn git_cli_ignores(fx: &Fixture, path: &str) -> bool {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(fx.dir.path())
        .args(["check-ignore", "--no-index", "-q", "--", path])
        .status()
        .expect("run git check-ignore");
    status.success()
}

#[test]
fn test_trailing_non_space_whitespace_in_a_gitignore_pattern_does_not_skip_the_trimmed_name() {
    let rules: &[u8] = b"Tab.java\t\nNbsp.java\xc2\xa0\nSpaces.java  \n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    for name in ["Tab", "Nbsp", "Spaces"] {
        fx.disk(&format!("{name}.java"), violating(name).as_bytes());
    }
    assert!(!git_cli_ignores(&fx, "Tab.java"));
    assert!(!git_cli_ignores(&fx, "Nbsp.java"));
    assert!(git_cli_ignores(&fx, "Spaces.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["Nbsp.java", "Tab.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_negation_ending_in_a_tab_still_reincludes_the_file_git_reincludes() {
    let rules: &[u8] = b"gen*\n!gen\t\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("gen\t/X.java", violating("TabDir").as_bytes());
    fx.disk("generated/X.java", violating("Generated").as_bytes());
    assert!(!git_cli_ignores(&fx, "gen\t/X.java"));
    assert!(git_cli_ignores(&fx, "generated/X.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["gen\t/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_non_utf8_gitignore_line_does_not_skip_a_replacement_char_name() {
    let rules: &[u8] = b"X\xff.java\nPlain.java\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("X\u{FFFD}.java", violating("Replaced").as_bytes());
    fx.disk("Plain.java", violating("Plain").as_bytes());
    assert!(!git_cli_ignores(&fx, "X\u{FFFD}.java"));
    assert!(git_cli_ignores(&fx, "Plain.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["X\u{FFFD}.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_tab_followed_by_a_space_is_not_trimmed_past_the_tab() {
    let rules: &[u8] = b"TabSp.java\t \n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("TabSp.java", violating("TabSp").as_bytes());
    assert!(!git_cli_ignores(&fx, "TabSp.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["TabSp.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_negation_ending_in_nbsp_is_widened_by_its_byte_length() {
    let rules: &[u8] = b"gen*\n!gen\xc2\xa0\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("gen\u{a0}/X.java", violating("NbspDir").as_bytes());
    fx.disk("genp/X.java", violating("Genp").as_bytes());
    assert!(!git_cli_ignores(&fx, "gen\u{a0}/X.java"));
    assert!(git_cli_ignores(&fx, "genp/X.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["gen\u{a0}/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_non_utf8_negation_is_widened_never_dropped() {
    let rules: &[u8] = b"bad*\n!bad\xff\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("bad\u{FFFD}/X.java", violating("Widened").as_bytes());
    assert!(git_cli_ignores(&fx, "bad\u{FFFD}/X.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["bad\u{FFFD}/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_gitignore_line_outside_the_safe_subset_never_skips_a_file() {
    let rules: &[u8] =
        b"Foo{a,b}.java\nCls[[:upper:]].java\nOk[a-c]?.java\n**/deep/*.java\n\\#Hash.java\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    for (path, name) in [
        ("Fooa.java", "Fooa"),
        ("Clsu].java", "Clsu"),
        ("Okb1.java", "Okb"),
        ("x/deep/Y.java", "Deep"),
        ("#Hash.java", "Hash"),
    ] {
        fx.disk(path, violating(name).as_bytes());
    }
    assert!(!git_cli_ignores(&fx, "Fooa.java"));
    assert!(!git_cli_ignores(&fx, "Clsu].java"));
    assert!(git_cli_ignores(&fx, "Okb1.java"));
    assert!(git_cli_ignores(&fx, "x/deep/Y.java"));
    assert!(git_cli_ignores(&fx, "#Hash.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["Clsu].java", "Fooa.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_negation_outside_the_safe_subset_still_reincludes_what_git_reincludes() {
    let rules: &[u8] = b"pkg/*\n!pkg/Gen{a,b}\nlib/*\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("pkg/Gen{a,b}/X.java", violating("Braced").as_bytes());
    fx.disk("lib/Gena/X.java", violating("Lib").as_bytes());
    assert!(!git_cli_ignores(&fx, "pkg/Gen{a,b}/X.java"));
    assert!(git_cli_ignores(&fx, "lib/Gena/X.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["pkg/Gen{a,b}/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_an_escaped_trailing_tab_in_a_negation_is_widened_without_its_backslash() {
    let rules: &[u8] = b"gen*\n!gen\\\t\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("gen\t/X.java", violating("TabDir").as_bytes());
    fx.disk("genuine/X.java", violating("Genuine").as_bytes());
    assert!(!git_cli_ignores(&fx, "gen\t/X.java"));
    assert!(git_cli_ignores(&fx, "genuine/X.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["gen\t/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);

    let pair_rules: &[u8] = b"gen*\n!gen\\\\\t\n";
    let pair = fixture();
    pair.commit(&[(".gitignore", pair_rules)]);
    pair.disk(".gitignore", pair_rules);
    pair.disk("gen\\\t/X.java", violating("PairDir").as_bytes());
    assert!(!git_cli_ignores(&pair, "gen\\\t/X.java"));

    let outcome = pair.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["gen\\\t/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_an_invalid_byte_inside_a_negation_class_widens_the_whole_segment() {
    let rules: &[u8] = b"bad*\n!bad[a-\xff]\next/\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("badz/X.java", violating("Badz").as_bytes());
    fx.disk("ext/X.java", violating("Ext").as_bytes());
    assert!(!git_cli_ignores(&fx, "badz/X.java"));
    assert!(git_cli_ignores(&fx, "ext/X.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["badz/X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
}

#[test]
fn test_a_gitignore_line_is_cut_at_its_first_nul_as_git_cuts_it() {
    let rules: &[u8] = b"*.java\n!X.java\0garbage\n";
    let fx = fixture();
    fx.commit(&[(".gitignore", rules)]);
    fx.disk(".gitignore", rules);
    fx.disk("X.java", violating("X").as_bytes());
    fx.disk("Y.java", violating("Y").as_bytes());
    assert!(!git_cli_ignores(&fx, "X.java"));
    assert!(git_cli_ignores(&fx, "Y.java"));

    let outcome = fx.check(base_mode("HEAD", true));

    assert_eq!(complexity_paths(&outcome), vec!["X.java"]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
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

#[test]
fn test_unreadable_changed_base_blob_raises_g101() {
    let fx = fixture();
    let committed = java_class("Foo", LOW_CC_IFS, 0);
    let oid = fx.commit(&[("Foo.java", committed.as_bytes())]);
    let tree = fx
        .repo
        .find_commit(oid)
        .and_then(|c| c.tree())
        .expect("tree");
    let blob = tree.get_name("Foo.java").expect("Foo.java entry").id();
    let header = format!("blob {}\0", committed.len());
    let reviewed = format!("// reviewed\n{committed}");
    let forged = "x".repeat(committed.len());
    corrupt_loose_object(&fx.repo, blob, format!("{header}{forged}").as_bytes());
    fx.stage("Foo.java", reviewed.as_bytes());

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![G101]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_unreadable_changed_candidate_blob_raises_g101() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    let staged = java_class("Foo", HIGH_CC_IFS, 0);
    fx.stage("Foo.java", staged.as_bytes());
    let blob = fx
        .repo
        .index()
        .expect("open the index")
        .get_path(Path::new("Foo.java"), 0)
        .expect("staged entry")
        .id;
    let header = format!("blob {}\0", staged.len());
    let forged = "x".repeat(staged.len());
    corrupt_loose_object(&fx.repo, blob, format!("{header}{forged}").as_bytes());

    let outcome = fx.staged();

    assert!(codes(&outcome).contains(&G101));
    assert_eq!(outcome.exit_status & EXIT_ERROR, EXIT_ERROR);
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

const V102_OFF: &str = "version: 1\npolicy:\n  NSD-V102: off\n";

#[test]
fn test_changed_invalid_encoding_file_raises_a102_while_v102_is_off() {
    let fx = fixture();
    fx.commit(&[("util.ts", b"export const x = 1;\n")]);
    fx.stage("util.ts", INVALID_UTF8_TS);
    let (_dir, off) = write_trusted_config(V102_OFF);

    let outcome = fx.check_with(CheckMode::Staged, Some(&off), false);

    assert_eq!(
        coverage_reasons(&outcome),
        vec![("util.ts".to_string(), "invalid_encoding")]
    );
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_changed_complexity_regression_raises_e101_while_v102_is_off() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());
    let (_dir, off) = write_trusted_config(V102_OFF);

    let outcome = fx.check_with(CheckMode::Staged, Some(&off), false);

    assert_eq!(codes(&outcome), vec![E101]);
    assert_eq!(outcome.exit_status, EXIT_REGRESSION);
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

/// A repository whose unchanged committed `Unchanged.java` has an unreadable
/// loose blob, with a changed `Foo.java` staged.
fn fixture_with_corrupt_unchanged_blob() -> Fixture {
    let fx = fixture();
    let unchanged = java_class("Unchanged", LOW_CC_IFS, 0);
    let oid = fx.commit(&[
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
        ("Unchanged.java", unchanged.as_bytes()),
    ]);
    let blob = fx
        .repo
        .find_commit(oid)
        .and_then(|c| c.tree())
        .expect("tree")
        .get_name("Unchanged.java")
        .expect("Unchanged.java entry")
        .id();
    let header = format!("blob {}\0", unchanged.len());
    let forged = "x".repeat(unchanged.len());
    corrupt_loose_object(&fx.repo, blob, format!("{header}{forged}").as_bytes());
    fx.stage(
        "Foo.java",
        format!("// reviewed\n{}", java_class("Foo", LOW_CC_IFS, 0)).as_bytes(),
    );
    fx
}

#[test]
fn test_unreadable_unchanged_blob_is_not_read_while_v102_is_off() {
    let fx = fixture_with_corrupt_unchanged_blob();
    let (_dir, off) = write_trusted_config(V102_OFF);

    let outcome = fx.check_with(CheckMode::Staged, Some(&off), false);

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_unreadable_unchanged_blob_raises_g101_while_v102_is_enabled() {
    let fx = fixture_with_corrupt_unchanged_blob();

    let outcome = fx.staged();

    assert_eq!(codes(&outcome), vec![G101]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_staged_over_ceiling_and_invalid_encoding_files_raise_a102_while_v102_is_off() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("Big.java", &java_of_length(ceiling() + 1));
    fx.stage("util.ts", INVALID_UTF8_TS);
    let (_dir, off) = write_trusted_config(V102_OFF);

    let outcome = fx.check_with(CheckMode::Staged, Some(&off), false);

    assert_eq!(
        coverage_reasons(&outcome),
        vec![
            ("Big.java".to_string(), "too_large"),
            ("util.ts".to_string(), "invalid_encoding"),
        ]
    );
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

/// `text` followed by one padding comment, `length` bytes in all.
fn padded_to(text: &str, length: usize) -> Vec<u8> {
    let mut bytes = format!("{text}// ").into_bytes();
    let pad = length - bytes.len() - 1;
    bytes.extend(std::iter::repeat_n(b'x', pad));
    bytes.push(b'\n');
    assert_eq!(bytes.len(), length);
    bytes
}

#[test]
fn test_changed_file_over_ceiling_only_in_the_base_raises_a102_on_the_base_path() {
    let fx = fixture();
    let class = java_class("Big", LOW_CC_IFS, 0);
    fx.commit(&[("Big.java", &padded_to(&class, ceiling() + 1))]);
    fx.stage("Big.java", class.as_bytes());

    let outcome = fx.staged();

    assert_eq!(
        coverage_reasons(&outcome),
        vec![("Big.java".to_string(), "too_large")]
    );
    assert_eq!(codes(&outcome), vec![A102]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_changed_file_over_ceiling_on_both_sides_raises_one_a102() {
    let fx = fixture();
    let class = java_class("Big", LOW_CC_IFS, 0);
    fx.commit(&[("Big.java", &padded_to(&class, ceiling() + 1))]);
    fx.stage("Big.java", &padded_to(&class, ceiling() + 2));

    let outcome = fx.staged();

    assert_eq!(
        coverage_reasons(&outcome),
        vec![("Big.java".to_string(), "too_large")]
    );
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_deleting_a_file_over_ceiling_raises_nothing() {
    let fx = fixture();
    let class = java_class("Big", LOW_CC_IFS, 0);
    fx.commit(&[
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
        ("Big.java", &padded_to(&class, ceiling() + 1)),
    ]);
    fx.unstage("Big.java");

    let outcome = fx.staged();

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

/// `class` followed by a comment holding a byte that is not UTF-8.
fn with_invalid_utf8(class: &str) -> Vec<u8> {
    let mut bytes = class.as_bytes().to_vec();
    bytes.extend_from_slice(b"// \xff\xfe\n");
    bytes
}

#[test]
fn test_changed_file_with_invalid_encoding_only_in_the_base_raises_a102_on_the_base_path() {
    let fx = fixture();
    let class = java_class("Old", LOW_CC_IFS, 0);
    fx.commit(&[("Old.java", &with_invalid_utf8(&class))]);
    fx.stage("Old.java", class.as_bytes());

    let outcome = fx.staged();

    assert_eq!(
        coverage_reasons(&outcome),
        vec![("Old.java".to_string(), "invalid_encoding")]
    );
    assert_eq!(codes(&outcome), vec![A102]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_deleting_an_invalid_encoding_file_raises_nothing() {
    let fx = fixture();
    fx.commit(&[
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
        (
            "Old.java",
            &with_invalid_utf8(&java_class("Old", LOW_CC_IFS, 0)),
        ),
    ]);
    fx.unstage("Old.java");

    let outcome = fx.staged();

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_changed_file_with_invalid_encoding_only_in_the_base_raises_a102_while_v102_is_off() {
    let fx = fixture();
    let class = java_class("Old", LOW_CC_IFS, 0);
    fx.commit(&[("Old.java", &with_invalid_utf8(&class))]);
    fx.stage("Old.java", class.as_bytes());
    let (_dir, off) = write_trusted_config(V102_OFF);

    let outcome = fx.check_with(CheckMode::Staged, Some(&off), false);

    assert_eq!(
        coverage_reasons(&outcome),
        vec![("Old.java".to_string(), "invalid_encoding")]
    );
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_renaming_an_invalid_encoding_file_out_of_scope_raises_nothing() {
    let fx = fixture();
    let bytes = with_invalid_utf8(&java_class("Old", LOW_CC_IFS, 0));
    fx.commit(&[
        ("nsd.yml", b"version: 1\nexclude:\n  - legacy/**\n"),
        ("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes()),
        ("Old.java", &bytes),
    ]);
    fx.unstage("Old.java");
    fx.stage("legacy/Old.java", &bytes);

    let outcome = fx.staged();

    assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_PASS);
}

#[test]
fn test_invalid_encoding_on_both_sides_raises_one_a102() {
    let fx = fixture();
    fx.commit(&[("util.ts", INVALID_UTF8_TS)]);
    fx.stage("util.ts", b"export const x = '\xfd';\n");

    let outcome = fx.staged();

    assert_eq!(
        coverage_reasons(&outcome),
        vec![("util.ts".to_string(), "invalid_encoding")]
    );
    assert_eq!(codes(&outcome), vec![A102]);
    assert_eq!(outcome.exit_status, EXIT_ERROR);
}

#[test]
fn test_invalid_encoding_base_with_a_high_cc_candidate_reports_a102_and_e101() {
    let fx = fixture();
    fx.commit(&[(
        "Old.java",
        &with_invalid_utf8(&java_class("Old", LOW_CC_IFS, 0)),
    )]);
    fx.stage("Old.java", java_class("Old", HIGH_CC_IFS, 0).as_bytes());

    let outcome = fx.staged();

    let found = codes(&outcome);
    assert!(found.contains(&A102), "{found:?}");
    assert!(found.contains(&E101), "{found:?}");
    assert_eq!(
        coverage_reasons(&outcome),
        vec![("Old.java".to_string(), "invalid_encoding")]
    );
    assert_eq!(outcome.exit_status, EXIT_BOTH);
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
fn test_candidate_config_exclude_cannot_hide_its_own_regression() {
    let fx = fixture();
    fx.commit(&[("src/Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("src/Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes());
    fx.stage("nsd.yml", b"version: 1\nexclude:\n  - src/**\n");

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

const GARBAGE_JAVA: &str = ")))\n}}}\n;;; @@@ ###\n(((\n]]]\n";

fn assert_total_parse_loss(outcome: &CheckOutcome, path: &str) {
    assert!(codes(outcome).contains(&A101), "{:?}", outcome.diagnostics);
    assert_eq!(outcome.exit_status, EXIT_ERROR, "{:?}", outcome.diagnostics);
    let details = outcome.details.as_ref().expect("a run with both sides");
    assert!(
        details.entities.iter().all(|entity| entity.path != path),
        "the garbage file yielded a callable: {:?}",
        details.entities
    );
    let coverage = details
        .coverage
        .iter()
        .find(|file| file.path == path)
        .expect("coverage for the garbage file");
    assert_eq!(coverage.analyzed_lines, 0, "{coverage:?}");
    assert!(!coverage.complete, "{coverage:?}");
    assert!(!coverage.gaps.is_empty(), "{coverage:?}");
}

#[test]
fn test_total_parse_loss_of_a_changed_file_raises_a101_and_exits_2() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", HIGH_CC_IFS, 0).as_bytes())]);
    fx.stage("Foo.java", GARBAGE_JAVA.as_bytes());

    let outcome = fx.staged();

    assert_total_parse_loss(&outcome, "Foo.java");
    let found = codes(&outcome);
    assert!(
        found.iter().all(|code| *code == A101),
        "the vanished callables raised {found:?}"
    );
    assert!(![E101, E102, V101].iter().any(|code| found.contains(code)));
}

#[test]
fn test_added_file_with_total_parse_loss_raises_a101() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS, 0).as_bytes())]);
    fx.stage("Junk.java", GARBAGE_JAVA.as_bytes());

    let outcome = fx.staged();

    assert_total_parse_loss(&outcome, "Junk.java");
}
