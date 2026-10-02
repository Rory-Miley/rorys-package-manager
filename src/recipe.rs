use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const RECIPES_DIR: &str = "recipes";

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

/// Loads all recipes from the recipes directory.
pub fn load_recipes() -> Result<Vec<Recipe>, String> {
    load_recipes_from_dir(Path::new(RECIPES_DIR))
}

pub fn load_recipes_from_dir(dir: &Path) -> Result<Vec<Recipe>, String> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create recipes directory {}: {}", dir.display(), e))?;
        return Ok(Vec::new());
    }

    let mut recipes = Vec::new();
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("Failed to read recipes directory {}: {}", dir.display(), e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("toml") {
            let content = fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read recipe file {}: {}", path.display(), e))?;
            let recipe: Recipe = toml::from_str(&content)
                .map_err(|e| format!("Failed to parse recipe {}: {}", path.display(), e))?;
            recipes.push(recipe);
        }
    }

    // Sort deterministically by canonical name
    recipes.sort_by(|a, b| a.name.cmp(&b.name));
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
}

