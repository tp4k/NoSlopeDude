use std::ffi::OsString;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use git2::Repository;

use nsd::check::{run_check, CheckDetails, CheckDiagnostic, CheckMode, CheckRequest};
use nsd::cli::{CheckArgs, Cli, Command, Format, ScanArgs};
use nsd::config::{Config, CODE_INVALID_CONFIG};
use nsd::format::{escape_terminal, render_check_json, render_terminal};
use nsd::model::ScanSettings;
use nsd::pipeline;
use nsd::policy::exit::exit_status;
use nsd::report;

/// A check that could not run to a verdict exits with the error status.
const CHECK_ERROR_EXIT: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan(args) => match run_scan(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error:#}");
                ExitCode::FAILURE
            }
        },
        Command::Check(args) => ExitCode::from(run_check_command(&args)),
    }
}

fn run_scan(args: ScanArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("cannot create output directory {}", args.output.display()))?;
    let settings = ScanSettings {
        output: args.output,
        include_tests: args.include_tests,
        exclude: args.exclude,
        min_clone_lines: args.min_clone_lines,
    };
    let output = pipeline::run(&args.target, settings)?;
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
    let (diagnostics, status, details) = match &args.config {
        Some(config) if config_is_inside_checkout(&repository, config)? => refused_config(),
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
                allow_new_suppressions: false,
            });
            for warning in &outcome.warnings {
                eprintln!("warning: {}", escape_terminal(warning));
            }
            (outcome.diagnostics, outcome.exit_status, outcome.details)
        }
    };
    let rendered = match args.format {
        Format::Terminal => render_terminal(&diagnostics),
        Format::Json => {
            let repository_forms = repository_paths(&repository);
            let config_forms: Vec<PathBuf> = args
                .config
                .iter()
                .flat_map(|config| [config.clone(), repository.join(config)])
                .collect();
            let repository_refs: Vec<&Path> =
                repository_forms.iter().map(PathBuf::as_path).collect();
            let config_refs: Vec<&Path> = config_forms.iter().map(PathBuf::as_path).collect();
            render_check_json(
                &diagnostics,
                status,
                details.as_ref(),
                &repository_refs,
                &config_refs,
            )
        }
    };
    std::io::stdout()
        .write_all(rendered.as_bytes())
        .context("cannot write the diagnostics")?;
    Ok(status)
}

/// The refusal names no path: the one given may be absolute, and the listing
/// never prints the checkout's location.
fn refused_config() -> (Vec<CheckDiagnostic>, u8, Option<CheckDetails>) {
    let diagnostics = vec![CheckDiagnostic::Failure {
        code: CODE_INVALID_CONFIG,
        message: "trusted config is inside the candidate checkout".to_string(),
    }];
    let status = exit_status(
        diagnostics.iter().map(CheckDiagnostic::code),
        &Config::default().policy,
        false,
    );
    (diagnostics, status, None)
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
