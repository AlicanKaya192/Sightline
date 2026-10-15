//! `sightline ozet <file>`: prints the summary of a data file.

mod args;
mod render;

use std::process::ExitCode;

use sightline_core::{Limits, Summary, summarize};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sightline: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args::Command::Summary(path) = args::parse_args(std::env::args().skip(1))?;
    let Summary::Table(table) = summarize(&path, &Limits::default())?;
    let file_name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    print!("{}", render::render_table(&file_name, &table));
    Ok(())
}
