//! CLI command dispatch.
//!
//! `main` stays thin: parse argv → `cli::run`. Commands live here and coordinate
//! `recipe` / `network` / `version` / `cache` / `local` / `installer`.

use crate::cache::{self, CachedApp, UpdateCache};
use crate::config;
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
            let track_override = if args.iter().any(|arg| arg == "--upstream" || arg == "-upstream") {
                Some(config::HomebrewTrack::Upstream)
            } else if args.iter().any(|arg| arg == "--verified" || arg == "-verified" || arg == "--default" || arg == "-default") {
                Some(config::HomebrewTrack::Default)
            } else {
                None
            };
            let app = args.iter().skip(2).find(|arg| !arg.starts_with('-'));
            match app {
                Some(app) => cmd_install(app, is_verbose, is_clean, track_override),
                None => {
                    eprintln!("\x1b[31mUsage:\x1b[0m {} install [-v] [-c] [--upstream | --verified] <app>", paths::runner_cmd());
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
        "homebrew" => {
            cmd_homebrew(args)
        },
        "list" | "-list" | "--list" => {
            let is_verbose = args
                .iter()
                .any(|arg| arg == "-v" || arg == "--verbose" || arg == "-verbose");
            cmd_list(is_verbose)
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
  install [-v] [-c] [--upstream | --verified] <app>
                         Install a package on-demand (-c for clean install)
  uninstall <app>        Safely remove a managed app from /Applications
  cleanup [app] [-v]     Remove cached downloads from staging (or for a specific app)
  list [-v]              List installed packages and their versions (use -v or -verbose for details)
  homebrew [-default | -upstream | -request]
                         View or set default download verification track for Homebrew casks
  help                   Show this message

\x1b[1mAvailable Recipes:\x1b[0m
  {available_recipes}

\x1b[1mExamples:\x1b[0m
  {cmd} update
  {cmd} update -v
  {cmd} list
  {cmd} list -v
  {cmd} homebrew
  {cmd} homebrew -upstream
  {cmd} install vlc
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

    let origin_str = format!(" {}", r.origin().colored_badge());

    println!(
        "  • \x1b[1m{}\x1b[0m {}{} — \x1b[36m{}\x1b[0m (Status: {}{}){}",
        r.name,
        ver_badge,
        aliases_str,
        r.bundle_name,
        if local_ver == "Not Installed" {
            "\x1b[90mNot Installed\x1b[0m"
        } else {
            "\x1b[32mInstalled\x1b[0m"
        },
        status_suffix,
        origin_str
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
        println!("    Run \x1b[1m{} install <app>\x1b[0m to install a package.", paths::runner_cmd());
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

/// `rpm homebrew [-default | -upstream | -request]` — view or configure default Homebrew download verification track.
fn cmd_homebrew(args: &[String]) -> i32 {
    let mut config = config::Config::load_or_default();
    let track_arg = args.iter().skip(2).find(|arg| !arg.is_empty());

    match track_arg {
        Some(arg) => {
            if let Some(track) = config::HomebrewTrack::parse(arg) {
                config.homebrew.track = track;
                if let Err(e) = config.save() {
                    eprintln!("\x1b[31mFailed to save config: {}\x1b[0m", e);
                    return 1;
                }
                println!(
                    "\x1b[1;34m==>\x1b[0m Homebrew default track set to: \x1b[1;32m{}\x1b[0m",
                    track.as_str()
                );
                0
            } else {
                eprintln!(
                    "\x1b[31mInvalid track option '{}'.\x1b[0m Valid options: default (verified sha), upstream (zero-day), request (prompt on install)",
                    arg
                );
                1
            }
        }
        None => {
            println!(
                "\x1b[1;34m==>\x1b[0m Active Homebrew Track: \x1b[1;32m{}\x1b[0m",
                config.homebrew.track.as_str()
            );
            println!("    Available tracks:");
            println!("      • \x1b[1mdefault\x1b[0m  — Homebrew verified SHA-256 (maximum security)");
            println!("      • \x1b[1mupstream\x1b[0m — Direct developer release (zero-day updates)");
            println!("      • \x1b[1mrequest\x1b[0m  — Prompt interactively on each Homebrew cask install");
            println!("\n    To change track: {} homebrew [-default | -upstream | -request]", paths::runner_cmd());
            0
        }
    }
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

fn prompt_user_for_track(app_name: &str) -> config::HomebrewTrack {
    println!("\nSelect verification track for '{}':", app_name);
    println!("  [1] Homebrew Verified (Recommended)");
    println!("      • Pre-verified SHA-256 checksum vetted by Homebrew CI");
    println!("      • Protection against compromised or modified downloads");
    println!("  [2] Direct Upstream (Zero-Day)");
    println!("      • Direct from developer releases, checksum computed on download");
    println!("      • Instant zero-day updates without waiting for Homebrew vetting");
    print!("Choose [1/2] (default: 1): ");
    let _ = std::io::stdout().flush();

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        let trimmed = input.trim();
        if trimmed == "2" || trimmed.eq_ignore_ascii_case("upstream") {
            return config::HomebrewTrack::Upstream;
        }
    }
    config::HomebrewTrack::Default
}

fn generate_cask_recipe_toml(
    info: &network::HomebrewCaskInfo,
    track: config::HomebrewTrack,
) -> String {
    let clean_conflicts = info
        .conflicts
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");

    let conflicts_line = if info.conflicts.is_empty() {
        String::new()
    } else {
        format!("conflicting_applications = [{}]\n", clean_conflicts)
    };

    let homepage_line = match &info.homepage {
        Some(hp) if !hp.is_empty() => format!("homepage = \"{}\"\n", hp),
        _ => String::new(),
    };

    let desc_line = match &info.desc {
        Some(d) if !d.is_empty() => format!("description = \"{}\"\n", d.replace('"', "\\\"")),
        _ => String::new(),
    };

    match track {
        config::HomebrewTrack::Default | config::HomebrewTrack::Request => {
            format!(
                "name = \"{}\"\naliases = [\"{}\"]\nbundle_name = \"{}\"\norigin = \"homebrew\"\ntrack = \"verified\"\n{}{}quarantine_strip = true\n{}\n[source]\ntype = \"homebrew_cask\"\ncask = \"{}\"\n\n[livecheck]\nstrategy = \"homebrew_cask\"\ncask = \"{}\"\n",
                info.name.replace('"', "\\\""),
                info.token,
                info.bundle_name,
                homepage_line,
                desc_line,
                conflicts_line,
                info.token,
                info.token
            )
        }
        config::HomebrewTrack::Upstream => {
            let gh_regex = regex::Regex::new(r"^https://github\.com/([^/]+)/([^/]+)/releases").unwrap();
            if let Some(caps) = gh_regex.captures(&info.download_url) {
                let owner = &caps[1];
                let repo = &caps[2];
                format!(
                    "name = \"{}\"\naliases = [\"{}\"]\nbundle_name = \"{}\"\norigin = \"homebrew\"\ntrack = \"upstream\"\n{}{}quarantine_strip = true\n{}\n[source]\ntype = \"github_release\"\nlocator = \"{}/{}\"\n\n[livecheck]\nstrategy = \"github_latest_release\"\n",
                    info.name.replace('"', "\\\""),
                    info.token,
                    info.bundle_name,
                    homepage_line,
                    desc_line,
                    conflicts_line,
                    owner,
                    repo
                )
            } else {
                format!(
                    "name = \"{}\"\naliases = [\"{}\"]\nbundle_name = \"{}\"\norigin = \"homebrew\"\ntrack = \"upstream\"\n{}{}quarantine_strip = true\n{}\n[source]\ntype = \"direct_url\"\nurl_template = \"{}\"\n\n[livecheck]\nstrategy = \"homebrew_cask\"\ncask = \"{}\"\n",
                    info.name.replace('"', "\\\""),
                    info.token,
                    info.bundle_name,
                    homepage_line,
                    desc_line,
                    conflicts_line,
                    info.download_url,
                    info.token
                )
            }
        }
    }
}

/// `rpm install <app>` — fetch latest release and install, then mark managed in cache.
fn cmd_install(
    query: &str,
    verbose: bool,
    clean: bool,
    track_override: Option<config::HomebrewTrack>,
) -> i32 {
    let recipes = match recipe::load_recipes() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\x1b[31mFailed to load recipes: {}\x1b[0m", e);
            return 1;
        }
    };

    let mut switching_track = false;
    let mut target_cask_token: Option<String> = None;

    let resolved_recipe = if let Some(r) = recipe::find_recipe(query, &recipes) {
        let track_mismatch = match track_override {
            Some(req_track) => !r.matches_homebrew_track(req_track),
            None => false,
        };

        if track_mismatch {
            switching_track = true;
            let token = r
                .recipe_path
                .as_ref()
                .and_then(|p| p.file_stem().and_then(|s| s.to_str()))
                .map(|s| s.to_string())
                .or_else(|| r.aliases.first().cloned())
                .unwrap_or_else(|| query.to_string());
            target_cask_token = Some(token);

            if verbose {
                let track_name = match track_override {
                    Some(config::HomebrewTrack::Upstream) => "upstream",
                    Some(config::HomebrewTrack::Default) => "verified",
                    _ => "default",
                };
                println!(
                    "\x1b[1;34m==>\x1b[0m Switching Homebrew track for '{}' to '{}'...",
                    r.name, track_name
                );
            }
            None
        } else {
            Some(r.clone())
        }
    } else {
        None
    };

    let resolved_recipe = if let Some(r) = resolved_recipe {
        Some(r)
    } else {
        if verbose && !switching_track {
            println!(
                "\x1b[1;34m==>\x1b[0m '{}' not found locally. Searching upstream catalogs...",
                query
            );
        }

        // 1. Check native GitHub raw (skip if switching track for known Homebrew cask)
        let native_opt = if !switching_track {
            match network::fetch_raw_github_recipe(query) {
                Ok(opt) => opt,
                Err(e) => {
                    if verbose {
                        eprintln!("    \x1b[90m(Native upstream check: {})\x1b[0m", e);
                    }
                    None
                }
            }
        } else {
            None
        };

        if let Some(raw_toml) = native_opt {
            match toml::from_str::<recipe::Recipe>(&raw_toml) {
                Ok(mut rec) => {
                    let dest_dir = paths::recipes_dir().join("rpm");
                    let _ = std::fs::create_dir_all(&dest_dir);
                    let target_path = dest_dir.join(format!("{}.toml", query.to_lowercase()));
                    if let Err(e) = std::fs::write(&target_path, &raw_toml) {
                        eprintln!("\x1b[31mFailed to save recipe: {}\x1b[0m", e);
                        return 1;
                    }
                    rec.recipe_path = Some(target_path);
                    if verbose {
                        println!("  \x1b[32m✓\x1b[0m Fetched native recipe: {}", rec.name);
                    }
                    Some(rec)
                }
                Err(e) => {
                    eprintln!("\x1b[31mFailed to parse remote recipe: {}\x1b[0m", e);
                    None
                }
            }
        } else {
            // 2. Check Homebrew Cask API
            let lookup_token = target_cask_token.as_deref().unwrap_or(query);
            match network::fetch_homebrew_cask_info(lookup_token) {
                Ok(info) => {
                    if verbose || !switching_track {
                        println!("\x1b[1;34m==>\x1b[0m Found '{}' in Homebrew Cask catalog.", info.name);
                    }

                    // Pre-flight check: strict refusal if target exists and is unmanaged
                    let clean_bundle = if info.bundle_name.ends_with(".app") {
                        info.bundle_name.clone()
                    } else {
                        format!("{}.app", info.bundle_name)
                    };
                    let target = std::path::PathBuf::from(format!("/Applications/{}", clean_bundle));
                    if target.exists() && !cache::was_managed(&info.name) && !cache::was_managed(&clean_bundle) {
                        eprintln!(
                            "\x1b[31mError:\x1b[0m '{}' is already installed at /Applications/{} but is not managed by rpm.\n\
                             For security and system integrity, rpm will not overwrite applications installed outside this package manager.\n\
                             To manage '{}' with rpm:\n  \
                             1. Uninstall or remove '/Applications/{}' (e.g. drag to Trash or run: rm -rf '/Applications/{}')\n  \
                             2. Reinstall via rpm: {} install {}",
                            info.name,
                            clean_bundle,
                            info.name,
                            clean_bundle,
                            clean_bundle,
                            paths::runner_cmd(),
                            info.token
                        );
                        return 1;
                    }

                    if info.disabled {
                        println!(
                            "  \x1b[33mNotice:\x1b[0m Homebrew cask is disabled ({}: {}). rpm will strip quarantine.",
                            info.name,
                            info.disable_reason.as_deref().unwrap_or("fails security check")
                        );
                    }

                    let effective_track = track_override.unwrap_or_else(|| {
                        let cfg = config::Config::load_or_default();
                        match cfg.homebrew.track {
                            config::HomebrewTrack::Request => prompt_user_for_track(&info.name),
                            other => other,
                        }
                    });

                    let toml_content = generate_cask_recipe_toml(&info, effective_track);
                    let cask_dir = paths::recipes_dir().join("homebrew").join("cask");
                    let _ = std::fs::create_dir_all(&cask_dir);
                    let recipe_path = cask_dir.join(format!("{}.toml", info.token));

                    if let Err(e) = std::fs::write(&recipe_path, &toml_content) {
                        eprintln!("\x1b[31mFailed to save generated recipe: {}\x1b[0m", e);
                        return 1;
                    }

                    if verbose {
                        println!(
                            "  \x1b[32m✓\x1b[0m Generated recipe: {} [track: {}]",
                            recipe_path.display(),
                            effective_track.as_str()
                        );
                    }

                    match toml::from_str::<recipe::Recipe>(&toml_content) {
                        Ok(mut rec) => {
                            rec.recipe_path = Some(recipe_path);
                            Some(rec)
                        }
                        Err(e) => {
                            eprintln!("\x1b[31mFailed to parse generated recipe: {}\x1b[0m", e);
                            None
                        }
                    }
                }
                Err(_) => {
                    eprintln!(
                        "\x1b[31mError:\x1b[0m Package '{}' not found in rpm recipes or Homebrew casks.",
                        query
                    );
                    return 1;
                }
            }
        }
    };

    let recipe = match resolved_recipe {
        Some(ref r) => r,
        None => return 1,
    };

    let all_recipes = recipe::load_recipes().unwrap_or_default();
    if let Err(e) = installer::check_conflicting_apps(recipe, &all_recipes) {
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
        println!("    Run \x1b[1m{} install <app>\x1b[0m to install a package.", paths::runner_cmd());
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

        match network::resolve_recipe_info(recipe) {
            Ok((remote_version, download_url, sha256)) => {
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
                    if let Some(ref expected_hash) = sha256 {
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
                    sha256,
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
