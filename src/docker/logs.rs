//! Docker log collection and cleanup utilities.
//!
//! This module provides functions to persist logs from all containers
//! whose images start with the prefix "foc" and to remove any dead
//! containers after a start attempt, regardless of success or failure.
//!
//! The log files are stored under the run-specific directory:
//! ~/.foc-devnet/run/<run_id>/logs/<container_name>.<image_name>.docker.log

use crate::constants::is_foc_devnet_image;
use crate::docker::containers::is_devnet_db_container_name;
use crate::docker::core::{docker_command, get_container_logs};
use crate::paths::foc_devnet_run_dir;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};

/// Information about a Docker container for logging and cleanup.
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
    pub status: String,
}

/// List all containers (running or stopped) with name, image, and status.
pub(crate) fn list_all_containers() -> Result<Vec<ContainerInfo>, Box<dyn Error>> {
    let output = docker_command(&["ps", "-a", "--format", "{{.Names}}|{{.Image}}|{{.Status}}"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut result = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 3 {
            result.push(ContainerInfo {
                name: parts[0].trim().to_string(),
                image: parts[1].trim().to_string(),
                status: parts[2].trim().to_string(),
            });
        }
    }
    Ok(result)
}

/// List all containers (running or stopped) belonging to foc-devnet: those whose
/// image repository is recognized via `is_foc_devnet_image`, plus the per-SP
/// database containers, which run stock images (postgres/scylla) and so are
/// matched structurally by name instead. Unrelated containers (including
/// stock-image ones like foc-observer's postgres) are excluded.
pub fn list_foc_devnet_containers() -> Result<Vec<ContainerInfo>, Box<dyn Error>> {
    Ok(list_all_containers()?
        .into_iter()
        .filter(|c| is_foc_devnet_image(&c.image) || is_devnet_db_container_name(&c.name))
        .collect())
}

/// Persist logs for all foc-devnet containers (including the per-SP database
/// containers) under the run logs directory.
pub fn persist_foc_container_logs(run_id: &str) -> Result<(), Box<dyn Error>> {
    let containers = list_foc_devnet_containers()?;
    let logs_dir = foc_devnet_run_dir(run_id).join("logs");
    fs::create_dir_all(&logs_dir)?;

    info!(
        "Persisting logs for {} foc-devnet containers to {}",
        containers.len(),
        logs_dir.display()
    );

    for c in containers {
        let safe_image = c.image.replace(':', "_");
        let file_path = logs_dir.join(format!("{}.{}.docker.log", c.name, safe_image));
        let content = match get_container_logs(&c.name) {
            Ok(logs) => {
                info!("✓ Captured logs for container '{}'", c.name);
                logs
            }
            Err(e) => {
                warn!("Failed to get logs for container '{}': {}", c.name, e);
                format!("Failed to get logs for container '{}': {}\n", c.name, e)
            }
        };
        fs::write(&file_path, content)?;
    }
    info!("✓ All container logs persisted");
    Ok(())
}

/// Remove all foc-devnet containers (including the per-SP database containers)
/// that are not running.
pub fn remove_dead_foc_containers() -> Result<(), Box<dyn Error>> {
    let containers = list_foc_devnet_containers()?;
    let mut removed_count = 0;

    for c in containers {
        // Heuristic: remove if status contains "Exited" or "Dead" or "Created"
        let status_lower = c.status.to_lowercase();
        let is_dead = status_lower.contains("exited")
            || status_lower.contains("dead")
            || status_lower.contains("created")
            || status_lower.contains("removing")
            || status_lower.contains("paused");
        if is_dead {
            // Best-effort remove; ignore errors so cleanup continues
            match docker_command(&["rm", &c.name]) {
                Ok(_) => {
                    info!(
                        "✓ Removed dead container: {} (status: {})",
                        c.name, c.status
                    );
                    removed_count += 1;
                }
                Err(e) => {
                    warn!("Failed to remove container '{}': {}", c.name, e);
                }
            }
        }
    }
    info!("✓ Removed {} dead foc-devnet containers", removed_count);
    Ok(())
}

/// Write the output of `foc-devnet status` to the run's post-start status log file.
pub fn write_post_start_status_log(run_id: &str) -> Result<PathBuf, Box<dyn Error>> {
    let run_dir = foc_devnet_run_dir(run_id);
    fs::create_dir_all(&run_dir)?;
    let status_file = run_dir.join("post_start_status.log");

    info!("Writing post-start status to: {}", status_file.display());

    let exe = std::env::current_exe()?;
    let output = std::process::Command::new(exe).arg("status").output()?;

    let mut content = String::new();
    content.push_str(&String::from_utf8_lossy(&output.stdout));
    if !output.status.success() {
        content.push_str("\n[status command failed]\n");
        content.push_str(&String::from_utf8_lossy(&output.stderr));
    }

    fs::write(&status_file, content)?;
    info!("✓ Post-start status logged");
    Ok(status_file)
}
