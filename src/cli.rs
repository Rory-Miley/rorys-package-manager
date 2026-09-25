//! CLI command dispatch.
//!
//! `main` stays thin: parse argv → `cli::run`. Commands live here and call
//! `network` / `cache` / `local` / `installer` — not the other way around.

use crate::cache::{self, CachedApp, UpdateCache};
use crate::installer;
use crate::local;
use crate::network;

/// Hardcoded catalog for now — later this moves to a user JSON config.
/// Tuple: (display name, source type, locator)
fn tracked_apps() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![("qBittorrent", "github", "qbittorrent/qBittorrent")]
}

fn resolve_catalog(
    query: &str,
) -> Result<(&'static str, &'static str, &'static str), String> {
    tracked_apps()
        .into_iter()
        .find(|(name, _, _)| name.eq_ignore_ascii_case(query))
        .ok_or_else(|| {
            let known: Vec<&str> = tracked_apps().iter().map(|(n, _, _)| *n).collect();
            format!(
                "Unknown app '{}'. Tracked apps: {}",
                query,
                known.join(", ")
            )
        })
}

/// Entry point from `main`. Returns a process exit code (0 = success).
pub fn run(args: &[String]) -> i32 {
    let command = args.get(1).map(String::as_str).unwrap_or("help");

    match command {
        "update" => cmd_update(),
        "upgrade" => cmd_upgrade(),
        "install" => match args.get(2) {
            Some(app) => cmd_install(app),
            None => {
                eprintln!("\x1b[31mUsage:\x1b[0m cargo run -- install <app>");
                1
            }
        },
        "uninstall" => match args.get(2) {
            Some(app) => cmd_uninstall(app),
            None => {
                eprintln!("\x1b[31mUsage:\x1b[0m cargo run -- uninstall <app>");
                1
            }
        },
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
  cargo run -- <command> [app]

\x1b[1mCommands:\x1b[0m
  update              Check apps against upstream and write local/cache.json
  upgrade             Download and install pending updates (from cache)
  install <app>       Install (or reinstall) a tracked app
  uninstall <app>     Remove a managed app from /Applications
  help                Show this message

\x1b[1mExamples:\x1b[0m
  cargo run -- update
  cargo run -- install qbittorrent
  cargo run -- uninstall qbittorrent
"
    );
}

/// `rpm upgrade` — read `local/cache.json`, download + install pending apps, strip quarantine.
fn cmd_upgrade() -> i32 {
    println!("\x1b[1;34m==>\x1b[0m \x1b[1mUpgrading apps from update cache...\x1b[0m");

    match installer::upgrade_pending() {
        Ok(0) => {
            println!(
                "\x1b[1;34m==>\x1b[0m Nothing to upgrade. Run \x1b[1mcargo run -- update\x1b[0m if you expect changes."
            );
            0
        }
        Ok(n) => {
            println!(
                "\x1b[1;34m==>\x1b[0m \x1b[32mSuccessfully upgraded {} app(s).\x1b[0m",
                n
            );
            0
        }
        Err(e) => {
            eprintln!("\x1b[31mupgrade failed:\x1b[0m {}", e);
            1
        }
    }
}

/// `rpm install <app>` — fetch latest release and install, then mark managed in cache.
fn cmd_install(query: &str) -> i32 {
    let (name, source_type, locator) = match resolve_catalog(query) {
        Ok(entry) => entry,
        Err(e) => {
            eprintln!("\x1b[31m{}\x1b[0m", e);
            return 1;
        }
    };

    let remote = match source_type {
        "github" => network::fetch_github_release(locator),
        "codeberg" => network::fetch_codeberg_release(locator),
        other => Err(format!("Unsupported source type: {}", other)),
    };

    let (remote_version, download_url) = match remote {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("\x1b[31mFailed to fetch release:\x1b[0m {}", e);
            return 1;
        }
    };

    let local_version =
        local::get_local_version(name).unwrap_or_else(|| "Not Installed".to_string());
    if local_version == remote_version && cache::was_managed(name) {
        println!(
            "\x1b[1;34m==>\x1b[0m {} {} is already installed (managed).",
            name, local_version
        );
        return 0;
    }

    match installer::install_app(name, &remote_version, &download_url) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("\x1b[31minstall failed:\x1b[0m {}", e);
            1
        }
    }
}

/// `rpm uninstall <app>` — remove managed app from /Applications and cache.
fn cmd_uninstall(query: &str) -> i32 {
    // Prefer canonical catalog name when known; otherwise pass query through for cache lookup.
    let name = resolve_catalog(query)
        .map(|(n, _, _)| n.to_string())
        .unwrap_or_else(|_| query.to_string());

    match installer::uninstall_app(&name) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("\x1b[31muninstall failed:\x1b[0m {}", e);
            1
        }
    }
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
        let managed = cache::was_managed(name);

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
                        "\n  🚀 Local: \x1b[33m{}\x1b[0m → Upstream: \x1b[32m{}\x1b[0m{}",
                        local_version,
                        remote_version,
                        if managed { " [managed]" } else { "" }
                    );
                } else {
                    println!(
                        "\n  ✨ Local: \x1b[32m{}\x1b[0m | Upstream: \x1b[32m{}\x1b[0m (Current){}",
                        local_version,
                        remote_version,
                        if managed { " [managed]" } else { "" }
                    );
                }
                println!("  🔗 \x1b[90m{}\x1b[0m", download_url);

                cached_apps.push(CachedApp {
                    name: name.to_string(),
                    local_version,
                    remote_version,
                    download_url,
                    update_available,
                    managed,
                });
            }
            Err(e) => {
                println!("\x1b[31mError ({})\x1b[0m", e);
                had_errors = true;
            }
        }
    }

    // Preserve any managed entries that aren't in the hardcoded catalog anymore.
    if let Ok(previous) = cache::load_cache_or_empty() {
        for app in previous.apps {
            if app.managed && !cached_apps.iter().any(|a| a.name == app.name) {
                cached_apps.push(app);
            }
        }
    }

    let update_cache = UpdateCache::new(cached_apps);
    let pending = cache::pending_upgrades(&update_cache).len();
    let managed_count = cache::managed_apps(&update_cache).len();

    if let Err(e) = cache::save_cache(&update_cache) {
        eprintln!("\n\x1b[31mFailed to write cache: {}\x1b[0m", e);
        return 1;
    }

    println!(
        "\n\x1b[1;34m==>\x1b[0m Wrote update cache → \x1b[1m{}\x1b[0m",
        cache::CACHE_PATH
    );
    println!(
        "\x1b[1;34m==>\x1b[0m Managed installs: \x1b[1m{}\x1b[0m",
        managed_count
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
