use std::io::{self, BufReader};
use std::process::ExitCode;

use clap::Parser;
use flint_app_service::{AppService, S3Connect};
use flint_cli::{Cli, Env, Input, StdinInput, TerminalInput, run};
use flint_registry::Registry;
use flint_sync::DeviceName;

#[cfg(target_os = "macos")]
fn secrets() -> flint_registry::KeychainSecrets {
    flint_registry::KeychainSecrets::new("dev.flint")
}

#[cfg(windows)]
fn secrets() -> flint_registry::CredentialSecrets {
    flint_registry::CredentialSecrets::new("dev.flint").expect("the Windows Credential Manager")
}

#[cfg(not(any(target_os = "macos", windows)))]
fn secrets() -> flint_registry::MemorySecrets {
    // No platform secret store is wired up yet, so nothing is remembered between runs.
    flint_registry::MemorySecrets::new()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match start(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn start(cli: Cli) -> Result<(), String> {
    let config_dir = match &cli.config_dir {
        Some(dir) => dir.clone(),
        None => dirs::config_dir()
            .ok_or("cannot find the config folder; give --config-dir")?
            .join("Flint"),
    };
    let registry = Registry::open(config_dir, secrets()).map_err(|e| e.to_string())?;
    let device = DeviceName::new(&gethostname::gethostname().to_string_lossy());
    let mut input: Box<dyn Input> = if cli.password_stdin {
        Box::new(StdinInput(BufReader::new(io::stdin())))
    } else {
        Box::new(TerminalInput)
    };
    let mut stdout = io::stdout();
    let mut env = Env {
        service: AppService::new(registry, S3Connect, device),
        input: input.as_mut(),
        out: &mut stdout,
    };
    run(cli, &mut env).await.map_err(|e| e.to_string())
}
