mod installer;
mod local;
mod network;

use std::path::Path;

fn main() {
    // Beautiful pip-like UI style formatting
    println!(
        "\x1b[1;34m==>\x1b[0m \x1b[1mChecking local apps against upstream distributions...\x1b[0m"
    );

    let apps = vec![
        ("Chromium", "chromium", "https://googleapis.com"),
        ("LibreWolf", "codeberg", "librewolf/source"),
        ("qBittorrent", "github", "qbittorrent/qBittorrent"),
    ];

    for (name, source_type, locator) in apps {
        // 1. Fetch current local system state
        let local_v = local::get_local_version(name).unwrap_or_else(|| "Not Installed".to_string());

        // 2. Fetch remote live deployment state
        let upstream_v = match source_type {
            "github" => network::fetch_github_version(locator),
            "chromium" => network::fetch_chromium_version("chromium"),
            "codeberg" => network::fetch_codeberg_version(locator),
            "url" => network::fetch_raw_url_version(locator),
            _ => Err("Invalid configuration target configuration type".to_string()),
        };

        // 3. Compare data deltas with proper formatting
        match upstream_v {
            Ok(remote_v) => {
                if local_v == remote_v {
                    println!(
                        "  ✨ {:<12} Local: \x1b[32m{:<15}\x1b[0m | Upstream: \x1b[32m{:<15}\x1b[0m (Current)",
                        name, local_v, remote_v
                    );
                } else {
                    println!(
                        "  🚀 {:<12} Local: \x1b[33m{:<15}\x1b[0m | Upstream: \x1b[32m{:<15}\x1b[0m (Update Available!)",
                        name, local_v, remote_v
                    );
                }
            }
            Err(e) => {
                println!(
                    "  ❌ {:<12} Local: {:<15} | Upstream: \x1b[31mError ({})\x1b[0m",
                    name, local_v, e
                );
            }
        }
    }

    // --- QUICK DOWNLOADER TEST BLOCK ---
    println!("\n\x1b[1;34m==>\x1b[0m \x1b[1mTesting Installer Engine Downloader...\x1b[0m");

    // We will download a small sample text file to verify your streaming IO works
    let test_url = "https://w3.org";
    let test_destination = Path::new("./test_download.txt");

    match installer::download_file(test_url, test_destination) {
        Ok(()) => {
            println!(
                "  ✅ Success! File downloaded safely to {:?}",
                test_destination
            );
            // Clean up the test file after ourselves
            let _ = std::fs::remove_file(test_destination);
        }
        Err(e) => println!("  ❌ Downloader engine failed: {}", e),
    }
}
