mod installer;
mod local;
mod network;

fn main() {
    let arch = network::detect_mac_architecture();
    println!(
        "\x1b[1;34m==>\x1b[0m \x1b[1mDetected System Architecture:\x1b[0m \x1b[1;32m{}\x1b[0m",
        arch
    );
    println!(
        "\x1b[1;34m==>\x1b[0m \x1b[1mChecking local apps against upstream distributions...\x1b[0m"
    );

    let apps = vec![
        ("Chromium", "chromium", "https://googleapis.com"),
        ("LibreWolf", "codeberg", "librewolf/source"),
        ("qBittorrent", "github", "qbittorrent/qBittorrent"),
    ];
    let giteo_apps = vec![("qBittorrent", "github", "qbittorrent/qBittorrent")];

    for (name, repo_type, repo) in giteo_apps {
        print!("🔍 Locating best matched release asset for {}... ", name);

        match network::fetch_github_release_details(repo) {
            Ok((version, url)) => {
                println!("\n  ✨ Found version: \x1b[32m{}\x1b[0m", version);
                println!(
                    "  🔗 Target Architecture Asset Link:\n     \x1b[90m{}\x1b[0m\n",
                    url
                );
            }
            Err(e) => println!("\x1b[31mError ({})\x1b[0m", e),
        }
    }

    /*
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
    }*/
}
