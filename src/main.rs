use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    discourse_rs::cli::run(Vec::new()).await
}
