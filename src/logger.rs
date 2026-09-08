use crate::paths::foc_devnet_run_log_file;
use std::fs;
use std::io::IsTerminal;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initializes the logging system.
///
/// This sets up two layers:
/// 1. A stdout layer with ANSI colors for terminal output.
/// 2. A file layer without ANSI colors for the execution log.
///
/// Note: The `state/latest` symlink is managed by the start command's
/// `setup_directories_and_run_id()` function to ensure proper sequencing.
pub fn init_logging(run_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let run_dir = crate::paths::foc_devnet_run_dir(run_id);
    fs::create_dir_all(&run_dir)?;

    let log_file_path = foc_devnet_run_log_file(run_id);
    let log_file = fs::File::create(log_file_path)?;

    let file_layer = fmt::layer().with_ansi(false).with_writer(log_file);

    let stdout_layer = fmt::layer()
        .with_ansi(std::io::stdout().is_terminal())
        .with_writer(std::io::stdout)
        .with_target(false)
        .with_file(true)
        .with_line_number(true)
        .without_time();

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(file_layer)
        .with(stdout_layer)
        .init();

    Ok(())
}
