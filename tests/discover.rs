use std::path::PathBuf;

use nsd::discover::discover;
use nsd::model::{LanguageFamily, ScanSettings, SkipReason};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/discover/sample")
}

fn settings(include_tests: bool, exclude: &[&str]) -> ScanSettings {
    ScanSettings {
        output: PathBuf::from("unused"),
        include_tests,
        exclude: exclude.iter().map(|glob| glob.to_string()).collect(),
        min_clone_lines: 10,
    }
}

fn discovered_paths(result: &nsd::discover::DiscoverResult) -> Vec<String> {
    let mut paths: Vec<String> = result
        .discovered
        .iter()
        .map(|file| file.relative_path.to_string_lossy().replace('\\', "/"))
        .collect();
    paths.sort();
    paths
}

fn reason_for(result: &nsd::discover::DiscoverResult, relative_path: &str) -> Option<SkipReason> {
    result
        .skipped
        .iter()
        .find(|file| file.relative_path.to_string_lossy().replace('\\', "/") == relative_path)
        .map(|file| file.reason)
}

#[test]
fn test_discover_default_exclusions() -> anyhow::Result<()> {
    let result = discover(&fixture_root(), &settings(false, &[]))?;

    assert_eq!(
        discovered_paths(&result),
        vec![
            ".config/app.js",
            "src/View.tsx",
            "src/api/svc.ts",
            "src/legacy.cjs",
            "src/main/java/App.java",
            "src/mod.mjs",
            "src/util.js",
            "src/widget.jsx",
        ]
    );

    assert_eq!(
        reason_for(&result, "node_modules/left-pad/index.js"),
        Some(SkipReason::DependencyOrBuildOutput)
    );
    assert_eq!(
        reason_for(&result, "target/classes/Cached.java"),
        Some(SkipReason::DependencyOrBuildOutput)
    );
    assert_eq!(
        reason_for(&result, "dist/bundle.js"),
        Some(SkipReason::DependencyOrBuildOutput)
    );
    assert_eq!(
        reason_for(&result, ".gradle/Cache.java"),
        Some(SkipReason::DependencyOrBuildOutput)
    );
    assert_eq!(
        reason_for(&result, "build/generated/Api_pb.js"),
        Some(SkipReason::GeneratedCode)
    );
    assert_eq!(
        reason_for(&result, "src/vendor.min.js"),
        Some(SkipReason::GeneratedCode)
    );
    assert_eq!(
        reason_for(&result, "src/types.d.ts"),
        Some(SkipReason::GeneratedCode)
    );
    assert_eq!(
        reason_for(&result, "src/test/java/AppTest.java"),
        Some(SkipReason::Test)
    );
    assert_eq!(
        reason_for(&result, "src/util.test.js"),
        Some(SkipReason::Test)
    );
    assert_eq!(
        reason_for(&result, "src/__tests__/helper.spec.ts"),
        Some(SkipReason::Test)
    );

    // 8 discovered + 4 dependency/build + 3 generated + 1 gitignore + 3 test.
    assert_eq!(result.skipped.len(), 11);
    Ok(())
}

#[test]
fn test_discover_respects_gitignore() -> anyhow::Result<()> {
    let result = discover(&fixture_root(), &settings(false, &[]))?;

    assert_eq!(
        reason_for(&result, "src/ignored.ts"),
        Some(SkipReason::Gitignore)
    );
    assert!(
        !discovered_paths(&result).contains(&"src/ignored.ts".to_string()),
        "src/ignored.ts must not be discovered"
    );
    Ok(())
}

#[test]
fn test_include_tests_flag_adds_test_files() -> anyhow::Result<()> {
    let result = discover(&fixture_root(), &settings(true, &[]))?;

    let discovered = discovered_paths(&result);
    assert_eq!(discovered.len(), 11);
    for test_file in [
        "src/test/java/AppTest.java",
        "src/util.test.js",
        "src/__tests__/helper.spec.ts",
    ] {
        assert!(
            discovered.contains(&test_file.to_string()),
            "{test_file} should be discovered with --include-tests"
        );
        assert_eq!(reason_for(&result, test_file), None);
    }
    Ok(())
}

#[test]
fn test_repeatable_exclude_globs() -> anyhow::Result<()> {
    let result = discover(&fixture_root(), &settings(false, &["src/**/*.ts", "*.jsx"]))?;

    let discovered = discovered_paths(&result);
    assert_eq!(discovered.len(), 6);
    assert!(!discovered.contains(&"src/api/svc.ts".to_string()));
    assert!(!discovered.contains(&"src/widget.jsx".to_string()));
    assert_eq!(
        reason_for(&result, "src/api/svc.ts"),
        Some(SkipReason::UserExclude)
    );
    assert_eq!(
        reason_for(&result, "src/widget.jsx"),
        Some(SkipReason::UserExclude)
    );
    Ok(())
}

#[test]
fn test_exclude_glob_excludes_rather_than_whitelists() -> anyhow::Result<()> {
    let result = discover(&fixture_root(), &settings(false, &["*.jsx"]))?;

    let discovered = discovered_paths(&result);
    assert_eq!(
        discovered,
        vec![
            ".config/app.js",
            "src/View.tsx",
            "src/api/svc.ts",
            "src/legacy.cjs",
            "src/main/java/App.java",
            "src/mod.mjs",
            "src/util.js",
        ]
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_unreadable_subdirectory_is_skipped_not_fatal() -> anyhow::Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir()?;
    fs::write(root.path().join("readable.js"), "module.exports = {};\n")?;
    let locked = root.path().join("locked");
    fs::create_dir(&locked)?;
    fs::write(locked.join("secret.js"), "module.exports = {};\n")?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;

    let outcome = discover(root.path(), &settings(false, &[]));

    // Restore the mode before propagating any error or dropping the
    // tempdir, so a failing assertion below still leaves a cleanable tree.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))?;

    let result = outcome?;
    assert!(
        discovered_paths(&result).contains(&"readable.js".to_string()),
        "readable.js should still be discovered when a sibling directory is unreadable"
    );
    assert_eq!(reason_for(&result, "locked"), Some(SkipReason::Unreadable));
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_unreadable_target_root_is_fatal() -> anyhow::Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir()?;
    fs::write(root.path().join("readable.js"), "module.exports = {};\n")?;
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o000))?;

    let outcome = discover(root.path(), &settings(false, &[]));

    // Restore the mode before propagating any error or dropping the
    // tempdir, so a failing assertion below still leaves a cleanable tree.
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755))?;

    assert!(
        outcome.is_err(),
        "an unreadable scan root must be a fatal error, not a successful empty scan"
    );
    Ok(())
}

#[test]
fn test_walk_prunes_git_and_excluded_directories() -> anyhow::Result<()> {
    use std::fs;

    let root = tempfile::tempdir()?;
    fs::write(root.path().join("keep.js"), "module.exports = {};\n")?;
    fs::write(root.path().join(".gitignore"), "node_modules/\n")?;
    let node_modules_pkg = root.path().join("node_modules").join("pkg");
    fs::create_dir_all(&node_modules_pkg)?;
    fs::write(node_modules_pkg.join("index.js"), "module.exports = {};\n")?;
    let git_hooks = root.path().join(".git").join("hooks");
    fs::create_dir_all(&git_hooks)?;
    fs::write(git_hooks.join("tool.js"), "module.exports = {};\n")?;

    let result = discover(root.path(), &settings(false, &[]))?;

    let discovered = discovered_paths(&result);
    assert!(
        discovered.contains(&"keep.js".to_string()),
        "keep.js should still be discovered"
    );
    assert!(
        !discovered.contains(&"node_modules/pkg/index.js".to_string()),
        "node_modules/pkg/index.js must not be discovered"
    );
    assert_ne!(
        reason_for(&result, "node_modules/pkg/index.js"),
        Some(SkipReason::Gitignore),
        "node_modules/pkg/index.js is excluded by D16, not by .gitignore"
    );
    assert!(
        !discovered.contains(&".git/hooks/tool.js".to_string()),
        ".git/hooks/tool.js must not be discovered"
    );
    assert_eq!(
        reason_for(&result, ".git/hooks/tool.js"),
        None,
        ".git/hooks/tool.js must not be skipped either — it must never be walked at all"
    );
    Ok(())
}

#[test]
fn test_extension_to_language_family() {
    assert_eq!(
        LanguageFamily::from_extension("java"),
        Some(LanguageFamily::Java)
    );
    for extension in ["js", "jsx", "mjs", "cjs", "ts", "tsx"] {
        assert_eq!(
            LanguageFamily::from_extension(extension),
            Some(LanguageFamily::JsTs),
            "{extension} should map to js_ts"
        );
    }
    assert_eq!(LanguageFamily::from_extension("py"), None);
    assert_eq!(LanguageFamily::from_extension("kt"), None);
}

/// Writes `files` (relative path, contents) under a fresh tempdir.
fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (relative, contents) in files {
        let path = dir.path().join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }
    dir
}

fn nested_rows(result: &nsd::discover::DiscoverResult) -> Vec<String> {
    result
        .skipped
        .iter()
        .filter(|file| file.reason == SkipReason::NestedCheckout)
        .map(|file| file.relative_path.to_string_lossy().replace('\\', "/"))
        .collect()
}

#[test]
fn test_nested_checkout_with_a_git_file_is_excluded_when_not_gitignored() -> anyhow::Result<()> {
    let dir = tree(&[
        ("A.java", "class A {}\n"),
        ("vendored/lib/.git", "gitdir: ../../.git/modules/lib\n"),
        ("vendored/lib/Inner.java", "class Inner {}\n"),
    ]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
    assert_eq!(nested_rows(&result), vec!["vendored/lib"]);
    Ok(())
}

#[test]
fn test_nested_checkout_with_a_git_directory_is_excluded() -> anyhow::Result<()> {
    // Depth three, and a `.git` directory rather than a file.
    let dir = tree(&[
        ("A.java", "class A {}\n"),
        ("a/b/c/.git/HEAD", "ref: refs/heads/main\n"),
        ("a/b/c/Inner.java", "class Inner {}\n"),
        ("a/b/c/deeper/Deep.java", "class Deep {}\n"),
    ]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
    assert_eq!(nested_rows(&result), vec!["a/b/c"]);
    Ok(())
}

#[cfg(unix)]
#[test]
fn test_nested_checkout_with_a_dangling_git_symlink_is_excluded() -> anyhow::Result<()> {
    let dir = tree(&[
        ("A.java", "class A {}\n"),
        ("vendored/lib/Inner.java", "class Inner {}\n"),
    ]);
    std::os::unix::fs::symlink("missing-target", dir.path().join("vendored/lib/.git"))?;
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert_eq!(nested_rows(&result), vec!["vendored/lib"]);
    Ok(())
}

#[test]
fn test_gitignored_nested_checkout_keeps_its_gitignore_rows() -> anyhow::Result<()> {
    let dir = tree(&[
        ("A.java", "class A {}\n"),
        (".gitignore", "vendored/\n"),
        ("vendored/lib/.git", "gitdir: elsewhere\n"),
        ("vendored/lib/Inner.java", "class Inner {}\n"),
    ]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert_eq!(nested_rows(&result), Vec::<String>::new());
    assert_eq!(
        reason_for(&result, "vendored/lib/Inner.java"),
        Some(SkipReason::Gitignore)
    );
    Ok(())
}

#[test]
fn test_scan_root_git_is_not_a_nested_checkout() -> anyhow::Result<()> {
    let dir = tree(&[
        ("A.java", "class A {}\n"),
        (".git/HEAD", "ref: refs/heads/main\n"),
    ]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert!(result.skipped.is_empty(), "{:?}", result.skipped);
    Ok(())
}

#[test]
fn test_nested_checkout_under_an_excluded_directory_keeps_its_existing_reason() -> anyhow::Result<()>
{
    let dir = tree(&[
        ("A.java", "class A {}\n"),
        ("node_modules/x/.git", "gitdir: elsewhere\n"),
        ("node_modules/x/dep.js", "const d = 1;\n"),
        ("custom/y/.git", "gitdir: elsewhere\n"),
        ("custom/y/Y.java", "class Y {}\n"),
    ]);
    let result = discover(dir.path(), &settings(false, &["custom/"]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert_eq!(nested_rows(&result), Vec::<String>::new());
    assert_eq!(
        reason_for(&result, "node_modules/x/dep.js"),
        Some(SkipReason::DependencyOrBuildOutput)
    );
    assert_eq!(
        reason_for(&result, "custom/y/Y.java"),
        Some(SkipReason::UserExclude)
    );
    Ok(())
}

#[test]
fn test_nested_checkout_in_a_test_glob_directory_keeps_its_test_rows() -> anyhow::Result<()> {
    let dir = tree(&[
        ("A.java", "class A {}\n"),
        ("fixtures/.git", "gitdir: elsewhere\n"),
        ("fixtures/F.java", "class F {}\n"),
    ]);

    let by_default = discover(dir.path(), &settings(false, &[]))?;
    assert_eq!(discovered_paths(&by_default), vec!["A.java"]);
    assert_eq!(
        reason_for(&by_default, "fixtures/F.java"),
        Some(SkipReason::Test)
    );
    assert_eq!(nested_rows(&by_default), Vec::<String>::new());

    let with_tests = discover(dir.path(), &settings(true, &[]))?;
    assert_eq!(discovered_paths(&with_tests), vec!["A.java"]);
    assert_eq!(nested_rows(&with_tests), vec!["fixtures"]);
    assert_eq!(reason_for(&with_tests, "fixtures/F.java"), None);
    Ok(())
}

const D15_TREE: &[(&str, &str)] = &[
    ("A.java", "class A {}\n"),
    ("web/META-INF/resources/webjars/jq.js", "const j = 1;\n"),
    ("src/fixtures/F.java", "class F {}\n"),
    ("fixture/G.java", "class G {}\n"),
    ("__fixtures__/f.js", "const f = 1;\n"),
    ("e2e/app.ts", "const a = 1;\n"),
    ("qa/Q.java", "class Q {}\n"),
    ("src/app.e2e-spec.ts", "const s = 1;\n"),
];

const D15_TEST_PATHS: [&str; 6] = [
    "src/fixtures/F.java",
    "fixture/G.java",
    "__fixtures__/f.js",
    "e2e/app.ts",
    "qa/Q.java",
    "src/app.e2e-spec.ts",
];

const WEBJAR_FILE: &str = "web/META-INF/resources/webjars/jq.js";

#[test]
fn test_webjar_paths_are_excluded_even_with_include_tests() -> anyhow::Result<()> {
    let dir = tree(D15_TREE);
    for include_tests in [false, true] {
        let result = discover(dir.path(), &settings(include_tests, &[]))?;
        assert_eq!(
            reason_for(&result, WEBJAR_FILE),
            Some(SkipReason::DependencyOrBuildOutput),
            "include_tests={include_tests}"
        );
    }
    Ok(())
}

#[test]
fn test_fixture_e2e_and_qa_paths_are_excluded_as_tests() -> anyhow::Result<()> {
    let dir = tree(D15_TREE);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    for path in D15_TEST_PATHS {
        assert_eq!(reason_for(&result, path), Some(SkipReason::Test), "{path}");
    }
    Ok(())
}

#[test]
fn test_include_tests_discovers_fixture_e2e_and_qa_paths() -> anyhow::Result<()> {
    let dir = tree(D15_TREE);
    let result = discover(dir.path(), &settings(true, &[]))?;

    let mut expected: Vec<&str> = D15_TEST_PATHS.to_vec();
    expected.push("A.java");
    expected.sort();
    assert_eq!(discovered_paths(&result), expected);
    Ok(())
}

fn checkout_tree(extra: &[(&'static str, &'static str)]) -> tempfile::TempDir {
    let mut files = vec![
        ("A.java", "class A {}\n"),
        ("lib/.git", "gitdir: elsewhere\n"),
    ];
    files.extend_from_slice(extra);
    tree(&files)
}

#[test]
fn test_filename_exclusion_does_not_open_a_nested_checkout() -> anyhow::Result<()> {
    let dir = checkout_tree(&[("lib/Inner.java", "class Inner {}\n")]);
    for glob in ["**/x", "x"] {
        let result = discover(dir.path(), &settings(false, &[glob]))?;
        assert_eq!(discovered_paths(&result), vec!["A.java"], "{glob}");
        assert_eq!(nested_rows(&result), vec!["lib"], "{glob}");
        assert_eq!(reason_for(&result, "lib/Inner.java"), None, "{glob}");
    }
    Ok(())
}

#[test]
fn test_nested_checkout_whose_files_are_all_gitignored_keeps_gitignore_rows() -> anyhow::Result<()>
{
    let dir = checkout_tree(&[
        (".gitignore", "lib/*\n"),
        ("lib/Inner.java", "class Inner {}\n"),
    ]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert_eq!(nested_rows(&result), Vec::<String>::new());
    assert_eq!(
        reason_for(&result, "lib/Inner.java"),
        Some(SkipReason::Gitignore)
    );
    Ok(())
}

#[test]
fn test_nested_checkout_whose_files_are_all_excluded_keeps_their_reasons() -> anyhow::Result<()> {
    let only = checkout_tree(&[("lib/Inner.java", "class Inner {}\n")]);
    let result = discover(only.path(), &settings(false, &["**/Inner.java"]))?;
    assert_eq!(nested_rows(&result), Vec::<String>::new());
    assert_eq!(
        reason_for(&result, "lib/Inner.java"),
        Some(SkipReason::UserExclude)
    );

    let sibling = checkout_tree(&[
        ("lib/Inner.java", "class Inner {}\n"),
        ("lib/Other.java", "class Other {}\n"),
    ]);
    let result = discover(sibling.path(), &settings(false, &["**/Inner.java"]))?;
    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert_eq!(nested_rows(&result), vec!["lib"]);
    assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
    Ok(())
}

#[test]
fn test_nested_checkout_without_scannable_files_gives_no_row() -> anyhow::Result<()> {
    let dir = checkout_tree(&[("lib/README.md", "docs\n"), ("lib/x.py", "x = 1\n")]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert!(result.skipped.is_empty(), "{:?}", result.skipped);
    Ok(())
}
#[test]
fn test_nested_checkout_with_only_a_deep_eligible_file_still_gives_a_row() -> anyhow::Result<()> {
    let dir = checkout_tree(&[("lib/a/b/Deep.java", "class Deep {}\n")]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(discovered_paths(&result), vec!["A.java"]);
    assert_eq!(nested_rows(&result), vec!["lib"]);
    Ok(())
}

#[test]
fn test_nested_checkout_own_gitignore_hides_its_only_file() -> anyhow::Result<()> {
    let dir = checkout_tree(&[
        ("lib/.gitignore", "Inner.java\n"),
        ("lib/Inner.java", "class Inner {}\n"),
    ]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(nested_rows(&result), Vec::<String>::new());
    assert_eq!(
        reason_for(&result, "lib/Inner.java"),
        Some(SkipReason::Gitignore)
    );
    Ok(())
}

#[test]
fn test_checkout_inside_a_pruned_checkout_gives_no_second_row() -> anyhow::Result<()> {
    let dir = checkout_tree(&[
        ("lib/Inner.java", "class Inner {}\n"),
        ("lib/sub/.git", "gitdir: elsewhere\n"),
        ("lib/sub/S.java", "class S {}\n"),
    ]);
    let result = discover(dir.path(), &settings(false, &[]))?;

    assert_eq!(nested_rows(&result), vec!["lib"]);
    assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
    Ok(())
}
