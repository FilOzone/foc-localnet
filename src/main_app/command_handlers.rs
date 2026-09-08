//! Command handlers for CLI subcommands.
//!
//! This module contains the execution logic for different CLI commands.

use std::fs;

use foc_devnet::cli::BuildCommands;
use foc_devnet::commands;
use foc_devnet::commands::build::Project;
use foc_devnet::commands::init::InitOptions;
use foc_devnet::config::Config;
use foc_devnet::paths::foc_devnet_config;
use foc_devnet::poison;

/// Execute the start command
pub fn handle_start(parallel: bool, run_id: String) -> Result<(), Box<dyn std::error::Error>> {
    poison::create_poison("Start")?;
    commands::start_cluster(parallel, run_id)
}

/// Execute the stop command
pub fn handle_stop() -> Result<(), Box<dyn std::error::Error>> {
    poison::create_poison("Stop")?;
    commands::stop_cluster()
}

/// Execute the clean command
pub fn handle_clean(all: bool, images: bool) -> Result<(), Box<dyn std::error::Error>> {
    poison::create_poison("Clean")?;
    commands::clean(all, images)
}

/// Execute the init command
pub fn handle_init(
    curio: Option<String>,
    lotus: Option<String>,
    filecoin_services: Option<String>,
    pdp: Option<String>,
    proof_params_dir: Option<String>,
    rand: bool,
    no_docker_build: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check before poison creation, which creates state/ as a side effect
    if !commands::is_clean_for_init()? {
        return Err(
            "Home directory is not clean. Run 'foc-devnet clean' first, then re-run init."
                .to_string()
                .into(),
        );
    }
    poison::create_poison("Init")?;
    commands::init_environment(InitOptions {
        curio_location: curio,
        lotus_location: lotus,
        filecoin_services_location: filecoin_services,
        pdp_location: pdp,
        proof_params_dir,
        use_random_mnemonic: rand,
        no_docker_build,
    })
}

/// Execute the build command
pub fn handle_build(build_command: BuildCommands) -> Result<(), Box<dyn std::error::Error>> {
    poison::create_poison("Build")?;

    // Load configuration
    let config_path = foc_devnet_config();
    let config_content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config file at {:?}: {}", config_path, e))?;
    let config: Config = toml::from_str(&config_content)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;

    match build_command {
        BuildCommands::Lotus { path: _ } => commands::build_project(&Project::Lotus, &config),
        BuildCommands::Curio { path: _ } => commands::build_project(&Project::Curio, &config),
    }
}

/// Execute the status command
pub fn handle_status() -> Result<(), Box<dyn std::error::Error>> {
    // Status is read-only, no poison protection needed
    commands::status()
}

/// Execute the logs command
pub fn handle_logs(follow: bool, tail: Option<usize>) -> Result<(), Box<dyn std::error::Error>> {
    // Logs is read-only, no poison protection needed
    commands::logs(follow, tail)
}
