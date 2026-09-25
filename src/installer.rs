use crate::cache::{self, CachedApp, UpdateCache};
use crate::local;
use std::fs::{self, File};
use std::io::copy;
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

/// Downloads, stages, installs, and strips quarantine for every pending app in the cache.
/// Returns how many apps were upgraded successfully.
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

    let mut upgraded = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for app in &pending {
        println!(
            "\x1b[1;34m==>\x1b[0m Upgrading \x1b[1m{}\x1b[0m ({} → {})",
            app.name, app.local_version, app.remote_version
        );
        match upgrade_one(app) {
            Ok(()) => {
                if let Err(e) = mark_managed(
                    &app.name,
                    &app.remote_version,
                    &app.remote_version,
                    &app.download_url,
                ) {
                    eprintln!("  \x1b[33mwarning:\x1b[0m installed but failed to update cache: {}", e);
                }
                println!(
                    "  \x1b[32m✓\x1b[0m {} installed and quarantine stripped.",
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

/// Install (or reinstall) one app from an explicit release URL.
pub fn install_app(
    name: &str,
    remote_version: &str,
    download_url: &str,
) -> Result<(), String> {
    let local_version =
        local::get_local_version(name).unwrap_or_else(|| "Not Installed".to_string());

    let app = CachedApp {
        name: name.to_string(),
        local_version,
        remote_version: remote_version.to_string(),
        download_url: download_url.to_string(),
        update_available: true,
        managed: false,
    };

    println!(
        "\x1b[1;34m==>\x1b[0m Installing \x1b[1m{}\x1b[0m ({})",
        name, remote_version
    );
    upgrade_one(&app)?;
    mark_managed(name, remote_version, remote_version, download_url)?;
    println!(
        "  \x1b[32m✓\x1b[0m {} installed and quarantine stripped.",
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

    let canonical = entry.as_ref().map(|a| a.name.as_str()).unwrap_or(name);
    let target = PathBuf::from(format!("/Applications/{}.app", canonical));

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

/// Record that rpm owns this install in `local/cache.json`.
fn mark_managed(
    name: &str,
    local_version: &str,
    remote_version: &str,
    download_url: &str,
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
    });
    cache::save_cache(&cache)
}

/// Full install path for a single cached app entry.
fn upgrade_one(app: &CachedApp) -> Result<(), String> {
    if app.download_url.is_empty() {
        return Err("No download URL in cache".to_string());
    }

    let archive_path = download_for_app(app)?;
    let staged_app = extract_app_bundle(&archive_path, &app.name)?;

    install_app_bundle(&staged_app, &app.name)?;
    local::remove_quarantine(&app.name)?;

    // Clean extract workspace; keep the downloaded archive for now (rollback later).
    let extract_root = staging_path().join(format!("extract-{}", sanitize(&app.name)));
    let _ = fs::remove_dir_all(&extract_root);

    Ok(())
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

fn download_for_app(app: &CachedApp) -> Result<PathBuf, String> {
    ensure_staging()?;
    let filename = filename_from_url(&app.download_url);
    let dest = staging_path().join(format!(
        "{}-{}-{}",
        sanitize(&app.name),
        sanitize(&app.remote_version),
        filename
    ));

    if dest.exists() {
        println!("    📦 Using existing download: {}", dest.display());
        return Ok(dest);
    }

    download_file(&app.download_url, &dest)?;
    Ok(dest)
}

/// Downloads a file from a URL and streams it to `destination_path`.
pub fn download_file(url: &str, destination_path: &Path) -> Result<(), String> {
    if let Some(parent) = destination_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create download directory: {}", e))?;
    }

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

    println!("    💾 Download complete → {}", destination_path.display());
    Ok(())
}

/// Extracts a `.dmg` or `.zip` into staging and returns the path to the `.app` bundle.
fn extract_app_bundle(archive: &Path, app_name: &str) -> Result<PathBuf, String> {
    let ext = archive
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "dmg" => extract_dmg(archive, app_name),
        "zip" => extract_zip(archive, app_name),
        other => Err(format!(
            "Unsupported archive type '.{}'. Expected .dmg or .zip",
            other
        )),
    }
}

fn extract_dmg(dmg: &Path, app_name: &str) -> Result<PathBuf, String> {
    let mount = staging_path().join(format!("mnt-{}", sanitize(app_name)));
    // Fresh mount point
    let _ = fs::remove_dir_all(&mount);
    fs::create_dir_all(&mount)
        .map_err(|e| format!("Failed to create mount point: {}", e))?;

    println!("    💿 Mounting DMG...");
    let attach = Command::new("hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-mountpoint",
            mount.to_str().unwrap_or_default(),
            dmg.to_str().unwrap_or_default(),
        ])
        .output()
        .map_err(|e| format!("Failed to run hdiutil attach: {}", e))?;

    if !attach.status.success() {
        let stderr = String::from_utf8_lossy(&attach.stderr);
        let _ = fs::remove_dir_all(&mount);
        return Err(format!("hdiutil attach failed: {}", stderr.trim()));
    }

    let result = (|| {
        let app_src = find_app_bundle(&mount, app_name)?;
        let extract_root = staging_path().join(format!("extract-{}", sanitize(app_name)));
        let _ = fs::remove_dir_all(&extract_root);
        fs::create_dir_all(&extract_root)
            .map_err(|e| format!("Failed to create extract dir: {}", e))?;

        let staged = extract_root.join(format!("{}.app", app_name));
        println!("    📂 Staging {}...", app_name);
        ditto_copy(&app_src, &staged)?;
        Ok(staged)
    })();

    // Always detach, even on failure
    let detach = Command::new("hdiutil")
        .args(["detach", mount.to_str().unwrap_or_default(), "-quiet"])
        .status();
    if let Ok(status) = detach {
        if !status.success() {
            // Force detach if quiet detach failed
            let _ = Command::new("hdiutil")
                .args(["detach", mount.to_str().unwrap_or_default(), "-force"])
                .status();
        }
    }
    let _ = fs::remove_dir_all(&mount);

    result
}

fn extract_zip(zip: &Path, app_name: &str) -> Result<PathBuf, String> {
    let extract_root = staging_path().join(format!("extract-{}", sanitize(app_name)));
    let _ = fs::remove_dir_all(&extract_root);
    fs::create_dir_all(&extract_root)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    println!("    📦 Extracting ZIP...");
    // ditto -x -k handles zip natively on macOS
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

    find_app_bundle(&extract_root, app_name)
}

/// Finds `{app_name}.app` under `root`, searching one level then recursively.
fn find_app_bundle(root: &Path, app_name: &str) -> Result<PathBuf, String> {
    let preferred = root.join(format!("{}.app", app_name));
    if preferred.is_dir() {
        return Ok(preferred);
    }

    // Case-insensitive match on *.app at top level
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("app") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem.eq_ignore_ascii_case(app_name) {
                        return Ok(path);
                    }
                }
            }
        }
    }

    // Shallow recursive walk (DMGs sometimes nest under a folder)
    find_app_bundle_walk(root, app_name, 0)?
        .ok_or_else(|| format!("Could not find {}.app inside archive", app_name))
}

fn find_app_bundle_walk(dir: &Path, app_name: &str, depth: usize) -> Result<Option<PathBuf>, String> {
    if depth > 3 {
        return Ok(None);
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read {}: {}", dir.display(), e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("app") && path.is_dir() {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if stem.eq_ignore_ascii_case(app_name) {
                    return Ok(Some(path));
                }
            }
            // Any .app if name matches poorly — only accept exact/ci name
        }
        if path.is_dir() && path.extension().and_then(|e| e.to_str()) != Some("app") {
            if let Some(found) = find_app_bundle_walk(&path, app_name, depth + 1)? {
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

/// Copies the staged `.app` into `/Applications`, replacing an existing install safely.
/// Uses `ditto` so an existing bundle gets content-replaced in place when possible.
fn install_app_bundle(staged_app: &Path, app_name: &str) -> Result<(), String> {
    let target = PathBuf::from(format!("/Applications/{}.app", app_name));
    println!("    🛠  Installing to {}...", target.display());

    // If the staged folder isn't already named correctly, still install as AppName.app
    ditto_copy(staged_app, &target)?;

    if !target.exists() {
        return Err(format!(
            "Install appeared to succeed but {} is missing",
            target.display()
        ));
    }

    Ok(())
}
