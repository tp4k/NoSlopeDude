//! WS-6: error-span salvage and the `SkipReason` split
//! (`nsd-plan-final.md`, *Architecture* -> salvage row). `parse::parse_one`
//! no longer drops a whole file on `tree.root_node().has_error()`; IR-level
//! typed damage spans (`src/lower/mod.rs::lower_file`) drive fail-closed
//! exclusion at entity (callable) granularity instead. These tests exercise
//! the full pipeline, not the lowering in isolation, since the salvage
//! guarantee is about what every analyzer stage (which each independently
//! re-lowers a `ParsedFile`, `src/metrics/mod.rs`'s own doc comment) ends up
//! publishing.

use std::fs;
use std::path::{Path, PathBuf};

use nsd::model::{ScanSettings, SkipReason, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline::{self, PipelineOutput};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Runs the full pipeline against `root`, writing the two reports into a
/// fresh temp directory that is kept alive for the caller's lifetime (the
/// returned `TempDir` guard must be held by the caller, or the directory --
/// and any `report.json`/`report.html` inside it -- is deleted the moment
/// this function returns).
fn run_scan(
    root: &Path,
    configure: impl FnOnce(&mut ScanSettings),
) -> (tempfile::TempDir, PipelineOutput) {
    let output_dir = tempfile::tempdir().expect("tempdir");
    let mut settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests: true,
        exclude: Vec::new(),
        min_clone_lines: DEFAULT_MIN_CLONE_LINES,
    };
    configure(&mut settings);
    let target_input = root
        .to_str()
        .expect("fixture path is valid UTF-8")
        .to_string();
    let output = pipeline::run(&target_input, settings).expect("pipeline run should succeed");
    (output_dir, output)
}

/// The new fixture the brief requires: at least two callables, exactly one
/// intersecting damage, in both a `.java` and a `.ts` file (the parity
/// test below). Deliberately not `tests/fixtures/rules/broken/`, whose one
/// callable is wholly damaged -- it cannot exercise "an undamaged callable
/// in the same file is still measured" at all.
fn salvage_fixture_root() -> PathBuf {
    fixtures_root().join("salvage")
}

/// The undamaged half of a partially-damaged file is measured, not dropped
/// wholesale: `Mixed.java`'s `safe` callable sits beside a damaged `broken`
/// method in the same file, and still shows up with real cc/sloc.
#[test]
fn test_callables_outside_the_damaged_span_are_measured() {
    let (_dir, output) = run_scan(&salvage_fixture_root(), |_| {});

    let safe = output
        .metrics
        .callables
        .iter()
        .find(|callable| {
            callable.name == "safe" && callable.relative_path == Path::new("Mixed.java")
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a measured `safe` callable: {:?}",
                output.metrics.callables
            )
        });
    assert_eq!(safe.cc, 2, "one `if` beyond the implicit base path");
    assert!(
        safe.sloc >= 2,
        "safe's body has at least the if and both returns: {safe:?}"
    );

    assert!(
        !output
            .metrics
            .callables
            .iter()
            .any(|callable| callable.name == "broken"
                && callable.relative_path == Path::new("Mixed.java")),
        "a damaged callable must not appear in metrics.callables: {:?}",
        output.metrics.callables
    );
}

/// Fail-closed is preserved on the pre-existing wholly-damaged fixture:
/// `Broken.java`'s one callable (`method`) stays unmeasured, and `Good.java`
/// beside it in the same scan (`classify`) is unaffected.
#[test]
fn test_callables_intersecting_the_damaged_span_are_not_measured() {
    let root = fixtures_root().join("rules/broken");
    let (_dir, output) = run_scan(&root, |_| {});

    assert!(
        !output
            .metrics
            .callables
            .iter()
            .any(|callable| callable.name == "method"),
        "Broken.java's one callable is wholly damaged and must stay unmeasured: {:?}",
        output.metrics.callables
    );
    let classify = output
        .metrics
        .callables
        .iter()
        .find(|callable| callable.name == "classify")
        .unwrap_or_else(|| {
            panic!(
                "Good.java's callable must be unaffected: {:?}",
                output.metrics.callables
            )
        });
    assert_eq!(classify.relative_path, PathBuf::from("Good.java"));
}

/// The same damage shape, salvaged the same way, in both languages: proof
/// the capability lives at the IR level (`lower::lower_file`) and is not
/// something either per-grammar lowering (`src/lower/java.rs` /
/// `src/lower/jsts.rs`) implements separately.
#[test]
fn test_salvage_is_identical_in_both_languages() {
    let (_dir, output) = run_scan(&salvage_fixture_root(), |_| {});

    for relative_path in ["Mixed.java", "Mixed.ts"] {
        let path = Path::new(relative_path);
        assert!(
            output
                .metrics
                .callables
                .iter()
                .any(|callable| callable.name == "safe" && callable.relative_path == path),
            "{relative_path}'s undamaged `safe` callable should be measured: {:?}",
            output.metrics.callables
        );
        assert!(
            !output
                .metrics
                .callables
                .iter()
                .any(|callable| callable.name == "broken" && callable.relative_path == path),
            "{relative_path}'s damaged `broken` callable must stay unmeasured: {:?}",
            output.metrics.callables
        );
    }
}

/// `broken/Broken.ts` used to be dropped wholesale and rendered in
/// `skipped_files` with `reason: "parse_syntax_error"`. It now salvage-
/// parses: it disappears from `skipped_files` entirely, while `incomplete`
/// stays driven by its residual damage (it is *also* still present in
/// `parse_failures`, so `pipeline.rs`'s own untouched
/// `!parse_failures.is_empty()` plumbing keeps working) rather than by a
/// whole-file drop -- proven by the file still reaching the metrics stage
/// (its file scan summary exists).
#[test]
fn test_a_file_with_damage_is_no_longer_listed_as_a_whole_file_skip() {
    let root = fixtures_root().join("metrics/broken");
    let (_dir, output) = run_scan(&root, |_| {});

    assert!(
        !output
            .report
            .skipped_files
            .iter()
            .any(|file| file.relative_path == Path::new("Broken.ts")),
        "Broken.ts should no longer be listed as skipped: {:?}",
        output.report.skipped_files
    );
    assert_eq!(
        output.parse_failures.len(),
        1,
        "{:?}",
        output.parse_failures
    );
    assert_eq!(
        output.parse_failures[0].relative_path,
        PathBuf::from("Broken.ts")
    );
    assert_eq!(
        output.metrics.file_scan_summaries.len(),
        1,
        "Broken.ts should still be handed to the metrics stage: {:?}",
        output.metrics.file_scan_summaries
    );
    assert!(
        output.report.incomplete,
        "residual damage in Broken.ts should still mark the report incomplete"
    );
}

/// A scan whose only skips are policy skips (`test`, `gitignore`,
/// `user_exclude`) reports `incomplete: false`: the `SkipReason` split
/// drives `incomplete` only from an analysis-failure skip, never from a
/// deliberate policy exclusion.
#[test]
fn test_incomplete_is_driven_only_by_analysis_failure() {
    let source_dir = tempfile::tempdir().expect("tempdir");
    let root = source_dir.path();
    fs::write(
        root.join("Sample.java"),
        "public class Sample { public void run() {} }\n",
    )
    .expect("write clean fixture file");
    fs::write(
        root.join("AppTest.java"),
        "public class AppTest { public void run() {} }\n",
    )
    .expect("write test-glob fixture file");
    fs::write(
        root.join("Excluded.java"),
        "public class Excluded { public void run() {} }\n",
    )
    .expect("write user-exclude fixture file");
    fs::write(
        root.join("Ignored.java"),
        "public class Ignored { public void run() {} }\n",
    )
    .expect("write gitignore fixture file");
    fs::write(root.join(".gitignore"), "Ignored.java\n").expect("write .gitignore");

    let (_dir, output) = run_scan(root, |settings| {
        settings.include_tests = false;
        settings.exclude = vec!["**/Excluded.java".to_string()];
    });

    assert!(
        output
            .discover
            .skipped
            .iter()
            .any(|file| file.reason == SkipReason::Test),
        "AppTest.java should be skipped via the Test glob: {:?}",
        output.discover.skipped
    );
    assert!(
        output
            .discover
            .skipped
            .iter()
            .any(|file| file.reason == SkipReason::Gitignore),
        "Ignored.java should be skipped via .gitignore: {:?}",
        output.discover.skipped
    );
    assert!(
        output
            .discover
            .skipped
            .iter()
            .any(|file| file.reason == SkipReason::UserExclude),
        "Excluded.java should be skipped via --exclude: {:?}",
        output.discover.skipped
    );
    assert!(
        !output
            .discover
            .skipped
            .iter()
            .any(|file| file.reason == SkipReason::Unreadable),
        "no analysis-failure skip should be present in this scan: {:?}",
        output.discover.skipped
    );
    assert!(
        !output.report.incomplete,
        "a scan whose only skips are policy skips should not be marked incomplete: {:?}",
        output.discover.skipped
    );
}

/// The `SkipReason` split is a model-level split, not a relabeling: every
/// legacy label string is untouched, byte-for-byte.
#[test]
fn test_policy_skip_labels_are_byte_identical_to_the_legacy_labels() {
    assert_eq!(SkipReason::Gitignore.label(), "gitignore");
    assert_eq!(
        SkipReason::DependencyOrBuildOutput.label(),
        "dependency_or_build_output"
    );
    assert_eq!(SkipReason::GeneratedCode.label(), "generated_code");
    assert_eq!(SkipReason::Test.label(), "test");
    assert_eq!(SkipReason::UserExclude.label(), "user_exclude");
    assert_eq!(SkipReason::Unreadable.label(), "unreadable");
}

/// `report.json`'s top-level key set is exactly what it was before salvage:
/// item 8's "retain legacy JSON serialization through this gate".
#[test]
fn test_report_json_gains_no_new_top_level_field() {
    let (_dir, output) = run_scan(&salvage_fixture_root(), |_| {});
    let json_text =
        fs::read_to_string(output.settings.output.join("report.json")).expect("report.json exists");
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("valid JSON");

    let mut keys: Vec<&str> = value
        .as_object()
        .expect("report.json is a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected_keys = vec![
        "adaptation",
        "duplicates",
        "findings",
        "incomplete",
        "scan",
        "scores",
        "skipped_files",
        "top25",
    ];
    expected_keys.sort_unstable();
    assert_eq!(
        keys, expected_keys,
        "report.json must keep its exact pre-salvage top-level key set: {keys:?}"
    );
}

/// WS-6 round 3 (security HIGH: a clean entity nested inside a damaged
/// outer entity survived in `metrics.callables` despite its own `IrNode`
/// subtree being wiped by pruning, publishing a bogus `cc:1,sloc:0`):
/// `broken`'s parameter list is truncated by a missing `)` (Java: a
/// `@Nullable` varargs annotation the grammar cannot place; TS: an
/// unterminated `formal_parameters`), so `broken` itself is excluded, and
/// its nested-but-otherwise-clean callable (`clean`'s lambda / arrow
/// function) must be excluded too -- it is physically inside `broken`'s own
/// span, so `prune_damage` wipes its `IrNode` subtree regardless of
/// whether the nested callable's *own* span happens to avoid the damage.
#[test]
fn test_a_clean_callable_nested_inside_a_damaged_outer_callable_is_not_measured() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("Nested.java"),
        "class Nested {\n    void broken(Class<?> @Nullable ... cs) {\n        Runnable clean = () -> {\n            System.out.println(\"hi\");\n        };\n    }\n}\n",
    )
    .expect("write Nested.java");
    fs::write(
        dir.path().join("nested.ts"),
        "export function broken(a: number {\n  const f = () => { return 1; };\n  return a;\n}\n",
    )
    .expect("write nested.ts");

    let (_output_dir, output) = run_scan(dir.path(), |_| {});

    for relative_path in ["Nested.java", "nested.ts"] {
        let path = Path::new(relative_path);
        assert!(
            !output
                .metrics
                .callables
                .iter()
                .any(|callable| callable.relative_path == path),
            "{relative_path}: no callable (outer damaged or nested-clean) should survive: {:?}",
            output.metrics.callables
        );
    }
}

/// WS-6 round 3 (security HIGH: `Span::intersects`'s old strict two-sided
/// `<` test never registered a zero-width `MISSING` span sitting exactly at
/// an entity's own `end_byte` as intersecting it): a truncated file (its
/// last `}` is simply absent) has its one callable's own span end exactly
/// where the `MISSING` token is inserted, so this fixture is only salvaged
/// correctly once that boundary case is fixed.
#[test]
fn test_a_callable_truncated_at_its_own_end_byte_is_not_measured() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("truncated.ts"),
        "export function broken(a: number) {\n  return a;\n",
    )
    .expect("write truncated.ts");

    let (_output_dir, output) = run_scan(dir.path(), |_| {});

    assert!(
        !output
            .metrics
            .callables
            .iter()
            .any(|callable| callable.relative_path == Path::new("truncated.ts")),
        "the truncated callable must not be measured: {:?}",
        output.metrics.callables
    );
}

/// WS-6 round 4 (security HIGH regression from round 3's `35508d9`): an
/// entity whose own span sits *inside* a bare damage span -- no enclosing
/// damaged callable/block between them -- used to stay measured, since the
/// old sweep only opened an excluded ancestor for a callable/block entry,
/// never for a damage-span entry, while `redact_targets` unconditionally
/// wiped the same span's `IrNode` subtree anyway; `find_ir_subtree` then
/// missed and `metrics::fallback_ir_body` published a fabricated `cc:1
/// sloc:0` measurement. Java: `alpha`'s own signature and body are clean,
/// but the enclosing class's own closing `}` is missing, so the parser's
/// error recovery cannot commit to a `class_declaration` at all -- it wraps
/// the class name, the whole (otherwise clean) `alpha` method, and the
/// trailing unterminated `beta` in one top-level `ERROR`/damage span, with no
/// `class_declaration`/block ancestor of `alpha` surviving between them at
/// all. TS: `alpha` sits clean at module scope, but a second, malformed
/// function's body (`{{{};`) produces the same bare-damage-contains-entity
/// shape (the whole `export_statement` wrapping `alpha` ends up inside one
/// top-level `ERROR` span too).
#[test]
fn test_a_clean_callable_contained_in_a_bare_damage_span_is_not_measured() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("Alpha.java"),
        "class Alpha { int alpha(int a) { if (a > 0) { return a + 1; } return 0; } int beta(i",
    )
    .expect("write Alpha.java");
    fs::write(
        dir.path().join("alpha.ts"),
        "export function alpha(a: number) { return a + 1; }\nfunction beta() {\n  {{{};\n",
    )
    .expect("write alpha.ts");

    let (_output_dir, output) = run_scan(dir.path(), |_| {});

    for relative_path in ["Alpha.java", "alpha.ts"] {
        let path = Path::new(relative_path);
        assert!(
            !output
                .metrics
                .callables
                .iter()
                .any(|callable| callable.relative_path == path && callable.name.contains("alpha")),
            "{relative_path}: alpha sits inside a bare damage span and must not be measured: {:?}",
            output.metrics.callables
        );
        assert!(
            !output
                .metrics
                .callables
                .iter()
                .any(|callable| callable.relative_path == path && callable.sloc == 0),
            "{relative_path}: no callable should ever be published with a fabricated cc:1 \
             sloc:0 measurement: {:?}",
            output.metrics.callables
        );
    }
}

/// WS-6 round 3 (security MEDIUM: bare/stray damage sitting outside every
/// callable and block used to escape pruning entirely): a syntax error that
/// sits after a clean function, not inside any callable or block, must
/// still be redacted -- the surrounding clean callable is measured with its
/// real body, and the file's `scanned_lines` count reflects only that
/// clean body, never anything derived from the stray damage.
#[test]
fn test_stray_damage_outside_any_callable_or_block_does_not_reach_metrics() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("stray.ts"),
        "export function safe(x) {\n  return x;\n}\n\n)));\n",
    )
    .expect("write stray.ts");

    let (_output_dir, output) = run_scan(dir.path(), |_| {});

    let safe = output
        .metrics
        .callables
        .iter()
        .find(|callable| callable.relative_path == Path::new("stray.ts"))
        .unwrap_or_else(|| {
            panic!(
                "expected the clean `safe` callable to be measured: {:?}",
                output.metrics.callables
            )
        });
    assert_eq!(safe.name, "safe");
    assert_eq!(safe.cc, 1, "no branching inside `safe`'s body: {safe:?}");
    assert_eq!(
        safe.sloc, 1,
        "safe's body is exactly the one `return x;` line: {safe:?}"
    );

    let summary = output
        .metrics
        .file_scan_summaries
        .iter()
        .find(|summary| summary.relative_path == Path::new("stray.ts"))
        .unwrap_or_else(|| {
            panic!(
                "expected a file scan summary for stray.ts: {:?}",
                output.metrics.file_scan_summaries
            )
        });
    assert_eq!(
        summary.scanned_lines, 2,
        "only safe's own two executable-leaf-bearing lines (its `x` parameter and its \
         `return x;` body) should ever count toward scanned_lines -- the stray `)));` \
         contributes none: {summary:?}"
    );
}

/// WS-6 round 3 (perf HIGH: `prune_damage`'s old `Vec<Span>` membership
/// test made the redact step `Θ(nodes × pruned-entities)`, and the old
/// per-entity `is_clear_of_damage` scan made entity classification
/// `Θ(entities × damage)`): 1,200 independently-damaged callables in one
/// file must still salvage in bounded wall-clock time, not the quadratic
/// blowup either bound would produce -- the same shape and assertion style
/// as `tests/metrics.rs::test_deeply_nested_file_does_not_abort_the_scan`.
///
/// WS-6 round 4 (code + security MEDIUM, merged): the N=1,200/20s bound
/// alone does not discriminate the regression it claims to guard -- under
/// the quadratic model the *old*, unfixed code at N=1,200 costs only
/// ≈220s × (1200/20000)² ≈ 0.8s, comfortably under 20s, so reintroducing the
/// linear membership scan in `prune_damage` leaves this test green.
///
/// A second scan at 4× the callable count keeps its own absolute ceiling
/// (60s) and "every callable stays unmeasured" check through the same
/// `run_scan` full pipeline the N=1,200 half above uses, per this file's own
/// module doc comment (the salvage guarantee is about what every analyzer
/// stage ends up publishing). But the *scaling* assertion this row exists to
/// add cannot be timed through `run_scan`: `clones::run`'s own inherent,
/// documented `Θ(n)`-per-container cost
/// (`src/clones/mod.rs::enumerate_container_candidates`) dominates
/// full-pipeline wall time at this exact fixture shape regardless of which
/// form `prune_damage`'s membership test takes, because each malformed
/// one-line function body leaks its own clean, undamaged `return` statement
/// as a sibling into one shared top-level clone-candidate container that
/// grows with N -- measured directly: full-pipeline `elapsed_4n/elapsed_n`
/// is ≈14.6x with the O(1) fix in place and ≈15.2x with it reverted to the
/// linear form, both over any usable threshold, so a full-pipeline scaling
/// assert can never discriminate this regression and must not gate on it.
/// `discover` + `parse` + `lower::lower_all`, timed in isolation on a fresh
/// pair of fixtures of the same generated shape and scale, isolates exactly
/// the WS-6-owned code path instead: Θ(n²) predicts `elapsed_4n` ≈16×
/// `elapsed_n`; the O(1)-membership fixed form predicts ≈4× (measured: ≈4.0x
/// fixed, ≈14.3x reverted-to-linear). `SCALING_ASSERT_MULTIPLIER` (8) splits
/// the two with margin.
#[test]
fn test_many_damaged_callables_do_not_cause_a_quadratic_blowup() {
    const DAMAGED_CALLABLE_COUNT: usize = 1_200;
    const SCALED_CALLABLE_COUNT: usize = DAMAGED_CALLABLE_COUNT * 4;
    const SCALING_ASSERT_MULTIPLIER: u32 = 8;
    const SCALED_ABSOLUTE_CEILING_SECS: u64 = 60;

    fn write_fixture(dir: &Path, count: usize) {
        let mut source = String::new();
        for index in 0..count {
            source.push_str(&format!(
                "export function broken{index}(a: number {{\n  return a;\n}}\n"
            ));
        }
        fs::write(dir.join("ManyBroken.ts"), source).expect("write ManyBroken.ts");
    }

    // Isolates exactly the `discover` + `parse` + `lower::lower_all` cost
    // the WS-6 round 4 doc comment above explains the full pipeline below
    // cannot discriminate.
    fn lower_only_elapsed(count: usize) -> std::time::Duration {
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture(dir.path(), count);
        let settings = ScanSettings {
            output: tempfile::tempdir().expect("tempdir").path().to_path_buf(),
            include_tests: true,
            exclude: Vec::new(),
            min_clone_lines: DEFAULT_MIN_CLONE_LINES,
        };
        let started = std::time::Instant::now();
        let discovered = nsd::discover::discover(dir.path(), &settings).expect("discover");
        let (parsed_files, _parse_failures) =
            nsd::parse::parse_all(dir.path(), &discovered.discovered);
        let _lowered = nsd::lower::lower_all(&parsed_files);
        started.elapsed()
    }

    let dir = tempfile::tempdir().expect("tempdir");
    write_fixture(dir.path(), DAMAGED_CALLABLE_COUNT);

    let started = std::time::Instant::now();
    let (_output_dir, output) = run_scan(dir.path(), |_| {});
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "salvage over {DAMAGED_CALLABLE_COUNT} damaged callables took {elapsed:?}, expected a bounded, non-quadratic run"
    );
    assert!(
        !output
            .metrics
            .callables
            .iter()
            .any(|callable| callable.relative_path == Path::new("ManyBroken.ts")),
        "every one of the {DAMAGED_CALLABLE_COUNT} callables is damaged and must stay unmeasured: {:?}",
        output.metrics.callables
    );

    let scaled_dir = tempfile::tempdir().expect("tempdir");
    write_fixture(scaled_dir.path(), SCALED_CALLABLE_COUNT);

    let scaled_started = std::time::Instant::now();
    let (_scaled_output_dir, scaled_output) = run_scan(scaled_dir.path(), |_| {});
    let scaled_elapsed = scaled_started.elapsed();

    assert!(
        scaled_elapsed < std::time::Duration::from_secs(SCALED_ABSOLUTE_CEILING_SECS),
        "salvage over {SCALED_CALLABLE_COUNT} damaged callables took {scaled_elapsed:?}, expected a bounded run even at 4x scale"
    );
    assert!(
        !scaled_output
            .metrics
            .callables
            .iter()
            .any(|callable| callable.relative_path == Path::new("ManyBroken.ts")),
        "every one of the {SCALED_CALLABLE_COUNT} callables is damaged and must stay unmeasured: {:?}",
        scaled_output.metrics.callables
    );

    let lower_elapsed = lower_only_elapsed(DAMAGED_CALLABLE_COUNT);
    let lower_scaled_elapsed = lower_only_elapsed(SCALED_CALLABLE_COUNT);
    assert!(
        lower_scaled_elapsed < lower_elapsed * SCALING_ASSERT_MULTIPLIER,
        "quadratic cost would scale ~16x from N to 4N; linear/O(1)-membership cost should scale \
         ~4x. discover+parse+lower_all at N={DAMAGED_CALLABLE_COUNT} took {lower_elapsed:?}, at \
         4N={SCALED_CALLABLE_COUNT} took {lower_scaled_elapsed:?}, expected 4N < \
         {SCALING_ASSERT_MULTIPLIER}x N"
    );
}

/// The reverse direction from `test_incomplete_is_driven_only_by_analysis_failure`:
/// an analysis-failure skip (an unreadable subdirectory, discovered but
/// never even reaching parsing) still flips `incomplete` to `true`, with no
/// parse failure and no analyzer-level incompleteness involved at all --
/// proving `incomplete` really is driven by `SkipReason::is_analysis_failure`
/// and not merely left permanently `false` by this split.
#[cfg(unix)]
#[test]
fn test_unreadable_subdirectory_skip_marks_incomplete() {
    use std::os::unix::fs::PermissionsExt;

    let unreadable_dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        unreadable_dir.path().join("readable.js"),
        "module.exports = {};\n",
    )
    .expect("write readable fixture file");
    let locked = unreadable_dir.path().join("locked");
    fs::create_dir(&locked).expect("create locked dir");
    fs::write(locked.join("secret.js"), "module.exports = {};\n").expect("write locked file");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock down");

    let outcome = std::panic::catch_unwind(|| run_scan(unreadable_dir.path(), |_| {}));

    // Restore the mode before propagating any panic, so a failing
    // assertion below still leaves a cleanable tree.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock");
    let (_dir, output) = match outcome {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    };

    assert!(
        output.parse_failures.is_empty(),
        "the locked subdirectory never reaches parsing at all: {:?}",
        output.parse_failures
    );
    assert!(
        !output.metrics.incomplete,
        "no parse failure occurred, so the analyzer-level incomplete stays false"
    );
    assert!(
        output.report.incomplete,
        "an unreadable-subdirectory skip should still mark the report incomplete"
    );
}
