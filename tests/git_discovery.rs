//! Git-backed discovery with immutable exclusions (WS-4).

mod common;

use nsd::config::Config;
use nsd::git::discovery::{discover, DiscoveryResult, SkipReason};
use nsd::git::path::RepoPath;
use nsd::git::snapshot::{
    CommitSnapshot, Entry, EntryKind, WorktreeSnapshot, SOURCE_CEILING_BYTES,
};
use nsd::model::{LanguageFamily, ScanSettings};

/// This suite's own fixture constants (D22): Git file modes used only
/// here.
const MODE_REGULAR: i32 = 0o100644;
const MODE_SYMLINK: i32 = 0o120000;
const MODE_SUBMODULE: i32 = 0o160000;

fn default_scope() -> nsd::config::CompiledScope {
    Config::default()
        .compiled_scope()
        .expect("the default config compiles")
}

fn scope_from_yaml(yaml: &[u8]) -> nsd::config::CompiledScope {
    Config::parse(yaml)
        .expect("a valid nsd.yml")
        .compiled_scope()
        .expect("the config compiles")
}

fn skip_reason_for(result: &DiscoveryResult, path: &str) -> Option<SkipReason> {
    result
        .skipped
        .iter()
        .find(|entry| entry.path.render() == path)
        .map(|entry| entry.reason)
}

fn is_included(result: &DiscoveryResult, path: &str) -> bool {
    result
        .included
        .iter()
        .any(|entry| entry.path.render() == path)
}

fn language_of(result: &DiscoveryResult, path: &str) -> Option<LanguageFamily> {
    result
        .included
        .iter()
        .find(|entry| entry.path.render() == path)
        .map(|entry| entry.language)
}

#[test]
fn candidate_gitignore_has_no_effect_on_tree_discovery() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (b".gitignore".to_vec(), MODE_REGULAR, b"src/\n".to_vec()),
            (b"src/a.ts".to_vec(), MODE_REGULAR, b"export {};\n".to_vec()),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    assert!(
        is_included(&result, "src/a.ts"),
        "a tracked file must be included even though the candidate .gitignore lists its directory"
    );
    assert_eq!(skip_reason_for(&result, "src/a.ts"), None);
}

#[test]
fn worktree_untracked_eligible_regardless_of_gitignore() {
    let (dir, repo) = common::init_repo();
    std::fs::write(dir.path().join(".gitignore"), b"ignored/\n").expect("write .gitignore");
    std::fs::create_dir_all(dir.path().join("ignored")).expect("create ignored dir");
    std::fs::write(dir.path().join("ignored/kept.ts"), b"export {};\n").expect("write kept.ts");

    let snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let result = discover(&snapshot.entries, &default_scope());

    assert!(
        is_included(&result, "ignored/kept.ts"),
        "an untracked file the worktree's own .gitignore matches must still be eligible (D6)"
    );
}

#[test]
fn builtin_exclusions_cannot_be_reincluded() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (
                b"node_modules/x.js".to_vec(),
                MODE_REGULAR,
                b"module.exports = {};\n".to_vec(),
            ),
            (
                b"dist/y.js".to_vec(),
                MODE_REGULAR,
                b"module.exports = {};\n".to_vec(),
            ),
            (
                b"lib/app.min.js".to_vec(),
                MODE_REGULAR,
                b"(function(){})();\n".to_vec(),
            ),
            (
                b"src/generated/G.java".to_vec(),
                MODE_REGULAR,
                b"class G {}\n".to_vec(),
            ),
            (
                b"web/META-INF/resources/webjars/jq.js".to_vec(),
                MODE_REGULAR,
                b"// jq\n".to_vec(),
            ),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let scope = scope_from_yaml(
        b"version: 1\ninclude: [\"node_modules/**\", \"dist/**\", \"lib/**\", \"src/**\", \"web/**\"]\n",
    );
    let result = discover(&snapshot.entries, &scope);

    assert_eq!(
        skip_reason_for(&result, "node_modules/x.js"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert_eq!(
        skip_reason_for(&result, "dist/y.js"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert_eq!(
        skip_reason_for(&result, "lib/app.min.js"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert_eq!(
        skip_reason_for(&result, "src/generated/G.java"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert_eq!(
        skip_reason_for(&result, "web/META-INF/resources/webjars/jq.js"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert!(result.included.is_empty());
}

#[test]
fn webjar_and_minified_excluded() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (
                b"lib/app.min.js".to_vec(),
                MODE_REGULAR,
                b"(function(){})();\n".to_vec(),
            ),
            (
                b"web/META-INF/resources/webjars/jq.js".to_vec(),
                MODE_REGULAR,
                b"// jq\n".to_vec(),
            ),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    assert_eq!(
        skip_reason_for(&result, "lib/app.min.js"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert_eq!(
        skip_reason_for(&result, "web/META-INF/resources/webjars/jq.js"),
        Some(SkipReason::BuiltinExclusion)
    );
}

#[test]
fn no_default_test_exclusion() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (
                b"src/FooTest.java".to_vec(),
                MODE_REGULAR,
                b"class FooTest {}\n".to_vec(),
            ),
            (
                b"src/a.test.ts".to_vec(),
                MODE_REGULAR,
                b"export {};\n".to_vec(),
            ),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    assert!(
        is_included(&result, "src/FooTest.java"),
        "check has no default test exclusion"
    );
    assert!(is_included(&result, "src/a.test.ts"));
}

#[test]
fn include_narrows_then_exclude_subtracts() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (b"lib/a.ts".to_vec(), MODE_REGULAR, b"export {};\n".to_vec()),
            (
                b"src/legacy/b.ts".to_vec(),
                MODE_REGULAR,
                b"export {};\n".to_vec(),
            ),
            (
                b"src/keep.ts".to_vec(),
                MODE_REGULAR,
                b"export {};\n".to_vec(),
            ),
            (
                b"lib/old/c.ts".to_vec(),
                MODE_REGULAR,
                b"export {};\n".to_vec(),
            ),
            (
                b"node_modules/z.js".to_vec(),
                MODE_REGULAR,
                b"module.exports = {};\n".to_vec(),
            ),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let scope = scope_from_yaml(
        b"version: 1\ninclude: [\"src/**\"]\nexclude: [\"src/legacy/**\", \"lib/old/**\", \"node_modules/**\"]\n",
    );
    let result = discover(&snapshot.entries, &scope);

    assert_eq!(
        skip_reason_for(&result, "lib/a.ts"),
        Some(SkipReason::OutsideInclude)
    );
    assert_eq!(
        skip_reason_for(&result, "src/legacy/b.ts"),
        Some(SkipReason::ConfigExclude)
    );
    assert!(is_included(&result, "src/keep.ts"));
    assert_eq!(
        skip_reason_for(&result, "lib/old/c.ts"),
        Some(SkipReason::OutsideInclude),
        "outside_include must win over config_exclude when a path matches both (D20)"
    );
    assert_eq!(
        skip_reason_for(&result, "node_modules/z.js"),
        Some(SkipReason::BuiltinExclusion),
        "builtin_exclusion must win over both outside_include and config_exclude (D20)"
    );
}

#[test]
fn config_exclude_directory_pattern_covers_nested_files() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (
                b"src/legacy/deep/b.ts".to_vec(),
                MODE_REGULAR,
                b"export {};\n".to_vec(),
            ),
            (
                b"src/keep.ts".to_vec(),
                MODE_REGULAR,
                b"export {};\n".to_vec(),
            ),
            (
                b"src/zz.ts".to_vec(),
                MODE_REGULAR,
                b"export {};\n".to_vec(),
            ),
            (
                b"pkg/node_modules/x/y.js".to_vec(),
                MODE_REGULAR,
                b"export {};\n".to_vec(),
            ),
            (b"pkg/z.ts".to_vec(), MODE_REGULAR, b"export {};\n".to_vec()),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let scope = scope_from_yaml(b"version: 1\nexclude: [\"src/legacy/\"]\n");
    let result = discover(&snapshot.entries, &scope);

    assert_eq!(
        skip_reason_for(&result, "src/legacy/deep/b.ts"),
        Some(SkipReason::ConfigExclude),
        "a directory-anchored exclude pattern must cover files nested beneath it (D19)"
    );
    assert!(is_included(&result, "src/keep.ts"));
    assert!(
        is_included(&result, "src/zz.ts"),
        "a sibling under the same parent as a matched exclude directory must not \
         inherit that directory's memoized verdict"
    );
    assert_eq!(
        skip_reason_for(&result, "pkg/node_modules/x/y.js"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert!(
        is_included(&result, "pkg/z.ts"),
        "a sibling under the same parent as a matched built-in-exclusion directory \
         must not inherit that directory's memoized verdict"
    );
}

#[test]
fn exactly_one_mib_accepted_one_byte_over_flagged() {
    let (_dir, repo) = common::init_repo();
    let at_ceiling = vec![b'x'; SOURCE_CEILING_BYTES as usize];
    let mut over_ceiling = at_ceiling.clone();
    over_ceiling.push(b'x');
    common::commit_entries(
        &repo,
        &[
            (b"src/at.ts".to_vec(), MODE_REGULAR, at_ceiling),
            (b"src/over.ts".to_vec(), MODE_REGULAR, over_ceiling),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    let at = result
        .included
        .iter()
        .find(|entry| entry.path.render() == "src/at.ts")
        .expect("src/at.ts is included");
    assert_eq!(at.size, SOURCE_CEILING_BYTES);
    assert!(!at.too_large, "exactly the ceiling must not be too_large");

    let over = result
        .included
        .iter()
        .find(|entry| entry.path.render() == "src/over.ts")
        .expect("src/over.ts is included");
    assert_eq!(over.size, SOURCE_CEILING_BYTES + 1);
    assert!(
        over.too_large,
        "one byte over the ceiling must be too_large"
    );
}

#[test]
fn symlink_and_submodule_skip_reasons() {
    let (_dir, repo) = common::init_repo();
    let gitlink_target: Vec<u8> = (1..=20).collect();
    common::commit_entries(
        &repo,
        &[
            (
                b"src/link.ts".to_vec(),
                MODE_SYMLINK,
                b"./other.ts".to_vec(),
            ),
            (b"third_party/lib".to_vec(), MODE_SUBMODULE, gitlink_target),
            (
                b"node_modules/l.ts".to_vec(),
                MODE_SYMLINK,
                b"./other.ts".to_vec(),
            ),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    assert_eq!(
        skip_reason_for(&result, "src/link.ts"),
        Some(SkipReason::Symlink)
    );
    assert_eq!(
        skip_reason_for(&result, "third_party/lib"),
        Some(SkipReason::Submodule)
    );
    assert_eq!(
        skip_reason_for(&result, "node_modules/l.ts"),
        Some(SkipReason::Symlink),
        "entry-kind symlink must win over builtin_exclusion (D20)"
    );
}

#[test]
fn nested_git_checkout_excluded_when_not_gitignored() {
    let (dir, repo) = common::init_repo();
    std::fs::create_dir_all(dir.path().join("sub")).expect("create sub dir");
    std::fs::write(
        dir.path().join("sub/.git"),
        b"gitdir: ../.git/modules/sub\n",
    )
    .expect("write nested .git marker");
    std::fs::write(dir.path().join("sub/file.ts"), b"export {};\n").expect("write sub/file.ts");

    let snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let result = discover(&snapshot.entries, &default_scope());

    assert_eq!(
        skip_reason_for(&result, "sub"),
        Some(SkipReason::NestedCheckout)
    );
    assert_eq!(
        skip_reason_for(&result, "sub/file.ts"),
        None,
        "a nested checkout's files must never be listed, not even as skipped"
    );
    assert!(!is_included(&result, "sub/file.ts"));
}

#[test]
fn non_utf8_path_included_flagged_and_escaped() {
    let (_dir, repo) = common::init_repo();
    let mut path_bytes = b"src/a".to_vec();
    path_bytes.push(0xFF);
    path_bytes.extend_from_slice(b".ts");
    common::commit_entries(
        &repo,
        &[(path_bytes.clone(), MODE_REGULAR, b"export {};\n".to_vec())],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    let found = result
        .included
        .iter()
        .find(|entry| entry.path.as_bytes() == path_bytes.as_slice())
        .expect("the non-UTF-8 path is included");
    assert!(found.non_utf8_path);
    assert_eq!(found.language, LanguageFamily::JsTs);
    assert_eq!(found.path.render(), "src/a%FF.ts");
}

#[test]
fn unsupported_extensions_not_listed() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (b"src/image.png".to_vec(), MODE_REGULAR, b"\x89PNG".to_vec()),
            (b"README.md".to_vec(), MODE_REGULAR, b"# hi\n".to_vec()),
            (b"src/a.ts".to_vec(), MODE_REGULAR, b"export {};\n".to_vec()),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    assert_eq!(skip_reason_for(&result, "src/image.png"), None);
    assert!(!is_included(&result, "src/image.png"));
    assert_eq!(skip_reason_for(&result, "README.md"), None);
    assert!(!is_included(&result, "README.md"));
    assert!(is_included(&result, "src/a.ts"));
}

#[test]
fn git_path_component_is_builtin_exclusion() {
    let make = |path: &[u8]| Entry {
        path: RepoPath::from_bytes(path.to_vec()),
        kind: EntryKind::Regular,
        oid: None,
        size: 0,
    };
    let entries = vec![
        make(b"pkg/.git/hooks/pre.ts"),
        make(b".git/x.ts"),
        make(b"pkg/.github/a.ts"),
        make(b"src/.gitkeep.ts"),
    ];
    let scope = scope_from_yaml(b"version: 1\ninclude: [\"**\"]\n");
    let result = discover(&entries, &scope);

    assert_eq!(
        skip_reason_for(&result, "pkg/.git/hooks/pre.ts"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert_eq!(
        skip_reason_for(&result, ".git/x.ts"),
        Some(SkipReason::BuiltinExclusion)
    );
    assert!(
        is_included(&result, "pkg/.github/a.ts"),
        "a `.github` component must not be caught by a `.git`-component substring match"
    );
    assert!(
        is_included(&result, "src/.gitkeep.ts"),
        "a `.gitkeep.ts` file name must not be caught by a `.git`-component substring match"
    );
}

fn fixture_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/discover/sample")
}

/// Recursively walks the scan-discovery fixture, collecting every regular
/// file as a D22-style fixture entry so it can be committed through git2
/// and re-classified by Git-backed discovery (this suite's own helper,
/// used only here).
fn walk_fixture(dir: &std::path::Path, prefix: &[u8], out: &mut Vec<(Vec<u8>, i32, Vec<u8>)>) {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .expect("read fixture directory")
        .map(|entry| entry.expect("read fixture directory entry"))
        .collect();
    names.sort_by_key(|entry| entry.file_name());
    for entry in names {
        let name = entry.file_name();
        let name_bytes = name.to_string_lossy().into_owned().into_bytes();
        let mut child_bytes = prefix.to_vec();
        if !child_bytes.is_empty() {
            child_bytes.push(b'/');
        }
        child_bytes.extend_from_slice(&name_bytes);

        let file_type = entry.file_type().expect("fixture entry file type");
        if file_type.is_dir() {
            walk_fixture(&entry.path(), &child_bytes, out);
        } else {
            let content = std::fs::read(entry.path()).expect("read fixture file");
            out.push((child_bytes, MODE_REGULAR, content));
        }
    }
}

#[test]
fn builtin_list_covers_scan_dependency_and_generated_globs() {
    let (_dir, repo) = common::init_repo();
    let mut fixture_entries = Vec::new();
    walk_fixture(&fixture_root(), &[], &mut fixture_entries);
    common::commit_entries(&repo, &fixture_entries);
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    let scan_settings = ScanSettings {
        output: std::path::PathBuf::from("unused"),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: 10,
    };
    let scan_result =
        nsd::discover::discover(&fixture_root(), &scan_settings).expect("scan discovery");

    let mut checked = 0;
    for skipped in &scan_result.skipped {
        let reason = skipped.reason;
        if !matches!(
            reason,
            nsd::model::SkipReason::DependencyOrBuildOutput | nsd::model::SkipReason::GeneratedCode
        ) {
            continue;
        }
        let path_str = skipped.relative_path.to_string_lossy().replace('\\', "/");
        assert_eq!(
            skip_reason_for(&result, &path_str),
            Some(SkipReason::BuiltinExclusion),
            "{path_str} is a dependency/build or generated skip in scan discovery, so it must \
             also be a built-in exclusion in Git-backed discovery"
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "the mirrored fixture must exercise at least one dependency/build or generated skip"
    );

    // Second pass (triage row 2): the sample fixture above only happens to
    // reach 8 of the 18 D19 built-in globs. Write one file per glob into a
    // fresh temp directory so a future drift in any of the other 10 (or a
    // regression in these 8) cannot pass unnoticed.
    let one_file_per_glob: Vec<(Vec<u8>, i32, Vec<u8>)> = vec![
        (
            b"node_modules/a.js".to_vec(),
            MODE_REGULAR,
            b"//\n".to_vec(),
        ),
        (b"target/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b"build/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b"dist/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b"out/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b"bin/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (
            b".gradle/A.java".to_vec(),
            MODE_REGULAR,
            b"class A {}\n".to_vec(),
        ),
        (
            b".mvn/A.java".to_vec(),
            MODE_REGULAR,
            b"class A {}\n".to_vec(),
        ),
        (b"vendor/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b"coverage/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b".next/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b".nuxt/a.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (
            b"src/generated/G.java".to_vec(),
            MODE_REGULAR,
            b"class G {}\n".to_vec(),
        ),
        (
            b"src/gen/a.ts".to_vec(),
            MODE_REGULAR,
            b"export {};\n".to_vec(),
        ),
        (b"src/app.min.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b"src/a.bundle.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (b"src/proto_pb.js".to_vec(), MODE_REGULAR, b"//\n".to_vec()),
        (
            b"src/types.d.ts".to_vec(),
            MODE_REGULAR,
            b"export {};\n".to_vec(),
        ),
    ];
    let all_dir = tempfile::TempDir::new().expect("create a temp dir for the glob fixture");
    for (path, _mode, content) in &one_file_per_glob {
        let fs_path = all_dir
            .path()
            .join(std::str::from_utf8(path).expect("fixture path is UTF-8"));
        std::fs::create_dir_all(fs_path.parent().expect("fixture path has a parent"))
            .expect("create fixture parent dirs");
        std::fs::write(&fs_path, content).expect("write glob fixture file");
    }
    let (_glob_repo_dir, glob_repo) = common::init_repo();
    common::commit_entries(&glob_repo, &one_file_per_glob);
    let glob_snapshot = CommitSnapshot::head_or_empty(&glob_repo).expect("snapshot HEAD");
    let glob_result = discover(&glob_snapshot.entries, &default_scope());

    let glob_scan_result =
        nsd::discover::discover(all_dir.path(), &scan_settings).expect("scan discovery");

    let mut all_checked = 0;
    for skipped in &glob_scan_result.skipped {
        let reason = skipped.reason;
        if !matches!(
            reason,
            nsd::model::SkipReason::DependencyOrBuildOutput | nsd::model::SkipReason::GeneratedCode
        ) {
            continue;
        }
        let path_str = skipped.relative_path.to_string_lossy().replace('\\', "/");
        assert_eq!(
            skip_reason_for(&glob_result, &path_str),
            Some(SkipReason::BuiltinExclusion),
            "{path_str} is a dependency/build or generated skip in scan discovery, so it must \
             also be a built-in exclusion in Git-backed discovery"
        );
        all_checked += 1;
    }
    assert_eq!(
        all_checked,
        one_file_per_glob.len(),
        "every one-file-per-glob fixture entry must be a scan dependency/build or generated skip"
    );
}

#[test]
fn skip_reason_label_spells_all_six() {
    assert_eq!(SkipReason::Symlink.label(), "symlink");
    assert_eq!(SkipReason::Submodule.label(), "submodule");
    assert_eq!(SkipReason::NestedCheckout.label(), "nested_checkout");
    assert_eq!(SkipReason::BuiltinExclusion.label(), "builtin_exclusion");
    assert_eq!(SkipReason::OutsideInclude.label(), "outside_include");
    assert_eq!(SkipReason::ConfigExclude.label(), "config_exclude");
}

#[test]
fn output_sorted_by_raw_path_bytes() {
    let make = |path: &[u8]| Entry {
        path: RepoPath::from_bytes(path.to_vec()),
        kind: EntryKind::Regular,
        oid: None,
        size: 0,
    };
    let entries = vec![
        make(b"src/z.ts"),
        make(b"src/a.ts"),
        make(b"node_modules/b.js"),
        make(b"node_modules/a.js"),
    ];
    let result = discover(&entries, &default_scope());

    let included_paths: Vec<String> = result
        .included
        .iter()
        .map(|entry| entry.path.render())
        .collect();
    assert_eq!(included_paths, vec!["src/a.ts", "src/z.ts"]);

    let skipped_paths: Vec<String> = result
        .skipped
        .iter()
        .map(|entry| entry.path.render())
        .collect();
    assert_eq!(
        skipped_paths,
        vec!["node_modules/a.js", "node_modules/b.js"]
    );
}

// Keeps `language_of` exercised (avoids an unused-function warning while
// still asserting a genuine outcome, not mere presence).
#[test]
fn included_entry_carries_the_correct_language() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (b"src/a.ts".to_vec(), MODE_REGULAR, b"export {};\n".to_vec()),
            (
                b"src/Main.java".to_vec(),
                MODE_REGULAR,
                b"class Main {}\n".to_vec(),
            ),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let result = discover(&snapshot.entries, &default_scope());

    assert_eq!(language_of(&result, "src/a.ts"), Some(LanguageFamily::JsTs));
    assert_eq!(
        language_of(&result, "src/Main.java"),
        Some(LanguageFamily::Java)
    );
}
