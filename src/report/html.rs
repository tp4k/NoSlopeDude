//! D19: the standalone HTML rendering. One file, inline CSS, no template
//! engine and no CDN/external asset — every interpolated value that comes
//! from the scanned repository (a source excerpt, a file path, a rule id,
//! a callable name) passes through `escape_html`, the single escaping
//! helper this whole module uses.
//!
//! Large sections render as native `<details>` blocks, all collapsed on
//! load: findings grouped per rule id, one block per duplicate group, the
//! top-25 table, and skipped files as a folder tree. No script — a static
//! link row at the top navigates between sections.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{Report, ReportFamilyScores, ReportFinding, ReportSkippedFile, SourceLocation};

/// D19's single escaping helper. Every value interpolated into the HTML
/// below that originates in the scanned repository goes through this and
/// nothing else.
fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

const STYLE: &str = "body{font-family:sans-serif;margin:2rem;color:#1a1a1a}\
table{border-collapse:collapse;margin-bottom:1.5rem;width:100%}\
th,td{border:1px solid #ccc;padding:0.35rem 0.6rem;text-align:left;vertical-align:top}\
th{background:#f0f0f0}\
pre{white-space:pre-wrap;margin:0;font-size:0.85em}\
section{margin-bottom:2rem}\
.metric{font-weight:bold}\
nav{margin-bottom:1.5rem;padding:0.5rem 0;border-bottom:1px solid #ccc}\
nav a{margin-right:1rem}\
details{margin:0.3rem 0}\
summary{cursor:pointer;padding:0.25rem 0}\
summary .muted{color:#666}\
details>table,details>ul,details>.tree{margin-top:0.4rem}\
.tree{margin-left:1.2rem}";

/// D17/D19: the standalone `report.html`, built entirely by hand-written
/// writers into one `String` (`std::fmt::Write`) — no template engine.
pub fn render_html(report: &Report) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>nsd report</title><style>{STYLE}</style></head><body>"
    );
    let _ = write!(out, "<h1>nsd scan report</h1>");
    render_nav(&mut out, report);
    render_scan_section(&mut out, report);
    render_scores_section(&mut out, report);
    render_findings_section(&mut out, report);
    render_duplicates_section(&mut out, report);
    render_top25_section(&mut out, report);
    render_skipped_section(&mut out, report);
    let _ = write!(out, "</body></html>");
    out
}

/// The static link row: one anchor per section, each list section's count
/// beside it. Counts here carry no `id` — the section headings own those.
fn render_nav(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<nav><a href=\"#scan-settings\">Settings</a><a href=\"#scores\">Scores</a>\
<a href=\"#findings\">Findings ({})</a><a href=\"#duplicates\">Duplicates ({})</a>\
<a href=\"#top25\">Top CC ({})</a><a href=\"#skipped-files\">Skipped ({})</a></nav>",
        report.findings.len(),
        report.duplicates.len(),
        report.top25.len(),
        report.skipped_files.len()
    );
}

fn render_scan_section(out: &mut String, report: &Report) {
    let scan = &report.scan;
    let _ = write!(
        out,
        "<section id=\"scan-settings\"><h2>Scan settings</h2><ul>"
    );
    let _ = write!(
        out,
        "<li>target: <code>{}</code></li>",
        escape_html(&scan.target)
    );
    let sha_text = scan.revision.sha.as_deref().unwrap_or("unavailable");
    let _ = write!(out, "<li>revision: <code>{}</code>", escape_html(sha_text));
    if let Some(dirty) = scan.revision.dirty {
        let _ = write!(out, " (dirty: {dirty})");
    }
    if let Some(reason) = &scan.revision.unavailable_reason {
        let _ = write!(out, " (unavailable: {})", escape_html(reason));
    }
    let _ = write!(out, "</li>");
    let _ = write!(out, "<li>include_tests: {}</li>", scan.include_tests);
    let _ = write!(out, "<li>exclude: [");
    for (index, glob) in scan.exclude.iter().enumerate() {
        if index > 0 {
            let _ = write!(out, ", ");
        }
        let _ = write!(out, "<code>{}</code>", escape_html(glob));
    }
    let _ = write!(out, "]</li>");
    let _ = write!(out, "<li>min_clone_lines: {}</li>", scan.min_clone_lines);
    let _ = write!(
        out,
        "<li id=\"incomplete\">incomplete: {}</li>",
        report.incomplete
    );
    let _ = write!(out, "</ul></section>");
}

fn render_scores_section(out: &mut String, report: &Report) {
    let _ = write!(out, "<section id=\"scores\"><h2>Scores</h2><table>");
    let _ = write!(
        out,
        "<tr><th>Family</th><th>Erosion</th><th>Verbosity ratio</th><th>Flagged / scanned lines</th></tr>"
    );
    render_score_row(out, "overall", &report.scores.overall);
    render_score_row(out, "java", &report.scores.java);
    render_score_row(out, "js_ts", &report.scores.js_ts);
    render_score_row(out, "rust", &report.scores.rust);
    let _ = write!(out, "</table></section>");
}

fn render_score_row(out: &mut String, family: &str, scores: &ReportFamilyScores) {
    let _ = write!(
        out,
        "<tr><td>{family}</td><td id=\"erosion-{family}\" class=\"metric\">{:.4}</td><td id=\"verbosity-{family}\" class=\"metric\">{:.4}</td><td>{} / {}</td></tr>",
        scores.erosion,
        scores.verbosity.ratio,
        scores.verbosity.flagged_lines,
        scores.verbosity.scanned_lines
    );
}

fn render_location(out: &mut String, location: &SourceLocation) {
    let path = escape_html(&location.relative_path.display().to_string());
    if location.is_remote_link {
        let _ = write!(
            out,
            "<a href=\"{link}\">{path}#L{start}-L{end}</a>",
            link = escape_html(&location.link),
            start = location.start_line,
            end = location.end_line
        );
    } else {
        let _ = write!(out, "{}", escape_html(&location.link));
    }
    let _ = write!(out, "<pre>{}</pre>", escape_html(&location.excerpt));
    if location.truncated {
        let _ = write!(
            out,
            "<p class=\"truncation\">excerpt truncated: the full span is {} lines</p>",
            location.end_line.saturating_sub(location.start_line) + 1
        );
    }
}

fn render_findings_section(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<section id=\"findings\"><h2>Rule findings (<span id=\"findings-count\">{}</span>)</h2>",
        report.findings.len()
    );
    // One collapsed block per rule id, in rule-id order; findings keep
    // their report order inside a block.
    let mut by_rule: BTreeMap<&str, Vec<&ReportFinding>> = BTreeMap::new();
    for finding in &report.findings {
        by_rule.entry(finding.rule_id).or_default().push(finding);
    }
    for (rule_id, findings) in &by_rule {
        let rule = escape_html(rule_id);
        let _ = write!(
            out,
            "<details class=\"rule\" id=\"rule-{rule}\"><summary><code>{rule}</code> <span class=\"muted\">({})</span></summary><table>\
<tr><th>Language</th><th>Location</th></tr>",
            findings.len()
        );
        for finding in findings {
            let _ = write!(out, "<tr><td>{}</td><td>", escape_html(finding.language));
            render_location(out, &finding.location);
            let _ = write!(out, "</td></tr>");
        }
        let _ = write!(out, "</table></details>");
    }
    let _ = write!(out, "</section>");
}

fn render_duplicates_section(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<section id=\"duplicates\"><h2>Duplicate blocks (<span id=\"duplicates-count\">{}</span>)</h2>",
        report.duplicates.len()
    );
    for group in &report.duplicates {
        let _ = write!(
            out,
            "<details class=\"duplicate\"><summary>{} — {} redundant lines <span class=\"muted\">({} occurrences)</span></summary><ul>",
            escape_html(group.language),
            group.redundant_lines,
            group.locations.len()
        );
        for location in &group.locations {
            let _ = write!(out, "<li>");
            render_location(out, location);
            let _ = write!(out, "</li>");
        }
        let _ = write!(out, "</ul></details>");
    }
    let _ = write!(out, "</section>");
}

fn render_top25_section(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<section id=\"top25\"><h2>Top callables by CC (<span id=\"top25-count\">{}</span>)</h2>\
<details><summary>Show table</summary><table>",
        report.top25.len()
    );
    let _ = write!(
        out,
        "<tr><th>Name</th><th>Language</th><th>CC</th><th>SLOC</th><th>Mass</th><th>Location</th></tr>"
    );
    for callable in &report.top25 {
        let _ = write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.4}</td><td>",
            escape_html(&callable.name),
            escape_html(callable.language),
            callable.cc,
            callable.sloc,
            callable.mass
        );
        render_location(out, &callable.location);
        let _ = write!(out, "</td></tr>");
    }
    let _ = write!(out, "</table></details></section>");
}

/// One folder of the skipped-files tree. `total` and `reasons` count every
/// file at or below this folder.
#[derive(Default)]
struct SkippedDir<'a> {
    dirs: BTreeMap<String, SkippedDir<'a>>,
    files: Vec<&'a ReportSkippedFile>,
    reasons: BTreeMap<&'a str, usize>,
    total: usize,
}

impl<'a> SkippedDir<'a> {
    fn build(files: &'a [ReportSkippedFile]) -> Self {
        let mut root = SkippedDir::default();
        for file in files {
            let mut node = &mut root;
            node.count(file);
            if let Some(parent) = file.relative_path.parent() {
                for part in parent.iter() {
                    node = node
                        .dirs
                        .entry(part.to_string_lossy().into_owned())
                        .or_default();
                    node.count(file);
                }
            }
            node.files.push(file);
        }
        root
    }

    fn count(&mut self, file: &'a ReportSkippedFile) {
        self.total += 1;
        *self.reasons.entry(file.reason.as_str()).or_default() += 1;
    }
}

fn render_skipped_section(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<section id=\"skipped-files\"><h2>Skipped files (<span id=\"skipped-count\">{}</span>)</h2>",
        report.skipped_files.len()
    );
    let root = SkippedDir::build(&report.skipped_files);
    render_skipped_contents(out, &root);
    let _ = write!(out, "</section>");
}

/// A folder's subfolders (each a collapsed block), then its own files.
fn render_skipped_contents(out: &mut String, dir: &SkippedDir) {
    for (name, child) in &dir.dirs {
        render_skipped_dir(out, name.clone(), child);
    }
    if dir.files.is_empty() {
        return;
    }
    let _ = write!(
        out,
        "<table><tr><th>Path</th><th>Reason</th><th>Detail</th></tr>"
    );
    for file in &dir.files {
        let _ = write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(&file.relative_path.display().to_string()),
            escape_html(&file.reason),
            escape_html(file.detail.as_deref().unwrap_or(""))
        );
    }
    let _ = write!(out, "</table>");
}

/// One collapsed folder block. A chain of folders holding nothing but a
/// single subfolder merges into one block (`a/b/c/`).
fn render_skipped_dir(out: &mut String, mut name: String, mut dir: &SkippedDir) {
    while dir.files.is_empty() && dir.dirs.len() == 1 {
        let (child_name, child) = dir.dirs.iter().next().expect("one subfolder");
        name = format!("{name}/{child_name}");
        dir = child;
    }
    let mut reasons: Vec<(&str, usize)> = dir.reasons.iter().map(|(r, n)| (*r, *n)).collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let breakdown = reasons
        .iter()
        .map(|(reason, n)| format!("{} {n}", escape_html(reason)))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = write!(
        out,
        "<details class=\"dir\"><summary><code>{}/</code> ({}) <span class=\"muted\">— {breakdown}</span></summary><div class=\"tree\">",
        escape_html(&name),
        dir.total
    );
    render_skipped_contents(out, dir);
    let _ = write!(out, "</div></details>");
}
