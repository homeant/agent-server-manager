mod cli;
mod client;
mod daemon;
mod model;
mod paths;
mod skill;
mod supervisor;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    if matches!(std::env::args().nth(1).as_deref(), Some("--version" | "-V")) {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if std::env::args().nth(1).as_deref() == Some("__daemon") {
        return match daemon::run(paths::Paths::discover()).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("[asvc-daemon] {error:#}");
                ExitCode::FAILURE
            }
        };
    }
    cli::run().await
}
