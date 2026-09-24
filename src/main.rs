//! Command-line entry point: scan, then write the JSON report to stdout or a file.

use anyhow::{Context, Result};
use clap::Parser;
use slop_check::Report;
use std::{
    fs,
    io::{self, ErrorKind},
    path::PathBuf,
    process::ExitCode,
};

/// Exit status for invalid arguments, a failed command, or an incomplete scan. Clap
/// uses the same status for argument errors.
const FAILURE: u8 = 2;

#[derive(Parser)]
#[command(
    version,
    about = "Report structural code metrics as JSON without changing source files",
    after_help = "Examples:\n  slop-check\n  slop-check src\n  slop-check . --output report.json\n\nThe report is JSON described by docs/report.schema.json.\nExit codes: 0 complete scan, 2 error or incomplete scan."
)]
struct Cli {
    /// File or directory to scan (respects repository ignore files)
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Write the report to FILE instead of stdout
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run(&Cli::parse()) {
        Ok(report) if report.complete => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(FAILURE),
        Err(error) => {
            eprintln!("slop-check: {error:#}");
            ExitCode::from(FAILURE)
        }
    }
}

fn run(cli: &Cli) -> Result<Report> {
    let report = slop_check::scan(&cli.path)?;
    match &cli.output {
        Some(path) => {
            let context = || format!("write {}", path.display());
            let file = fs::File::create(path).with_context(context)?;
            slop_check::write_json(file, &report).with_context(context)?;
            eprintln!("wrote report to {}", path.display());
        }
        None => match slop_check::write_json(io::stdout().lock(), &report) {
            // A reader such as `head` may close the pipe early. The scan has already
            // finished, so its status still stands.
            Err(error) if error.kind() == ErrorKind::BrokenPipe => {}
            result => result.context("write report to stdout")?,
        },
    }
    Ok(report)
}
