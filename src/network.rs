use crate::recipe::{LivecheckConfig, Recipe, SourceConfig};
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

#[derive(Deserialize, Debug)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize, Debug)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize, Debug)]
struct CodebergAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize, Debug)]
struct CodebergRelease {
    tag_name: String,
    assets: Vec<CodebergAsset>,
}

#[derive(Deserialize, Debug)]
struct ChromiumRelease {
    version: String,
}

pub fn detect_mac_architecture() -> &'static str {
    if std::env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        "x86_64"
    }
}

pub fn normalise_version(version: &str) -> String {
    let trimmed = version.trim();
    if let Some(stripped) = trimmed.strip_prefix("release-") {
        stripped.to_string()
    } else if trimmed.starts_with('v') || trimmed.starts_with('V') {
        trimmed[1..].to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))
}

/// Resolves a recipe into its latest version and download URL.
pub fn resolve_recipe(recipe: &Recipe) -> Result<(String, String), String> {
    match &recipe.source {
        SourceConfig::GithubRelease { locator, asset_filter } => {
            fetch_github_release_with_filter(locator, asset_filter.as_ref())
        }
        SourceConfig::CodebergRelease { locator, asset_filter } => {
            fetch_codeberg_release_with_filter(locator, asset_filter.as_ref())
        }
        SourceConfig::ChromiumSnapshot { channel } => {
            let arch = detect_mac_architecture();
            let effective_channel = if channel == "auto" {
                if arch == "arm64" { "Mac_Arm" } else { "Mac" }
            } else {
                channel.as_str()
            };

            let version = resolve_livecheck(&recipe.livecheck, effective_channel)?;
            let download_url = format!(
                "https://commondatastorage.googleapis.com/chromium-browser-snapshots/{}/{}/chrome-mac.zip",
                effective_channel, version
            );
            Ok((version, download_url))
        }
        SourceConfig::DirectUrl { url_template } => {
            let version = resolve_livecheck(&recipe.livecheck, "")?;
            let arch = detect_mac_architecture();
            let url = url_template
                .replace("{version}", &version)
                .replace("{arch}", arch);
            Ok((version, url))
        }
    }
}

/// Dispatches livecheck strategies to retrieve the current upstream version.
pub fn resolve_livecheck(livecheck: &LivecheckConfig, context: &str) -> Result<String, String> {
    match livecheck {
        LivecheckConfig::GithubLatestRelease => {
            Err("GithubLatestRelease livecheck should be evaluated via GitHub source".to_string())
        }
        LivecheckConfig::CodebergLatestRelease => {
            Err("CodebergLatestRelease livecheck should be evaluated via Codeberg source".to_string())
        }
        LivecheckConfig::ChromiumDash { channel } => {
            fetch_chromium_version(channel)
        }
        LivecheckConfig::RawUrl { url } => {
            let resolved_url = if url.contains("{channel}") {
                url.replace("{channel}", context)
            } else {
                url.clone()
            };
            fetch_raw_url_version(&resolved_url)
        }
        LivecheckConfig::RegexScrape { url, regex } => {
            fetch_regex_scraped_version(url, regex)
        }
    }
}

/// Picks the best macOS download URL for the current CPU architecture and container formats.
pub fn pick_mac_download_url<'a, I>(
    asset_names_and_urls: I,
    repo: &str,
    asset_filter: Option<&HashMap<String, String>>,
) -> Result<String, String>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let current_arch = detect_mac_architecture();
    let assets: Vec<(&str, &str)> = asset_names_and_urls.into_iter().collect();

    // 1. If explicit recipe filter exists for current arch or fallback, try regex matching
    if let Some(filter_map) = asset_filter {
        if let Some(pattern) = filter_map.get(current_arch).or_else(|| filter_map.get("fallback")) {
            if let Ok(re) = Regex::new(pattern) {
                if let Some((_, url)) = assets.iter().find(|(name, _)| re.is_match(name)) {
                    return Ok(url.to_string());
                }
            }
        }
    }

    // 2. Standard heuristic matching: check for macOS container + architecture keywords
    let arch_match = assets.iter().find(|(name, _)| {
        let name_lower = name.to_lowercase();
        let is_mac_file = name_lower.ends_with(".dmg")
            || name_lower.ends_with(".zip")
            || name_lower.ends_with(".tar.gz")
            || name_lower.ends_with(".tar.xz")
            || name_lower.contains("mac")
            || name_lower.contains("macos")
            || name_lower.contains("darwin")
            || name_lower.contains("osx");

        let matches_arch = if current_arch == "arm64" {
            name_lower.contains("arm64")
                || name_lower.contains("aarch64")
                || name_lower.contains("apple-silicon")
                || name_lower.contains("m1")
        } else {
            name_lower.contains("x86_64")
                || name_lower.contains("intel")
                || name_lower.contains("x64")
                || name_lower.contains("amd64")
        };

        let is_universal = name_lower.contains("universal");
        is_mac_file && (matches_arch || is_universal)
    });

    if let Some((_, url)) = arch_match {
        return Ok(url.to_string());
    }

    // 3. Fallback: any .dmg or .zip if no arch-specific asset was found
    assets
        .iter()
        .find(|(name, _)| {
            let n = name.to_lowercase();
            n.ends_with(".dmg") || n.ends_with(".zip")
        })
        .map(|(_, url)| url.to_string())
        .ok_or_else(|| {
            format!(
                "Could not find a valid macOS binary asset in repo: {}",
                repo
            )
        })
}

/// Queries GitHub Releases for the latest release that includes a matching macOS download URL.
pub fn fetch_github_release_with_filter(
    repo: &str,
    asset_filter: Option<&HashMap<String, String>>,
) -> Result<(String, String), String> {
    let url = format!("https://api.github.com/repos/{}/releases?per_page=10", repo);
    let client = build_client()?;

    let response = client
        .get(&url)
        .header("User-Agent", "rorys-package-manager-cli")
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "GitHub API returned HTTP status: {}",
            response.status()
        ));
    }

    let releases: Vec<GitHubRelease> = response
        .json()
        .map_err(|e| format!("Failed to parse JSON payload: {}", e))?;

    for release in &releases {
        let version = normalise_version(&release.tag_name);
        if let Ok(download_url) = pick_mac_download_url(
            release
                .assets
                .iter()
                .map(|a| (a.name.as_str(), a.browser_download_url.as_str())),
            repo,
            asset_filter,
        ) {
            return Ok((version, download_url));
        }
    }

    Err(format!(
        "Could not find a valid macOS binary asset in any recent release of {}",
        repo
    ))
}

#[allow(dead_code)]
pub fn fetch_github_release(repo: &str) -> Result<(String, String), String> {
    fetch_github_release_with_filter(repo, None)
}

/// Queries Codeberg (Gitea) for the latest release that includes a matching macOS download URL.
pub fn fetch_codeberg_release_with_filter(
    repo: &str,
    asset_filter: Option<&HashMap<String, String>>,
) -> Result<(String, String), String> {
    let url = format!("https://codeberg.org/api/v1/repos/{}/releases?limit=10", repo);
    let client = build_client()?;

    let response = client
        .get(&url)
        .header("User-Agent", "rorys-package-manager-cli")
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Codeberg API returned HTTP status: {}",
            response.status()
        ));
    }

    let releases: Vec<CodebergRelease> = response
        .json()
        .map_err(|e| format!("Failed to parse JSON payload: {}", e))?;

    for release in &releases {
        let version = normalise_version(&release.tag_name);
        if let Ok(download_url) = pick_mac_download_url(
            release
                .assets
                .iter()
                .map(|a| (a.name.as_str(), a.browser_download_url.as_str())),
            repo,
            asset_filter,
        ) {
            return Ok((version, download_url));
        }
    }

    Err(format!(
        "Could not find a valid macOS binary asset in any recent release of {}",
        repo
    ))
}

#[allow(dead_code)]
pub fn fetch_codeberg_release(repo: &str) -> Result<(String, String), String> {
    fetch_codeberg_release_with_filter(repo, None)
}

#[allow(dead_code)]
pub fn fetch_github_version(repo: &str) -> Result<String, String> {
    fetch_github_release(repo).map(|(version, _)| version)
}

#[allow(dead_code)]
pub fn fetch_codeberg_version(repo: &str) -> Result<String, String> {
    fetch_codeberg_release(repo).map(|(version, _)| version)
}

/// Queries the Chromium Dash release API for the specified channel version on macOS.
pub fn fetch_chromium_version(channel: &str) -> Result<String, String> {
    let clean_channel = if channel.is_empty() { "Stable" } else { channel };
    let url = format!(
        "https://chromiumdash.appspot.com/fetch_releases?channel={}&platform=Mac&num=1",
        clean_channel
    );
    let client = build_client()?;

    let response = client
        .get(&url)
        .header("User-Agent", "rorys-package-manager-cli")
        .send()
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Chromium Dash API returned HTTP status: {}",
            response.status()
        ));
    }

    let releases: Vec<ChromiumRelease> = response
        .json()
        .map_err(|e| format!("Failed to parse JSON payload: {}", e))?;

    let latest = releases
        .first()
        .ok_or_else(|| "Chromium Dash API returned no releases".to_string())?;

    Ok(normalise_version(&latest.version))
}

/// Downloads a raw plain-text payload (e.g. snapshot LAST_CHANGE manifests).
pub fn fetch_raw_url_version(url: &str) -> Result<String, String> {
    let client = build_client()?;

    let response = client
        .get(url)
        .header("User-Agent", "rorys-package-manager-cli")
        .send()
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Server returned HTTP status: {}",
            response.status()
        ));
    }

    let raw_text = response
        .text()
        .map_err(|e| format!("Failed to read body text: {}", e))?;

    Ok(normalise_version(&raw_text))
}

/// Fetches a web page and runs a regex with a capture group to extract the version.
pub fn fetch_regex_scraped_version(url: &str, pattern: &str) -> Result<String, String> {
    let client = build_client()?;

    let response = client
        .get(url)
        .header("User-Agent", "rorys-package-manager-cli")
        .send()
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Scrape failed with status: {}", response.status()));
    }

    let text = response.text().map_err(|e| e.to_string())?;
    let re = Regex::new(pattern).map_err(|e| format!("Invalid regex: {}", e))?;

    if let Some(caps) = re.captures(&text) {
        if let Some(matched) = caps.get(1).or_else(|| caps.get(0)) {
            return Ok(normalise_version(matched.as_str()));
        }
    }

    Err(format!("Regex pattern '{}' found no matches in {}", pattern, url))
}

/// Downloads a file, streams bytes to disk, computes its SHA-256 hash in a single pass,
/// and validates against `expected_sha256` if one is provided.
pub fn download_file_with_hash(
    url: &str,
    destination_path: &Path,
    expected_sha256: Option<&str>,
) -> Result<String, String> {
    if let Some(parent) = destination_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create download directory: {}", e))?;
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("Failed to create download client: {}", e))?;

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

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 32768];

    loop {
        let bytes_read = response
            .read(&mut buffer)
            .map_err(|e| format!("Stream read error: {}", e))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
        dest_file
            .write_all(&buffer[..bytes_read])
            .map_err(|e| format!("Stream write error: {}", e))?;
    }

    let calculated_hash = format!("{:x}", hasher.finalize());

    if let Some(expected) = expected_sha256 {
        if !calculated_hash.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(destination_path);
            return Err(format!(
                "SHA-256 integrity check failed!\n  Expected:   {}\n  Calculated: {}\nDeleted corrupt file.",
                expected, calculated_hash
            ));
        }
    }

    Ok(calculated_hash)
}

/// Backwards-compatible download without explicit expected hash.
#[allow(dead_code)]
pub fn download_file(url: &str, destination_path: &Path) -> Result<(), String> {
    download_file_with_hash(url, destination_path, None).map(|_| ())
}
