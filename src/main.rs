use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;

use nsd::cli::{Cli, Command};
use nsd::model::ScanSettings;
use nsd::pipeline;
use nsd::report;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let Command::Scan(args) = cli.command;
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
