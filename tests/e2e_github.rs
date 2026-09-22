//! WS-6 GitHub end-to-end test: `#[ignore]`d and network-dependent (D5).
//! Scans `https://github.com/jonschlinkert/is-number` through the real
//! shallow-clone path (`target::resolve_remote`) via `pipeline::run` (the
//! exact function `main.rs` calls) and checks four facts pinned against
//! the tiny, long-frozen upstream repository at its own `master` head
//! (`98e8ff1da1a89f93d1397a24d7413ed15421c139`, 2018-07-04 — verified
//! against the live repository while writing this test, not recalled): 4
//! discovered `.js` files, `index.js`'s single callable at line 10 with
//! `cc == 5`, the resolved sha (cross-checked against `git rev-parse HEAD`
//! run independently in the clone), and every source link carrying that
//! same sha (the remote half of D6).

use std::path::Path;
use std::process::Command;

use agent_slope::model::{ScanSettings, DEFAULT_MIN_CLONE_LINES};
use agent_slope::pipeline;
use agent_slope::report::SourceLocation;

const IS_NUMBER_URL: &str = "https://github.com/jonschlinkert/is-number";

#[test]
#[ignore = "network-dependent: shallow-clones a real public GitHub repository"]
fn test_public_github_url_shallow_clone_scan() {
    let output_dir = tempfile::tempdir().expect("output tempdir");
    let settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: DEFAULT_MIN_CLONE_LINES,
    };
    let output = pipeline::run(IS_NUMBER_URL, settings)
        .expect("pipeline run should succeed against a live GitHub URL");

    // 4 discovered .js files, none excluded (D16): index.js, test.js,
    // benchmark/index.js, benchmark/fixtures.js.
    assert_eq!(
        output.discover.discovered.len(),
        4,
        "expected all 4 of is-number's .js files to be discovered: {:?}",
        output.discover.discovered
    );
    assert!(
        output
            .discover
            .discovered
            .iter()
            .any(|file| file.relative_path == Path::new("index.js")),
        "index.js should be among the discovered files"
    );

    // index.js's single callable, pinned against the live repository.
    let index_callable = output
        .metrics
        .callables
        .iter()
        .find(|callable| callable.relative_path == Path::new("index.js"))
        .expect("index.js should contribute a callable");
    assert_eq!(
        index_callable.start_line, 10,
        "module.exports = function(num) starts at line 10"
    );
    assert_eq!(
        index_callable.cc, 5,
        "1 base + two if + one && + one ternary"
    );

    // The resolved sha: cross-checked against `git rev-parse HEAD` run
    // independently in the clone directory `pipeline::run` itself
    // resolved, not just read back from the same struct that wrote it.
    let reported_sha = output
        .resolved_target
        .revision
        .sha
        .clone()
        .expect("a shallow clone should resolve a sha");
    assert_eq!(reported_sha.len(), 40, "a git sha is 40 hex characters");
    let git_output = Command::new("git")
        .args(["-C"])
        .arg(&output.resolved_target.root)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("spawn git rev-parse HEAD in the clone");
    assert!(git_output.status.success());
    let actual_sha = String::from_utf8(git_output.stdout)
        .expect("valid UTF-8")
        .trim()
        .to_string();
    assert_eq!(
        reported_sha, actual_sha,
        "the reported revision must match the clone's own HEAD"
    );

    // Every source link carries that same sha (the remote half of D6).
    let expected_prefix = format!("{IS_NUMBER_URL}/blob/{reported_sha}/");
    let mut link_count = 0;
    for finding in &output.report.findings {
        assert_link_pinned(&finding.location, &expected_prefix);
        link_count += 1;
    }
    for group in &output.report.duplicates {
        for location in &group.locations {
            assert_link_pinned(location, &expected_prefix);
            link_count += 1;
        }
    }
    for callable in &output.report.top25 {
        assert_link_pinned(&callable.location, &expected_prefix);
        link_count += 1;
    }
    assert!(
        link_count > 0,
        "the scan should have produced at least one source location to check"
    );
}

fn assert_link_pinned(location: &SourceLocation, expected_prefix: &str) {
    assert!(
        location.is_remote_link,
        "a remote-target location should be a remote link: {location:?}"
    );
    assert!(
        location.link.starts_with(expected_prefix),
        "{} should start with {expected_prefix}",
        location.link
    );
    assert!(
        location
            .link
            .ends_with(&format!("#L{}-L{}", location.start_line, location.end_line)),
        "{} should end with its line-span anchor",
        location.link
    );
    assert_eq!(
        location.link,
        format!(
            "{expected_prefix}{}#L{}-L{}",
            location.relative_path.display(),
            location.start_line,
            location.end_line
        ),
        "the link's <path> segment must be the location's own relative_path"
    );
}
