use crate::cache::{self, CachedApp, UpdateCache};
use crate::local;
use crate::network;
use crate::progress::{InstallProgress, TreeTranscript, TREE_LAST_INDENT, TREE_TRUNK_INDENT};
use crate::recipe::{self, Recipe, SourceConfig};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::paths;

/// Reads the update cache that `update` wrote to `local/cache.toml`.
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

/// Formats a package branch header line inside the Download or Install tree (`├─` / `└─`).
pub fn format_tree_branch_header(
    branch: &str,
    task: &BatchInstallTask,
    is_upgrade: bool,
    include_origin: bool,
) -> String {
    let ver_display = match (&task.local_version, is_upgrade) {
        (Some(local_ver), true) => format!("([{}] → [{}])", local_ver, task.remote_version),
        _ => format!("[{}]", task.remote_version),
    };
    let origin = if include_origin {
        task.origin_badge.as_str()
    } else {
        ""
    };
    format!(
        "  \x1b[38;2;90;90;90m{}\x1b[0m \x1b[1m{}\x1b[0m {}{}",
        branch, task.name, ver_display, origin
    )
}
/// Represents a single resolved application ready for the two-phase Download → Install tree pipeline.
#[derive(Clone, Debug)]
pub struct BatchInstallTask {
    pub name: String,
    pub bundle_name: String,
    pub local_version: Option<String>,
    pub remote_version: String,
    pub download_url: String,
    pub expected_sha256: Option<String>,
    pub quarantine_strip: bool,
    pub origin_badge: String,
}

/// Upgrades all pending apps using the unified two-phase Download → Install tree pipeline.
pub fn upgrade_pending(verbose: bool, clean: bool) -> Result<usize, String> {
    let pending = apps_ready_to_upgrade().map_err(|e| {
        format!(
            "{}\nRun \x1b[1m{} update\x1b[0m first to refresh the cache.",
            e,
            paths::runner_cmd()
        )
    })?;

    if pending.is_empty() {
        return Ok(0);
    }

    ensure_staging()?;
    let recipes = recipe::load_recipes().unwrap_or_default();

    let mut tasks: Vec<BatchInstallTask> = Vec::new();

    for app in &pending {
        if !app.managed {
            println!(
                "  \x1b[33mSkipping unmanaged app\x1b[0m '{}'. Only rpm-managed apps can be upgraded.",
                app.name
            );
            continue;
        }

        let matching_recipe = recipe::find_recipe(&app.name, &recipes);
        let bundle_name = matching_recipe
            .map(|r| r.bundle_name.clone())
            .unwrap_or_else(|| app.name.clone());
        let quarantine_strip = matching_recipe.map(|r| r.quarantine_strip).unwrap_or(true);
        let expected_sha256 = matching_recipe
            .and_then(|r| r.sha256.clone())
            .or_else(|| app.sha256.clone());
        let origin_badge = matching_recipe
            .map(|r| format!(" {}", r.origin().colored_badge()))
            .unwrap_or_default();

        tasks.push(BatchInstallTask {
            name: app.name.clone(),
            bundle_name,
            local_version: Some(app.local_version.clone()),
            remote_version: app.remote_version.clone(),
            download_url: app.download_url.clone(),
            expected_sha256,
            quarantine_strip,
            origin_badge,
        });
    }

    if tasks.is_empty() {
        return Ok(0);
    }

    execute_batch_pipeline(&tasks, true, verbose, clean)
}

/// Installs a batch of resolved recipes using the unified two-phase Download → Install tree pipeline.
pub fn install_recipes_batch(
    items: &[(Recipe, String, String)],
    verbose: bool,
    clean: bool,
) -> Result<usize, String> {
    if items.is_empty() {
        return Ok(0);
    }

    let all_recipes = recipe::load_recipes().unwrap_or_default();
    let mut tasks: Vec<BatchInstallTask> = Vec::new();

    for (recipe, remote_version, download_url) in items {
        check_conflicting_apps(recipe, &all_recipes)?;

        let clean_bundle = if recipe.bundle_name.ends_with(".app") {
            recipe.bundle_name.clone()
        } else {
            format!("{}.app", recipe.bundle_name)
        };
        let target = PathBuf::from(format!("/Applications/{}", clean_bundle));
        if target.exists() && !cache::was_managed(&recipe.name) {
            let alias = recipe
                .aliases
                .first()
                .map(|s| s.as_str())
                .unwrap_or(&recipe.name);
            return Err(format!(
                "'{}' is already installed at /Applications/{} but is not managed by rpm.\n\
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
            ));
        }

        let dynamic_cask_sha = if recipe.sha256.is_none() {
            if let SourceConfig::HomebrewCask { cask } = &recipe.source {
                network::fetch_homebrew_cask_info(cask)
                    .ok()
                    .and_then(|i| i.sha256)
            } else {
                None
            }
        } else {
            None
        };
        let effective_sha = recipe.sha256.clone().or(dynamic_cask_sha);

        tasks.push(BatchInstallTask {
            name: recipe.name.clone(),
            bundle_name: recipe.bundle_name.clone(),
            local_version: None,
            remote_version: remote_version.clone(),
            download_url: download_url.clone(),
            expected_sha256: effective_sha,
            quarantine_strip: recipe.quarantine_strip,
            origin_badge: format!(" {}", recipe.origin().colored_badge()),
        });
    }

    execute_batch_pipeline(&tasks, false, verbose, clean)
}

/// Core two-phase tree pipeline:
/// 1. Downloads all packages in `tasks` under `==> Downloading N package(s)...`
/// 2. Connects via `│` trunk into `==> Installing N package(s)...`
/// 3. Extracts, validates, strips quarantine, and atomically swaps each bundle into `/Applications`.
pub fn execute_batch_pipeline(
    tasks: &[BatchInstallTask],
    is_upgrade: bool,
    verbose: bool,
    clean: bool,
) -> Result<usize, String> {
    if tasks.is_empty() {
        return Ok(0);
    }

    ensure_staging()?;

    // Pre-flight unmanaged check for all tasks
    for task in tasks {
        if task.download_url.is_empty() {
            return Err(format!("No download URL provided for '{}'", task.name));
        }
        let clean_bundle = if task.bundle_name.ends_with(".app") {
            task.bundle_name.clone()
        } else {
            format!("{}.app", task.bundle_name)
        };
        let target = PathBuf::from(format!("/Applications/{}", clean_bundle));
        if target.exists()
            && !cache::was_managed(&task.name)
            && !cache::was_managed(&task.bundle_name)
        {
            return Err(format!(
                "'{}' is already installed at /Applications/{} but is not managed by rpm.\n\
                 For security and system integrity, rpm will not overwrite applications installed outside this package manager.\n\
                 To manage '{}' with rpm:\n  \
                 1. Uninstall or remove '/Applications/{}' (e.g. drag to Trash or run: rm -rf '/Applications/{}')\n  \
                 2. Reinstall via rpm: {} install {}",
                task.name,
                clean_bundle,
                task.name,
                clean_bundle,
                clean_bundle,
                paths::runner_cmd(),
                task.name.to_lowercase()
            ));
        }
    }

    let mut transcript = TreeTranscript::new();
    let dl_pkg_word = if tasks.len() == 1 {
        "package"
    } else {
        "packages"
    };
    transcript.println(format!(
        "\x1b[1;34m==>\x1b[0m \x1b[1mDownloading {} {}...\x1b[0m",
        tasks.len(),
        dl_pkg_word
    ));

    let mut downloaded_items: Vec<(&BatchInstallTask, PathBuf, String)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for (idx, task) in tasks.iter().enumerate() {
        let branch = if idx + 1 == tasks.len() { "└─" } else { "├─" };
        transcript.println(format_tree_branch_header(branch, task, is_upgrade, true));

        match download_for_app_in_tree(
            &task.name,
            &task.remote_version,
            &task.download_url,
            task.expected_sha256.as_deref(),
            verbose,
            clean,
            TREE_TRUNK_INDENT,
            &mut transcript,
        ) {
            Ok((archive_path, sha256_hash)) => {
                downloaded_items.push((task, archive_path, sha256_hash));
            }
            Err(e) => {
                transcript.println(format!(
                    "{}\x1b[31m✗\x1b[0m {}: {}",
                    TREE_TRUNK_INDENT, task.name, e
                ));
                errors.push(format!("{}: {}", task.name, e));
            }
        }
    }

    let mut completed_names: Vec<String> = Vec::new();

    if !downloaded_items.is_empty() {
        transcript.println("  \x1b[38;2;90;90;90m│\x1b[0m");
        let inst_pkg_word = if downloaded_items.len() == 1 {
            "package"
        } else {
            "packages"
        };
        transcript.println(format!(
            "\x1b[1;34m==>\x1b[0m \x1b[1mInstalling {} {}...\x1b[0m",
            downloaded_items.len(),
            inst_pkg_word
        ));

        let total_to_install = downloaded_items.len();
        for (idx, (task, archive_path, sha256_hash)) in downloaded_items.into_iter().enumerate() {
            let is_last = idx + 1 == total_to_install;
            let branch = if is_last { "└─" } else { "├─" };
            let bar_indent = if is_last {
                TREE_LAST_INDENT
            } else {
                TREE_TRUNK_INDENT
            };

            transcript.println(format_tree_branch_header(branch, task, is_upgrade, false));

            match install_extracted_bundle_in_tree(
                &archive_path,
                &task.bundle_name,
                task.quarantine_strip,
                verbose,
                bar_indent,
                &mut transcript,
            ) {
                Ok(()) => {
                    local::set_bundle_rpm_version(&task.bundle_name, &task.remote_version);
                    if let Err(e) = mark_managed(
                        &task.name,
                        &task.remote_version,
                        &task.remote_version,
                        &task.download_url,
                        Some(sha256_hash),
                    ) {
                        transcript.println(format!(
                            "{}\x1b[33mwarning:\x1b[0m installed but failed to update cache: {}",
                            bar_indent, e
                        ));
                    }
                    completed_names.push(task.name.clone());
                }
                Err(e) => {
                    transcript.println(format!(
                        "{}\x1b[31m✗\x1b[0m {}: {}",
                        bar_indent, task.name, e
                    ));
                    errors.push(format!("{}: {}", task.name, e));
                }
            }
        }
    }

    if !completed_names.is_empty() {
        println!(
            "\n\x1b[1;34m==>\x1b[0m \x1b[32m{} {} package(s).\x1b[0m",
            if is_upgrade { "Upgraded" } else { "Installed" },
            completed_names.len()
        );
    }

    if !errors.is_empty() {
        return Err(format!(
            "Finished with {} success(es) and {} failure(s):\n  - {}",
            completed_names.len(),
            errors.len(),
            errors.join("\n  - ")
        ));
    }

    Ok(completed_names.len())
}

/// Compatibility handler: Checks if any application conflicting with `recipe` is currently installed.
/// Returns an error if an installed conflicting application is found.
pub fn check_conflicting_apps(recipe: &Recipe, recipes: &[Recipe]) -> Result<(), String> {
    // 1. Direct conflicts listed in this recipe's configuration
    for conflict in &recipe.conflicting_applications {
        if let Some(err) = check_one_conflict(recipe, conflict, recipes) {
            return Err(err);
        }
    }

    // 2. Bidirectional conflicts: check if any already-installed recipe conflicts with this recipe
    for other in recipes {
        if other.name.eq_ignore_ascii_case(&recipe.name) {
            continue;
        }
        let conflicts_with_this = other
            .conflicting_applications
            .iter()
            .any(|c| recipe.matches(c) || c.eq_ignore_ascii_case(&recipe.bundle_name));
        if conflicts_with_this {
            let other_installed =
                local::get_recipe_local_version(other).is_some() || other.app_path().exists();
            if other_installed {
                let clean_bundle = if other.bundle_name.ends_with(".app") {
                    other.bundle_name.clone()
                } else {
                    format!("{}.app", other.bundle_name)
                };
                let is_managed = cache::was_managed(&other.name);
                let uninstall_hint = if is_managed {
                    let alias = other
                        .aliases
                        .first()
                        .map(|s| s.as_str())
                        .unwrap_or(&other.name);
                    format!(
                        "To resolve this, uninstall it first:\n  {} uninstall {}",
                        paths::runner_cmd(),
                        alias
                    )
                } else {
                    format!(
                        "To resolve this, remove it first:\n  rm -rf '/Applications/{}'",
                        clean_bundle
                    )
                };
                return Err(format!(
                    "Cannot install '{}' because conflicting application '{}' is currently installed at /Applications/{}.\n{}",
                    recipe.name, other.name, clean_bundle, uninstall_hint
                ));
            }
        }
    }

    Ok(())
}

fn check_one_conflict(recipe: &Recipe, conflict: &str, recipes: &[Recipe]) -> Option<String> {
    if let Some(target_recipe) = recipe::find_recipe(conflict, recipes) {
        let is_installed = local::get_recipe_local_version(target_recipe).is_some()
            || target_recipe.app_path().exists();
        if is_installed {
            let clean_bundle = if target_recipe.bundle_name.ends_with(".app") {
                target_recipe.bundle_name.clone()
            } else {
                format!("{}.app", target_recipe.bundle_name)
            };
            let is_managed = cache::was_managed(&target_recipe.name);
            let uninstall_hint = if is_managed {
                let alias = target_recipe
                    .aliases
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or(&target_recipe.name);
                format!(
                    "To resolve this, uninstall it first:\n  {} uninstall {}",
                    paths::runner_cmd(),
                    alias
                )
            } else {
                format!(
                    "To resolve this, remove it first:\n  rm -rf '/Applications/{}'",
                    clean_bundle
                )
            };
            return Some(format!(
                "Cannot install '{}' because conflicting application '{}' is currently installed at /Applications/{}.\n{}",
                recipe.name, target_recipe.name, clean_bundle, uninstall_hint
            ));
        }
    } else {
        // Not a known recipe name: check if bundle or app exists directly in /Applications
        let clean_bundle = if conflict.ends_with(".app") {
            conflict.to_string()
        } else {
            format!("{}.app", conflict)
        };
        let target_path = PathBuf::from(format!("/Applications/{}", clean_bundle));
        if target_path.exists() {
            let is_managed = cache::was_managed(conflict) || cache::was_managed(&clean_bundle);
            let uninstall_hint = if is_managed {
                format!(
                    "To resolve this, uninstall it first:\n  {} uninstall {}",
                    paths::runner_cmd(),
                    conflict
                )
            } else {
                format!(
                    "To resolve this, remove it first:\n  rm -rf '/Applications/{}'",
                    clean_bundle
                )
            };
            return Some(format!(
                "Cannot install '{}' because conflicting application '{}' is currently installed at /Applications/{}.\n{}",
                recipe.name, conflict, clean_bundle, uninstall_hint
            ));
        }
    }
    None
}

/// Install one app by Recipe configuration.
pub fn install_recipe(
    recipe: &Recipe,
    remote_version: &str,
    download_url: &str,
    verbose: bool,
    clean: bool,
) -> Result<(), String> {
    install_recipes_batch(
        &[(
            recipe.clone(),
            remote_version.to_string(),
            download_url.to_string(),
        )],
        verbose,
        clean,
    )
    .map(|_| ())
}

/// Backwards-compatible install helper.
#[allow(dead_code)]
pub fn install_app(name: &str, remote_version: &str, download_url: &str) -> Result<(), String> {
    let recipes = recipe::load_recipes().unwrap_or_default();
    if let Some(r) = recipe::find_recipe(name, &recipes) {
        return install_recipe(r, remote_version, download_url, false, false);
    }

    let bundle_name = format!("{}.app", name);
    println!(
        "\x1b[1;34m==>\x1b[0m Installing \x1b[1m{}\x1b[0m ([{}])",
        name, remote_version
    );

    let calculated_hash = install_bundle_pipeline(
        name,
        &bundle_name,
        remote_version,
        download_url,
        None,
        true,
        false,
        false,
    )?;

    mark_managed(
        name,
        remote_version,
        remote_version,
        download_url,
        Some(calculated_hash),
    )?;

    println!("  \x1b[32m✓\x1b[0m {} installed successfully.", name);
    Ok(())
}

/// Remove a managed app from `/Applications` and clear its managed flag in the cache.
pub fn uninstall_app(name: &str) -> Result<(), String> {
    let mut cache = cache::load_cache_or_empty()?;
    let entry = cache.find(name).cloned();

    match &entry {
        Some(app) if app.managed => {}
        Some(_) => {
            return Err(format!(
                "{} is in the cache but not marked as managed by this tool.\n\
                 Only apps installed/upgraded via rpm can be uninstalled this way.",
                name
            ));
        }
        None => {
            return Err(format!(
                "{} is not in {} as a managed package.\n\
                 Install it first with \x1b[1m{} install {}\x1b[0m",
                name,
                cache::cache_file_path().display(),
                paths::runner_cmd(),
                name
            ));
        }
    }

    let recipes = recipe::load_recipes().unwrap_or_default();
    let bundle_name = recipe::find_recipe(name, &recipes)
        .map(|r| r.bundle_name.clone())
        .unwrap_or_else(|| {
            let canonical = entry.as_ref().map(|a| a.name.as_str()).unwrap_or(name);
            if canonical.ends_with(".app") {
                canonical.to_string()
            } else {
                format!("{}.app", canonical)
            }
        });

    let target = PathBuf::from(format!("/Applications/{}", bundle_name));

    if local::is_app_running(&bundle_name) {
        println!("  ⚠️  Warning: {} is currently running.", bundle_name);
        println!("     Requesting application to quit...");
        let _ = local::request_quit(&bundle_name);
        std::thread::sleep(Duration::from_millis(500));
    }

    if target.exists() {
        println!(
            "\x1b[1;34m==>\x1b[0m Removing \x1b[1m{}\x1b[0m...",
            target.display()
        );
        local::unregister_app_from_system(&target);
        fs::remove_dir_all(&target).map_err(|e| {
            format!(
                "Failed to remove {}: {}\n(You may need to quit the app or check permissions.)",
                target.display(),
                e
            )
        })?;
    } else {
        println!(
            "\x1b[33mwarning:\x1b[0m {} was not present; clearing managed state anyway.",
            target.display()
        );
    }

    let canonical = entry.as_ref().map(|a| a.name.as_str()).unwrap_or(name);
    if let Some(app) = cache.find_mut(canonical) {
        app.managed = false;
        app.local_version = "Not Installed".to_string();
        app.update_available = !app.download_url.is_empty() && !app.remote_version.is_empty();
    }
    cache.touch();
    cache::save_cache(&cache)?;

    // If this was an auto-generated Homebrew cask recipe, remove the recipe file from disk
    let matched_recipe = recipe::find_recipe(name, &recipes)
        .or_else(|| recipe::find_recipe(canonical, &recipes));
    if let Some(r) = matched_recipe {
        recipe::remove_cask_recipe(r);
    } else {
        for candidate in [name, canonical] {
            let candidate_path = paths::recipes_dir()
                .join("homebrew")
                .join("cask")
                .join(format!("{}.toml", candidate.to_lowercase()));
            if candidate_path.exists() {
                let _ = fs::remove_file(&candidate_path);
            }
        }
    }

    println!("  \x1b[32m✓\x1b[0m {} uninstalled.", canonical);
    Ok(())
}

/// Unpacks a downloaded archive in staging, validates its `.app` structure, strips quarantine,
/// and atomically installs it to `/Applications` while rendering an `InstallProgress` bar in the tree.
fn install_extracted_bundle_in_tree(
    archive_path: &Path,
    bundle_name: &str,
    quarantine_strip: bool,
    verbose: bool,
    bar_indent: &'static str,
    transcript: &mut TreeTranscript,
) -> Result<(), String> {
    let initial_estimate = estimate_archive_uncompressed_size(archive_path);
    let mut install_progress =
        InstallProgress::with_tree(initial_estimate, bar_indent, Some(transcript));

    let pipeline_result = (|| -> Result<u64, String> {
        let (staged_app, app_bytes) = extract_app_bundle(
            archive_path,
            bundle_name,
            verbose,
            bar_indent,
            &mut install_progress,
        )?;
        local::validate_app_bundle(&staged_app)?;

        if app_bytes > 0 {
            install_progress.set_total(app_bytes);
            install_progress.set_installed_bytes((app_bytes * 96) / 100);
        }

        install_app_bundle_atomic(
            &staged_app,
            bundle_name,
            quarantine_strip,
            verbose,
            bar_indent,
            &mut install_progress,
            app_bytes,
        )?;

        Ok(app_bytes)
    })();

    // Clean extract workspace; keep the downloaded archive in staging for cache/rollback
    let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&extract_root);

    match pipeline_result {
        Ok(final_bytes) => {
            install_progress.finish(Some(final_bytes));
            Ok(())
        }
        Err(e) => {
            install_progress.clear();
            Err(e)
        }
    }
}

/// Core pipeline: Download with SHA-256 verification, unpack, inspect, and atomically install.
fn install_bundle_pipeline(
    name: &str,
    bundle_name: &str,
    version: &str,
    download_url: &str,
    expected_sha256: Option<&str>,
    quarantine_strip: bool,
    verbose: bool,
    clean: bool,
) -> Result<String, String> {
    if download_url.is_empty() {
        return Err("No download URL provided".to_string());
    }

    let clean_bundle = if bundle_name.ends_with(".app") {
        bundle_name.to_string()
    } else {
        format!("{}.app", bundle_name)
    };
    let target = PathBuf::from(format!("/Applications/{}", clean_bundle));
    if target.exists() && !cache::was_managed(name) && !cache::was_managed(bundle_name) {
        return Err(format!(
            "'{}' is already installed at /Applications/{} but is not managed by rpm.\n\
             For security and system integrity, rpm will not overwrite applications installed outside this package manager.\n\
             To manage '{}' with rpm:\n  \
             1. Uninstall or remove '/Applications/{}' (e.g. drag to Trash or run: rm -rf '/Applications/{}')\n  \
             2. Reinstall via rpm: {} install {}",
            name,
            clean_bundle,
            name,
            clean_bundle,
            clean_bundle,
            paths::runner_cmd(),
            name.to_lowercase()
        ));
    }

    let mut transcript = TreeTranscript::new();
    let (archive_path, sha256_hash) = download_for_app_in_tree(
        name,
        version,
        download_url,
        expected_sha256,
        verbose,
        clean,
        TREE_TRUNK_INDENT,
        &mut transcript,
    )?;

    install_extracted_bundle_in_tree(
        &archive_path,
        bundle_name,
        quarantine_strip,
        verbose,
        TREE_LAST_INDENT,
        &mut transcript,
    )?;

    Ok(sha256_hash)
}

fn staging_path() -> PathBuf {
    paths::staging_dir()
}

fn ensure_staging() -> Result<(), String> {
    let dir = staging_path();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create staging dir {}: {}", dir.display(), e))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn filename_from_url(url: &str) -> String {
    url.split('/')
        .next_back()
        .filter(|s| !s.is_empty())
        .unwrap_or("download.bin")
        .split('?')
        .next()
        .unwrap_or("download.bin")
        .to_string()
}

fn calculate_file_sha256(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 32768];

    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| format!("Failed reading {}: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Formats a byte count into a human-readable string (B, KB, MB, GB).
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

/// Core cleanup logic operating on a given directory.
fn cleanup_dir(dir: &Path, app_query: Option<&str>, verbose: bool) -> Result<(usize, u64), String> {
    if !dir.exists() {
        return Ok((0, 0));
    }

    let entries = fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?;

    let recipes = recipe::load_recipes().unwrap_or_default();
    let matching_prefixes: Vec<String> = if let Some(q) = app_query {
        if let Some(r) = recipe::find_recipe(q, &recipes) {
            let mut prefixes = vec![format!("{}-", sanitize(&r.name).to_lowercase())];
            for alias in &r.aliases {
                prefixes.push(format!("{}-", sanitize(alias).to_lowercase()));
            }
            prefixes
        } else {
            vec![format!("{}-", sanitize(q).to_lowercase())]
        }
    } else {
        vec![]
    };

    let mut removed_count = 0usize;
    let mut total_bytes = 0u64;

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        if file_name.starts_with('.') {
            continue;
        }

        let is_match = if matching_prefixes.is_empty() {
            true
        } else {
            matching_prefixes.iter().any(|p| {
                file_name.to_lowercase().starts_with(p) || file_name.to_lowercase().contains(p)
            })
        };

        if !is_match {
            continue;
        }

        if path.is_dir() {
            if let Ok(()) = fs::remove_dir_all(&path) {
                if verbose {
                    println!("  \x1b[32m✓\x1b[0m Removed temp directory {}", file_name);
                }
            }
            continue;
        }

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        match fs::remove_file(&path) {
            Ok(()) => {
                removed_count += 1;
                total_bytes += size;
                println!(
                    "  \x1b[32m✓\x1b[0m Removed {} ({})",
                    file_name,
                    format_bytes(size)
                );
            }
            Err(e) => {
                eprintln!("  \x1b[31m✗\x1b[0m Failed to remove {}: {}", file_name, e);
            }
        }
    }

    Ok((removed_count, total_bytes))
}

/// Removes cached payload archives and staging files.
/// If `app_query` is Some, only removes cached files matching that application.
/// If `app_query` is None, removes all cached payload archives in `local/staging`.
pub fn cleanup_staging(app_query: Option<&str>, verbose: bool) -> Result<(), String> {
    let staging = staging_path();
    if !staging.exists() {
        println!(
            "\x1b[1;34m==>\x1b[0m No cached payloads found in {}.",
            staging.display()
        );
        return Ok(());
    }

    println!(
        "\x1b[1;34m==>\x1b[0m Cleaning cached payloads in {}...",
        staging.display()
    );
    let (removed_count, total_bytes) = cleanup_dir(&staging, app_query, verbose)?;

    if removed_count == 0 {
        if let Some(q) = app_query {
            println!("  No cached payloads found matching '{}'.", q);
        } else {
            println!("  No cached payloads found.");
        }
    } else {
        println!(
            "\x1b[1;34m==>\x1b[0m Cleaned {} file(s), freed {} disk space.",
            removed_count,
            format_bytes(total_bytes)
        );
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn download_for_app_in_tree(
    name: &str,
    version: &str,
    download_url: &str,
    expected_sha256: Option<&str>,
    verbose: bool,
    clean: bool,
    bar_indent: &'static str,
    transcript: &mut TreeTranscript,
) -> Result<(PathBuf, String), String> {
    ensure_staging()?;
    let filename = filename_from_url(download_url);
    let dest = staging_path().join(format!(
        "{}-{}-{}",
        sanitize(name),
        sanitize(version),
        filename
    ));

    if clean {
        // Clean install requested: remove dest and any cached payload for this app
        let prefix = format!("{}-", sanitize(name).to_lowercase());
        if let Ok(entries) = fs::read_dir(staging_path()) {
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.to_lowercase().starts_with(&prefix) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let _ = fs::remove_file(&dest);
        if verbose {
            transcript.println(format!(
                "{}🧹 Clean install requested: removed cached payload(s) for {}",
                bar_indent, name
            ));
        } else {
            transcript.println(format!(
                "{}🧹 Clean install: purging cached payload",
                bar_indent
            ));
        }
    } else if dest.exists() && dest.is_file() {
        if let Ok(metadata) = fs::metadata(&dest) {
            if metadata.len() > 0 {
                if let Ok(hash) = calculate_file_sha256(&dest) {
                    let is_valid = match expected_sha256 {
                        Some(expected) => hash.eq_ignore_ascii_case(expected),
                        None => true,
                    };

                    if is_valid {
                        if verbose {
                            transcript.println(format!(
                                "{}📦 Using cached payload: {}",
                                bar_indent,
                                dest.display()
                            ));
                            transcript.println(format!(
                                "{}🔒 SHA-256 Verified: \x1b[32m{}\x1b[0m",
                                bar_indent, hash
                            ));
                        } else {
                            transcript.println(format!("{}📦 Using cached payload", bar_indent));
                        }
                        return Ok((dest, hash));
                    } else {
                        // Stale or corrupt cached payload: remove and redownload
                        let _ = fs::remove_file(&dest);
                    }
                }
            }
        }
    }

    if verbose {
        transcript.println(format!("{}📥 Fetching payload from {}", bar_indent, download_url));
        transcript.println(format!("{}📂 Staging path: {}", bar_indent, dest.display()));
        if let Some(exp) = expected_sha256 {
            transcript.println(format!(
                "{}🔒 Expected SHA-256: \x1b[90m{}\x1b[0m",
                bar_indent, exp
            ));
        }
    }
    let (hash, _summary) = network::download_file_with_hash_in_tree(
        download_url,
        &dest,
        expected_sha256,
        bar_indent,
        Some(transcript),
    )?;
    if verbose {
        transcript.println(format!(
            "{}🔒 Calculated SHA-256: \x1b[32m{}\x1b[0m",
            bar_indent, hash
        ));
    }
    Ok((dest, hash))
}

/// Computes total byte size of regular files inside `dir` without following symlinks
/// (avoiding double-counting macOS `.framework/Versions/Current` symlinks).
pub fn compute_dir_size(dir: &Path) -> u64 {
    let meta = match fs::symlink_metadata(dir) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let mut total = 0u64;
    for entry in entries.flatten() {
        let p = entry.path();
        if let Ok(m) = fs::symlink_metadata(&p) {
            if m.file_type().is_symlink() {
                continue;
            }
            if m.is_file() {
                total = total.saturating_add(m.len());
            } else if m.is_dir() {
                total = total.saturating_add(compute_dir_size(&p));
            }
        }
    }
    total
}

/// Parses the uncompressed byte total from `unzip -Zt` output:
/// e.g. `1432 files, 245891234 bytes uncompressed, 98123456 bytes compressed:  60.1%`
pub fn parse_unzip_zt_total(output: &str) -> Option<u64> {
    for line in output.lines() {
        if let Some(idx) = line.find("bytes uncompressed") {
            let before = line[..idx].trim_end();
            let token = before.split_whitespace().next_back()?;
            let cleaned: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(val) = cleaned.parse::<u64>() {
                if val > 0 {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Estimates uncompressed byte size before archive extraction starts.
/// For `.dmg`, returns `None` initially (until mounted, when exact `.app` size is read in milliseconds).
fn estimate_archive_uncompressed_size(archive: &Path) -> Option<u64> {
    let name_lower = archive
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();

    if name_lower.ends_with(".dmg") {
        // DMG mounts first; once mounted we measure the exact .app bundle size
        return None;
    }

    if name_lower.ends_with(".zip") {
        if let Ok(out) = Command::new("unzip")
            .args(["-Zt", archive.to_str().unwrap_or_default()])
            .output()
        {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if let Some(bytes) = parse_unzip_zt_total(&stdout) {
                    return Some(bytes);
                }
            }
        }
    } else if name_lower.ends_with(".tar.xz") {
        if let Ok(out) = Command::new("xz")
            .args(["--robot", "-l", archive.to_str().unwrap_or_default()])
            .output()
        {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    if line.starts_with("totals\t") {
                        let cols: Vec<&str> = line.split('\t').collect();
                        if let Some(uncomp) = cols.get(4).and_then(|s| s.parse::<u64>().ok()) {
                            if uncomp > 0 {
                                return Some(uncomp);
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback heuristic for compressed archives (.zip, .tar.gz, .tgz, .tar.xz): ~2.5x archive size
    fs::metadata(archive)
        .ok()
        .map(|m| m.len().saturating_mul(25) / 10)
        .filter(|&b| b > 0)
}

/// Polls a running child extraction process every 40ms, updating `progress` with the size of `watched_dir`
/// scaled up to 95% of the bar (leaving 95%–100% for validation, quarantine strip, and atomic install).
fn wait_child_with_dir_progress(
    mut child: std::process::Child,
    watched_dir: &Path,
    progress: &mut InstallProgress,
    err_context: &str,
) -> Result<(), String> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                } else {
                    return Err(format!("{} failed (exit {})", err_context, status));
                }
            }
            Ok(None) => {
                let current_bytes = compute_dir_size(watched_dir);
                let scaled = (current_bytes.saturating_mul(95)) / 100;
                progress.set_installed_bytes(scaled);
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => return Err(format!("{}: {}", err_context, e)),
        }
    }
}

/// Extracts `.dmg`, `.zip`, `.tar.gz`, or `.tar.xz` into staging and returns `(staged_app_path, app_bytes)`.
fn extract_app_bundle(
    archive: &Path,
    bundle_name: &str,
    verbose: bool,
    bar_indent: &'static str,
    progress: &mut InstallProgress,
) -> Result<(PathBuf, u64), String> {
    let name_lower = archive
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();

    if name_lower.ends_with(".dmg") {
        extract_dmg(archive, bundle_name, verbose, bar_indent, progress)
    } else if name_lower.ends_with(".zip") {
        extract_zip(archive, bundle_name, verbose, bar_indent, progress)
    } else if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
        extract_tar_gz(archive, bundle_name, verbose, bar_indent, progress)
    } else if name_lower.ends_with(".tar.xz") {
        extract_tar_xz(archive, bundle_name, verbose, bar_indent, progress)
    } else {
        Err(format!(
            "Unsupported archive type in '{}'. Expected .dmg, .zip, .tar.gz, or .tar.xz",
            name_lower
        ))
    }
}

fn extract_dmg(
    dmg: &Path,
    bundle_name: &str,
    verbose: bool,
    bar_indent: &'static str,
    progress: &mut InstallProgress,
) -> Result<(PathBuf, u64), String> {
    let mount = staging_path().join(format!("mnt-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&mount);
    fs::create_dir_all(&mount).map_err(|e| format!("Failed to create mount point: {}", e))?;

    if verbose {
        progress.println(&format!(
            "{}💿 Mounting DMG (unattended): {} → {}...",
            bar_indent,
            dmg.display(),
            mount.display()
        ));
    }
    let mut child = Command::new("hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-noverify",
            "-noautoopen",
            "-mountpoint",
            mount.to_str().unwrap_or_default(),
            dmg.to_str().unwrap_or_default(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run hdiutil attach: {}", e))?;

    // Auto-accept any license prompt by streaming 'Y\n'
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"Y\n");
    }

    // Tick the progress bar while hdiutil attaches the DMG
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                progress.tick();
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&mount);
                return Err(format!("hdiutil attach failed: {}", e));
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("hdiutil attach failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_dir_all(&mount);
        return Err(format!("hdiutil attach failed: {}", stderr.trim()));
    }

    let result = (|| {
        let app_src = find_app_bundle(&mount, bundle_name)?;
        let app_bytes = compute_dir_size(&app_src);
        if app_bytes > 0 {
            progress.set_total(app_bytes);
        }

        let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
        let _ = fs::remove_dir_all(&extract_root);
        fs::create_dir_all(&extract_root)
            .map_err(|e| format!("Failed to create extract dir: {}", e))?;

        let clean_name = if bundle_name.ends_with(".app") {
            bundle_name.to_string()
        } else {
            format!("{}.app", bundle_name)
        };
        let staged = extract_root.join(clean_name);
        if verbose {
            progress.println(&format!(
                "{}📂 Staging {} → {}...",
                bar_indent,
                bundle_name,
                staged.display()
            ));
        }
        ditto_copy_with_progress(&app_src, &staged, progress)?;
        let final_bytes = if app_bytes > 0 {
            app_bytes
        } else {
            compute_dir_size(&staged)
        };
        Ok((staged, final_bytes))
    })();

    // Always detach cleanly
    let _ = Command::new("hdiutil")
        .args(["detach", mount.to_str().unwrap_or_default(), "-quiet"])
        .status();
    let _ = fs::remove_dir_all(&mount);

    result
}

fn extract_zip(
    zip: &Path,
    bundle_name: &str,
    verbose: bool,
    bar_indent: &'static str,
    progress: &mut InstallProgress,
) -> Result<(PathBuf, u64), String> {
    let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&extract_root);
    fs::create_dir_all(&extract_root)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    if verbose {
        progress.println(&format!(
            "{}📦 Extracting ZIP archive: {} → {}...",
            bar_indent,
            zip.display(),
            extract_root.display()
        ));
    }
    let child = Command::new("ditto")
        .args([
            "-x",
            "-k",
            zip.to_str().unwrap_or_default(),
            extract_root.to_str().unwrap_or_default(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to run ditto for zip: {}", e))?;

    wait_child_with_dir_progress(child, &extract_root, progress, "ditto failed to extract zip")?;

    let staged = find_app_bundle(&extract_root, bundle_name)?;
    let app_bytes = compute_dir_size(&staged);
    Ok((staged, app_bytes))
}

fn extract_tar_gz(
    tar: &Path,
    bundle_name: &str,
    verbose: bool,
    bar_indent: &'static str,
    progress: &mut InstallProgress,
) -> Result<(PathBuf, u64), String> {
    let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&extract_root);
    fs::create_dir_all(&extract_root)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    if verbose {
        progress.println(&format!(
            "{}📦 Extracting TAR.GZ archive: {} → {}...",
            bar_indent,
            tar.display(),
            extract_root.display()
        ));
    }
    let child = Command::new("tar")
        .args([
            "-xzf",
            tar.to_str().unwrap_or_default(),
            "-C",
            extract_root.to_str().unwrap_or_default(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to run tar for tar.gz: {}", e))?;

    wait_child_with_dir_progress(child, &extract_root, progress, "tar failed to extract tar.gz")?;

    let staged = find_app_bundle(&extract_root, bundle_name)?;
    let app_bytes = compute_dir_size(&staged);
    Ok((staged, app_bytes))
}

fn extract_tar_xz(
    tar: &Path,
    bundle_name: &str,
    verbose: bool,
    bar_indent: &'static str,
    progress: &mut InstallProgress,
) -> Result<(PathBuf, u64), String> {
    let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&extract_root);
    fs::create_dir_all(&extract_root)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    if verbose {
        progress.println(&format!(
            "{}📦 Extracting TAR.XZ archive: {} → {}...",
            bar_indent,
            tar.display(),
            extract_root.display()
        ));
    }
    let child = Command::new("tar")
        .args([
            "-xJf",
            tar.to_str().unwrap_or_default(),
            "-C",
            extract_root.to_str().unwrap_or_default(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to run tar for tar.xz: {}", e))?;

    wait_child_with_dir_progress(child, &extract_root, progress, "tar failed to extract tar.xz")?;

    let staged = find_app_bundle(&extract_root, bundle_name)?;
    let app_bytes = compute_dir_size(&staged);
    Ok((staged, app_bytes))
}

/// Finds the target `.app` bundle under `root`, with shallow recursive search.
fn find_app_bundle(root: &Path, bundle_name: &str) -> Result<PathBuf, String> {
    let clean_name = if bundle_name.ends_with(".app") {
        bundle_name.to_string()
    } else {
        format!("{}.app", bundle_name)
    };

    let preferred = root.join(&clean_name);
    if preferred.is_dir() {
        return Ok(preferred);
    }

    // Top-level case-insensitive match
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("app") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let search_stem = clean_name.strip_suffix(".app").unwrap_or(&clean_name);
                    if stem.eq_ignore_ascii_case(search_stem) {
                        return Ok(path);
                    }
                }
            }
        }
    }

    // Recursive search up to 4 levels deep
    find_app_bundle_walk(root, &clean_name, 0)?
        .ok_or_else(|| format!("Could not find {} inside archive", clean_name))
}

fn find_app_bundle_walk(
    dir: &Path,
    bundle_name: &str,
    depth: usize,
) -> Result<Option<PathBuf>, String> {
    if depth > 4 {
        return Ok(None);
    }

    let entries =
        fs::read_dir(dir).map_err(|e| format!("Failed to read {}: {}", dir.display(), e))?;
    let search_stem = bundle_name.strip_suffix(".app").unwrap_or(bundle_name);

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("app") && path.is_dir() {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if stem.eq_ignore_ascii_case(search_stem) {
                    return Ok(Some(path));
                }
            }
        }
        if path.is_dir() && path.extension().and_then(|e| e.to_str()) != Some("app") {
            if let Some(found) = find_app_bundle_walk(&path, bundle_name, depth + 1)? {
                return Ok(Some(found));
            }
        }
    }

    Ok(None)
}

fn ditto_copy(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent: {}", e))?;
    }
    let status = Command::new("ditto")
        .args([
            from.to_str().unwrap_or_default(),
            to.to_str().unwrap_or_default(),
        ])
        .status()
        .map_err(|e| format!("Failed to run ditto: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "ditto failed copying {} → {}",
            from.display(),
            to.display()
        ))
    }
}

fn ditto_copy_with_progress(
    from: &Path,
    to: &Path,
    progress: &mut InstallProgress,
) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent: {}", e))?;
    }
    let child = Command::new("ditto")
        .args([
            from.to_str().unwrap_or_default(),
            to.to_str().unwrap_or_default(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to run ditto: {}", e))?;

    wait_child_with_dir_progress(
        child,
        to,
        progress,
        &format!("ditto failed copying {} → {}", from.display(), to.display()),
    )
}

/// Relocates a directory hierarchy via fast filesystem rename (O(1) on same APFS container),
/// falling back to ditto copy + source cleanup if across filesystem boundaries.
fn move_or_copy_bundle(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent: {}", e))?;
    }

    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Fallback for cross-device moves
            ditto_copy(from, to)?;
            let _ = fs::remove_dir_all(from);
            Ok(())
        }
    }
}

/// Atomically installs the staged bundle to `/Applications/<bundle_name>`, protecting running apps.
#[allow(clippy::too_many_arguments)]
fn install_app_bundle_atomic(
    staged_app: &Path,
    bundle_name: &str,
    quarantine_strip: bool,
    verbose: bool,
    bar_indent: &'static str,
    progress: &mut InstallProgress,
    app_bytes: u64,
) -> Result<(), String> {
    let clean_bundle = if bundle_name.ends_with(".app") {
        bundle_name.to_string()
    } else {
        format!("{}.app", bundle_name)
    };

    let target = PathBuf::from(format!("/Applications/{}", clean_bundle));
    let stem = clean_bundle.strip_suffix(".app").unwrap_or(&clean_bundle);
    if target.exists() && !cache::was_managed(&clean_bundle) && !cache::was_managed(stem) {
        return Err(format!(
            "Target /Applications/{} already exists and is not managed by rpm. Refusing to overwrite.",
            clean_bundle
        ));
    }
    if verbose {
        progress.println(&format!("{}🛠  Installing to {}...", bar_indent, target.display()));
    }

    // 1. Process active check (ALWAYS visible)
    if local::is_app_running(&clean_bundle) {
        progress.println(&format!(
            "{}⚠️  Notice: {} is currently running.",
            bar_indent, clean_bundle
        ));
        progress.println(&format!(
            "{}   Attempting graceful quit before upgrade...",
            bar_indent
        ));
        let _ = local::request_quit(&clean_bundle);
        std::thread::sleep(Duration::from_millis(800));
        if local::is_app_running(&clean_bundle) {
            progress.println(&format!(
                "{}   App still active in memory. Utilizing atomic swap to prevent corruption.",
                bar_indent
            ));
        }
    }

    // 2. Strip quarantine on the staged directory BEFORE moving
    if quarantine_strip {
        if verbose {
            progress.println(&format!(
                "{}🛡️  Stripping Apple Gatekeeper quarantine attribute...",
                bar_indent
            ));
        }
        let _ = local::remove_quarantine(staged_app);
    }

    if app_bytes > 0 {
        progress.set_installed_bytes((app_bytes * 97) / 100);
    }

    // 3. Atomic rename backup
    let backup_path = if target.exists() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = PathBuf::from(format!("/Applications/{}.rpm-backup-{}", clean_bundle, ts));
        if verbose {
            progress.println(&format!(
                "{}🔄 Backing up current install → {}",
                bar_indent,
                backup.display()
            ));
        }
        fs::rename(&target, &backup)
            .map_err(|e| format!("Failed to rename old installation: {}", e))?;
        Some(backup)
    } else {
        None
    };

    // 4. Install staged bundle via fast atomic move (O(1) filesystem relocation)
    let move_result = move_or_copy_bundle(staged_app, &target);

    match move_result {
        Ok(()) => {
            if app_bytes > 0 {
                progress.set_installed_bytes((app_bytes * 98) / 100);
            }

            // Verify bundle structure
            if let Err(e) = local::validate_app_bundle(&target) {
                let _ = fs::remove_dir_all(&target);
                if let Some(ref backup) = backup_path {
                    let _ = fs::rename(backup, &target);
                }
                return Err(format!("Installed bundle validation failed: {}", e));
            }

            // 5. Ensure quarantine is cleanly stripped from the final /Applications bundle
            if quarantine_strip {
                if verbose {
                    progress.println(&format!(
                        "{}🛡️  Stripping Apple Gatekeeper quarantine attribute on target...",
                        bar_indent
                    ));
                }
                let _ = local::remove_quarantine(&target);
            }

            if app_bytes > 0 {
                progress.set_installed_bytes((app_bytes * 99) / 100);
            }

            // 6. Register with macOS Launch Services and Spotlight so app appears in Spotlight & Launchpad immediately
            if verbose {
                progress.println(&format!(
                    "{}🔍 Registering application with Launch Services & Spotlight...",
                    bar_indent
                ));
            }
            if let Err(e) = local::register_app_with_system(&target) {
                if verbose {
                    progress.println(&format!(
                        "{}⚠️  Launch Services registration notice: {}",
                        bar_indent, e
                    ));
                }
            }

            // 7. Cleanup backup
            if let Some(ref backup) = backup_path {
                let _ = fs::remove_dir_all(backup);
            }

            Ok(())
        }
        Err(err) => {
            if let Some(ref backup) = backup_path {
                progress.println(&format!(
                    "{}❌ Installation failed. Rolling back to previous install...",
                    bar_indent
                ));
                let _ = fs::rename(backup, &target);
            }
            Err(format!("Installation failed: {}", err))
        }
    }
}

/// Record that rpm owns this install in `local/cache.toml`.
fn mark_managed(
    name: &str,
    local_version: &str,
    remote_version: &str,
    download_url: &str,
    sha256: Option<String>,
) -> Result<(), String> {
    let mut cache = cache::load_cache_or_empty()?;
    let previous = cache.find(name).cloned();

    cache.upsert(CachedApp {
        name: previous
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| name.to_string()),
        local_version: local_version.to_string(),
        remote_version: remote_version.to_string(),
        download_url: download_url.to_string(),
        update_available: false,
        managed: true,
        sha256,
    });
    cache::save_cache(&cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{LivecheckConfig, SourceConfig};

    fn make_test_recipe(name: &str, bundle: &str, conflicts: Vec<&str>) -> Recipe {
        Recipe {
            name: name.to_string(),
            aliases: vec![name.to_lowercase()],
            bundle_name: bundle.to_string(),
            homepage: None,
            description: None,
            quarantine_strip: true,
            sha256: None,
            conflicting_applications: conflicts.into_iter().map(String::from).collect(),
            source: SourceConfig::ChromiumSnapshot {
                channel: "Mac_Arm".to_string(),
            },
            livecheck: LivecheckConfig::GithubLatestRelease,
            origin: None,
            track: None,
            recipe_path: None,
        }
    }

    #[test]
    fn test_check_conflicts_no_conflicts() {
        let r1 = make_test_recipe("AppA", "AppA_NonExistent_12345.app", vec![]);
        let r2 = make_test_recipe("AppB", "AppB_NonExistent_12345.app", vec![]);
        let recipes = vec![r1.clone(), r2];

        assert!(check_conflicting_apps(&r1, &recipes).is_ok());
    }

    #[test]
    fn test_check_conflicts_uninstalled_target() {
        let r1 = make_test_recipe("AppA", "AppA_NonExistent_12345.app", vec!["AppB"]);
        let r2 = make_test_recipe("AppB", "AppB_NonExistent_12345.app", vec!["AppA"]);
        let recipes = vec![r1.clone(), r2];

        // Neither is installed in /Applications, so no conflict error
        assert!(check_conflicting_apps(&r1, &recipes).is_ok());
    }

    #[test]
    fn test_calculate_file_sha256() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("rpm_test_sha256_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("test.txt");
        let mut f = fs::File::create(&file_path).unwrap();
        f.write_all(b"hello world\n").unwrap();
        drop(f);

        let hash = calculate_file_sha256(&file_path).unwrap();
        assert_eq!(
            hash,
            "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(1024 * 1024 * 5), "5.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 2), "2.00 GB");
    }

    #[test]
    fn test_cleanup_dir_all_and_filtered() {
        use std::io::Write;
        let test_dir =
            std::env::temp_dir().join(format!("rpm_test_cleanup_{}", std::process::id()));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir).unwrap();

        let file_a = test_dir.join("Chromium-1.0-app.dmg");
        let file_b = test_dir.join("LibreWolf-2.0-app.dmg");
        let file_hidden = test_dir.join(".DS_Store");

        fs::File::create(&file_a)
            .unwrap()
            .write_all(b"12345")
            .unwrap();
        fs::File::create(&file_b)
            .unwrap()
            .write_all(b"1234567890")
            .unwrap();
        fs::File::create(&file_hidden)
            .unwrap()
            .write_all(b"meta")
            .unwrap();

        // 1. Filtered cleanup: only clean "chromium"
        let (count, bytes) = cleanup_dir(&test_dir, Some("chromium"), false).unwrap();
        assert_eq!(count, 1);
        assert_eq!(bytes, 5);
        assert!(!file_a.exists());
        assert!(file_b.exists());
        assert!(file_hidden.exists());

        // 2. Full cleanup: cleans remaining payload files, ignores hidden .DS_Store
        let (count, bytes) = cleanup_dir(&test_dir, None, false).unwrap();
        assert_eq!(count, 1);
        assert_eq!(bytes, 10);
        assert!(!file_b.exists());
        assert!(file_hidden.exists());

        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_move_or_copy_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Source.app");
        let dst = tmp.path().join("Destination.app");

        fs::create_dir_all(src.join("Contents")).unwrap();
        fs::write(src.join("Contents/test.txt"), "hello").unwrap();

        assert!(src.exists());
        assert!(!dst.exists());

        move_or_copy_bundle(&src, &dst).unwrap();

        assert!(!src.exists());
        assert!(dst.exists());
        assert!(dst.join("Contents/test.txt").exists());
        assert_eq!(
            fs::read_to_string(dst.join("Contents/test.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn test_format_tree_branch_headers() {
        let mut r_rpm = make_test_recipe("LibreWolf", "LibreWolf.app", vec![]);
        r_rpm.origin = Some("rpm".to_string());
        let task_rpm = BatchInstallTask {
            name: r_rpm.name.clone(),
            bundle_name: r_rpm.bundle_name.clone(),
            local_version: None,
            remote_version: "135.0-1".to_string(),
            download_url: "https://example.com/lw.dmg".to_string(),
            expected_sha256: None,
            quarantine_strip: true,
            origin_badge: format!(" {}", r_rpm.origin().colored_badge()),
        };
        let header_rpm = format_tree_branch_header("├─", &task_rpm, false, true);
        assert!(header_rpm.contains("├─"));
        assert!(header_rpm.contains("LibreWolf"));
        assert!(header_rpm.contains("[135.0-1]"));
        assert!(header_rpm.contains("[recipe: \x1b[34mrpm\x1b[0m]"));

        let mut r_hb_up = make_test_recipe("HandBrake", "HandBrake.app", vec![]);
        r_hb_up.origin = Some("homebrew".to_string());
        r_hb_up.track = Some("upstream".to_string());
        let task_upg = BatchInstallTask {
            name: r_hb_up.name.clone(),
            bundle_name: r_hb_up.bundle_name.clone(),
            local_version: Some("1.8.0".to_string()),
            remote_version: "1.8.2".to_string(),
            download_url: "https://example.com/hb.dmg".to_string(),
            expected_sha256: None,
            quarantine_strip: true,
            origin_badge: format!(" {}", r_hb_up.origin().colored_badge()),
        };
        let upg_header = format_tree_branch_header("└─", &task_upg, true, true);
        assert!(upg_header.contains("└─"));
        assert!(upg_header.contains("([1.8.0] → [1.8.2])"));
        assert!(upg_header.contains("[recipe: \x1b[35mhomebrew (upstream)\x1b[0m]"));
    }

    #[test]
    fn test_compute_dir_size_skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("Sample.app");
        let framework_a = app_dir.join("Contents/Frameworks/Foo.framework/Versions/A");
        fs::create_dir_all(&framework_a).unwrap();
        fs::write(framework_a.join("binary"), vec![0u8; 4096]).unwrap();

        #[cfg(unix)]
        {
            let current_link = app_dir.join("Contents/Frameworks/Foo.framework/Versions/Current");
            std::os::unix::fs::symlink(&framework_a, &current_link).unwrap();
        }

        // Size must equal only the real 4096-byte file, not double-counted via symlink
        assert_eq!(compute_dir_size(&app_dir), 4096);
    }

    #[test]
    fn test_parse_unzip_zt_total() {
        let sample = "1432 files, 245891234 bytes uncompressed, 98123456 bytes compressed:  60.1%\n";
        assert_eq!(parse_unzip_zt_total(sample), Some(245_891_234));
        assert_eq!(parse_unzip_zt_total("invalid output"), None);
    }
}
