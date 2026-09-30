use crate::cache::{self, CachedApp, UpdateCache};
use crate::local;
use crate::network;
use crate::recipe::{self, Recipe};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const STAGING_DIR: &str = "local/staging";

/// Reads the update cache that `update` wrote to `local/cache.json`.
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

/// Upgrades all pending apps using their associated recipes or cache definitions.
pub fn upgrade_pending() -> Result<usize, String> {
    let pending = apps_ready_to_upgrade().map_err(|e| {
        format!(
            "{}\nRun \x1b[1mcargo run -- update\x1b[0m first to refresh the cache.",
            e
        )
    })?;

    if pending.is_empty() {
        return Ok(0);
    }

    ensure_staging()?;
    let recipes = recipe::load_recipes().unwrap_or_default();

    let mut upgraded = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for app in &pending {
        if !app.managed {
            println!(
                "  \x1b[33mSkipping unmanaged app\x1b[0m '{}'. Only rpm-managed apps can be upgraded.",
                app.name
            );
            continue;
        }

        println!(
            "\x1b[1;34m==>\x1b[0m Upgrading \x1b[1m{}\x1b[0m ([{}] → [{}])",
            app.name, app.local_version, app.remote_version
        );

        let matching_recipe = recipe::find_recipe(&app.name, &recipes);
        let bundle_name = matching_recipe
            .map(|r| r.bundle_name.as_str())
            .unwrap_or(app.name.as_str());
        let quarantine_strip = matching_recipe
            .map(|r| r.quarantine_strip)
            .unwrap_or(true);
        let expected_sha256 = matching_recipe.and_then(|r| r.sha256.as_deref());

        match install_bundle_pipeline(
            &app.name,
            bundle_name,
            &app.remote_version,
            &app.download_url,
            expected_sha256,
            quarantine_strip,
        ) {
            Ok(calculated_hash) => {
                if let Err(e) = mark_managed(
                    &app.name,
                    &app.remote_version,
                    &app.remote_version,
                    &app.download_url,
                    Some(calculated_hash),
                ) {
                    eprintln!("  \x1b[33mwarning:\x1b[0m installed but failed to update cache: {}", e);
                }
                println!(
                    "  \x1b[32m✓\x1b[0m {} installed successfully.",
                    app.name
                );
                upgraded += 1;
            }
            Err(e) => {
                eprintln!("  \x1b[31m✗\x1b[0m {}: {}", app.name, e);
                errors.push(format!("{}: {}", app.name, e));
            }
        }
    }

    if !errors.is_empty() {
        return Err(format!(
            "Finished with {} success(es) and {} failure(s):\n  - {}",
            upgraded,
            errors.len(),
            errors.join("\n  - ")
        ));
    }

    Ok(upgraded)
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
        let conflicts_with_this = other.conflicting_applications.iter().any(|c| {
            recipe.matches(c) || c.eq_ignore_ascii_case(&recipe.bundle_name)
        });
        if conflicts_with_this {
            let other_installed = local::get_recipe_local_version(other).is_some()
                || other.app_path().exists();
            if other_installed {
                let clean_bundle = if other.bundle_name.ends_with(".app") {
                    other.bundle_name.clone()
                } else {
                    format!("{}.app", other.bundle_name)
                };
                let is_managed = cache::was_managed(&other.name);
                let uninstall_hint = if is_managed {
                    let alias = other.aliases.first().map(|s| s.as_str()).unwrap_or(&other.name);
                    format!("To resolve this, uninstall it first:\n  cargo run -- uninstall {}", alias)
                } else {
                    format!("To resolve this, remove it first:\n  rm -rf '/Applications/{}'", clean_bundle)
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
                let alias = target_recipe.aliases.first().map(|s| s.as_str()).unwrap_or(&target_recipe.name);
                format!("To resolve this, uninstall it first:\n  cargo run -- uninstall {}", alias)
            } else {
                format!("To resolve this, remove it first:\n  rm -rf '/Applications/{}'", clean_bundle)
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
                format!("To resolve this, uninstall it first:\n  cargo run -- uninstall {}", conflict)
            } else {
                format!("To resolve this, remove it first:\n  rm -rf '/Applications/{}'", clean_bundle)
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
) -> Result<(), String> {
    let all_recipes = recipe::load_recipes().unwrap_or_default();
    check_conflicting_apps(recipe, &all_recipes)?;

    let clean_bundle = if recipe.bundle_name.ends_with(".app") {
        recipe.bundle_name.clone()
    } else {
        format!("{}.app", recipe.bundle_name)
    };
    let target = PathBuf::from(format!("/Applications/{}", clean_bundle));
    if target.exists() && !cache::was_managed(&recipe.name) {
        let alias = recipe.aliases.first().map(|s| s.as_str()).unwrap_or(&recipe.name);
        return Err(format!(
            "'{}' is already installed at /Applications/{} but is not managed by rpm.\n\
             For security and system integrity, rpm will not overwrite applications installed outside this package manager.\n\
             To manage '{}' with rpm:\n  \
             1. Uninstall or remove '/Applications/{}' (e.g. drag to Trash or run: rm -rf '/Applications/{}')\n  \
             2. Reinstall via rpm: cargo run -- install {}",
            recipe.name,
            clean_bundle,
            recipe.name,
            clean_bundle,
            clean_bundle,
            alias
        ));
    }

    println!(
        "\x1b[1;34m==>\x1b[0m Installing \x1b[1m{}\x1b[0m ([{}])",
        recipe.name, remote_version
    );

    let calculated_hash = install_bundle_pipeline(
        &recipe.name,
        &recipe.bundle_name,
        remote_version,
        download_url,
        recipe.sha256.as_deref(),
        recipe.quarantine_strip,
    )?;

    let local_version =
        local::get_recipe_local_version(recipe).unwrap_or_else(|| remote_version.to_string());

    mark_managed(
        &recipe.name,
        &local_version,
        remote_version,
        download_url,
        Some(calculated_hash),
    )?;

    println!(
        "  \x1b[32m✓\x1b[0m {} installed successfully.",
        recipe.name
    );
    Ok(())
}

/// Backwards-compatible install helper.
#[allow(dead_code)]
pub fn install_app(
    name: &str,
    remote_version: &str,
    download_url: &str,
) -> Result<(), String> {
    let recipes = recipe::load_recipes().unwrap_or_default();
    if let Some(r) = recipe::find_recipe(name, &recipes) {
        return install_recipe(r, remote_version, download_url);
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
    )?;

    mark_managed(
        name,
        remote_version,
        remote_version,
        download_url,
        Some(calculated_hash),
    )?;

    println!(
        "  \x1b[32m✓\x1b[0m {} installed successfully.",
        name
    );
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
                 Install it first with \x1b[1mcargo run -- install {}\x1b[0m",
                name,
                cache::CACHE_PATH,
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

    println!("  \x1b[32m✓\x1b[0m {} uninstalled.", canonical);
    Ok(())
}

/// Core pipeline: Download with SHA-256 verification, unpack, inspect, and atomically install.
fn install_bundle_pipeline(
    name: &str,
    bundle_name: &str,
    version: &str,
    download_url: &str,
    expected_sha256: Option<&str>,
    quarantine_strip: bool,
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
             2. Reinstall via rpm: cargo run -- install {}",
            name,
            clean_bundle,
            name,
            clean_bundle,
            clean_bundle,
            name.to_lowercase()
        ));
    }

    let (archive_path, sha256_hash) =
        download_for_app(name, version, download_url, expected_sha256)?;

    let staged_app = extract_app_bundle(&archive_path, bundle_name)?;
    local::validate_app_bundle(&staged_app)?;

    install_app_bundle_atomic(&staged_app, bundle_name, quarantine_strip)?;

    // Clean extract workspace; keep the downloaded archive in staging for cache/rollback
    let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&extract_root);

    Ok(sha256_hash)
}

fn staging_path() -> PathBuf {
    PathBuf::from(STAGING_DIR)
}

fn ensure_staging() -> Result<(), String> {
    fs::create_dir_all(staging_path())
        .map_err(|e| format!("Failed to create staging dir {}: {}", STAGING_DIR, e))
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

fn download_for_app(
    name: &str,
    version: &str,
    download_url: &str,
    expected_sha256: Option<&str>,
) -> Result<(PathBuf, String), String> {
    ensure_staging()?;
    let filename = filename_from_url(download_url);
    let dest = staging_path().join(format!(
        "{}-{}-{}",
        sanitize(name),
        sanitize(version),
        filename
    ));

    println!("    📥 Fetching payload from {}", download_url);
    let hash = network::download_file_with_hash(download_url, &dest, expected_sha256)?;
    println!("    🔒 SHA-256 Verified: \x1b[32m{}\x1b[0m", hash);
    Ok((dest, hash))
}

/// Extracts `.dmg`, `.zip`, `.tar.gz`, or `.tar.xz` into staging and returns path to the `.app`.
fn extract_app_bundle(archive: &Path, bundle_name: &str) -> Result<PathBuf, String> {
    let name_lower = archive
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();

    if name_lower.ends_with(".dmg") {
        extract_dmg(archive, bundle_name)
    } else if name_lower.ends_with(".zip") {
        extract_zip(archive, bundle_name)
    } else if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
        extract_tar_gz(archive, bundle_name)
    } else if name_lower.ends_with(".tar.xz") {
        extract_tar_xz(archive, bundle_name)
    } else {
        Err(format!(
            "Unsupported archive type in '{}'. Expected .dmg, .zip, .tar.gz, or .tar.xz",
            name_lower
        ))
    }
}

fn extract_dmg(dmg: &Path, bundle_name: &str) -> Result<PathBuf, String> {
    let mount = staging_path().join(format!("mnt-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&mount);
    fs::create_dir_all(&mount)
        .map_err(|e| format!("Failed to create mount point: {}", e))?;

    println!("    💿 Mounting DMG (unattended)...");
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
        println!("    📂 Staging {}...", bundle_name);
        ditto_copy(&app_src, &staged)?;
        Ok(staged)
    })();

    // Always detach cleanly
    let _ = Command::new("hdiutil")
        .args(["detach", mount.to_str().unwrap_or_default(), "-quiet"])
        .status();
    let _ = fs::remove_dir_all(&mount);

    result
}

fn extract_zip(zip: &Path, bundle_name: &str) -> Result<PathBuf, String> {
    let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&extract_root);
    fs::create_dir_all(&extract_root)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    println!("    📦 Extracting ZIP archive...");
    let status = Command::new("ditto")
        .args([
            "-x",
            "-k",
            zip.to_str().unwrap_or_default(),
            extract_root.to_str().unwrap_or_default(),
        ])
        .status()
        .map_err(|e| format!("Failed to run ditto for zip: {}", e))?;

    if !status.success() {
        return Err("ditto failed to extract zip".to_string());
    }

    find_app_bundle(&extract_root, bundle_name)
}

fn extract_tar_gz(tar: &Path, bundle_name: &str) -> Result<PathBuf, String> {
    let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&extract_root);
    fs::create_dir_all(&extract_root)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    println!("    📦 Extracting TAR.GZ archive...");
    let status = Command::new("tar")
        .args([
            "-xzf",
            tar.to_str().unwrap_or_default(),
            "-C",
            extract_root.to_str().unwrap_or_default(),
        ])
        .status()
        .map_err(|e| format!("Failed to run tar for tar.gz: {}", e))?;

    if !status.success() {
        return Err("tar failed to extract tar.gz".to_string());
    }

    find_app_bundle(&extract_root, bundle_name)
}

fn extract_tar_xz(tar: &Path, bundle_name: &str) -> Result<PathBuf, String> {
    let extract_root = staging_path().join(format!("extract-{}", sanitize(bundle_name)));
    let _ = fs::remove_dir_all(&extract_root);
    fs::create_dir_all(&extract_root)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    println!("    📦 Extracting TAR.XZ archive...");
    let status = Command::new("tar")
        .args([
            "-xJf",
            tar.to_str().unwrap_or_default(),
            "-C",
            extract_root.to_str().unwrap_or_default(),
        ])
        .status()
        .map_err(|e| format!("Failed to run tar for tar.xz: {}", e))?;

    if !status.success() {
        return Err("tar failed to extract tar.xz".to_string());
    }

    find_app_bundle(&extract_root, bundle_name)
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

fn find_app_bundle_walk(dir: &Path, bundle_name: &str, depth: usize) -> Result<Option<PathBuf>, String> {
    if depth > 4 {
        return Ok(None);
    }

    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read {}: {}", dir.display(), e))?;
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
        .args([from.to_str().unwrap_or_default(), to.to_str().unwrap_or_default()])
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

/// Atomically installs the staged bundle to `/Applications/<bundle_name>`, protecting running apps.
fn install_app_bundle_atomic(
    staged_app: &Path,
    bundle_name: &str,
    quarantine_strip: bool,
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
    println!("    🛠  Installing to {}...", target.display());

    // 1. Process active check
    if local::is_app_running(&clean_bundle) {
        println!("    ⚠️  Notice: {} is currently running.", clean_bundle);
        println!("       Attempting graceful quit before upgrade...");
        let _ = local::request_quit(&clean_bundle);
        std::thread::sleep(Duration::from_millis(800));
        if local::is_app_running(&clean_bundle) {
            println!("       App still active in memory. Utilizing atomic swap to prevent corruption.");
        }
    }

    // 2. Strip quarantine on the staged directory BEFORE copying to /Applications
    if quarantine_strip {
        println!("    🛡️  Stripping Apple Gatekeeper quarantine attribute...");
        let _ = local::remove_quarantine(staged_app);
    }

    // 3. Atomic rename backup
    let backup_path = if target.exists() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = PathBuf::from(format!("/Applications/{}.rpm-backup-{}", clean_bundle, ts));
        println!("    🔄 Backing up current install → {}", backup.display());
        fs::rename(&target, &backup)
            .map_err(|e| format!("Failed to rename old installation: {}", e))?;
        Some(backup)
    } else {
        None
    };

    // 4. Install staged bundle
    let copy_result = ditto_copy(staged_app, &target);

    match copy_result {
        Ok(()) => {
            // Verify bundle structure
            if let Err(e) = local::validate_app_bundle(&target) {
                if let Some(ref backup) = backup_path {
                    let _ = fs::rename(backup, &target);
                }
                return Err(format!("Installed bundle validation failed: {}", e));
            }

            // Also strip target in place if requested
            if quarantine_strip {
                let _ = local::remove_quarantine(&target);
            }

            // Cleanup backup
            if let Some(ref backup) = backup_path {
                let _ = fs::remove_dir_all(backup);
            }

            Ok(())
        }
        Err(err) => {
            if let Some(ref backup) = backup_path {
                eprintln!("    ❌ Installation failed. Rolling back to previous install...");
                let _ = fs::rename(backup, &target);
            }
            Err(format!("Installation failed: {}", err))
        }
    }
}

/// Record that rpm owns this install in `local/cache.json`.
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
}

