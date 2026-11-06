//! Runs only the tests a change can reach, from a map of the functions each
//! test ran (docs/test-impact.md).
//!
//!   test-impact runner <binary> [args...]
//!       cargo's target runner while recording: runs one test and records
//!       the workspace functions it ran
//!   test-impact build-map <dir> <map> [<nextest list json>]
//!       merges the recordings in <dir> into <map>
//!   test-impact select <map> [--base <ref>] [--list <nextest list json>]
//!       prints a nextest filterset of the tests the diff from <ref> reaches
//!   test-impact replay <map> [<count>]
//!       selects for each of the last <count> commits on origin/main

mod inputs;
mod map;
mod record;
mod select;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("runner") if args.len() >= 2 => record::runner(&args[1], &args[2..]),
        Some("build-map") if (3..=4).contains(&args.len()) => {
            map::build(&args[1], &args[2], args.get(3).map(String::as_str)).map(|()| 0)
        }
        Some("select") if args.len() >= 2 => select::select_command(&args[1], &args[2..]),
        Some("replay") if args.len() >= 2 => select::replay(&args[1], args.get(2)).map(|()| 0),
        _ => {
            eprintln!("usage: see crates/test-impact/src/main.rs");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("test-impact: {err}");
            ExitCode::from(2)
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
