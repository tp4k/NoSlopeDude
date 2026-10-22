use std::ffi::OsString;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::builder::{StyledStr, Styles};
use clap::error::{ContextValue, Error as ClapError};
use clap::{CommandFactory, FromArgMatches, Parser};
use git2::Repository;

use nsd::check::{run_check, CheckDetails, CheckDiagnostic, CheckMode, CheckRequest};
use nsd::cli::{CheckArgs, Cli, Command, Format, ScanArgs};
use nsd::config::{Config, OutputConfig, PolicyConfig, CODE_INVALID_CONFIG};
use nsd::format::{
    escape_terminal, render_agent, render_check_json, render_github, render_terminal,
};
use nsd::model::{ScanSettings, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline;
use nsd::policy::exit::exit_status;
use nsd::report;

/// A check that could not run to a verdict exits with the error status.
const CHECK_ERROR_EXIT: u8 = 2;

/// The error bit a check ORs into its verdict when it reached one but could
/// not write `--json-out`, so a denied regression still shows (exit 3).
const JSON_OUT_ERROR_BIT: u8 = 2;

/// What the trusted configuration (the defaults when none resolved) decides
/// about the listing.
struct Trusted {
    policy: PolicyConfig,
    output: OutputConfig,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => error.exit(),
        Err(error) => {
            let code = error.exit_code();
            let error = plain_argument_error().unwrap_or(error);
            let _ = std::io::stderr().write_all(escaped_argument_error(error).as_bytes());
            std::process::exit(code);
        }
    };
    match cli.command {
        Command::Scan(args) => match run_scan(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {}", escape_terminal(&format!("{error:#}")));
                ExitCode::FAILURE
            }
        },
        Command::Check(args) => ExitCode::from(run_check_command(&args)),
    }
}

/// The same argument error parsed again without styles, so that no text clap
/// adds carries a style code a quoted value could be mistaken for.
fn plain_argument_error() -> Option<ClapError> {
    let mut command = Cli::command().styles(Styles::plain());
    match command.try_get_matches_from_mut(std::env::args_os()) {
        Ok(matches) => Cli::from_arg_matches(&matches).err(),
        Err(error) => Some(error),
    }
    .map(|error| error.format(&mut command))
}

/// Clap's message for an argument error, uncoloured, with every value it
/// quotes passed through `escape_terminal` before rendering (a newline inside
/// a value included, so it cannot start a line of its own), then every line
/// escaped again. Values with nothing to escape leave the message as clap
/// renders it.
fn escaped_argument_error(mut error: ClapError) -> String {
    let mut raw_values: Vec<(String, String)> = Vec::new();
    let mut escape_value = |value: &str| -> String {
        let escaped = escape_terminal(value);
        if escaped != value {
            raw_values.push((value.to_string(), escaped.clone()));
        }
        escaped
    };
    let mut replaced = Vec::new();
    for (kind, value) in error.context() {
        match value {
            ContextValue::String(text) => {
                replaced.push((kind, ContextValue::String(escape_value(text))));
            }
            ContextValue::Strings(texts) => {
                let escaped = texts.iter().map(|text| escape_value(text)).collect();
                replaced.push((kind, ContextValue::Strings(escaped)));
            }
            _ => {}
        }
    }
    // Longest first, so a value that contains another is replaced whole.
    raw_values.sort_by_key(|(raw, _)| std::cmp::Reverse(raw.len()));
    // The raw text: stripping styles would strip escape sequences from the
    // values too, so they would no longer match.
    let reescape = |styled: &StyledStr| -> StyledStr {
        let mut text = styled.ansi().to_string();
        for (raw, escaped) in &raw_values {
            text = text.replace(raw.as_str(), escaped);
        }
        StyledStr::from(text)
    };
    if !raw_values.is_empty() {
        for (kind, value) in error.context() {
            match value {
                ContextValue::StyledStr(styled) => {
                    replaced.push((kind, ContextValue::StyledStr(reescape(styled))));
                }
                ContextValue::StyledStrs(styled) => {
                    let escaped = styled.iter().map(reescape).collect();
                    replaced.push((kind, ContextValue::StyledStrs(escaped)));
                }
                _ => {}
            }
        }
    }
    for (kind, value) in replaced {
        error.insert(kind, value);
    }
    let rendered = error.render().to_string();
    rendered
        .split_inclusive('\n')
        .map(|line| match line.strip_suffix('\n') {
            Some(body) => format!("{}\n", escape_terminal(body)),
            None => escape_terminal(line),
        })
        .collect()
}

fn run_scan(args: ScanArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("cannot create output directory {}", args.output.display()))?;
    let settings = ScanSettings {
        output: args.output,
        include_tests: args.include_tests,
        exclude: args.exclude,
        min_clone_lines: args.min_clone_lines.unwrap_or(DEFAULT_MIN_CLONE_LINES),
    };
    let output = match args.min_clone_lines {
        Some(_) => pipeline::run(&args.target, settings)?,
        None => pipeline::run_with_root_config(&args.target, settings)?,
    };
    print!("{}", report::terminal_summary(&output.report));
    Ok(())
}

fn run_check_command(args: &CheckArgs) -> u8 {
    match check_command(args) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("error: {}", escape_terminal(&format!("{error:#}")));
            CHECK_ERROR_EXIT
        }
    }
}

fn check_command(args: &CheckArgs) -> anyhow::Result<u8> {
    let repository = std::env::current_dir().context("cannot read the current directory")?;
    let verdict = match &args.config {
        Some(config) if config_is_inside_checkout(&repository, config)? => {
            refused_config(args.allow_new_suppressions)
        }
        _ => {
            let mode = match &args.base {
                Some(reference) => CheckMode::Base {
                    reference: reference.clone(),
                    worktree: args.worktree,
                },
                None => CheckMode::Staged,
            };
            let outcome = run_check(&CheckRequest {
                repository: &repository,
                mode,
                config_path: args.config.as_deref(),
                allow_new_suppressions: args.allow_new_suppressions,
            });
            for warning in &outcome.warnings {
                eprintln!("warning: {}", escape_terminal(warning));
            }
            Verdict {
                diagnostics: outcome.diagnostics,
                status: outcome.exit_status,
                details: outcome.details,
                trusted: Trusted {
                    policy: outcome.policy,
                    output: outcome.output,
                },
            }
        }
    };
    print_check(args, &repository, &verdict)
}

/// What one check decided, and the trusted settings that list it.
struct Verdict {
    diagnostics: Vec<CheckDiagnostic>,
    status: u8,
    details: Option<CheckDetails>,
    trusted: Trusted,
}

/// Writes `--json-out` when given, then the `--format` listing to stdout.
/// A `--json-out` that cannot be written prints nothing to stdout and adds
/// the error bit to the verdict's status.
fn print_check(args: &CheckArgs, repository: &Path, verdict: &Verdict) -> anyhow::Result<u8> {
    let json = || check_json(repository, args.config.as_deref(), verdict);
    if let Some(path) = &args.json_out {
        if let Err(error) = std::fs::write(path, json()).context("cannot write --json-out") {
            eprintln!("error: {}", escape_terminal(&format!("{error:#}")));
            return Ok(verdict.status | JSON_OUT_ERROR_BIT);
        }
    }
    let rendered = render_format(args.format, args.allow_new_suppressions, verdict, json);
    std::io::stdout()
        .write_all(rendered.as_bytes())
        .context("cannot write the diagnostics")?;
    Ok(verdict.status)
}

/// What `--format` prints: `json` renders the canonical document, every
/// other format its capped listing.
fn render_format(
    format: Format,
    allow_new_suppressions: bool,
    verdict: &Verdict,
    json: impl FnOnce() -> String,
) -> String {
    let Verdict {
        diagnostics,
        details,
        trusted,
        ..
    } = verdict;
    match format {
        Format::Terminal => render_terminal(diagnostics, trusted.output.max_terminal_diagnostics),
        Format::Agent => render_agent(
            diagnostics,
            &trusted.policy,
            allow_new_suppressions,
            trusted.output.max_agent_diagnostics,
        ),
        Format::Github => render_github(
            diagnostics,
            details.as_ref(),
            &trusted.policy,
            allow_new_suppressions,
            trusted.output.max_github_diagnostics,
        ),
        Format::Json => json(),
    }
}

/// The canonical check document, with every spelling of the checkout and of
/// `--config` a failure message could name.
fn check_json(repository: &Path, config: Option<&Path>, verdict: &Verdict) -> String {
    let repository_forms = repository_paths(repository);
    let config_forms: Vec<PathBuf> = config
        .iter()
        .flat_map(|config| [config.to_path_buf(), repository.join(config)])
        .collect();
    let repository_refs: Vec<&Path> = repository_forms.iter().map(PathBuf::as_path).collect();
    let config_refs: Vec<&Path> = config_forms.iter().map(PathBuf::as_path).collect();
    render_check_json(
        &verdict.diagnostics,
        verdict.status,
        verdict.details.as_ref(),
        &repository_refs,
        &config_refs,
    )
}

/// The refusal names no path: the one given may be absolute, and the listing
/// never prints the checkout's location.
fn refused_config(allow_new_suppressions: bool) -> Verdict {
    let diagnostics = vec![CheckDiagnostic::Failure {
        code: CODE_INVALID_CONFIG,
        message: "trusted config is inside the candidate checkout".to_string(),
    }];
    let Config { policy, output, .. } = Config::default();
    let status = exit_status(
        diagnostics.iter().map(CheckDiagnostic::code),
        &policy,
        allow_new_suppressions,
    );
    Verdict {
        diagnostics,
        status,
        details: None,
        trusted: Trusted { policy, output },
    }
}

/// Every spelling of the checkout a failure message could name: the working
/// directory itself and, when it opens as a repository, its work tree, its
/// git directory and its common directory (a linked work tree's lives
/// outside it).
fn repository_paths(repository: &Path) -> Vec<PathBuf> {
    let mut paths = vec![repository.to_path_buf()];
    if let Ok(opened) = Repository::open(repository) {
        paths.push(opened.path().to_path_buf());
        paths.push(opened.commondir().to_path_buf());
        paths.extend(opened.workdir().map(Path::to_path_buf));
    }
    paths
}

/// Whether `config` lies under the work tree of the repository at
/// `repository` (`.git/` included): by the absolute path as given, by any
/// resolved form of it (so `..` after a symlink is followed as the OS does),
/// or because an existing ancestor of those forms is the work-tree root under
/// another name (same device and inode). A directory that is not a work tree
/// has no inside; `run_check` reports it as G101.
fn config_is_inside_checkout(repository: &Path, config: &Path) -> anyhow::Result<bool> {
    let Some(workdir) = Repository::open(repository)
        .ok()
        .and_then(|repo| repo.workdir().map(Path::to_path_buf))
    else {
        return Ok(false);
    };
    let root = std::fs::canonicalize(&workdir).context("cannot resolve the checkout root")?;
    let absolute = lexically_normalized(&repository.join(config));
    let mut forms = vec![absolute.clone(), canonical_even_if_missing(&absolute)?];
    if let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) {
        forms.push(canonical_even_if_missing(parent)?.join(name));
    }
    let raw = repository.join(config);
    forms.push(canonical_even_if_missing(&raw)?);
    if let (Some(parent), Some(name)) = (raw.parent(), raw.file_name()) {
        forms.push(canonical_even_if_missing(parent)?.join(name));
    }
    if forms.iter().any(|form| form.starts_with(&root)) {
        return Ok(true);
    }
    let Some(root_identity) = std::fs::metadata(&root).ok().as_ref().and_then(identity) else {
        return Ok(false);
    };
    Ok(forms
        .iter()
        .flat_map(|form| form.ancestors())
        .any(|ancestor| {
            std::fs::metadata(ancestor)
                .is_ok_and(|meta| meta.is_dir() && identity(&meta) == Some(root_identity))
        }))
}

/// The `(st_dev, st_ino)` pair that names a file on unix.
#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

/// No stable identity is read off other platforms, so containment there rests
/// on the lexical and canonical forms alone.
#[cfg(not(unix))]
fn identity(_meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

fn lexically_normalized(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    normalized
}

/// `fs::canonicalize` of the longest existing prefix of `path`, with the
/// missing remainder appended as given (a `..` in it included) and then
/// normalized lexically.
fn canonical_even_if_missing(path: &Path) -> anyhow::Result<PathBuf> {
    let mut missing: Vec<OsString> = Vec::new();
    let mut existing = path;
    loop {
        match std::fs::canonicalize(existing) {
            Ok(mut resolved) => {
                resolved.extend(missing.iter().rev());
                return Ok(lexically_normalized(&resolved));
            }
            Err(error) => match (existing.components().next_back(), existing.parent()) {
                (Some(last), Some(parent)) => {
                    missing.push(last.as_os_str().to_os_string());
                    existing = parent;
                }
                _ => {
                    return Err(error)
                        .with_context(|| format!("cannot resolve {}", path.display()));
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;

    use super::*;

    #[test]
    fn test_escaped_argument_error_escapes_controls_outside_quoted_values() {
        let error = ClapError::raw(ErrorKind::InvalidValue, "x\u{202e}y\u{2066}\nnext line\n");
        assert_eq!(
            escaped_argument_error(error),
            "error: x\\u{202e}y\\u{2066}\nnext line\n"
        );
    }
}
