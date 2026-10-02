//! CLI command dispatch.
//!
//! `main` stays thin: parse argv → `cli::run`. Commands live here and coordinate
//! `recipe` / `network` / `version` / `cache` / `local` / `installer`.

use crate::cache::{self, CachedApp, UpdateCache};
use crate::installer;
use crate::local;
use crate::network;
use crate::paths;
use crate::recipe;
use crate::version::{self, VersionComparison};
use std::io::Write;

/// Entry point from `main`. Returns a process exit code (0 = success).
pub fn run(args: &[String]) -> i32 {
    let command = args.get(1).map(String::as_str).unwrap_or("help");

    match command {
        "update" => {
            let is_verbose = args
                .iter()
                .any(|arg| arg == "-v" || arg == "--verbose" || arg == "-verbose");
            cmd_update(is_verbose)
        }
        "upgrade" => {
            let is_verbose = args
                .iter()
                .any(|arg| arg == "-v" || arg == "--verbose" || arg == "-verbose");
            let is_clean = args
                .iter()
                .any(|arg| arg == "-c" || arg == "--clean" || arg == "-clean");
            cmd_upgrade(is_verbose, is_clean)
        }
        "install" => {
            let is_verbose = args
                .iter()
                .any(|arg| arg == "-v" || arg == "--verbose" || arg == "-verbose");
            let is_clean = args
                .iter()
                .any(|arg| arg == "-c" || arg == "--clean" || arg == "-clean");
            let app = args.iter().skip(2).find(|arg| !arg.starts_with('-'));
            match app {
                Some(app) => cmd_install(app, is_verbose, is_clean),
                None => {
                    eprintln!("\x1b[31mUsage:\x1b[0m {} install [-v] [-c] <app>", paths::runner_cmd());
                    1
                }
            }
        },
        "uninstall" => match args.get(2) {
            Some(app) => cmd_uninstall(app),
            None => {
                eprintln!("\x1b[31mUsage:\x1b[0m {} uninstall <app>", paths::runner_cmd());
                1
            }
        },
        "cleanup" | "clean" => {
            let is_verbose = args
                .iter()
                .any(|arg| arg == "-v" || arg == "--verbose" || arg == "-verbose");
            let app = args.iter().skip(2).find(|arg| !arg.starts_with('-'));
            cmd_cleanup(app.map(|s| s.as_str()), is_verbose)
        },
        "list" => {
            let is_all = args.iter().any(|arg| arg == "--all" || arg == "-a" || arg == "all");
            let is_verbose = args
                .iter()
                .any(|arg| arg == "-v" || arg == "--verbose" || arg == "-verbose");
            if is_all {
                cmd_list_all(is_verbose)
            } else {
                cmd_list(is_verbose)
            }
        }
        "list-all" => {
            let is_verbose = args
                .iter()
                .any(|arg| arg == "-v" || arg == "--verbose" || arg == "-verbose");
            cmd_list_all(is_verbose)
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
    let recipes = recipe::load_recipes().unwrap_or_default();
    let tracked_names: Vec<String> = recipes
        .iter()
        .map(|r| {
            if r.aliases.is_empty() {
                r.name.clone()
            } else {
                format!("{} ({})", r.name, r.aliases.join(", "))
            }
        })
        .collect();

    let version = env!("CARGO_PKG_VERSION");
    let available_recipes = if tracked_names.is_empty() {
        "None found in recipes/".to_string()
    } else {
        tracked_names.join("\n  ")
    };

    let cmd = paths::runner_cmd();
    let mode_str = if paths::is_dev_mode() { " [dev mode]" } else { "" };

    println!(
        "\x1b[1mrorys-package-manager\x1b[0m (v{version}{mode_str}) — unsigned macOS app package manager

\x1b[1mUsage:\x1b[0m
  {cmd} <command> [app]

\x1b[1mCommands:\x1b[0m
  update [-v]            Check installed apps against upstream, evaluate versions, and write update cache
  upgrade [-v] [-c]      Download, verify SHA-256, and install pending updates (from cache)
  install [-v] [-c] <app> Install (or reinstall) a tracked app by name or alias (-c for clean install)
  uninstall <app>        Safely remove a managed app from /Applications
  cleanup [app] [-v]     Remove cached downloads from staging (or for a specific app)
  list [-v]              List installed packages and their versions (use -v or -verbose for details)
  list-all [-v]          List all available package recipes (use -v or -verbose for details)
  help                   Show this message

\x1b[1mAvailable Recipes:\x1b[0m
  {available_recipes}

\x1b[1mExamples:\x1b[0m
  {cmd} update
  {cmd} update -v
  {cmd} list
  {cmd} list -v
  {cmd} list-all
  {cmd} install qbt
  {cmd} install -c qbt
  {cmd} cleanup
  {cmd} cleanup librewolf
  {cmd} upgrade
  {cmd} uninstall qbittorrent
"
    );
}

fn print_recipe_entry(r: &recipe::Recipe, local_ver: &str, managed: bool) {
    let aliases_str = if r.aliases.is_empty() {
        String::new()
    } else {
        format!(" [aliases: {}]", r.aliases.join(", "))
    };

    let ver_badge = if local_ver == "Not Installed" {
        "[\x1b[90mNot Installed\x1b[0m]".to_string()
    } else {
        format!("[\x1b[32m{}\x1b[0m]", local_ver)
    };

    let status_suffix = if local_ver == "Not Installed" {
        ""
    } else if managed {
        " [managed]"
    } else {
        " [unmanaged]"
    };

    println!(
        "  • \x1b[1m{}\x1b[0m {}{} — \x1b[36m{}\x1b[0m (Status: {}{})",
        r.name,
        ver_badge,
        aliases_str,
        r.bundle_name,
        if local_ver == "Not Installed" {
            "\x1b[90mNot Installed\x1b[0m"
        } else {
            "\x1b[32mInstalled\x1b[0m"
        },
        status_suffix
    );
    if let Some(desc) = &r.description {
        println!("    \x1b[90m{}\x1b[0m", desc);
    }
}

/// Lists only packages that are currently installed locally.
fn cmd_list(verbose: bool) -> i32 {
    let recipes = match recipe::load_recipes() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\x1b[31mFailed to load recipes: {}\x1b[0m", e);
            return 1;
        }
    };

    let installed_recipes: Vec<(&recipe::Recipe, String, bool)> = recipes
        .iter()
        .filter_map(|r| {
            local::get_recipe_local_version(r).map(|ver| {
                let managed = cache::was_managed(&r.name);
                (r, ver, managed)
            })
        })
        .collect();

    if installed_recipes.is_empty() {
        println!("\x1b[1;34m==>\x1b[0m No packages currently installed.");
        println!("    Run \x1b[1m{} list-all\x1b[0m to see available packages to install.", paths::runner_cmd());
        return 0;
    }

    println!(
        "\x1b[1;34m==>\x1b[0m \x1b[1mInstalled Packages ({}):\x1b[0m",
        installed_recipes.len()
    );
    for (r, local_ver, managed) in &installed_recipes {
        if verbose {
            print_recipe_entry(r, local_ver, *managed);
        } else {
            println!("  • \x1b[1m{}\x1b[0m [\x1b[32m{}\x1b[0m]", r.name, local_ver);
        }
    }
    0
}

/// Lists all available package recipes and their local installation status.
fn cmd_list_all(verbose: bool) -> i32 {
    let recipes = match recipe::load_recipes() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\x1b[31mFailed to load recipes: {}\x1b[0m", e);
            return 1;
        }
    };

    println!(
        "\x1b[1;34m==>\x1b[0m \x1b[1mAvailable Package Recipes ({}):\x1b[0m",
        recipes.len()
    );
    for r in &recipes {
        let local_ver = local::get_recipe_local_version(r)
            .unwrap_or_else(|| "Not Installed".to_string());
        let managed = cache::was_managed(&r.name);
        if verbose {
            print_recipe_entry(r, &local_ver, managed);
        } else if local_ver == "Not Installed" {
            println!("  • \x1b[1m{}\x1b[0m [\x1b[90mNot Installed\x1b[0m]", r.name);
        } else {
            println!("  • \x1b[1m{}\x1b[0m [\x1b[32m{}\x1b[0m]", r.name, local_ver);
        }
    }
    0
}

/// `rpm upgrade` — read `local/cache.toml`, download + verify + install pending apps.
fn cmd_upgrade(verbose: bool, clean: bool) -> i32 {
    println!("\x1b[1;34m==>\x1b[0m \x1b[1mUpgrading apps from update cache...\x1b[0m");

    match installer::upgrade_pending(verbose, clean) {
        Ok(0) => {
            println!(
                "\x1b[1;34m==>\x1b[0m Nothing to upgrade. Run \x1b[1m{} update\x1b[0m if you expect changes.",
                paths::runner_cmd()
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
fn cmd_install(query: &str, verbose: bool, clean: bool) -> i32 {
    let recipes = match recipe::load_recipes() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\x1b[31mFailed to load recipes: {}\x1b[0m", e);
            return 1;
        }
    };

    let recipe = match recipe::find_recipe(query, &recipes) {
        Some(r) => r,
        None => {
            let available: Vec<&str> = recipes.iter().map(|r| r.name.as_str()).collect();
            eprintln!(
                "\x1b[31mUnknown app '{}'.\x1b[0m Available recipes: {}",
                query,
                available.join(", ")
            );
            return 1;
        }
    };

    if let Err(e) = installer::check_conflicting_apps(recipe, &recipes) {
        eprintln!("\x1b[31mConflict error:\x1b[0m {}", e);
        return 1;
    }

    let clean_bundle = if recipe.bundle_name.ends_with(".app") {
        recipe.bundle_name.clone()
    } else {
        format!("{}.app", recipe.bundle_name)
    };
    let target = std::path::PathBuf::from(format!("/Applications/{}", clean_bundle));
    let is_managed = cache::was_managed(&recipe.name);

    if target.exists() && !is_managed {
        let alias = recipe.aliases.first().map(|s| s.as_str()).unwrap_or(&recipe.name);
        eprintln!(
            "\x1b[31mError:\x1b[0m '{}' is already installed at /Applications/{} but is not managed by rpm.\n\
             For security and system integrity, rpm will not overwrite applications installed outside this package manager.\n\
             To manage '{}' with rpm:\n  \
             1. Uninstall or remove '/Applications/{}' (e.g. drag to Trash or run: rm -rf '/Applications/{}')\n  \
             2. Reinstall via rpm: {} install {}",
            recipe.name,
            clean_bundle,
            recipe.name,
            clean_bundle,
            clean_bundle,
            paths::runner_cmd(),
            alias
        );
        return 1;
    }

    if verbose {
        println!(
            "\x1b[1;34m==>\x1b[0m Resolving package \x1b[1m{}\x1b[0m...",
            recipe.name
        );
    }

    let (remote_version, download_url) = match network::resolve_recipe(recipe) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("\x1b[31mFailed to resolve release for {}: {}\x1b[0m", recipe.name, e);
            return 1;
        }
    };

    let local_version = local::get_recipe_local_version(recipe)
        .unwrap_or_else(|| "Not Installed".to_string());

    let comparison = version::compare(&local_version, &remote_version);
    if comparison == VersionComparison::UpToDate && is_managed && !clean {
        println!(
            "\x1b[1;34m==>\x1b[0m {} [{}] is already installed and current.",
            recipe.name, local_version
        );
        return 0;
    } else if comparison == VersionComparison::AheadOfUpstream && !clean {
        println!(
            "\x1b[33mNotice:\x1b[0m Local version [{}] is newer than upstream [{}].",
            local_version, remote_version
        );
    }

    match installer::install_recipe(recipe, &remote_version, &download_url, verbose, clean) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("\x1b[31minstall failed:\x1b[0m {}", e);
            1
        }
    }
}

/// `rpm cleanup [app]` — remove cached download archives from staging.
fn cmd_cleanup(app_query: Option<&str>, verbose: bool) -> i32 {
    match installer::cleanup_staging(app_query, verbose) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("\x1b[31mcleanup failed:\x1b[0m {}", e);
            1
        }
    }
}

/// `rpm uninstall <app>` — remove managed app from /Applications and cache.
fn cmd_uninstall(query: &str) -> i32 {
    let recipes = recipe::load_recipes().unwrap_or_default();
    let name = recipe::find_recipe(query, &recipes)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| query.to_string());

    match installer::uninstall_app(&name) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("\x1b[31muninstall failed:\x1b[0m {}", e);
            1
        }
    }
}

/// `rpm update` — check remotes, compare versions with >, and write `local/cache.toml`.
fn cmd_update(verbose: bool) -> i32 {
    if verbose {
        let arch = network::detect_mac_architecture();
        println!(
            "\x1b[1;34m==>\x1b[0m \x1b[1mDetected System Architecture:\x1b[0m \x1b[1;32m{}\x1b[0m",
            arch
        );
    }

    let recipes = match recipe::load_recipes() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\x1b[31mFailed to load recipes: {}\x1b[0m", e);
            return 1;
        }
    };

    let installed_recipes: Vec<(&recipe::Recipe, String)> = recipes
        .iter()
        .filter_map(|r| {
            local::get_recipe_local_version(r).map(|ver| (r, ver))
        })
        .collect();

    if installed_recipes.is_empty() {
        println!("\x1b[1;34m==>\x1b[0m No installed packages found to check.");
        println!("    Run \x1b[1m{} list-all\x1b[0m to see available packages to install.", paths::runner_cmd());
        return 0;
    }

    println!(
        "\x1b[1;34m==>\x1b[0m \x1b[1mChecking {} installed package(s)...\x1b[0m",
        installed_recipes.len()
    );

    let mut cached_apps: Vec<CachedApp> = Vec::new();
    let mut had_errors = false;

    for (recipe, local_version) in installed_recipes {
        print!("🔍 Checking {}... ", recipe.name);
        let _ = std::io::stdout().flush();

        let managed = cache::was_managed(&recipe.name);

        match network::resolve_recipe(recipe) {
            Ok((remote_version, download_url)) => {
                // If local_version was read without -X (e.g. "156.0.1"), but this app is managed
                // and remote_version has a revision suffix matching base local_version (e.g. "156.0.1-1"):
                let effective_local = if managed
                    && (remote_version.starts_with(&format!("{}-", local_version))
                        || remote_version.starts_with(&format!("{}_", local_version)))
                {
                    local::set_bundle_rpm_version(&recipe.bundle_name, &remote_version);
                    remote_version.clone()
                } else {
                    local_version.clone()
                };

                let comparison = version::compare(&effective_local, &remote_version);
                let update_available = match comparison {
                    VersionComparison::UpdateAvailable => managed,
                    _ => false,
                };

                if verbose {
                    match comparison {
                        VersionComparison::UpdateAvailable => {
                            if !managed {
                                println!(
                                    "\n  ⚠️  Local: [\x1b[33m{}\x1b[0m] → Upstream: [\x1b[32m{}\x1b[0m] (Unmanaged: installed outside rpm)",
                                    effective_local, remote_version
                                );
                                println!(
                                    "     \x1b[90mSkipping updates. To manage with rpm, uninstall it from /Applications and run '{} install {}'.\x1b[0m",
                                    paths::runner_cmd(),
                                    recipe.aliases.first().unwrap_or(&recipe.name.to_lowercase())
                                );
                            } else {
                                println!(
                                    "\n  🚀 Local: [\x1b[33m{}\x1b[0m] → Upstream: [\x1b[32m{}\x1b[0m] (Update available){}",
                                    effective_local,
                                    remote_version,
                                    if managed { " [managed]" } else { "" }
                                );
                            }
                        }
                        VersionComparison::UpToDate => {
                            println!(
                                "\n  ✨ Local: [\x1b[32m{}\x1b[0m] == Upstream: [\x1b[32m{}\x1b[0m] (Current){}",
                                effective_local,
                                remote_version,
                                if managed { " [managed]" } else { " [unmanaged]" }
                            );
                        }
                        VersionComparison::AheadOfUpstream => {
                            println!(
                                "\n  💎 Local: [\x1b[35m{}\x1b[0m] > Upstream: [\x1b[36m{}\x1b[0m] (Ahead of upstream / Dev build){}",
                                effective_local,
                                remote_version,
                                if managed { " [managed]" } else { " [unmanaged]" }
                            );
                        }
                        VersionComparison::NotInstalled => {}
                    }

                    println!("  🔗 \x1b[90m{}\x1b[0m", download_url);
                    if let Some(ref expected_hash) = recipe.sha256 {
                        println!("  🔒 Expected SHA-256: \x1b[90m{}\x1b[0m", expected_hash);
                    }
                } else {
                    match comparison {
                        VersionComparison::UpdateAvailable => {
                            println!(
                                "\n  Local: [\x1b[33m{}\x1b[0m] < Upstream: [\x1b[32m{}\x1b[0m]",
                                effective_local, remote_version
                            );
                        }
                        VersionComparison::UpToDate => {
                            println!(
                                "\n  Local: [\x1b[32m{}\x1b[0m] == Upstream: [\x1b[32m{}\x1b[0m]",
                                effective_local, remote_version
                            );
                        }
                        VersionComparison::AheadOfUpstream => {
                            println!(
                                "\n  Local: [\x1b[35m{}\x1b[0m] > Upstream: [\x1b[36m{}\x1b[0m]",
                                effective_local, remote_version
                            );
                        }
                        VersionComparison::NotInstalled => {}
                    }
                }

                cached_apps.push(CachedApp {
                    name: recipe.name.clone(),
                    local_version: effective_local,
                    remote_version,
                    download_url,
                    update_available,
                    managed,
                    sha256: recipe.sha256.clone(),
                });
            }
            Err(e) => {
                println!("\x1b[31mError ({})\x1b[0m", e);
                had_errors = true;
            }
        }
    }

    // Preserve any legacy managed entries from cache that are still installed but aren't in recipes
    if let Ok(previous) = cache::load_cache_or_empty() {
        for app in previous.apps {
            if app.managed
                && local::get_local_version(&app.name).is_some()
                && !cached_apps.iter().any(|a| a.name.eq_ignore_ascii_case(&app.name))
            {
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

    if verbose {
        println!(
            "\n\x1b[1;34m==>\x1b[0m Wrote update cache → \x1b[1m{}\x1b[0m",
            cache::cache_file_path().display()
        );
        println!(
            "\x1b[1;34m==>\x1b[0m Managed installs: \x1b[1m{}\x1b[0m",
            managed_count
        );
    } else {
        println!();
    }

    if pending == 0 {
        println!("\x1b[1;34m==>\x1b[0m All tracked apps are up to date.");
    } else {
        println!(
            "\x1b[1;34m==>\x1b[0m {} update(s) available. Run \x1b[1m{} upgrade\x1b[0m when ready.",
            pending,
            paths::runner_cmd()
        );
    }

    if had_errors {
        1
    } else {
        0
    }
}
