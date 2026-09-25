//! CLI command dispatch.
//!
//! `main` stays thin: parse argv → `cli::run`. Commands live here and call
//! `network` / `cache` / `local` / `installer` — not the other way around.

use crate::cache::{self, CachedApp, UpdateCache};
use crate::local;
use crate::network;

/// Hardcoded catalog for now — later this moves to a user JSON config.
fn tracked_apps() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![("qBittorrent", "github", "qbittorrent/qBittorrent")]
}

/// Entry point from `main`. Returns a process exit code (0 = success).
pub fn run(args: &[String]) -> i32 {
    let command = args.get(1).map(String::as_str).unwrap_or("help");

    match command {
        "update" => cmd_update(),
        "upgrade" => {
            eprintln!(
                "\x1b[33mupgrade\x1b[0m is not implemented yet. Run \x1b[1mupdate\x1b[0m first."
            );
            1
        }
        "help" | "--help" | "-h" => {
            print_help();
            0
        }
        other => {
            eprintln!("\x1b[31mUnknown command:\x1b[0m {}", other);
            print_help();
            1
        }
    }
}

fn print_help() {
    println!(
        "\x1b[1mrorys-package-manager\x1b[0m — unsigned macOS app updater

\x1b[1mUsage:\x1b[0m
  cargo run -- <command>

\x1b[1mCommands:\x1b[0m
  update    Check apps against upstream and write local/cache.json
  upgrade   Download and install pending updates (from cache)  [todo]
  help      Show this message
"
    );
}

/// `rpm update` — check remotes, print status, write `local/cache.json`.
fn cmd_update() -> i32 {
    let arch = network::detect_mac_architecture();
    println!(
        "\x1b[1;34m==>\x1b[0m \x1b[1mDetected System Architecture:\x1b[0m \x1b[1;32m{}\x1b[0m",
        arch
    );
    println!("\x1b[1;34m==>\x1b[0m \x1b[1mChecking apps and writing update cache...\x1b[0m");

    let mut cached_apps: Vec<CachedApp> = Vec::new();
    let mut had_errors = false;

    for (name, source_type, locator) in tracked_apps() {
        print!("🔍 Checking {}... ", name);

        let local_version =
            local::get_local_version(name).unwrap_or_else(|| "Not Installed".to_string());

        let remote = match source_type {
            "github" => network::fetch_github_release(locator),
            "codeberg" => network::fetch_codeberg_release(locator),
            other => Err(format!(
                "Unsupported source type for cache export: {}",
                other
            )),
        };

        match remote {
            Ok((remote_version, download_url)) => {
                let update_available = local_version != remote_version;
                if update_available {
                    println!(
                        "\n  🚀 Local: \x1b[33m{}\x1b[0m → Upstream: \x1b[32m{}\x1b[0m",
                        local_version, remote_version
                    );
                } else {
                    println!(
                        "\n  ✨ Local: \x1b[32m{}\x1b[0m | Upstream: \x1b[32m{}\x1b[0m (Current)",
                        local_version, remote_version
                    );
                }
                println!("  🔗 \x1b[90m{}\x1b[0m", download_url);

                cached_apps.push(CachedApp {
                    name: name.to_string(),
                    local_version,
                    remote_version,
                    download_url,
                    update_available,
                });
            }
            Err(e) => {
                println!("\x1b[31mError ({})\x1b[0m", e);
                had_errors = true;
            }
        }
    }

    let update_cache = UpdateCache::new(cached_apps);
    let pending = cache::pending_upgrades(&update_cache).len();

    if let Err(e) = cache::save_cache(&update_cache) {
        eprintln!("\n\x1b[31mFailed to write cache: {}\x1b[0m", e);
        return 1;
    }

    println!(
        "\n\x1b[1;34m==>\x1b[0m Wrote update cache → \x1b[1m{}\x1b[0m",
        cache::CACHE_PATH
    );

    if pending == 0 {
        println!("\x1b[1;34m==>\x1b[0m All tracked apps are up to date.");
    } else {
        println!(
            "\x1b[1;34m==>\x1b[0m {} update(s) available. Run \x1b[1mcargo run -- upgrade\x1b[0m when ready.",
            pending
        );
    }

    if had_errors {
        1
    } else {
        0
    }
}
