//! Differential harness: nsd's base-AND-candidate skip decision for untracked
//! files must equal `git check-ignore --no-index` over a pattern x path
//! corpus and over every rule set the earlier Codex gates reported.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use git2::Repository;
use nsd::git::snapshot::{CommitSnapshot, WorktreeSnapshot};
use tempfile::TempDir;

const MODE_REGULAR: i32 = 0o100644;
const FILE_BODY: &[u8] = b"class X {}\n";
const GITIGNORE: &str = ".gitignore";
const PLACEMENTS: [&str; 2] = ["", "p/"];
const SHAPES: [(&str, &str); 5] = [
    ("", ".java"),
    ("p/", ".java"),
    ("p/q/", ".java"),
    ("", "/X.java"),
    ("p/", "/X.java"),
];
const NAMES: [&str; 31] = [
    "a", "b", "ab", "aa", "abc", "a b", "a\\b", "a*", "a?", "a!", "!a", "#a", "a#", "a{b}", "a[b]",
    "a]", "a^", "a-", "a\t", "a\u{a0}", "é", "aé", "a.", "a..", " a", "a\\", "a\\ ", "a  ", "a\r",
    "[a]", "foo",
];
const KEEP_PATHS: [&str; 3] = ["a/Keep.java", "a/q/Keep.java", "a/q/r/Keep.java"];
const PRODUCT_LEFT: [&str; 8] = ["a", "*", "?", "**", "\\*", "\\?", "[!a]", "[a-c]"];
const PRODUCT_RIGHT: [&str; 5] = ["*", "?", ".java", "/**", "/X.java"];
const PATTERNS: [&str; 57] = [
    "*",
    "?",
    "**",
    "**/",
    "p/**",
    "p/**/",
    "p/**/X.java",
    "**/*.java",
    "**/a*",
    "p/*",
    "p/",
    "p//",
    "/p/",
    "a*",
    "a?.java",
    "a\\*.java",
    "a\\?.java",
    "\\!a.java",
    "\\#a.java",
    "a\\b.java",
    "a\\\\b.java",
    "a\\ .java",
    "a\\  ",
    "[a-b]*",
    "[!a]*",
    "[a-cx]*",
    "[!a-c]*",
    "[ -~]*",
    "[a-a]*",
    "[!!]*",
    "[!]*",
    "[a!]*",
    "[a#]*",
    "[a:]*",
    "[a=]*",
    "[a.]*",
    "[a,]*",
    "[a{]*",
    "[a}]*",
    "[a?]*",
    "[a*]*",
    "[a\t]*",
    "[\x01-z]*",
    "[!-~]*",
    "[.-0]*",
    "[Z-a]*",
    "p/../a.java",
    "./a.java",
    "a.java\r",
    "a.java\t",
    "a.java\0x",
    "é*",
    "a\\é*",
    "[a-b]/**",
    "*/*",
    "**/**/a*",
    "p[!x]q[!x]a.java",
];
const EXTRA_PATTERNS: [&str; 8] = [
    "p[!x]a.java",
    "p[.-0]a.java",
    "p[ -~]a.java",
    "p[!x]*.java",
    "p[!a-c]a.java",
    "a/***/Keep.java",
    "a/**\\/Keep.java",
    "a/*/Keep.java",
];

/// One repository whose files exist on disk; rule sets are swapped in and out
/// (committed and written to disk with identical bytes) between comparisons.
struct Arena {
    dir: TempDir,
    repo: Repository,
    paths: Vec<String>,
    rule_dirs: BTreeSet<String>,
}

struct Verdict {
    path: String,
    nsd_skips: bool,
    git_ignores: bool,
}

impl Arena {
    fn new(paths: &[String]) -> Arena {
        let (dir, repo) = common::init_repo();
        for path in paths {
            write_file(&dir.path().join(path), FILE_BODY);
        }
        let info = dir.path().join(".git/info");
        fs::create_dir_all(&info).expect("create .git/info");
        fs::write(info.join("exclude"), b"").expect("empty info/exclude");
        Arena {
            dir,
            repo,
            paths: paths.to_vec(),
            rule_dirs: BTreeSet::new(),
        }
    }

    /// Installs `rules` (`.gitignore` path, bytes) on both sides and compares.
    fn compare(&mut self, rules: &[(String, Vec<u8>)]) -> Vec<Verdict> {
        for stale in std::mem::take(&mut self.rule_dirs) {
            let _ = fs::remove_file(self.dir.path().join(&stale));
        }
        let mut entries = Vec::new();
        for (path, bytes) in rules {
            write_file(&self.dir.path().join(path), bytes);
            self.rule_dirs.insert(path.clone());
            entries.push((path.as_bytes().to_vec(), MODE_REGULAR, bytes.clone()));
        }
        common::commit_entries(&self.repo, &entries);
        let base = CommitSnapshot::head_or_empty(&self.repo).expect("snapshot the base");
        let overlay = WorktreeSnapshot::open_over_base_ignores(&self.repo, &base)
            .expect("snapshot the worktree over the base ignores");
        let present: BTreeSet<Vec<u8>> = overlay
            .entries
            .iter()
            .map(|entry| entry.path.as_bytes().to_vec())
            .collect();
        let ignored = git_ignored(self.dir.path(), &self.paths);
        self.paths
            .iter()
            .map(|path| Verdict {
                path: path.clone(),
                nsd_skips: !present.contains(path.as_bytes()),
                git_ignores: ignored.contains(path),
            })
            .collect()
    }
}

fn write_file(target: &Path, bytes: &[u8]) {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).expect("create the parent directories");
    }
    fs::write(target, bytes).expect("write a file");
}

/// The subset of `paths` git ignores with `--no-index`, under the oracle
/// settings: no global or system configuration, no excludes file, case-exact.
fn git_ignored(repo: &Path, paths: &[String]) -> BTreeSet<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "core.excludesFile=/dev/null",
            "-c",
            "core.ignoreCase=false",
            "check-ignore",
            "--no-index",
            "-z",
            "--stdin",
        ])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn git check-ignore");
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    let mut stdin = child.stdin.take().expect("git stdin");
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().expect("wait for git check-ignore");
    writer
        .join()
        .expect("join the stdin writer")
        .expect("feed git check-ignore");
    let code = output.status.code();
    assert!(
        matches!(code, Some(0 | 1)),
        "git check-ignore failed ({code:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8(part.to_vec()).expect("a UTF-8 path"))
        .collect()
}

fn corpus_paths() -> Vec<String> {
    let mut paths = Vec::new();
    for name in NAMES {
        for (prefix, suffix) in SHAPES {
            paths.push(format!("{prefix}{name}{suffix}"));
        }
    }
    paths.extend(KEEP_PATHS.iter().map(|path| path.to_string()));
    paths
}

fn corpus_patterns() -> Vec<String> {
    let mut patterns: Vec<String> = PATTERNS.iter().map(|pattern| pattern.to_string()).collect();
    patterns.extend(EXTRA_PATTERNS.iter().map(|pattern| pattern.to_string()));
    for left in PRODUCT_LEFT {
        for right in PRODUCT_RIGHT {
            patterns.push(format!("{left}{right}"));
        }
    }
    let mut seen = BTreeSet::new();
    patterns.retain(|pattern| seen.insert(pattern.clone()));
    patterns
}

/// The rule sets (one `.gitignore` text per variant) a pattern is tried in:
/// the pattern alone, and behind `*.java` as a negation.
fn variants(pattern: &str) -> [(&'static str, Vec<u8>); 2] {
    [
        ("plain", format!("{pattern}\n").into_bytes()),
        ("negated", format!("*.java\n!{pattern}\n").into_bytes()),
    ]
}

fn describe(rules: &[(String, Vec<u8>)]) -> String {
    rules
        .iter()
        .map(|(path, bytes)| format!("{path}: {:?}", String::from_utf8_lossy(bytes)))
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn test_nsd_agrees_with_git_check_ignore_over_the_pattern_corpus() {
    let started = Instant::now();
    let paths = corpus_paths();
    let patterns = corpus_patterns();
    let mut arena = Arena::new(&paths);
    let mut comparisons = 0usize;
    let mut git_ignored_count = 0usize;
    let mut git_kept_count = 0usize;
    let mut disagreements = Vec::new();
    for pattern in &patterns {
        for (variant, bytes) in variants(pattern) {
            for placement in PLACEMENTS {
                let rules = vec![(format!("{placement}{GITIGNORE}"), bytes.clone())];
                for verdict in arena.compare(&rules) {
                    comparisons += 1;
                    if verdict.git_ignores {
                        git_ignored_count += 1;
                    } else {
                        git_kept_count += 1;
                    }
                    if verdict.nsd_skips != verdict.git_ignores {
                        disagreements.push(format!(
                            "{variant} {} path {:?}: nsd skips {}, git ignores {}",
                            describe(&rules),
                            verdict.path,
                            verdict.nsd_skips,
                            verdict.git_ignores
                        ));
                    }
                }
            }
        }
    }
    eprintln!(
        "differential corpus: {} patterns, {comparisons} comparisons in {:?}",
        patterns.len(),
        started.elapsed()
    );
    assert_eq!(
        comparisons,
        patterns.len() * variants("").len() * PLACEMENTS.len() * paths.len()
    );
    assert!(git_ignored_count > 0, "git ignored no corpus path");
    assert!(git_kept_count > 0, "git kept every corpus path");
    assert!(
        disagreements.is_empty(),
        "{} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

struct GateCase {
    rules: Vec<(&'static str, Vec<u8>)>,
    paths: &'static [&'static str],
}

fn root(rules: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    vec![(GITIGNORE, rules.to_vec())]
}

fn gate_cases() -> Vec<GateCase> {
    const KEEP: &[&str] = &["a/Keep.java", "a/q/Keep.java", "a/q/r/Keep.java"];
    let mut cases = Vec::new();
    let mut add = |rules: Vec<(&'static str, Vec<u8>)>, paths: &'static [&'static str]| {
        cases.push(GateCase { rules, paths });
    };
    // Gate 1.
    let gate1: &'static [&'static str] = &["X.java", "X.java\t", "X.java\u{a0}", "X\u{FFFD}.java"];
    add(root(b"X.java\t\n"), gate1);
    add(root(b"X.java\xc2\xa0\n"), gate1);
    add(root(b"X\xff.java\n"), gate1);
    // Gate 2.
    add(root(b"Foo{a,b}.java\n"), &["Fooa.java", "Foo{a,b}.java"]);
    add(
        root(b"pkg/*\n!pkg/Gen{a,b}\n"),
        &["pkg/Gen{a,b}/X.java", "pkg/Gena/X.java"],
    );
    let classes: &'static [&'static str] = &["Cls[a.java", "Clsa.java", "a["];
    add(root(b"Cls[a.java\n"), classes);
    add(root(b"a[\n"), classes);
    let upper: &'static [&'static str] = &["ClsQ.java", "Cls1.java", "Clsu].java"];
    for rule in [
        &b"Cls[[:upper:]].java\n"[..],
        b"Cls[![:upper:]].java\n",
        b"Cls[^[:digit:]].java\n",
        b"Cls[[:bogus:]].java\n",
    ] {
        add(root(rule), upper);
    }
    let gen: &'static [&'static str] = &["gen\t/X.java", "gen\\\t/X.java", "genuine/X.java"];
    add(root(b"gen*\n!gen\\\t\n"), gen);
    add(root(b"gen*\n!gen\\\\\t\n"), gen);
    add(
        root(b"bad*\n!bad[a-\xff]\n"),
        &["badz/X.java", "badé/X.java"],
    );
    add(root(b"*.java\n!X.java\0garbage\n"), &["X.java", "Y.java"]);
    // Gate 3.
    for rule in [&b"p[!x]a.java\n"[..], b"p[.-0]a.java\n", b"p[ -~]a.java\n"] {
        add(root(rule), &["p/a.java", "pya.java"]);
    }
    add(root(b"gen*\n!gen\t\n"), &["gen\t/X.java", "genuine/X.java"]);
    // Triage rounds 1 to 3.
    add(
        root(b"gen*\n!gen\xc2\xa0\n"),
        &["gen\u{a0}/X.java", "genp/X.java"],
    );
    add(root(b"bad*\n!bad\xff\n"), &["bad\u{FFFD}/X.java"]);
    add(
        root(b"Tab.java\t\nTabSp.java\t \nSpaces.java  \na\\ \n"),
        &["Tab.java", "TabSp.java", "Spaces.java", "a "],
    );
    // Plan review round 1: star runs.
    for rule in [
        &b"*.java\n!a/***/Keep.java\n"[..],
        b"*.java\n!a/**\\/Keep.java\n",
        b"*.java\n!a/**/Keep.java\n",
        b"*.java\n!a/*/Keep.java\n",
    ] {
        add(root(rule), KEEP);
    }
    // Parsing.
    add(root(b"\xEF\xBB\xBFpw/\n"), &["pw/X.java"]);
    add(root(b"a.java\r\n"), &["a.java"]);
    add(
        root(b"\\#Hash.java\n\\!Bang.java\n#Comment.java\n"),
        &["#Hash.java", "!Bang.java", "#Comment.java"],
    );
    add(
        vec![("sub/.gitignore", b"/anchored.java\n".to_vec())],
        &["sub/anchored.java", "sub/deep/anchored.java"],
    );
    add(
        root(b"emitted/\n!emitted/Keep.java\n"),
        &["emitted/Keep.java"],
    );
    add(
        root(b"**/deep/*.java\nx/**\n"),
        &["x/deep/Y.java", "x/Y.java", "deep/Y.java"],
    );
    add(root(b"x/**\n"), &["x"]);
    add(root(b"a/**/b\n"), &["a/b", "a/q/r/b", "a/qb"]);
    // A lone trailing backslash never matches.
    add(root(b"a\\\n"), &["a", "a\\"]);
    // A `**` that follows a literal prefix is not a segment start.
    let below: &'static [&'static str] = &["a/b/c", "a/b/cx"];
    for rule in [
        &b"a/b/*\n!a/b**\n"[..],
        b"a/b/*\n!a/b**/\n",
        b"a/b/*\n!a/b**c\n",
        b"a/b/*\n!a/bx**\n",
        b"a/b/*\n!a**\n",
        b"a/b/*\n!a/b/**\n",
        b"a/b/c\n!a/b**\n",
    ] {
        add(root(rule), below);
    }
    // The large nested block next to a root `*.java`.
    let mut large = b"!X.java\n".to_vec();
    for index in 0..4_000 {
        large.extend_from_slice(format!("a?{index}{}z\n", "?".repeat(35)).as_bytes());
    }
    add(
        vec![(GITIGNORE, b"*.java\n".to_vec()), ("p/.gitignore", large)],
        &["p/X.java", "p/Y.java"],
    );
    cases
}

#[test]
fn test_nsd_agrees_with_git_check_ignore_on_every_gate_finding() {
    let mut disagreements = Vec::new();
    let mut compared = 0usize;
    let mut git_ignored_count = 0usize;
    let mut git_kept_count = 0usize;
    for case in gate_cases() {
        let paths: Vec<String> = case.paths.iter().map(|path| path.to_string()).collect();
        let rules: Vec<(String, Vec<u8>)> = case
            .rules
            .iter()
            .map(|(path, bytes)| (path.to_string(), bytes.clone()))
            .collect();
        let mut arena = Arena::new(&paths);
        for verdict in arena.compare(&rules) {
            compared += 1;
            if verdict.git_ignores {
                git_ignored_count += 1;
            } else {
                git_kept_count += 1;
            }
            if verdict.nsd_skips != verdict.git_ignores {
                let shown: Vec<String> = rules
                    .iter()
                    .map(|(path, bytes)| {
                        let text = String::from_utf8_lossy(bytes);
                        let head: String = text.chars().take(80).collect();
                        format!("{path}: {head:?}")
                    })
                    .collect();
                disagreements.push(format!(
                    "{} path {:?}: nsd skips {}, git ignores {}",
                    shown.join("; "),
                    verdict.path,
                    verdict.nsd_skips,
                    verdict.git_ignores
                ));
            }
        }
    }
    assert!(compared > 0 && git_ignored_count > 0 && git_kept_count > 0);
    assert!(
        disagreements.is_empty(),
        "{} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}
