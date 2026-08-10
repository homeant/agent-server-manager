mod cli;
use asvc::{config, daemon, i18n, paths};

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    if matches!(std::env::args().nth(1).as_deref(), Some("--version" | "-V")) {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let paths = paths::Paths::discover();
    match config::Config::load(&paths) {
        Ok(config) => i18n::set_locale(config.locale),
        Err(error) => {
            eprintln!("Error: {error}");
            return ExitCode::FAILURE;
        }
    }
    if std::env::args().nth(1).as_deref() == Some("__daemon") {
        return match daemon::run(paths).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("[asvc-daemon] {error:#}");
                ExitCode::FAILURE
            }
        };
    }
    cli::run(paths).await
}
