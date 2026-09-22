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
