use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use git2::Repository;

use nsd::check::{run_check, CheckDiagnostic, CheckMode, CheckRequest};
use nsd::cli::{CheckArgs, Cli, Command, ScanArgs};
use nsd::config::{Config, CODE_INVALID_CONFIG};
use nsd::format::{escape_terminal, render_terminal};
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
    let (diagnostics, status) = match &args.config {
        Some(config) if config_is_inside_checkout(&repository, config)? => refused_config(config),
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
            (outcome.diagnostics, outcome.exit_status)
        }
    };
    std::io::stdout()
        .write_all(render_terminal(&diagnostics).as_bytes())
        .context("cannot write the diagnostics")?;
    Ok(status)
}

fn refused_config(config: &Path) -> (Vec<CheckDiagnostic>, u8) {
    let diagnostics = vec![CheckDiagnostic::Failure {
        code: CODE_INVALID_CONFIG,
        message: format!(
            "trusted config {} is inside the candidate checkout",
            config.display()
        ),
    }];
    let status = exit_status(
        diagnostics.iter().map(CheckDiagnostic::code),
        &Config::default().policy,
        false,
    );
    (diagnostics, status)
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
    if let Ok(resolved) = std::fs::canonicalize(&raw) {
        forms.push(resolved);
    }
    if let (Some(parent), Some(name)) = (raw.parent(), raw.file_name()) {
        if let Ok(resolved) = std::fs::canonicalize(parent) {
            forms.push(resolved.join(name));
        }
    }
    if forms.iter().any(|form| form.starts_with(&root)) {
        return Ok(true);
    }
    let Ok(root_identity) = std::fs::metadata(&root).map(|meta| (meta.dev(), meta.ino())) else {
        return Ok(false);
    };
    Ok(forms
        .iter()
        .flat_map(|form| form.ancestors())
        .any(|ancestor| {
            std::fs::metadata(ancestor)
                .is_ok_and(|meta| meta.is_dir() && (meta.dev(), meta.ino()) == root_identity)
        }))
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
/// missing remainder appended as given.
fn canonical_even_if_missing(path: &Path) -> anyhow::Result<PathBuf> {
    let mut missing: Vec<OsString> = Vec::new();
    let mut existing = path;
    loop {
        match std::fs::canonicalize(existing) {
            Ok(mut resolved) => {
                resolved.extend(missing.iter().rev());
                return Ok(lexically_normalized(&resolved));
            }
            Err(error) => match (existing.file_name(), existing.parent()) {
                (Some(name), Some(parent)) => {
                    missing.push(name.to_os_string());
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
