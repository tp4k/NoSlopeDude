//! M5-4 acceptance for the binary: `nsd check` over scratch repositories,
//! plus the terminal listing's escaping and ordering.

mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use git2::{IndexEntry, IndexTime, Oid, Repository};
use tempfile::TempDir;

use nsd::check::CheckDiagnostic;
use nsd::format::render_terminal;
use nsd::git::path::RepoPath;
use nsd::policy::diagnostics::{
    CoverageDiagnostic, FindingDiagnostic, PolicyDiagnostic, CODE_ANALYSIS_UNAVAILABLE,
    CODE_COMPLEXITY_ABOVE_THRESHOLD, CODE_UNMATCHED_FINDING,
};

use nsd::analysis::UnanalyzableReason;
use nsd::config::CODE_INVALID_CONFIG;

const MODE_REGULAR: i32 = 0o100644;
const EXIT_PASS: i32 = 0;
const EXIT_REGRESSION: i32 = 1;
const EXIT_ERROR: i32 = 2;
const EXIT_BOTH: i32 = 3;
const E101: &str = "NSD-E101";
const S102: &str = "NSD-S102";
const A102: &str = "NSD-A102";
const C102: &str = CODE_INVALID_CONFIG;
const G101: &str = "NSD-G101";
const LOW_CC_IFS: usize = 4;
const HIGH_CC_IFS: usize = 11;
const ESC: u8 = 0x1b;
const DEL: u8 = 0x7f;
const FIRST_PRINTABLE: u8 = 0x20;
const LISTED_DIAGNOSTICS: usize = 120;
const WARN_E101: &str = "version: 1\npolicy:\n  NSD-E101: warn\n";
const WARN_S102: &str = "version: 1\npolicy:\n  NSD-S102: warn\n";
const INVALID_UTF8_TS: &[u8] = b"export const x = '\xff\xfe';\n";

struct Fixture {
    dir: TempDir,
    repo: Repository,
}

fn fixture() -> Fixture {
    let (dir, repo) = common::init_repo();
    Fixture { dir, repo }
}

impl Fixture {
    fn root(&self) -> &Path {
        self.dir.path()
    }

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

    /// A committed CC-5 method with a staged CC-12 version of it.
    fn staged_regression() -> Fixture {
        let fx = fixture();
        fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS).as_bytes())]);
        fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS).as_bytes());
        fx
    }

    fn nsd(&self, args: &[&str]) -> Output {
        nsd_in(self.root(), args)
    }
}

fn nsd_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run the nsd binary")
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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn exit_code(output: &Output) -> Option<i32> {
    output.status.code()
}

fn write_config(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).expect("write a config file");
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("the scratch path is UTF-8")
}

fn has_raw_control_byte(text: &[u8]) -> bool {
    text.iter()
        .any(|&byte| (byte < FIRST_PRINTABLE && byte != b'\n') || byte == DEL)
}

fn complexity(name: &str, path: RepoPath) -> CheckDiagnostic {
    CheckDiagnostic::Complexity(Box::new(PolicyDiagnostic {
        code: CODE_COMPLEXITY_ABOVE_THRESHOLD,
        candidate_path: path,
        candidate_name: name.to_string(),
        candidate_start_line: 1,
        candidate_end_line: 2,
        base: None,
    }))
}

#[test]
fn test_check_staged_regression_exits_1_and_names_the_code() {
    let fx = Fixture::staged_regression();

    let output = fx.nsd(&["check", "--staged"]);

    assert_eq!(exit_code(&output), Some(EXIT_REGRESSION));
    let listing = stdout(&output);
    assert!(
        listing
            .lines()
            .any(|line| line.contains(E101) && line.contains("Foo.java")),
        "{listing}"
    );
}

#[test]
fn test_check_staged_clean_change_exits_0() {
    let fx = fixture();
    let base = java_class("Foo", LOW_CC_IFS);
    fx.commit(&[("Foo.java", base.as_bytes())]);
    fx.stage("Foo.java", format!("// reviewed\n{base}").as_bytes());

    let output = fx.nsd(&["check", "--staged"]);

    assert_eq!(exit_code(&output), Some(EXIT_PASS));
    assert_eq!(stdout(&output), "");
}

#[test]
fn test_check_regression_and_analysis_error_exit_3() {
    let fx = Fixture::staged_regression();
    fx.stage("bad.ts", INVALID_UTF8_TS);

    let output = fx.nsd(&["check", "--staged"]);

    assert_eq!(exit_code(&output), Some(EXIT_BOTH));
    let listing = stdout(&output);
    assert!(
        listing.contains(E101) && listing.contains(A102),
        "{listing}"
    );
}

#[test]
fn test_check_missing_base_ref_exits_2_with_g101() {
    let fx = Fixture::staged_regression();

    let output = fx.nsd(&["check", "--base", "nope"]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert!(stdout(&output).contains(G101), "{}", stdout(&output));
}

#[test]
fn test_check_staged_and_base_are_mutually_exclusive() {
    let fx = Fixture::staged_regression();

    let output = fx.nsd(&["check", "--staged", "--base", "main"]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert_eq!(stdout(&output), "");
    let usage = String::from_utf8_lossy(&output.stderr);
    assert!(
        usage.contains("--staged") && usage.contains("--base"),
        "{usage}"
    );
}

#[test]
fn test_check_worktree_requires_base() {
    let fx = Fixture::staged_regression();

    let output = fx.nsd(&["check", "--worktree"]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert_eq!(stdout(&output), "");
    let usage = String::from_utf8_lossy(&output.stderr);
    assert!(
        usage.contains("--worktree") && usage.contains("--base"),
        "{usage}"
    );
}

#[test]
fn test_check_outside_a_repository_exits_2_with_g101() {
    let dir = TempDir::new().expect("create a non-repository directory");

    let output = nsd_in(dir.path(), &["check", "--staged"]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert!(stdout(&output).contains(G101), "{}", stdout(&output));
}

#[test]
fn test_check_trusted_config_downgrade_prints_but_exits_0() {
    let fx = Fixture::staged_regression();
    let outside = TempDir::new().expect("create a directory outside the checkout");
    let config = write_config(outside.path(), "trusted.yml", WARN_E101);

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&config)]);

    assert_eq!(exit_code(&output), Some(EXIT_PASS));
    assert!(stdout(&output).contains(E101), "{}", stdout(&output));
}

#[test]
fn test_check_config_inside_the_checkout_is_refused_with_c102() {
    let fx = Fixture::staged_regression();
    let inside = write_config(fx.root(), "trusted.yml", WARN_E101);
    fs::create_dir(fx.root().join("sub")).expect("create a subdirectory");
    let nested = write_config(&fx.root().join("sub"), "trusted.yml", WARN_E101);

    for given in [
        path_str(&inside),
        "trusted.yml",
        "sub/trusted.yml",
        path_str(&nested),
    ] {
        let output = fx.nsd(&["check", "--staged", "--config", given]);

        assert_eq!(exit_code(&output), Some(EXIT_ERROR), "{given}");
        let listing = stdout(&output);
        assert!(listing.contains(C102), "{given}: {listing}");
        assert!(!listing.contains(E101), "{given}: {listing}");
    }
}

#[test]
fn test_check_config_symlink_resolving_inside_the_checkout_is_refused() {
    let fx = Fixture::staged_regression();
    let target = write_config(fx.root(), "trusted.yml", WARN_E101);
    let outside = TempDir::new().expect("create a directory outside the checkout");
    let link = outside.path().join("link.yml");
    symlink(&target, &link).expect("link an outside path to a file in the checkout");

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&link)]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    let listing = stdout(&output);
    assert!(listing.contains(C102), "{listing}");
    assert!(!listing.contains(E101), "{listing}");
}

#[test]
fn test_check_config_symlink_inside_the_checkout_to_an_outside_file_is_refused() {
    let fx = Fixture::staged_regression();
    let outside = TempDir::new().expect("create a directory outside the checkout");
    let target = write_config(outside.path(), "trusted.yml", WARN_E101);
    let link = fx.root().join("link.yml");
    symlink(&target, &link).expect("link a checkout path to an outside file");

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&link)]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert!(stdout(&output).contains(C102), "{}", stdout(&output));
}

#[test]
fn test_check_config_inside_the_git_directory_is_refused() {
    let fx = Fixture::staged_regression();
    let inside_git = write_config(&fx.root().join(".git"), "trusted.yml", WARN_E101);

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&inside_git)]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert!(stdout(&output).contains(C102), "{}", stdout(&output));
}

#[test]
fn test_check_config_beside_the_checkout_is_not_mistaken_for_inside() {
    let parent = TempDir::new().expect("create a parent directory");
    let root = parent.path().join("repo");
    fs::create_dir(&root).expect("create the checkout directory");
    let repo = Repository::init(&root).expect("init the repository");
    let fx = Fixture { dir: parent, repo };
    let sibling = write_config(fx.dir.path(), "repo-trusted.yml", WARN_E101);
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS).as_bytes())]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS).as_bytes());

    let output = nsd_in(
        &root,
        &["check", "--staged", "--config", path_str(&sibling)],
    );

    assert_eq!(exit_code(&output), Some(EXIT_PASS));
    assert!(stdout(&output).contains(E101), "{}", stdout(&output));
}

#[test]
fn test_check_output_has_no_absolute_paths() {
    let fx = Fixture::staged_regression();
    fx.stage("bad.ts", INVALID_UTF8_TS);

    let output = fx.nsd(&["check", "--staged"]);

    let listing = stdout(&output);
    assert!(listing.contains(E101), "{listing}");
    let canonical = fs::canonicalize(fx.root()).expect("canonicalize the scratch repository");
    assert!(!listing.contains(path_str(fx.root())), "{listing}");
    assert!(!listing.contains(path_str(&canonical)), "{listing}");
}

#[test]
fn test_check_warn_s102_prints_but_exits_0() {
    let fx = fixture();
    let quiet = "class Widget {\n    void run() {\n        work();\n    }\n}\n";
    fx.commit(&[("Widget.java", quiet.as_bytes())]);
    let unused = "class Widget {\n    void run() {\n        // nsd-ignore[JAVA-EMPTY-CATCH]: stale\n        work();\n    }\n}\n";
    fx.stage("Widget.java", unused.as_bytes());
    let outside = TempDir::new().expect("create a directory outside the checkout");
    let config = write_config(outside.path(), "trusted.yml", WARN_S102);

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&config)]);

    assert_eq!(exit_code(&output), Some(EXIT_PASS));
    assert!(stdout(&output).contains(S102), "{}", stdout(&output));
}

#[test]
fn test_render_escapes_control_characters_in_candidate_names() {
    let hostile = "run\u{1b}[31m\u{7}\u{7f}\u{9b}\nNSD-FAKE";
    let diagnostics = [complexity(hostile, RepoPath::from_bytes("src/Foo.java"))];

    let rendered = render_terminal(&diagnostics);

    assert!(!has_raw_control_byte(rendered.as_bytes()), "{rendered:?}");
    assert!(!rendered.contains('\u{9b}'), "{rendered:?}");
    assert_eq!(rendered.lines().count(), 1, "{rendered:?}");
    assert!(rendered.starts_with(CODE_COMPLEXITY_ABOVE_THRESHOLD));
    assert!(rendered.contains("run"), "{rendered:?}");
}

#[test]
fn test_render_escapes_control_bytes_in_paths_and_failure_messages() {
    let hostile_path = RepoPath::from_bytes("dir\u{1b}]0;x\u{7}/Foo.java");
    let failure = CheckDiagnostic::Failure {
        code: G101,
        message: format!("cannot read {}", hostile_path.render()),
    };
    let diagnostics = [
        failure,
        complexity("run", hostile_path.clone()),
        CheckDiagnostic::Coverage(CoverageDiagnostic {
            code: CODE_ANALYSIS_UNAVAILABLE,
            path: hostile_path,
            reason: UnanalyzableReason::TooLarge,
        }),
    ];

    let rendered = render_terminal(&diagnostics);

    assert!(!has_raw_control_byte(rendered.as_bytes()), "{rendered:?}");
    assert_eq!(rendered.lines().count(), 3, "{rendered:?}");
    assert!(rendered.contains("Foo.java"), "{rendered:?}");
}

#[test]
fn test_check_control_bytes_in_a_failure_message_never_reach_the_terminal() {
    let fx = Fixture::staged_regression();
    let outside = TempDir::new().expect("create a directory outside the checkout");
    let missing = outside.path().join("bad\u{1b}[2Jname.yml");

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&missing)]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert!(
        stdout(&output).contains("bad"),
        "the failure must name the path: {}",
        stdout(&output)
    );
    assert!(!output.stdout.contains(&ESC), "{:?}", stdout(&output));
    assert!(!output.stderr.contains(&ESC), "{:?}", output.stderr);
}

#[test]
fn test_render_percent_escapes_a_non_utf8_coverage_path() {
    let diagnostics = [CheckDiagnostic::Coverage(CoverageDiagnostic {
        code: CODE_ANALYSIS_UNAVAILABLE,
        path: RepoPath::from_bytes(b"src/100%\xff.java".to_vec()),
        reason: UnanalyzableReason::NonUtf8Path,
    })];

    let rendered = render_terminal(&diagnostics);

    assert!(rendered.contains("src/100%25%FF.java"), "{rendered:?}");
    assert!(rendered.contains(CODE_ANALYSIS_UNAVAILABLE), "{rendered:?}");
}

#[test]
fn test_render_lists_every_diagnostic_uncapped() {
    let diagnostics: Vec<CheckDiagnostic> = (0..LISTED_DIAGNOSTICS)
        .rev()
        .map(|index| {
            CheckDiagnostic::Finding(FindingDiagnostic {
                code: CODE_UNMATCHED_FINDING,
                rule_id: "JAVA-EMPTY-CATCH",
                candidate_path: RepoPath::from_bytes(format!("f{index:03}.java")),
                candidate_start_line: 1,
                candidate_end_line: 1,
            })
        })
        .collect();

    let rendered = render_terminal(&diagnostics);

    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(lines.len(), LISTED_DIAGNOSTICS);
    for (position, line) in lines.iter().enumerate() {
        let expected = format!("f{:03}.java", LISTED_DIAGNOSTICS - 1 - position);
        assert!(line.contains(&expected), "line {position}: {line}");
    }
}

#[test]
fn test_check_staged_with_worktree_is_a_usage_error() {
    let fx = Fixture::staged_regression();

    let output = fx.nsd(&["check", "--staged", "--worktree"]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert_eq!(stdout(&output), "");
}

#[test]
fn test_check_base_worktree_judges_the_working_tree_and_plain_base_judges_head() {
    let fx = fixture();
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS).as_bytes())]);
    fs::write(
        fx.root().join("Foo.java"),
        java_class("Foo", HIGH_CC_IFS).as_bytes(),
    )
    .expect("edit the worktree file");

    let overlay = fx.nsd(&["check", "--base", "HEAD", "--worktree"]);
    let committed = fx.nsd(&["check", "--base", "HEAD"]);

    assert_eq!(exit_code(&overlay), Some(EXIT_REGRESSION));
    assert!(stdout(&overlay).contains(E101), "{}", stdout(&overlay));
    assert_eq!(exit_code(&committed), Some(EXIT_PASS));
    assert_eq!(stdout(&committed), "");
}

#[test]
fn test_check_unwritable_stdout_exits_2_not_1() {
    let fx = Fixture::staged_regression();
    let (reader, writer) = std::io::pipe().expect("create a pipe");
    drop(reader);

    let status = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(["check", "--staged"])
        .current_dir(fx.root())
        .stdout(writer)
        .status()
        .expect("run the nsd binary");

    assert_eq!(status.code(), Some(EXIT_ERROR));
}

#[test]
fn test_check_config_dotdot_path_resolving_inside_the_checkout_is_refused() {
    let fx = Fixture::staged_regression();
    write_config(fx.root(), "trusted.yml", WARN_E101);

    let output = fx.nsd(&["check", "--staged", "--config", "missing/../trusted.yml"]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert!(stdout(&output).contains(C102), "{}", stdout(&output));
}

#[test]
fn test_check_config_dotdot_path_leaving_the_checkout_is_used() {
    let parent = TempDir::new().expect("create a parent directory");
    let root = parent.path().join("repo");
    fs::create_dir_all(root.join("sub")).expect("create the checkout directory");
    let repo = Repository::init(&root).expect("init the repository");
    let fx = Fixture { dir: parent, repo };
    write_config(fx.dir.path(), "trusted.yml", WARN_E101);
    fx.commit(&[("Foo.java", java_class("Foo", LOW_CC_IFS).as_bytes())]);
    fx.stage("Foo.java", java_class("Foo", HIGH_CC_IFS).as_bytes());

    let output = nsd_in(
        &root,
        &["check", "--staged", "--config", "sub/../../trusted.yml"],
    );

    assert_eq!(exit_code(&output), Some(EXIT_PASS));
    assert!(stdout(&output).contains(E101), "{}", stdout(&output));
}

#[test]
fn test_check_config_outside_symlink_then_dotdot_resolving_inside_is_refused() {
    let fx = Fixture::staged_regression();
    write_config(fx.root(), "trusted.yml", WARN_E101);
    fs::create_dir_all(fx.root().join("sub")).expect("create a directory in the checkout");
    let outside = TempDir::new().expect("create a directory outside the checkout");
    let link = outside.path().join("l");
    symlink(fx.root().join("sub"), &link).expect("link an outside path to a checkout directory");
    let given = link.join("..").join("trusted.yml");

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&given)]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    let listing = stdout(&output);
    assert!(listing.contains(C102), "{listing}");
    assert!(!listing.contains(E101), "{listing}");
}

#[test]
fn test_check_missing_config_outside_symlink_then_dotdot_inside_is_refused() {
    let fx = Fixture::staged_regression();
    fs::create_dir_all(fx.root().join("sub")).expect("create a directory in the checkout");
    let outside = TempDir::new().expect("create a directory outside the checkout");
    let link = outside.path().join("l");
    symlink(fx.root().join("sub"), &link).expect("link an outside path to a checkout directory");
    let given = link.join("..").join("absent.yml");

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&given)]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    assert!(stdout(&output).contains(C102), "{}", stdout(&output));
}

#[test]
fn test_check_config_symlink_reached_through_symlink_dotdot_is_refused() {
    let fx = Fixture::staged_regression();
    let target = write_config(fx.root(), "trusted.yml", WARN_E101);
    fs::create_dir_all(fx.root().join("sub")).expect("create a directory in the checkout");
    let beside = fx.root().parent().expect("the checkout has a parent");
    symlink(&target, beside.join("ln")).expect("link a path beside the checkout into it");
    let outside = TempDir::new().expect("create a directory outside the checkout");
    let link = outside.path().join("l");
    symlink(fx.root().join("sub"), &link).expect("link an outside path to a checkout directory");
    let given = link.join("..").join("..").join("ln");

    let output = fx.nsd(&["check", "--staged", "--config", path_str(&given)]);

    assert_eq!(exit_code(&output), Some(EXIT_ERROR));
    let listing = stdout(&output);
    assert!(listing.contains(C102), "{listing}");
    assert!(!listing.contains(E101), "{listing}");
}
