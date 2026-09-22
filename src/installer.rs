use std::fs::File;
use std::io::copy;
use std::path::Path;
use std::time::Duration;

/// Downloads a file from a specified URL and saves it directly to a local target path.
/// Handles multi-megabyte files safely by streaming the response body to disk.
pub fn download_file(url: &str, destination_path: &Path) -> Result<(), String> {
    // 1. Build our synchronous client matching your network module standard
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300)) // Generous 5-minute timeout for heavy app downloads
        .build()
        .map_err(|e| format!("Failed to create download client: {}", e))?;

    println!("    📥 Downloading asset payload...");

    // 2. Fire the request
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

    // 3. Create the destination file on your hard drive
    let mut dest_file = File::create(destination_path)
        .map_err(|e| format!("Failed to create local staging file: {}", e))?;

    // 4. Stream the data directly from the network socket into the file buffer
    copy(&mut response, &mut dest_file)
        .map_err(|e| format!("Failed to write stream bytes to disk: {}", e))?;

    println!("    💾 Download complete.");
    Ok(())
}
