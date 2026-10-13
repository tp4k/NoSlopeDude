//! WS-5: `report.html` — D19's escaping, D19's "standalone" (self-contained,
//! no CDN/network asset) constraint, D6's revision-scoped links, and
//! agreement with `report.json`'s own numbers.

use std::fs;
use std::path::{Path, PathBuf};

use nsd::model::{RemoteTarget, Revision, ScanSettings, Target, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline::{self, PipelineOutput};
use nsd::report::{self, ReportInput};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/report")
}

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

fn written_html(output: &PipelineOutput) -> String {
    fs::read_to_string(output.settings.output.join("report.html")).expect("report.html exists")
}

fn written_json_value(output: &PipelineOutput) -> serde_json::Value {
    let text =
        fs::read_to_string(output.settings.output.join("report.json")).expect("report.json exists");
    serde_json::from_str(&text).expect("valid JSON")
}

#[test]
fn test_html_escapes_source_excerpts_and_paths() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let html = written_html(&output);

    // No raw `<script>` tag anywhere: this document never emits one of its
    // own, so any occurrence would be the fixture's payload escaping
    // failing.
    assert!(
        !html.contains("<script>"),
        "a raw <script> tag leaked into the HTML"
    );
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "the source-excerpt payload should be escaped"
    );

    // The file name itself carries the second payload (`"><img onerror=1>`)
    // — both as a path and inside the excerpt.
    assert!(
        !html.contains("\"><img onerror=1>"),
        "the raw filename payload leaked unescaped into the HTML"
    );
    assert!(
        html.contains("&quot;&gt;&lt;img onerror=1&gt;"),
        "the filename payload should be escaped wherever it is interpolated"
    );

    // A top-25 row's `name` also passes through `escape_html` (html.rs:194):
    // `Anonymous.js`'s anonymous function-expression argument gets the D10
    // fallback name `<anonymous>@<line>`, which must render escaped.
    assert!(
        html.contains("&lt;anonymous&gt;@"),
        "an anonymous callable's name should be escaped: {html}"
    );
    assert!(
        !html.contains("<anonymous>"),
        "a raw <anonymous> tag leaked into the HTML"
    );

    // A skipped-file row's path also passes through `escape_html`
    // (html.rs:217): excluding every `.js` file skips the XSS-payload-named
    // fixture via `SkipReason::UserExclude`, and its path must render
    // escaped in the skipped-files table.
    let (_excluded_dir, excluded_output) = run_scan(&fixture_root(), |settings| {
        settings.exclude = vec!["**/*.js".to_string()];
    });
    let excluded_html = written_html(&excluded_output);

    let skipped = excluded_output
        .report
        .skipped_files
        .iter()
        .find(|file| file.relative_path == Path::new("src/\"><img onerror=1>.js"))
        .expect("the payload-named file should be listed as skipped");
    assert_eq!(skipped.reason, "user_exclude");

    // `build_skipped_files` sorts by relative_path (mod.rs:373): excluding
    // every `.js` file skips several of them, a real multi-element list on
    // which an unsorted order is observably different, not an equivalent
    // mutant. `Broken.java`'s salvaged `parse_syntax_error` row is part of
    // this same list.
    let observed_relative_paths: Vec<_> = excluded_output
        .report
        .skipped_files
        .iter()
        .map(|file| file.relative_path.clone())
        .collect();
    let mut sorted_relative_paths = observed_relative_paths.clone();
    sorted_relative_paths.sort();
    assert_eq!(
        observed_relative_paths, sorted_relative_paths,
        "skipped_files should be sorted by relative_path"
    );

    assert!(
        !excluded_html.contains("src/\"><img onerror=1>.js"),
        "the raw payload filename leaked unescaped into the skipped-files table"
    );
    assert!(
        excluded_html.contains("src/&quot;&gt;&lt;img onerror=1&gt;.js"),
        "the skipped file's path should be escaped: {excluded_html}"
    );
}

#[test]
fn test_exclude_glob_echo_is_html_escaped() {
    // B2: the --exclude glob is echoed verbatim into the scan-settings list
    // (html.rs:83) through the same escape_html helper as everything else
    // the scanned repository can influence, but it had no dedicated
    // mutation test of its own -- a mutant turning escape_html(glob) into
    // a plain glob at that one call site survived the suite.
    let (_dir, output) = run_scan(&fixture_root(), |settings| {
        settings.exclude = vec!["<script>x</script>/**".to_string()];
    });
    let html = written_html(&output);

    assert!(
        !html.contains("<script>x</script>/**"),
        "the raw --exclude glob leaked unescaped into the HTML: {html}"
    );
    assert!(
        html.contains("&lt;script&gt;x&lt;/script&gt;/**"),
        "the --exclude glob should be escaped wherever it is echoed: {html}"
    );
}

#[test]
fn test_html_title_and_heading_name_nsd() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let html = written_html(&output);

    assert!(
        html.contains("<title>nsd report</title>"),
        "expected the <title> to name nsd: {html}"
    );
    assert!(
        html.contains("<h1>nsd scan report</h1>"),
        "expected the <h1> to name nsd: {html}"
    );
}

#[test]
fn test_html_is_self_contained() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let html = written_html(&output);

    // A local-target scan has no GitHub links, so a self-contained
    // `report.html` should reference no network resource at all.
    assert!(
        !html.contains("http://"),
        "http:// leaked into a local-target report"
    );
    assert!(
        !html.contains("https://"),
        "https:// leaked into a local-target report"
    );
    assert!(
        !html.contains("//cdn"),
        "a CDN reference leaked into the report"
    );
}

#[test]
fn test_html_totals_match_json_totals() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let html = written_html(&output);
    let json = written_json_value(&output);

    assert_eq!(
        count_of(&html, "findings-count"),
        json["findings"].as_array().unwrap().len()
    );
    assert_eq!(
        count_of(&html, "duplicates-count"),
        json["duplicates"].as_array().unwrap().len()
    );
    assert_eq!(
        count_of(&html, "top25-count"),
        json["top25"].as_array().unwrap().len()
    );
    assert_eq!(
        count_of(&html, "skipped-count"),
        json["skipped_files"].as_array().unwrap().len()
    );

    for family in ["overall", "java", "js_ts"] {
        let json_erosion = json["scores"][family]["erosion"].as_f64().unwrap();
        let html_erosion = metric_of(&html, &format!("erosion-{family}"));
        assert!(
            (json_erosion - html_erosion).abs() < 1e-4,
            "{family} erosion: json {json_erosion} vs html {html_erosion}"
        );

        let json_ratio = json["scores"][family]["verbosity"]["ratio"]
            .as_f64()
            .unwrap();
        let html_ratio = metric_of(&html, &format!("verbosity-{family}"));
        assert!(
            (json_ratio - html_ratio).abs() < 1e-4,
            "{family} verbosity ratio: json {json_ratio} vs html {html_ratio}"
        );
    }
}

/// Extracts the integer text of `<span id="{id}">N</span>` (or any tag
/// carrying that id) from a rendered HTML document.
fn count_of(html: &str, id: &str) -> usize {
    extract_id_text(html, id)
        .parse()
        .unwrap_or_else(|_| panic!("id=\"{id}\" should contain a plain integer in: {html}"))
}

fn metric_of(html: &str, id: &str) -> f64 {
    extract_id_text(html, id)
        .parse()
        .unwrap_or_else(|_| panic!("id=\"{id}\" should contain a plain float"))
}

fn extract_id_text(html: &str, id: &str) -> String {
    let marker = format!("id=\"{id}\"");
    let marker_index = html
        .find(&marker)
        .unwrap_or_else(|| panic!("no id=\"{id}\" in the HTML"));
    let after_marker = &html[marker_index + marker.len()..];
    let tag_close = after_marker.find('>').expect("tag should close");
    let content_start = tag_close + 1;
    let content_end = after_marker[content_start..]
        .find('<')
        .expect("tag should have a closing tag");
    after_marker[content_start..content_start + content_end].to_string()
}

#[test]
fn test_links_point_at_the_scanned_revision() {
    // Local target: repo-relative `path#Lstart-Lend`, no scheme.
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let html = written_html(&output);
    // Picked on a path with no special characters: the label is rendered
    // as escaped text, and the XSS fixture's own escaping is covered by
    // `test_html_escapes_source_excerpts_and_paths` instead.
    let finding = output
        .report
        .findings
        .iter()
        .find(|finding| finding.location.relative_path == Path::new("src/Sample.java"))
        .expect("Sample.java should have at least one finding");
    let local_label = format!(
        "{}#L{}-L{}",
        finding.location.relative_path.display(),
        finding.location.start_line,
        finding.location.end_line
    );
    assert!(
        html.contains(&local_label),
        "local link label {local_label} missing from HTML"
    );
    assert!(!finding.location.is_remote_link);

    // Remote target: a real `<a href>` GitHub blob link at the scanned sha
    // (D6), exercised without network access via a hand-crafted
    // `Target::Remote`/`Revision`, same as report_json.rs.
    let target = Target::Remote(RemoteTarget {
        url: "https://github.com/an-owner/a-repo".to_string(),
    });
    let revision = Revision {
        sha: Some("deadbeefcafe".to_string()),
        dirty: Some(false),
        unavailable_reason: None,
    };
    let input = ReportInput {
        target: &target,
        target_input: "https://github.com/an-owner/a-repo",
        root: &fixture_root(),
        revision: &revision,
        settings: &output.settings,
        discover: &output.discover,
        parse_failures: &output.parse_failures,
        metrics: &output.metrics,
        clones: &output.clones,
        rules: &output.rules,
    };
    let remote_report = report::aggregate(&input);
    let remote_html = report::render_html(&remote_report);
    let remote_finding = remote_report
        .findings
        .iter()
        .find(|finding| finding.location.relative_path == Path::new("src/Sample.java"))
        .expect("Sample.java should have at least one finding");
    assert!(remote_finding.location.is_remote_link);
    let expected_href = format!(
        "href=\"https://github.com/an-owner/a-repo/blob/deadbeefcafe/{}#L{}-L{}\"",
        remote_finding.location.relative_path.display(),
        remote_finding.location.start_line,
        remote_finding.location.end_line
    );
    assert!(
        remote_html.contains(&expected_href),
        "{expected_href} missing from HTML"
    );

    // The XSS-payload-named file's remote anchor text (html.rs:119) and its
    // `href` value (html.rs:124) must both be escaped: the raw filename
    // contains `"`, `<` and `>`, so an unescaped anchor text would break out
    // of the `<a>` element and an unescaped `href` would break out of the
    // attribute.
    let payload_finding = remote_report
        .findings
        .iter()
        .find(|finding| finding.location.relative_path == Path::new("src/\"><img onerror=1>.js"))
        .expect("the payload-named file should have at least one finding");
    assert!(payload_finding.location.is_remote_link);
    assert!(
        !remote_html
            .contains("href=\"https://github.com/an-owner/a-repo/blob/deadbeefcafe/src/\"><img"),
        "an unescaped href value leaked the raw payload filename"
    );
    // Anchor *text* (the visible label between `>` and `</a>`) is a
    // separate interpolation site from the `href` value above — checked
    // independently so a regression in either one is caught on its own.
    // Derived from `payload_finding.location` itself, not the old
    // hardcoded `#L1-L1` literal: that literal never matched this
    // finding (`payload_finding.location` has always spanned lines 3-4,
    // never 1-1) — it happened to match the Top-25 row for
    // `unreachableDemo` in the same file, back when every Top-25 span
    // duplicated its `start_line` as its `end_line`. Now that `end_line`
    // is real (M0c-13), that coincidence is gone, so the anchor has to be
    // derived to stay non-vacuous.
    let payload_anchor = format!(
        ">src/\"><img onerror=1>.js#L{}-L{}</a>",
        payload_finding.location.start_line, payload_finding.location.end_line
    );
    assert!(
        !remote_html.contains(&payload_anchor),
        "an unescaped anchor text leaked the raw payload filename"
    );

    // A third input surface for the same `href` attribute (html.rs:124):
    // the remote repository name itself. `parse_github_owner_repo` takes
    // everything after the owner's first `/` as `repo`, unescaped, so a
    // malicious repo name must still render escaped in every link's
    // `href` — independent of the payload-filename case above, which by
    // now has no escapable byte left in its path segment (percent-encoded
    // instead).
    let malicious_target = Target::Remote(RemoteTarget {
        url: "https://github.com/an-owner/a-repo\"><script>x</script>".to_string(),
    });
    let malicious_input = ReportInput {
        target: &malicious_target,
        target_input: "https://github.com/an-owner/a-repo\"><script>x</script>",
        root: &fixture_root(),
        revision: &revision,
        settings: &output.settings,
        discover: &output.discover,
        parse_failures: &output.parse_failures,
        metrics: &output.metrics,
        clones: &output.clones,
        rules: &output.rules,
    };
    let malicious_report = report::aggregate(&malicious_input);
    let malicious_html = report::render_html(&malicious_report);
    assert!(
        !malicious_html.contains("a-repo\"><script"),
        "an unescaped repo name leaked raw markup into an href: {malicious_html}"
    );
    assert!(
        malicious_html.contains("a-repo&quot;&gt;&lt;script"),
        "the repo name should be escaped wherever it is interpolated into an href: {malicious_html}"
    );
}

/// C7 (triage-ws5-r1/r3): `read_excerpt`'s per-file cache must be
/// output-identical to a fresh read/split per location. `src/Sample.java`
/// carries at least two locations at distinct line spans (a
/// JAVA-REDUNDANT-ELSE-AFTER-RETURN finding on `classify` and a
/// JAVA-EMPTY-CATCH finding on `risky`, plus every top25 callable in the
/// file) -- every one of them is checked against the same span read
/// straight off the fixture file, independently of the cache.
#[test]
fn test_two_locations_in_one_file_render_identical_excerpts_to_single_reads() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let sample_path = Path::new("src/Sample.java");
    let sample_locations: Vec<_> = output
        .report
        .findings
        .iter()
        .map(|finding| &finding.location)
        .chain(
            output
                .report
                .top25
                .iter()
                .map(|callable| &callable.location),
        )
        .filter(|location| location.relative_path == sample_path)
        .collect();
    let mut distinct_spans: Vec<(usize, usize)> = sample_locations
        .iter()
        .map(|location| (location.start_line, location.end_line))
        .collect();
    distinct_spans.sort_unstable();
    distinct_spans.dedup();
    assert!(
        distinct_spans.len() >= 2,
        "need at least two distinct Sample.java spans to exercise the cache across locations: {distinct_spans:?}"
    );

    let raw = fs::read_to_string(fixture_root().join(sample_path)).expect("read fixture directly");
    let raw_lines: Vec<&str> = raw.lines().collect();
    for location in &sample_locations {
        let start_index = location.start_line.saturating_sub(1);
        let end_index = location.end_line.min(raw_lines.len());
        let expected = raw_lines[start_index..end_index].join("\n");
        assert_eq!(
            location.excerpt, expected,
            "cached excerpt for {location:?} should match a direct read of the same span"
        );
    }
}

// M6-4: each excerpt is capped at 20 lines / 4 KiB with an explicit marker.

const MARKER_WORD: &str = "truncated";

/// One Java class per file; its single method occupies lines 2..=1+`lines`.
fn java_method_source(class: &str, lines: usize) -> String {
    let mut text = format!("class {class} {{\n    void run() {{\n");
    for index in 0..lines - 2 {
        text.push_str(&format!("        count += {index};\n"));
    }
    text.push_str("    }\n}\n");
    text
}

fn scan_sources(
    files: &[(&str, String)],
) -> (tempfile::TempDir, tempfile::TempDir, PipelineOutput) {
    let root = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(root.path().join("src")).expect("src dir");
    for (name, text) in files {
        fs::write(root.path().join("src").join(name), text).expect("write source");
    }
    let (output_dir, output) = run_scan(root.path(), |_| {});
    (root, output_dir, output)
}

/// The top-25 `<pre>` text of `path`'s span, and what follows it up to the
/// end of its cell (the marker, if any).
fn top25_excerpt(html: &str, path: &str, start: usize, end: usize) -> (String, String) {
    let section = html.split("id=\"top25\"").nth(1).expect("top25 section");
    let needle = format!("{path}#L{start}-L{end}<pre>");
    let after_needle = section
        .split(&needle)
        .nth(1)
        .unwrap_or_else(|| panic!("no top25 location {needle}"));
    let (pre, rest) = after_needle.split_once("</pre>").expect("closing pre");
    let marker = rest.split("</td>").next().expect("cell end");
    (pre.to_string(), marker.to_string())
}

fn first_lines(text: &str, count: usize) -> String {
    text.lines()
        .skip(1)
        .take(count)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn test_excerpt_of_exactly_twenty_lines_is_not_truncated() {
    let source = java_method_source("F20", 20);
    let (_root, _out, output) = scan_sources(&[("F20.java", source.clone())]);
    let (pre, marker) = top25_excerpt(&written_html(&output), "src/F20.java", 2, 21);
    assert_eq!(pre, first_lines(&source, 20));
    assert!(
        !marker.contains(MARKER_WORD),
        "a 20-line excerpt got a marker: {marker}"
    );
}

#[test]
fn test_excerpt_over_twenty_lines_keeps_the_first_twenty_and_marks_truncation() {
    let source21 = java_method_source("F21", 21);
    let source57 = java_method_source("F57", 57);
    let (_root, _out, output) = scan_sources(&[
        ("F21.java", source21.clone()),
        ("F57.java", source57.clone()),
    ]);
    let html = written_html(&output);
    for (path, source, span_lines) in [
        ("src/F21.java", &source21, 21),
        ("src/F57.java", &source57, 57),
    ] {
        let (pre, marker) = top25_excerpt(&html, path, 2, 1 + span_lines);
        assert_eq!(pre, first_lines(source, 20), "{path}");
        assert!(
            marker.contains(MARKER_WORD),
            "{path}: no marker in {marker}"
        );
        assert!(
            marker.contains(&span_lines.to_string()),
            "{path}: marker should name {span_lines} lines: {marker}"
        );
    }
}

#[test]
fn test_excerpt_over_four_kib_is_cut_on_a_char_boundary_and_marked() {
    // Every `é` starts at an odd byte offset of the excerpt, so byte 4096
    // falls inside a character; the longest valid prefix is 4095 bytes.
    let filler = "é".repeat(2200);
    let source =
        format!("class Big {{\n    void run() {{\n        String s = \"{filler}\";\n    }}\n}}\n");
    let (_root, _out, output) = scan_sources(&[("Big.java", source.clone())]);
    let (pre, marker) = top25_excerpt(&written_html(&output), "src/Big.java", 2, 4);
    let pre = pre.replace("&quot;", "\"");
    let full = first_lines(&source, 3);
    assert_eq!(pre.len(), 4095, "cut length in bytes");
    assert!(
        full.starts_with(&pre),
        "the cut must be a prefix of the span"
    );
    assert!(marker.contains(MARKER_WORD), "no marker in {marker}");
    assert!(marker.contains('3'), "marker should name 3 lines: {marker}");
}

#[test]
fn test_excerpt_of_exactly_4096_bytes_is_not_truncated() {
    // Raw bytes are exactly 4096; the HTML-escaped form is far longer, so a
    // cap applied after escaping would truncate it.
    let head = "    void run() {";
    let tail = "    }";
    let middle_prefix = "        String s = \"";
    let middle_suffix = "\";";
    let fixed = head.len() + tail.len() + middle_prefix.len() + middle_suffix.len() + 2;
    let filler = "<".repeat(4096 - fixed);
    let source =
        format!("class Edge {{\n{head}\n{middle_prefix}{filler}{middle_suffix}\n{tail}\n}}\n");
    let excerpt = first_lines(&source, 3);
    assert_eq!(excerpt.len(), 4096, "fixture must sit exactly on the cap");
    let (_root, _out, output) = scan_sources(&[("Edge.java", source)]);
    let (pre, marker) = top25_excerpt(&written_html(&output), "src/Edge.java", 2, 4);
    assert_eq!(pre, excerpt.replace('<', "&lt;").replace('"', "&quot;"));
    assert!(
        !marker.contains(MARKER_WORD),
        "a 4096-byte excerpt got a marker: {marker}"
    );
}

#[test]
fn test_truncation_marker_appears_only_on_truncated_excerpts() {
    let (_root, _out, output) = scan_sources(&[
        ("A20.java", java_method_source("A20", 20)),
        ("B21.java", java_method_source("B21", 21)),
        ("C30.java", java_method_source("C30", 30)),
    ]);
    let report = &output.report;
    let spans = report
        .findings
        .iter()
        .map(|finding| &finding.location)
        .chain(report.duplicates.iter().flat_map(|group| &group.locations))
        .chain(report.top25.iter().map(|callable| &callable.location));
    let truncated = spans
        .filter(|location| location.end_line - location.start_line + 1 > 20)
        .count();
    assert!(truncated >= 2, "fixture should truncate at least two spans");
    let html = written_html(&output);
    assert_eq!(html.matches(MARKER_WORD).count(), truncated);
}

/// Two classes with an identical 30-line method body: the clone group's
/// locations span more than 20 lines, so their excerpts must be capped too.
fn clone_source(class: &str) -> String {
    let mut text = format!("class {class} {{\n    int run(int count) {{\n");
    for index in 0..28 {
        text.push_str(&format!(
            "        if (count > {index}) {{ count += {index}; }}\n"
        ));
    }
    text.push_str("        return count;\n    }\n}\n");
    text
}

/// A redundant `else` whose block spans 25 lines: the rule finding's
/// location is the `else` block.
fn long_else_source() -> String {
    let mut text = String::from(
        "class LongElse {\n    int run(int count) {\n        if (count > 0) {\n            return 1;\n        } else {\n",
    );
    for index in 0..22 {
        text.push_str(&format!("            count += {index};\n"));
    }
    text.push_str("            return count;\n        }\n    }\n}\n");
    text
}

/// Asserts that every location of `locations` over 20 lines renders, inside
/// the HTML section `section_id`, as its first 20 source lines followed by a
/// marker naming the span's line count. Returns how many were checked.
fn assert_long_locations_capped(
    html: &str,
    section_id: &str,
    locations: &[&report::SourceLocation],
    sources: &[(&str, String)],
) -> usize {
    let section = html
        .split(&format!("id=\"{section_id}\""))
        .nth(1)
        .and_then(|rest| rest.split("</section>").next())
        .unwrap_or_else(|| panic!("no {section_id} section"));
    let mut checked = 0;
    for location in locations
        .iter()
        .filter(|location| location.end_line - location.start_line + 1 > 20)
    {
        let path = location.relative_path.display().to_string();
        let source = &sources
            .iter()
            .find(|(name, _)| path.ends_with(name))
            .expect("source for location")
            .1;
        let span_lines = location.end_line - location.start_line + 1;
        let needle = format!(
            "{path}#L{}-L{}<pre>",
            location.start_line, location.end_line
        );
        let after = section
            .split(&needle)
            .nth(1)
            .unwrap_or_else(|| panic!("no {section_id} location {needle}"));
        let (pre, rest) = after.split_once("</pre>").expect("closing pre");
        let expected = source
            .lines()
            .skip(location.start_line - 1)
            .take(20)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            pre,
            expected.replace('>', "&gt;"),
            "{path}: first 20 lines only"
        );
        let marker = rest.split("</p>").next().expect("marker end");
        assert!(
            marker.contains(MARKER_WORD),
            "{path}: no marker in {marker}"
        );
        assert!(
            marker.contains(&span_lines.to_string()),
            "{path}: marker should name {span_lines} lines: {marker}"
        );
        checked += 1;
    }
    checked
}

#[test]
fn test_duplicate_group_excerpts_are_capped_and_marked() {
    let sources = [
        ("CloneA.java", clone_source("CloneA")),
        ("CloneB.java", clone_source("CloneB")),
    ];
    let (_root, _out, output) = scan_sources(&sources);
    let locations: Vec<_> = output
        .report
        .duplicates
        .iter()
        .flat_map(|group| &group.locations)
        .collect();
    let checked =
        assert_long_locations_capped(&written_html(&output), "duplicates", &locations, &sources);
    assert!(
        checked >= 2,
        "fixture should yield duplicate locations over 20 lines"
    );
}

#[test]
fn test_finding_excerpts_are_capped_and_marked() {
    let sources = [("LongElse.java", long_else_source())];
    let (_root, _out, output) = scan_sources(&sources);
    let locations: Vec<_> = output
        .report
        .findings
        .iter()
        .map(|finding| &finding.location)
        .collect();
    let checked =
        assert_long_locations_capped(&written_html(&output), "findings", &locations, &sources);
    assert!(checked >= 1, "fixture should yield a finding over 20 lines");
}
