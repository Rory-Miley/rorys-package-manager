use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub bundle_name: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_quarantine_strip")]
    pub quarantine_strip: bool,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default, alias = "conflicts_with", alias = "conflicts", alias = "conflicting_apps")]
    pub conflicting_applications: Vec<String>,
    pub source: SourceConfig,
    pub livecheck: LivecheckConfig,
}

fn default_quarantine_strip() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceConfig {
    GithubRelease {
        locator: String,
        #[serde(default)]
        asset_filter: Option<HashMap<String, String>>,
    },
    CodebergRelease {
        locator: String,
        #[serde(default)]
        asset_filter: Option<HashMap<String, String>>,
    },
    DirectUrl {
        url_template: String,
    },
    ChromiumSnapshot {
        #[serde(default = "default_chromium_channel")]
        channel: String,
    },
    HomebrewCask {
        cask: String,
    },
}

fn default_chromium_channel() -> String {
    "Mac_Arm".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum LivecheckConfig {
    GithubLatestRelease,
    CodebergLatestRelease,
    ChromiumDash {
        #[serde(default = "default_chromium_dash_channel")]
        channel: String,
    },
    RegexScrape {
        url: String,
        regex: String,
    },
    RawUrl {
        url: String,
    },
    HomebrewCask {
        cask: String,
    },
}

fn default_chromium_dash_channel() -> String {
    "Stable".to_string()
}

impl Recipe {
    /// Checks if a given query matches this recipe's canonical name or any alias.
    pub fn matches(&self, query: &str) -> bool {
        if self.name.eq_ignore_ascii_case(query) {
            return true;
        }
        self.aliases.iter().any(|a| a.eq_ignore_ascii_case(query))
    }

    /// Helper to get bundle path under /Applications.
    #[allow(dead_code)]
    pub fn app_path(&self) -> PathBuf {
        PathBuf::from("/Applications").join(&self.bundle_name)
    }
}

/// Helper to recursively collect all .toml recipe files within a directory tree.
fn collect_toml_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_toml_files(&path, files);
            } else if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                files.push(path);
            }
        }
    }
}

/// Loads all recipes from the active recipes directory, including subdirectories.
pub fn load_recipes() -> Result<Vec<Recipe>, String> {
    load_recipes_from_dir(&paths::recipes_dir())
}

pub fn load_recipes_from_dir(dir: &Path) -> Result<Vec<Recipe>, String> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create recipes directory {}: {}", dir.display(), e))?;
        return Ok(Vec::new());
    }

    let mut toml_files = Vec::new();
    collect_toml_files(dir, &mut toml_files);

    let mut recipes = Vec::new();
    for path in toml_files {
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read recipe file {}: {}", path.display(), e))?;
        let recipe: Recipe = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse recipe {}: {}", path.display(), e))?;
        recipes.push(recipe);
    }

    // Sort deterministically by canonical name and deduplicate (e.g. if alias symlinks exist)
    recipes.sort_by(|a, b| a.name.cmp(&b.name));
    recipes.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    Ok(recipes)
}

/// Resolves a recipe by name or alias from a list of recipes.
pub fn find_recipe<'a>(query: &str, recipes: &'a [Recipe]) -> Option<&'a Recipe> {
    recipes.iter().find(|r| r.matches(query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_qbittorrent_recipe() {
        let toml_str = r#"
            name = "qBittorrent"
            aliases = ["qbittorrent", "qbt"]
            bundle_name = "qBittorrent.app"
            homepage = "https://www.qbittorrent.org"
            quarantine_strip = true

            [source]
            type = "github_release"
            locator = "qbittorrent/qBittorrent"

            [livecheck]
            strategy = "github_latest_release"
        "#;

        let recipe: Recipe = toml::from_str(toml_str).expect("Failed to deserialize recipe");
        assert_eq!(recipe.name, "qBittorrent");
        assert!(recipe.matches("qbt"));
        assert!(recipe.matches("qbittorrent"));
        assert!(recipe.matches("QBITTORRENT"));
        assert!(!recipe.matches("firefox"));
        assert!(recipe.quarantine_strip);
        assert!(recipe.conflicting_applications.is_empty());
    }

    #[test]
    fn test_parse_conflicting_applications() {
        let toml_a = r#"
            name = "Chromium"
            bundle_name = "Chromium.app"
            conflicting_applications = ["Chromium Snapshot"]

            [source]
            type = "chromium_snapshot"

            [livecheck]
            strategy = "github_latest_release"
        "#;

        let recipe_a: Recipe = toml::from_str(toml_a).expect("Failed to deserialize recipe");
        assert_eq!(recipe_a.conflicting_applications, vec!["Chromium Snapshot"]);

        let toml_b = r#"
            name = "Chromium Snapshot"
            bundle_name = "Chromium Snapshot.app"
            conflicts_with = ["Chromium"]

            [source]
            type = "chromium_snapshot"

            [livecheck]
            strategy = "github_latest_release"
        "#;

        let recipe_b: Recipe = toml::from_str(toml_b).expect("Failed to deserialize recipe");
        assert_eq!(recipe_b.conflicting_applications, vec!["Chromium"]);
    }

    #[test]
    fn test_parse_homebrew_cask_recipe() {
        let toml_str = r#"
            name = "VLC media player"
            aliases = ["vlc"]
            bundle_name = "VLC.app"
            homepage = "https://www.videolan.org/vlc/"
            quarantine_strip = true

            [source]
            type = "homebrew_cask"
            cask = "vlc"

            [livecheck]
            strategy = "homebrew_cask"
            cask = "vlc"
        "#;

        let recipe: Recipe = toml::from_str(toml_str).expect("Failed to deserialize cask recipe");
        assert_eq!(recipe.name, "VLC media player");
        assert!(recipe.matches("vlc"));
        match recipe.source {
            SourceConfig::HomebrewCask { cask } => assert_eq!(cask, "vlc"),
            _ => panic!("Expected HomebrewCask source"),
        }
        match recipe.livecheck {
            LivecheckConfig::HomebrewCask { cask } => assert_eq!(cask, "vlc"),
            _ => panic!("Expected HomebrewCask livecheck"),
        }
    }

    #[test]
    fn test_recursive_recipe_loading() {
        let tmp = tempfile::tempdir().unwrap();
        let sub_cask = tmp.path().join("homebrew").join("cask");
        fs::create_dir_all(&sub_cask).unwrap();

        let top_file = tmp.path().join("app1.toml");
        let sub_file = sub_cask.join("app2.toml");

        let toml1 = r#"
            name = "App One"
            bundle_name = "AppOne.app"
            [source]
            type = "direct_url"
            url_template = "https://example.com/one.dmg"
            [livecheck]
            strategy = "raw_url"
            url = "https://example.com/version"
        "#;

        let toml2 = r#"
            name = "App Two"
            bundle_name = "AppTwo.app"
            [source]
            type = "homebrew_cask"
            cask = "app2"
            [livecheck]
            strategy = "homebrew_cask"
            cask = "app2"
        "#;

        fs::write(top_file, toml1).unwrap();
        fs::write(sub_file, toml2).unwrap();

        let loaded = load_recipes_from_dir(tmp.path()).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "App One");
        assert_eq!(loaded[1].name, "App Two");
    }
}

