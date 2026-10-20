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
        SourceConfig::HomebrewCask { cask } => {
            let info = fetch_homebrew_cask_info(cask)?;
            Ok((info.version, info.download_url))
        }
    }
}

/// Resolves a recipe into its latest version, download URL, and expected SHA-256 hash.
pub fn resolve_recipe_info(recipe: &Recipe) -> Result<(String, String, Option<String>), String> {
    match &recipe.source {
        SourceConfig::HomebrewCask { cask } => {
            let info = fetch_homebrew_cask_info(cask)?;
            Ok((info.version, info.download_url, info.sha256))
        }
        _ => {
            let (ver, url) = resolve_recipe(recipe)?;
            Ok((ver, url, recipe.sha256.clone()))
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
        LivecheckConfig::HomebrewCask { cask } => {
            let info = fetch_homebrew_cask_info(cask)?;
            Ok(info.version)
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

    // 2. Filter candidate assets and score them based on macOS suitability, architecture, and container type
    let mut candidates: Vec<(i32, &str)> = Vec::new();

    for (name, url) in &assets {
        let name_lower = name.to_lowercase();

        // Exclude non-package extensions (checksums, signatures, metadata, etc.)
        let is_non_package = name_lower.ends_with(".sha256sum")
            || name_lower.ends_with(".sha256")
            || name_lower.ends_with(".sha512")
            || name_lower.ends_with(".sig")
            || name_lower.ends_with(".asc")
            || name_lower.ends_with(".txt")
            || name_lower.ends_with(".json")
            || name_lower.ends_with(".blockmap")
            || name_lower.ends_with(".zsync")
            || name_lower.ends_with(".sbom");
        if is_non_package {
            continue;
        }

        // Exclude non-macOS target OSes (Linux, Windows, Android, BSDs)
        let is_windows = name_lower.contains("windows")
            || name_lower.contains("win32")
            || name_lower.contains("win64")
            || name_lower.ends_with(".exe")
            || name_lower.ends_with(".msi")
            || name_lower.ends_with(".msix")
            || name_lower.contains("-win-")
            || name_lower.contains("-win_")
            || name_lower.contains("_win_")
            || name_lower.contains("-win.")
            || name_lower.contains("_win.");

        let is_other_os = name_lower.contains("linux")
            || is_windows
            || name_lower.contains("android")
            || name_lower.contains("freebsd")
            || name_lower.contains("openbsd")
            || name_lower.contains("netbsd")
            || name_lower.ends_with(".deb")
            || name_lower.ends_with(".rpm")
            || name_lower.ends_with(".appimage")
            || name_lower.ends_with(".apk");
        if is_other_os {
            continue;
        }

        // Identify archive / container types
        let is_dmg = name_lower.ends_with(".dmg");
        let is_pkg = name_lower.ends_with(".pkg");
        let is_zip = name_lower.ends_with(".zip");
        let is_tar = name_lower.ends_with(".tar.gz")
            || name_lower.ends_with(".tar.xz")
            || name_lower.ends_with(".tgz")
            || name_lower.ends_with(".tar.bz2")
            || name_lower.ends_with(".tbz2");

        if !(is_dmg || is_pkg || is_zip || is_tar) {
            continue;
        }

        // Check for explicit macOS indicators
        let has_mac_indicator = is_dmg
            || name_lower.contains("macos")
            || name_lower.contains("darwin")
            || name_lower.contains("osx")
            || name_lower.contains("apple")
            || (name_lower.contains("mac") && !name_lower.contains("machine"));

        // Tarballs MUST have an explicit macOS keyword; generic tarballs are usually source or Linux builds
        if is_tar && !has_mac_indicator {
            continue;
        }

        // Architecture detection & compatibility checks
        let is_arm64_indicator = name_lower.contains("arm64")
            || name_lower.contains("aarch64")
            || name_lower.contains("apple-silicon")
            || name_lower.contains("m1")
            || name_lower.contains("m2")
            || name_lower.contains("m3");

        let is_x86_indicator = name_lower.contains("x86_64")
            || name_lower.contains("intel")
            || name_lower.contains("amd64")
            || name_lower.contains("x64");

        let is_universal = name_lower.contains("universal");

        let (matches_arch, is_incompatible) = if current_arch == "arm64" {
            let matches = is_arm64_indicator || is_universal;
            let incompatible = is_x86_indicator && !is_universal && !is_arm64_indicator;
            (matches, incompatible)
        } else {
            let matches = is_x86_indicator || is_universal;
            let incompatible = is_arm64_indicator && !is_universal && !is_x86_indicator;
            (matches, incompatible)
        };

        if is_incompatible {
            continue;
        }

        // Calculate match quality score
        let mut score = 0;

        // Architecture scoring: specific > universal > unspecified
        if matches_arch {
            if is_universal {
                score += 200;
            } else {
                score += 300;
            }
        } else {
            score += 100;
        }

        // Container scoring: prioritize .dmg over .zip, and .zip over .tar
        if is_dmg {
            score += 60;
        } else if is_zip && has_mac_indicator {
            score += 40;
        } else if is_pkg {
            score += 30;
        } else if is_zip {
            score += 20;
        } else if is_tar && has_mac_indicator {
            score += 10;
        }

        candidates.push((score, url));
    }

    // Sort by score descending
    candidates.sort_by(|a, b| b.0.cmp(&a.0));

    if let Some((_, url)) = candidates.first() {
        Ok(url.to_string())
    } else {
        Err(format!(
            "Could not find a valid macOS binary asset in repo: {}",
            repo
        ))
    }
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
) -> Result<(String, crate::progress::DownloadSummary), String> {
    download_file_with_hash_in_tree(
        url,
        destination_path,
        expected_sha256,
        crate::progress::DOWNLOAD_INDENT,
        None,
    )
}

/// Downloads a file within a tree layout with a specified indent and optional `TreeTranscript`.
pub fn download_file_with_hash_in_tree(
    url: &str,
    destination_path: &Path,
    expected_sha256: Option<&str>,
    indent: &'static str,
    transcript: Option<&mut crate::progress::TreeTranscript>,
) -> Result<(String, crate::progress::DownloadSummary), String> {
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

    let total_bytes = response.content_length();
    let mut progress = crate::progress::DownloadProgress::with_tree(total_bytes, indent, transcript);

    let part_path = destination_path.with_extension(format!(
        "{}.part",
        destination_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin")
    ));

    let mut dest_file = File::create(&part_path)
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
        progress.update(bytes_read);
    }

    let summary = progress.finish();

    let calculated_hash = format!("{:x}", hasher.finalize());

    if let Some(expected) = expected_sha256 {
        if !calculated_hash.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&part_path);
            return Err(format!(
                "SHA-256 integrity check failed!\n  Expected:   {}\n  Calculated: {}\nDeleted corrupt file.",
                expected, calculated_hash
            ));
        }
    }

    std::fs::rename(&part_path, destination_path)
        .map_err(|e| format!("Failed to finalize downloaded file: {}", e))?;

    Ok((calculated_hash, summary))
}

/// Backwards-compatible download without explicit expected hash.
#[allow(dead_code)]
pub fn download_file(url: &str, destination_path: &Path) -> Result<(), String> {
    download_file_with_hash(url, destination_path, None).map(|_| ())
}

#[derive(Debug, Clone)]
pub struct HomebrewCaskInfo {
    pub token: String,
    pub name: String,
    pub desc: Option<String>,
    pub homepage: Option<String>,
    pub version: String,
    pub bundle_name: String,
    pub download_url: String,
    pub sha256: Option<String>,
    pub conflicts: Vec<String>,
    pub disabled: bool,
    pub disable_reason: Option<String>,
}

pub fn parse_cask_json_value(val: &serde_json::Value) -> Result<HomebrewCaskInfo, String> {
    let token = val
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let name = val
        .get("name")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .unwrap_or(&token)
        .to_string();

    let desc = val
        .get("desc")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let homepage = val
        .get("homepage")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let version = val
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let disabled = val
        .get("disabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let disable_reason = val
        .get("disable_reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Extract bundle_name from artifacts
    let mut bundle_name = String::new();
    if let Some(artifacts) = val.get("artifacts").and_then(|v| v.as_array()) {
        for art in artifacts {
            if let Some(app_val) = art.get("app").and_then(|v| v.as_array()) {
                if let Some(first) = app_val.first().and_then(|v| v.as_str()) {
                    let target = app_val
                        .get(1)
                        .and_then(|v| v.get("target"))
                        .and_then(|v| v.as_str());
                    bundle_name = target.unwrap_or(first).to_string();
                    break;
                }
            }
        }
    }
    if bundle_name.is_empty() {
        bundle_name = if name.ends_with(".app") {
            name.clone()
        } else {
            format!("{}.app", name)
        };
    }

    let arch = detect_mac_architecture();
    let default_url = val.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let default_sha = val
        .get("sha256")
        .and_then(|v| v.as_str())
        .filter(|s| *s != "no_check")
        .map(|s| s.to_string());

    let (download_url, sha256) = if arch == "arm64" {
        (default_url, default_sha)
    } else {
        let mut x86_url = default_url;
        let mut x86_sha = default_sha;

        if let Some(variations) = val.get("variations").and_then(|v| v.as_object()) {
            for (key, var) in variations {
                if !key.starts_with("arm64") {
                    if let Some(u) = var.get("url").and_then(|v| v.as_str()) {
                        x86_url = u.to_string();
                    }
                    if let Some(s) = var
                        .get("sha256")
                        .and_then(|v| v.as_str())
                        .filter(|s| *s != "no_check")
                    {
                        x86_sha = Some(s.to_string());
                    }
                    break;
                }
            }
        }
        (x86_url, x86_sha)
    };

    let mut conflicts = Vec::new();
    if let Some(conf_casks) = val
        .get("conflicts_with")
        .and_then(|v| v.get("cask"))
        .and_then(|v| v.as_array())
    {
        for c in conf_casks {
            if let Some(s) = c.as_str() {
                conflicts.push(s.to_string());
            }
        }
    }

    Ok(HomebrewCaskInfo {
        token,
        name,
        desc,
        homepage,
        version,
        bundle_name,
        download_url,
        sha256,
        conflicts,
        disabled,
        disable_reason,
    })
}

pub fn fetch_homebrew_cask_info(cask: &str) -> Result<HomebrewCaskInfo, String> {
    let clean_cask = cask.trim().to_lowercase();
    let url = format!("https://formulae.brew.sh/api/cask/{}.json", clean_cask);
    let client = build_client()?;

    let response = client
        .get(&url)
        .header("User-Agent", "rorys-package-manager-cli")
        .send()
        .map_err(|e| format!("Failed to request Homebrew Cask API: {}", e))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("Homebrew cask '{}' not found (404)", clean_cask));
    }
    if !response.status().is_success() {
        return Err(format!(
            "Homebrew Cask API returned HTTP status {}: {}",
            response.status(),
            clean_cask
        ));
    }

    let val: serde_json::Value = response
        .json()
        .map_err(|e| format!("Failed to parse Homebrew Cask JSON for '{}': {}", clean_cask, e))?;

    parse_cask_json_value(&val)
}

pub fn fetch_raw_github_recipe(query: &str) -> Result<Option<String>, String> {
    let clean = query.trim().to_lowercase();
    let url = format!(
        "https://raw.githubusercontent.com/rorymiley/rorys-package-manager/master/recipes/rpm/{}.toml",
        clean
    );
    let client = build_client()?;

    let response = client
        .get(&url)
        .header("User-Agent", "rorys-package-manager-cli")
        .send()
        .map_err(|e| format!("Network request failed: {}", e))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("GitHub raw returned HTTP status: {}", response.status()));
    }

    let body = response.text().map_err(|e| e.to_string())?;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pick_mac_download_url_librewolf_codeberg() {
        let assets = vec![
            ("librewolf-156.0.1-1-linux-arm64-appimage.AppImage", "https://example.com/appimage"),
            ("librewolf-156.0.1-1-linux-arm64-deb.deb", "https://example.com/deb"),
            ("librewolf-156.0.1-1-linux-arm64-package.tar.xz", "https://example.com/linux-arm64-tar"),
            ("librewolf-156.0.1-1-linux-x86_64-package.tar.xz", "https://example.com/linux-x86-tar"),
            ("librewolf-156.0.1-1-macos-arm64-package.dmg", "https://example.com/macos-arm64-dmg"),
            ("librewolf-156.0.1-1-macos-x86_64-package.dmg", "https://example.com/macos-x86-dmg"),
            ("librewolf-156.0.1-1-windows-arm64-package.zip", "https://example.com/win-zip"),
            ("sha256sums.txt", "https://example.com/sha256sums"),
        ];

        let result = pick_mac_download_url(assets, "librewolf/bsys6", None);
        assert!(result.is_ok());
        let selected_url = result.unwrap();

        if detect_mac_architecture() == "arm64" {
            assert_eq!(selected_url, "https://example.com/macos-arm64-dmg");
        } else {
            assert_eq!(selected_url, "https://example.com/macos-x86-dmg");
        }
    }

    #[test]
    fn test_pick_mac_download_url_rejects_generic_tarballs() {
        let assets = vec![
            ("qbittorrent-5.2.4.tar.xz", "https://example.com/source.tar.xz"),
            ("qbittorrent-5.2.4.tar.xz.asc", "https://example.com/source.tar.xz.asc"),
            ("qbittorrent-5.2.4_x86_64.AppImage", "https://example.com/appimage"),
        ];

        let result = pick_mac_download_url(assets, "qbittorrent/qBittorrent", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_pick_mac_download_url_prefers_dmg_over_zip_and_tar() {
        let current_arch = detect_mac_architecture();
        let dmg_name = format!("app-macos-{}.dmg", current_arch);
        let zip_name = format!("app-macos-{}.zip", current_arch);
        let tar_name = format!("app-macos-{}.tar.gz", current_arch);

        let assets = vec![
            (tar_name.as_str(), "https://example.com/app.tar.gz"),
            (zip_name.as_str(), "https://example.com/app.zip"),
            (dmg_name.as_str(), "https://example.com/app.dmg"),
        ];

        let result = pick_mac_download_url(assets, "owner/repo", None);
        assert_eq!(result.unwrap(), "https://example.com/app.dmg");
    }

    #[test]
    fn test_parse_cask_json_value() {
        let json_str = r#"{
            "token": "vlc",
            "name": ["VLC media player"],
            "desc": "Multimedia player",
            "homepage": "https://www.videolan.org/vlc/",
            "url": "https://get.videolan.org/vlc/3.0.24/macosx/vlc-3.0.24-arm64.dmg",
            "version": "3.0.24",
            "sha256": "64a89d93cdd30b0e97131743e246373db82d6826ea882d265d46d74b136da2b7",
            "artifacts": [
                {"app": ["VLC.app"]}
            ],
            "conflicts_with": {"cask": ["vlc@nightly"]}
        }"#;

        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let info = parse_cask_json_value(&val).unwrap();

        assert_eq!(info.token, "vlc");
        assert_eq!(info.name, "VLC media player");
        assert_eq!(info.bundle_name, "VLC.app");
        assert_eq!(info.version, "3.0.24");
        assert_eq!(info.conflicts, vec!["vlc@nightly"]);
    }
}

