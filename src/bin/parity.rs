//! Diff discourse-rs responses against real Discourse.
//!
//!   parity record --rails URL   save Rails responses as golden files
//!   parity check  --rs URL      compare a running discourse-rs to golden files
//!   parity live   --rails URL --rs URL
//!                               compare both servers directly
//!
//! For meaningful results both servers must read the same database.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use discourse_rs::parity::{self, Case, Golden, Recorded};

#[derive(Parser)]
struct Cli {
    /// Cases file, one `METHOD /path [ignore=/ptr,...]` per line.
    #[arg(long, global = true, default_value = "parity/cases")]
    cases: PathBuf,
    /// Directory of golden responses.
    #[arg(long, global = true, default_value = "parity/golden")]
    golden: PathBuf,
    /// Only run cases whose request contains this string.
    #[arg(long, global = true)]
    only: Option<String>,
    /// Pause between requests to Rails, which rate-limits anonymous search.
    #[arg(long, global = true, default_value_t = 0)]
    delay_ms: u64,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Record {
        #[arg(long, env = "RAILS_URL")]
        rails: String,
    },
    Check {
        #[arg(long, env = "RS_URL", default_value = "http://127.0.0.1:8080")]
        rs: String,
    },
    Live {
        #[arg(long, env = "RAILS_URL")]
        rails: String,
        #[arg(long, env = "RS_URL", default_value = "http://127.0.0.1:8080")]
        rs: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("parity: {e}");
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<bool, String> {
    let mut cases = parity::load_cases(&cli.cases)?;
    if let Some(only) = &cli.only {
        cases.retain(|c| c.label().contains(only.as_str()));
    }

    // Redirects are part of the response under test, never followed.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;

    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;

    for case in &cases {
        if cli.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(cli.delay_ms)).await;
        }
        let outcome = match &cli.command {
            Command::Record { rails } => {
                let response = fetch(&client, rails, case).await?;
                let golden = Golden {
                    request: case.label(),
                    source: rails.trim_end_matches('/').to_string(),
                    response,
                };
                let path = parity::write_golden(&cli.golden, case, &golden)?;
                println!("recorded  {}  -> {}", case.label(), path.display());
                passed += 1;
                continue;
            }
            Command::Check { rs } => match parity::read_golden(&cli.golden, case)? {
                None => {
                    println!("skip      {}  (no golden file, run record)", case.label());
                    skipped += 1;
                    continue;
                }
                Some(golden) => {
                    let actual = fetch(&client, rs, case).await?;
                    parity::compare(case, &golden.response, &actual, ("golden", "rs"))
                }
            },
            Command::Live { rails, rs } => {
                let expected = fetch(&client, rails, case).await?;
                let actual = fetch(&client, rs, case).await?;
                parity::compare(case, &expected, &actual, ("rails", "rs"))
            }
        };

        match outcome {
            Ok(()) => {
                println!("ok        {}", case.label());
                passed += 1;
            }
            Err(report) => {
                println!("MISMATCH  {}", case.label());
                for line in report.lines() {
                    println!("    {line}");
                }
                failed += 1;
            }
        }
    }

    println!("\n{passed} ok, {failed} mismatched, {skipped} skipped");
    Ok(failed == 0)
}

async fn fetch(client: &reqwest::Client, base: &str, case: &Case) -> Result<Recorded, String> {
    let base = base.trim_end_matches('/').to_string();
    parity::run_case(case, |exchange: parity::Exchange| {
        let client = client.clone();
        let base = base.clone();
        async move {
            let url = format!("{base}{}", exchange.path);
            let method = reqwest::Method::from_bytes(exchange.method.as_bytes())
                .map_err(|e| e.to_string())?;
            let mut request = client.request(method, &url);
            for (name, value) in &exchange.headers {
                request = request.header(name.as_str(), value.as_str());
            }
            if let Some(body) = exchange.body {
                request = request.body(body);
            }
            let response = request
                .send()
                .await
                .map_err(|e| format!("{} {url}: {e}", exchange.method))?;
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let set_cookies = response
                .headers()
                .get_all(reqwest::header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .map(str::to_string)
                .collect();
            let body = response
                .text()
                .await
                .map_err(|e| format!("reading body of {url}: {e}"))?;
            Ok(parity::Reply {
                status,
                content_type,
                body,
                set_cookies,
            })
        }
    })
    .await
}
