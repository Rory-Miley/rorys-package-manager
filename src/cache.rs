use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Project-local update index written by `update` / `install` and read by `upgrade` / installer.
/// Path: `<repo>/local/cache.json` (relative to the process working directory for now).
pub const CACHE_PATH: &str = "local/cache.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedApp {
    pub name: String,
    /// Version currently installed under /Applications, or "Not Installed".
    pub local_version: String,
    /// Latest upstream version discovered during update/install.
    pub remote_version: String,
    /// Direct download URL for the matching macOS asset (empty if unknown).
    pub download_url: String,
    pub update_available: bool,
    /// True when this tool installed or upgraded the app (eligible for `uninstall`).
    #[serde(default)]
    pub managed: bool,
    /// Cryptographic SHA-256 hash of the installed download asset (if verified/recorded).
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCache {
    /// Unix seconds when this cache was last written.
    pub updated_at: u64,
    pub apps: Vec<CachedApp>,
}

impl UpdateCache {
    pub fn new(apps: Vec<CachedApp>) -> Self {
        Self {
            updated_at: now_unix(),
            apps,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = now_unix();
    }

    pub fn find(&self, query: &str) -> Option<&CachedApp> {
        self.apps
            .iter()
            .find(|app| app.name.eq_ignore_ascii_case(query))
    }

    pub fn find_mut(&mut self, query: &str) -> Option<&mut CachedApp> {
        self.apps
            .iter_mut()
            .find(|app| app.name.eq_ignore_ascii_case(query))
    }

    /// Insert or replace an app entry by case-insensitive name.
    pub fn upsert(&mut self, app: CachedApp) {
        if let Some(existing) = self.find_mut(&app.name) {
            *existing = app;
        } else {
            self.apps.push(app);
        }
        self.touch();
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn cache_file_path() -> PathBuf {
    PathBuf::from(CACHE_PATH)
}

/// Ensures `local/` exists, then writes the update cache as pretty JSON.
pub fn save_cache(cache: &UpdateCache) -> Result<(), String> {
    let path = cache_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create cache directory {:?}: {}", parent, e))?;
    }

    let json = serde_json::to_string_pretty(cache)
        .map_err(|e| format!("Failed to serialise cache: {}", e))?;

    fs::write(&path, json).map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// Loads `local/cache.json`. Returns an error if missing or invalid.
pub fn load_cache() -> Result<UpdateCache, String> {
    load_cache_from(&cache_file_path())
}

/// Loads cache, or an empty one if the file does not exist yet.
pub fn load_cache_or_empty() -> Result<UpdateCache, String> {
    let path = cache_file_path();
    if !path.exists() {
        return Ok(UpdateCache::new(Vec::new()));
    }
    load_cache_from(&path)
}

pub fn load_cache_from(path: &Path) -> Result<UpdateCache, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    serde_json::from_str(&raw).map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
}

/// Apps that have an update and a download URL ready for the installer.
pub fn pending_upgrades(cache: &UpdateCache) -> Vec<&CachedApp> {
    cache
        .apps
        .iter()
        .filter(|app| app.update_available && !app.download_url.is_empty())
        .collect()
}

/// Apps this tool has installed or upgraded (`managed: true`).
pub fn managed_apps(cache: &UpdateCache) -> Vec<&CachedApp> {
    cache.apps.iter().filter(|app| app.managed).collect()
}

/// Look up whether an app was previously marked managed (for preserving across `update`).
pub fn was_managed(name: &str) -> bool {
    load_cache_or_empty()
        .ok()
        .and_then(|c| c.find(name).map(|a| a.managed))
        .unwrap_or(false)
}
