use crate::cache::{self, CachedApp, UpdateCache};
use std::fs::File;
use std::io::copy;
use std::path::Path;
use std::time::Duration;

/// Reads the update cache that `update` wrote to `local/cache.json`.
/// Upgrade / install flows should start here instead of re-querying the network.
pub fn load_update_cache() -> Result<UpdateCache, String> {
    cache::load_cache()
}

/// Returns only apps marked update_available with a non-empty download URL.
pub fn apps_ready_to_upgrade() -> Result<Vec<CachedApp>, String> {
    let cache = load_update_cache()?;
    Ok(cache::pending_upgrades(&cache)
        .into_iter()
        .cloned()
        .collect())
}

/// Downloads a file from a URL and streams it to `destination_path`.
pub fn download_file(url: &str, destination_path: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("Failed to create download client: {}", e))?;

    println!("    📥 Downloading asset payload...");

    let mut response = client
        .get(url)
        .header("User-Agent", "rorys-package-manager-cli")
        .send()
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Server returned HTTP error code: {}",
            response.status()
        ));
    }

    let mut dest_file = File::create(destination_path)
        .map_err(|e| format!("Failed to create local staging file: {}", e))?;

    copy(&mut response, &mut dest_file)
        .map_err(|e| format!("Failed to write stream bytes to disk: {}", e))?;

    println!("    💾 Download complete.");
    Ok(())
}
