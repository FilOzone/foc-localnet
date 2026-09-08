//! Proof parameters management for genesis preparation.
//!
//! This module handles downloading and caching Filecoin proof parameters
//! required for lotus operations.

use crate::docker::push_bind_mount;
use crate::paths::{
    foc_devnet_bin, foc_devnet_proof_parameters, CONTAINER_FILECOIN_PROOF_PARAMS_PATH,
};
use crate::utils::retry::{retry_with_fixed_delay, DEFAULT_MAX_RETRIES, DEFAULT_RETRY_DELAY_SECS};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// S3 URL for pre-packaged Filecoin proof parameters (2KiB sectors)
const PROOF_PARAMS_S3_URL: &str =
    "https://fil-proof-params-2k-cache.s3.us-east-2.amazonaws.com/filecoin-proof-params-2k.tar";

/// Ensure Filecoin proof parameters are downloaded.
///
/// Parameters are downloaded once and cached in /var/tmp/filecoin-proof-parameters/
/// This directory is mounted into lotus containers at /var/tmp/filecoin-proof-parameters/
pub fn ensure_proof_parameters() -> Result<(), Box<dyn std::error::Error>> {
    let params_dir = foc_devnet_proof_parameters();

    // Check if parameters already exist
    if dir_has_entries(&params_dir)? {
        info!(
            "✓ Proof parameters already exist at: {}",
            params_dir.display()
        );
        return Ok(());
    }

    info!("Proof parameters not found at: {}", params_dir.display());

    info!("⬇ Downloading proof parameters (this may take a while)...");

    // Try primary method: lotus fetch-params
    let primary_result = download_via_lotus_fetch_params(&params_dir);

    match primary_result {
        Ok(_) => {
            info!("✓ Proof parameters downloaded successfully via lotus fetch-params");
            return Ok(());
        }
        Err(e) => {
            warn!("Primary download method (lotus fetch-params) failed: {}", e);
            warn!("Falling back to S3 tarball download...");
        }
    }

    // Fallback: Download from S3
    download_from_s3(&params_dir)?;

    info!("✓ Proof parameters downloaded successfully via S3 fallback");
    Ok(())
}

/// Download proof parameters using lotus fetch-params.
///
/// This is the primary download method that uses the lotus binary's
/// built-in parameter fetching functionality.
fn download_via_lotus_fetch_params(params_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Retry the download operation in case of network issues
    retry_with_fixed_delay(
        || {
            let staging_dir = proof_params_staging_dir("foc-proof-params-fetch-", params_dir)?;
            let staging_path = staging_dir.path().to_path_buf();

            // Run lotus fetch-params in builder container
            let bin_dir = foc_devnet_bin();

            // Create a progress bar
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
            );

            let start_time = Instant::now();
            let staging_path_for_progress = staging_path.clone();
            let stop_progress = Arc::new(AtomicBool::new(false));
            let stop_progress_clone = Arc::clone(&stop_progress);

            // Spawn a thread to update progress by monitoring directory size
            let pb_clone = pb.clone();
            let update_handle = thread::spawn(move || {
                while !stop_progress_clone.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(500));

                    // Calculate directory size
                    if let Ok(size) = get_dir_size(&staging_path_for_progress) {
                        let elapsed = start_time.elapsed().as_secs_f64();
                        if elapsed > 0.0 {
                            let speed_mbps = (size as f64 / 1_048_576.0) / elapsed;
                            let total_mb = size as f64 / 1_048_576.0;
                            pb_clone.set_message(format!(
                                "Downloaded {:.1} MB ({:.2} MB/s)",
                                total_mb, speed_mbps
                            ));
                        }
                    }

                    pb_clone.tick();
                }
            });

            let container_name = format!(
                "foc-proof-params-fetch-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs()
            );
            let mut docker_args = vec![
                "run".to_string(),
                "--name".to_string(),
                container_name,
                "-e".to_string(),
                format!(
                    "FIL_PROOFS_PARAMETER_CACHE={}",
                    CONTAINER_FILECOIN_PROOF_PARAMS_PATH
                ),
            ];
            push_bind_mount(&mut docker_args, &bin_dir, "/output")?;
            push_bind_mount(
                &mut docker_args,
                &staging_path,
                CONTAINER_FILECOIN_PROOF_PARAMS_PATH,
            )?;
            docker_args.extend([
                crate::constants::BUILDER_DOCKER_IMAGE.to_string(),
                "/bin/bash".to_string(),
                "-c".to_string(),
                format!(
                    "/output/lotus fetch-params {}",
                    super::constants::PROOF_PARAMS_SECTOR_SIZE
                ),
            ]);
            let child = match Command::new("docker")
                .args(&docker_args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    stop_progress.store(true, Ordering::Relaxed);
                    pb.finish_and_clear();
                    let _ = update_handle.join();
                    return Err(error.into());
                }
            };

            let output = match child.wait_with_output() {
                Ok(output) => output,
                Err(error) => {
                    stop_progress.store(true, Ordering::Relaxed);
                    pb.finish_and_clear();
                    let _ = update_handle.join();
                    return Err(error.into());
                }
            };

            stop_progress.store(true, Ordering::Relaxed);
            pb.finish_and_clear();
            let _ = update_handle.join();

            if !output.status.success() {
                return Err(format!(
                    "Failed to download proof parameters: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
                .into());
            }

            if !dir_has_entries(&staging_path)? {
                return Err("lotus fetch-params produced no files".into());
            }

            merge_dir_contents(&staging_path, params_dir)?;

            Ok(())
        },
        DEFAULT_MAX_RETRIES,
        DEFAULT_RETRY_DELAY_SECS,
        "Proof parameters download",
    )
}

/// Download proof parameters from S3 as a fallback.
///
/// This method downloads a pre-packaged tarball of proof parameters from S3,
/// extracts it, and places the files in the correct location.
fn download_from_s3(params_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let staging_dir = proof_params_staging_dir("foc-proof-params-s3-", params_dir)?;
    let tarball_path = staging_dir.path().join("filecoin-proof-params-2k.tar");
    let extract_dir = staging_dir.path().join("extracted");
    fs::create_dir_all(&extract_dir)?;

    // Download tarball with retry
    retry_with_fixed_delay(
        || {
            info!("Downloading proof parameters tarball from S3...");

            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
            );
            pb.set_message("Downloading tarball from S3...");

            let output = Command::new("curl")
                .args([
                    "-L",
                    PROOF_PARAMS_S3_URL,
                    "-o",
                    &tarball_path.to_string_lossy(),
                ])
                .output()?;

            pb.finish_and_clear();

            if !output.status.success() {
                // Clean up failed download
                if tarball_path.exists() {
                    let _ = fs::remove_file(&tarball_path);
                }
                return Err(format!(
                    "Failed to download tarball from S3: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
                .into());
            }

            Ok(())
        },
        DEFAULT_MAX_RETRIES,
        DEFAULT_RETRY_DELAY_SECS,
        "S3 tarball download",
    )?;

    // Extract tarball
    info!("Extracting proof parameters tarball...");
    let extract_output = Command::new("tar")
        .args([
            "-xf",
            &tarball_path.to_string_lossy(),
            "-C",
            &extract_dir.to_string_lossy(),
        ])
        .output()?;

    if !extract_output.status.success() {
        let _ = fs::remove_file(&tarball_path);
        return Err(format!(
            "Failed to extract tarball: {}",
            String::from_utf8_lossy(&extract_output.stderr)
        )
        .into());
    }

    // Clean up tarball after successful extraction
    if tarball_path.exists() {
        fs::remove_file(&tarball_path)?;
    }

    // Verify extraction succeeded by checking for files
    if !dir_has_entries(&extract_dir)? {
        return Err("Tarball extraction produced no files".into());
    }

    merge_dir_contents(&extract_dir, params_dir)?;

    info!("Proof parameters extracted successfully");
    Ok(())
}

/// Calculate total size of a directory recursively
fn get_dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total_size = 0u64;

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;

            if metadata.is_dir() {
                total_size += get_dir_size(&entry.path())?;
            } else {
                total_size += metadata.len();
            }
        }
    }

    Ok(total_size)
}

fn dir_has_entries(path: &Path) -> std::io::Result<bool> {
    Ok(path.is_dir() && path.read_dir()?.next().is_some())
}

fn proof_params_staging_dir(
    prefix: &str,
    params_dir: &Path,
) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let staging_parent = params_dir.parent().unwrap_or_else(|| Path::new("/var/tmp"));
    fs::create_dir_all(staging_parent)?;

    Ok(tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(staging_parent)?)
}

fn merge_dir_contents(src: &Path, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if path.is_dir() {
            merge_dir_contents(&path, &dest_path)?;
            continue;
        }

        if dest_path.exists() {
            warn!(
                "Proof parameter file already exists, keeping existing file: {}",
                dest_path.display()
            );
            continue;
        }

        match fs::rename(&path, &dest_path) {
            Ok(()) => {}
            Err(_) => {
                fs::copy(&path, &dest_path)?;
                fs::remove_file(&path)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{dir_has_entries, merge_dir_contents, proof_params_staging_dir};
    use std::fs;

    #[test]
    fn dir_has_entries_is_false_for_missing_directory() {
        let root = tempfile::tempdir().unwrap();
        assert!(!dir_has_entries(&root.path().join("missing")).unwrap());
    }

    #[test]
    fn merge_dir_contents_keeps_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("param"), "new").unwrap();
        fs::write(dst.join("param"), "existing").unwrap();

        merge_dir_contents(&src, &dst).unwrap();

        assert_eq!(fs::read_to_string(dst.join("param")).unwrap(), "existing");
    }

    #[test]
    fn merge_dir_contents_copies_nested_files() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("nested").join("param"), "contents").unwrap();

        merge_dir_contents(&src, &dst).unwrap();

        assert_eq!(
            fs::read_to_string(dst.join("nested").join("param")).unwrap(),
            "contents"
        );
    }

    #[test]
    fn proof_params_staging_dir_uses_params_parent() {
        let root = tempfile::tempdir().unwrap();
        let params_dir = root.path().join("filecoin-proof-parameters");

        let staging_dir = proof_params_staging_dir("test-proof-params-", &params_dir).unwrap();

        assert_eq!(staging_dir.path().parent().unwrap(), root.path());
    }
}
