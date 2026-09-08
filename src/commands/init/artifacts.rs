//! Artifact staging utilities for foc-devnet initialization.
//!
//! Handles staging optional local artifacts, currently the filecoin proof
//! parameters.

use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::Path;
use tracing::info;

use crate::paths::{foc_devnet_artifacts, foc_devnet_proof_parameters};

/// Stage optional local artifacts for foc-devnet.
///
/// # Arguments
/// * `proof_params_dir` - Optional path to a local filecoin-proof-params directory
///
/// # Returns
/// Returns `Ok(())` if staging succeeds, or an error if it fails.
pub fn stage_artifacts(proof_params_dir: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    // Ensure the artifacts directory exists for anything staged later
    // (e.g. the Foundry toolchain installed by the scenario prerequisites).
    fs::create_dir_all(foc_devnet_artifacts())?;

    if let Some(params_path) = proof_params_dir {
        copy_proof_params_from_local(&params_path)?;
    }

    Ok(())
}

/// Copy filecoin-proof-params from a local directory into the global cache.
///
/// This function copies proof parameters from a local directory instead of downloading them.
///
/// # Arguments
/// * `params_path` - Path to the local proof parameters directory
///
/// # Returns
/// Returns `Ok(())` if copy succeeds, or an error if it fails.
fn copy_proof_params_from_local(params_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let params_path = Path::new(params_path);
    if !params_path.exists() {
        return Err(format!(
            "Local proof parameters not found: {}",
            params_path.display()
        )
        .into());
    }

    info!(
        "Copying proof parameters from local directory: {}",
        params_path.display()
    );

    let dest_path = foc_devnet_proof_parameters();

    // Create destination directory
    fs::create_dir_all(&dest_path)?;

    if params_path.canonicalize()? == dest_path.canonicalize()? {
        info!(
            "Proof parameters already use the global cache: {}",
            dest_path.display()
        );
        return Ok(());
    }

    // Copy all files from source to destination
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    pb.set_message("Copying proof parameters...");

    copy_dir_recursive(params_path, &dest_path)?;

    pb.finish_with_message("✓ Proof parameters copied");

    info!("Proof parameters copied successfully");
    Ok(())
}

/// Recursively copy a directory.
///
/// # Arguments
/// * `src` - Source directory path
/// * `dst` - Destination directory path
///
/// # Returns
/// Returns `Ok(())` if copy succeeds, or an error if it fails.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dst.join(&file_name);

        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)?;
        }
    }

    Ok(())
}
