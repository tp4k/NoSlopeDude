//! D19: the standalone HTML rendering. One file, inline CSS, no template
//! engine and no CDN/external asset — every interpolated value that comes
//! from the scanned repository (a source excerpt, a file path, a rule id,
//! a callable name) passes through `escape_html`, the single escaping
//! helper this whole module uses.

use std::fmt::Write as _;

use super::{Report, ReportFamilyScores, SourceLocation};

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
.metric{font-weight:bold}";

/// D17/D19: the standalone `report.html`, built entirely by hand-written
/// writers into one `String` (`std::fmt::Write`) — no template engine.
pub fn render_html(report: &Report) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>nsd report</title><style>{STYLE}</style></head><body>"
    );
    let _ = write!(out, "<h1>nsd scan report</h1>");
    render_scan_section(&mut out, report);
    render_scores_section(&mut out, report);
    render_findings_section(&mut out, report);
    render_duplicates_section(&mut out, report);
    render_top25_section(&mut out, report);
    render_skipped_section(&mut out, report);
    render_adaptation_section(&mut out, report);
    let _ = write!(out, "</body></html>");
    out
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
}

fn render_findings_section(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<section id=\"findings\"><h2>Rule findings (<span id=\"findings-count\">{}</span>)</h2><table>",
        report.findings.len()
    );
    let _ = write!(
        out,
        "<tr><th>Rule</th><th>Language</th><th>Location</th></tr>"
    );
    for finding in &report.findings {
        let _ = write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>",
            escape_html(finding.rule_id),
            escape_html(finding.language)
        );
        render_location(out, &finding.location);
        let _ = write!(out, "</td></tr>");
    }
    let _ = write!(out, "</table></section>");
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
            "<div><p>{} — {} redundant lines</p><ul>",
            escape_html(group.language),
            group.redundant_lines
        );
        for location in &group.locations {
            let _ = write!(out, "<li>");
            render_location(out, location);
            let _ = write!(out, "</li>");
        }
        let _ = write!(out, "</ul></div>");
    }
    let _ = write!(out, "</section>");
}

fn render_top25_section(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<section id=\"top25\"><h2>Top callables by CC (<span id=\"top25-count\">{}</span>)</h2><table>",
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
    let _ = write!(out, "</table></section>");
}

fn render_skipped_section(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<section id=\"skipped-files\"><h2>Skipped files (<span id=\"skipped-count\">{}</span>)</h2><table>",
        report.skipped_files.len()
    );
    let _ = write!(out, "<tr><th>Path</th><th>Reason</th><th>Detail</th></tr>");
    for file in &report.skipped_files {
        let _ = write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(&file.relative_path.display().to_string()),
            escape_html(&file.reason),
            escape_html(file.detail.as_deref().unwrap_or(""))
        );
    }
    let _ = write!(out, "</table></section>");
}

fn render_adaptation_section(out: &mut String, report: &Report) {
    let _ = write!(
        out,
        "<section id=\"adaptation\"><h2>Adaptation note</h2><p>{}</p><p>See <code>{}</code> and <code>{}</code>.</p></section>",
        escape_html(&report.adaptation.summary),
        escape_html(&report.adaptation.cc_rules_doc),
        escape_html(&report.adaptation.wasteful_rules_doc)
    );
}
