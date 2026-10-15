use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HomebrewTrack {
    Default,
    Upstream,
    Request,
}

impl HomebrewTrack {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Upstream => "upstream",
            Self::Request => "request",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let clean = s.trim().trim_start_matches('-').trim_start_matches('-');
        match clean.to_ascii_lowercase().as_str() {
            "default" | "verified" => Some(Self::Default),
            "upstream" => Some(Self::Upstream),
            "request" | "prompt" => Some(Self::Request),
            _ => None,
        }
    }
}

impl std::fmt::Display for HomebrewTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomebrewConfig {
    #[serde(default = "default_track")]
    pub track: HomebrewTrack,
}

fn default_track() -> HomebrewTrack {
    HomebrewTrack::Request
}

impl Default for HomebrewConfig {
    fn default() -> Self {
        Self {
            track: default_track(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub homebrew: HomebrewConfig,
}

impl Config {
    pub fn load_or_default() -> Self {
        let path = paths::config_file_path();
        Self::load_from_path(&path).unwrap_or_default()
    }

    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file {}: {}", path.display(), e))?;
        toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config file {}: {}", path.display(), e))
    }

    pub fn save(&self) -> Result<(), String> {
        let path = paths::config_file_path();
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        fs::write(path, content)
            .map_err(|e| format!("Failed to write config file {}: {}", path.display(), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_track() {
        assert_eq!(HomebrewTrack::parse("default"), Some(HomebrewTrack::Default));
        assert_eq!(HomebrewTrack::parse("-default"), Some(HomebrewTrack::Default));
        assert_eq!(HomebrewTrack::parse("--default"), Some(HomebrewTrack::Default));
        assert_eq!(HomebrewTrack::parse("verified"), Some(HomebrewTrack::Default));
        assert_eq!(HomebrewTrack::parse("--verified"), Some(HomebrewTrack::Default));

        assert_eq!(HomebrewTrack::parse("upstream"), Some(HomebrewTrack::Upstream));
        assert_eq!(HomebrewTrack::parse("-upstream"), Some(HomebrewTrack::Upstream));

        assert_eq!(HomebrewTrack::parse("request"), Some(HomebrewTrack::Request));
        assert_eq!(HomebrewTrack::parse("-request"), Some(HomebrewTrack::Request));
        assert_eq!(HomebrewTrack::parse("prompt"), Some(HomebrewTrack::Request));

        assert_eq!(HomebrewTrack::parse("invalid"), None);
    }

    #[test]
    fn test_load_save_config() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();

        let mut config = Config::default();
        assert_eq!(config.homebrew.track, HomebrewTrack::Request);

        config.homebrew.track = HomebrewTrack::Upstream;
        config.save_to_path(path).unwrap();

        let loaded = Config::load_from_path(path).unwrap();
        assert_eq!(loaded.homebrew.track, HomebrewTrack::Upstream);
    }
}
