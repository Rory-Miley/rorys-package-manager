use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::HomebrewTrack;
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
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub track: Option<String>,
    #[serde(skip)]
    pub recipe_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeOrigin {
    Rpm,
    Homebrew { track: String },
}

impl RecipeOrigin {
    pub fn display_label(&self) -> String {
        match self {
            RecipeOrigin::Rpm => "rpm".to_string(),
            RecipeOrigin::Homebrew { track } => {
                if track.eq_ignore_ascii_case("default") || track.eq_ignore_ascii_case("verified") {
                    "homebrew (verified)".to_string()
                } else if track.eq_ignore_ascii_case("upstream") {
                    "homebrew (upstream)".to_string()
                } else {
                    format!("homebrew ({})", track)
                }
            }
        }
    }

    pub fn colored_badge(&self) -> String {
        match self {
            RecipeOrigin::Rpm => "[recipe: \x1b[34mrpm\x1b[0m]".to_string(),
            RecipeOrigin::Homebrew { .. } => {
                format!("[recipe: \x1b[35m{}\x1b[0m]", self.display_label())
            }
        }
    }
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

    /// Determines the origin and download track of this recipe (rpm native vs homebrew cask).
    pub fn origin(&self) -> RecipeOrigin {
        // 1. Check explicit origin / track in recipe manifest
        if let Some(ref orig) = self.origin {
            if orig.eq_ignore_ascii_case("homebrew") {
                let trk = self.track.clone().unwrap_or_else(|| {
                    if matches!(self.source, SourceConfig::HomebrewCask { .. }) {
                        "verified".to_string()
                    } else {
                        "upstream".to_string()
                    }
                });
                return RecipeOrigin::Homebrew { track: trk };
            } else if orig.eq_ignore_ascii_case("rpm") {
                return RecipeOrigin::Rpm;
            }
        }

        // 2. Check recipe file path if available
        if let Some(ref path) = self.recipe_path {
            if path.components().any(|c| c.as_os_str() == "homebrew") {
                let trk = self.track.clone().unwrap_or_else(|| {
                    if matches!(self.source, SourceConfig::HomebrewCask { .. }) {
                        "verified".to_string()
                    } else {
                        "upstream".to_string()
                    }
                });
                return RecipeOrigin::Homebrew { track: trk };
            }
            if path.components().any(|c| c.as_os_str() == "rpm") {
                return RecipeOrigin::Rpm;
            }
        }

        // 3. Fallback to source / livecheck inspection
        if matches!(self.source, SourceConfig::HomebrewCask { .. })
            || matches!(self.livecheck, LivecheckConfig::HomebrewCask { .. })
        {
            let trk = self.track.clone().unwrap_or_else(|| {
                if matches!(self.source, SourceConfig::HomebrewCask { .. }) {
                    "verified".to_string()
                } else {
                    "upstream".to_string()
                }
            });
            RecipeOrigin::Homebrew { track: trk }
        } else {
            RecipeOrigin::Rpm
        }
    }

    /// Checks if this recipe matches the requested Homebrew track.
    /// Returns true for native recipes (track is ignored), or if the recipe's track matches `requested`.
    pub fn matches_homebrew_track(&self, requested: HomebrewTrack) -> bool {
        match (self.origin(), requested) {
            (RecipeOrigin::Homebrew { track }, HomebrewTrack::Upstream) => {
                track.eq_ignore_ascii_case("upstream")
            }
            (RecipeOrigin::Homebrew { track }, HomebrewTrack::Default) => {
                track.eq_ignore_ascii_case("default") || track.eq_ignore_ascii_case("verified")
            }
            (RecipeOrigin::Homebrew { .. }, HomebrewTrack::Request) => true,
            (RecipeOrigin::Rpm, _) => true,
        }
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
        let mut recipe: Recipe = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse recipe {}: {}", path.display(), e))?;
        recipe.recipe_path = Some(path.clone());
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

/// If the given recipe originated from Homebrew, removes its local recipe file from recipes/homebrew/cask/.
/// Native recipes in recipes/rpm/ are never deleted. Returns true if a recipe file was removed.
pub fn remove_cask_recipe(recipe: &Recipe) -> bool {
    if matches!(recipe.origin(), RecipeOrigin::Homebrew { .. }) {
        if let Some(ref path) = recipe.recipe_path {
            if path.exists() && path.components().any(|c| c.as_os_str() == "homebrew") {
                let _ = fs::remove_file(path);
                return true;
            }
        }
        for candidate in std::iter::once(&recipe.name).chain(recipe.aliases.iter()) {
            let cask_path = paths::recipes_dir()
                .join("homebrew")
                .join("cask")
                .join(format!("{}.toml", candidate.to_lowercase()));
            if cask_path.exists() {
                let _ = fs::remove_file(&cask_path);
                return true;
            }
        }
    }
    false
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
        assert_eq!(loaded[0].origin(), RecipeOrigin::Rpm);
        assert_eq!(
            loaded[1].origin(),
            RecipeOrigin::Homebrew {
                track: "verified".to_string()
            }
        );
    }

    #[test]
    fn test_recipe_origin_detection() {
        let rpm_toml = r#"
            name = "NativeApp"
            bundle_name = "NativeApp.app"
            [source]
            type = "direct_url"
            url_template = "https://example.com/native.dmg"
            [livecheck]
            strategy = "raw_url"
            url = "https://example.com/ver"
        "#;
        let r_rpm: Recipe = toml::from_str(rpm_toml).unwrap();
        assert_eq!(r_rpm.origin(), RecipeOrigin::Rpm);

        let hb_ver_toml = r#"
            name = "CaskApp"
            bundle_name = "CaskApp.app"
            origin = "homebrew"
            track = "verified"
            [source]
            type = "homebrew_cask"
            cask = "cask-app"
            [livecheck]
            strategy = "homebrew_cask"
            cask = "cask-app"
        "#;
        let r_hb_ver: Recipe = toml::from_str(hb_ver_toml).unwrap();
        assert_eq!(
            r_hb_ver.origin(),
            RecipeOrigin::Homebrew {
                track: "verified".to_string()
            }
        );
        assert_eq!(r_hb_ver.origin().display_label(), "homebrew (verified)");

        let hb_up_toml = r#"
            name = "UpstreamCask"
            bundle_name = "UpstreamCask.app"
            origin = "homebrew"
            track = "upstream"
            [source]
            type = "github_release"
            locator = "owner/repo"
            [livecheck]
            strategy = "github_latest_release"
        "#;
        let r_hb_up: Recipe = toml::from_str(hb_up_toml).unwrap();
        assert_eq!(
            r_hb_up.origin(),
            RecipeOrigin::Homebrew {
                track: "upstream".to_string()
            }
        );
        assert_eq!(r_hb_up.origin().display_label(), "homebrew (upstream)");
    }

    #[test]
    fn test_remove_cask_recipe() {
        let tmp = tempfile::tempdir().unwrap();
        let hb_dir = tmp.path().join("homebrew").join("cask");
        let rpm_dir = tmp.path().join("rpm");
        fs::create_dir_all(&hb_dir).unwrap();
        fs::create_dir_all(&rpm_dir).unwrap();

        let hb_file = hb_dir.join("test-cask.toml");
        fs::write(&hb_file, "name = \"test-cask\"\n").unwrap();

        let rpm_file = rpm_dir.join("native-app.toml");
        fs::write(&rpm_file, "name = \"native-app\"\n").unwrap();

        let mut hb_recipe: Recipe = toml::from_str(r#"
            name = "TestCask"
            bundle_name = "TestCask.app"
            origin = "homebrew"
            track = "verified"
            [source]
            type = "homebrew_cask"
            cask = "test-cask"
            [livecheck]
            strategy = "homebrew_cask"
            cask = "test-cask"
        "#).unwrap();
        hb_recipe.recipe_path = Some(hb_file.clone());

        let mut rpm_recipe: Recipe = toml::from_str(r#"
            name = "NativeApp"
            bundle_name = "NativeApp.app"
            [source]
            type = "direct_url"
            url_template = "https://example.com/app.dmg"
            [livecheck]
            strategy = "raw_url"
            url = "https://example.com/ver"
        "#).unwrap();
        rpm_recipe.recipe_path = Some(rpm_file.clone());

        assert!(hb_file.exists());
        assert!(rpm_file.exists());

        // Native recipes should never be deleted
        assert!(!remove_cask_recipe(&rpm_recipe));
        assert!(rpm_file.exists());

        // Homebrew recipes should be deleted
        assert!(remove_cask_recipe(&hb_recipe));
        assert!(!hb_file.exists());
    }

    #[test]
    fn test_matches_homebrew_track() {
        let hb_ver: Recipe = toml::from_str(r#"
            name = "Test"
            bundle_name = "Test.app"
            origin = "homebrew"
            track = "verified"
            [source]
            type = "homebrew_cask"
            cask = "test"
            [livecheck]
            strategy = "homebrew_cask"
            cask = "test"
        "#).unwrap();

        assert!(hb_ver.matches_homebrew_track(HomebrewTrack::Default));
        assert!(!hb_ver.matches_homebrew_track(HomebrewTrack::Upstream));
        assert!(hb_ver.matches_homebrew_track(HomebrewTrack::Request));

        let hb_up: Recipe = toml::from_str(r#"
            name = "Test"
            bundle_name = "Test.app"
            origin = "homebrew"
            track = "upstream"
            [source]
            type = "github_release"
            locator = "owner/repo"
            [livecheck]
            strategy = "github_latest_release"
        "#).unwrap();

        assert!(!hb_up.matches_homebrew_track(HomebrewTrack::Default));
        assert!(hb_up.matches_homebrew_track(HomebrewTrack::Upstream));
        assert!(hb_up.matches_homebrew_track(HomebrewTrack::Request));

        let rpm_rec: Recipe = toml::from_str(r#"
            name = "Native"
            bundle_name = "Native.app"
            [source]
            type = "direct_url"
            url_template = "https://example.com/app.dmg"
            [livecheck]
            strategy = "raw_url"
            url = "https://example.com/ver"
        "#).unwrap();

        // Native recipes match any track (track is ignored)
        assert!(rpm_rec.matches_homebrew_track(HomebrewTrack::Default));
        assert!(rpm_rec.matches_homebrew_track(HomebrewTrack::Upstream));
        assert!(rpm_rec.matches_homebrew_track(HomebrewTrack::Request));
    }
}

