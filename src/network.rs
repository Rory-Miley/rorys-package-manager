use serde::Deserialize;
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

/// Chromium Dash returns an array of release objects; we only need "version".
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

fn normalise_version(version: &str) -> String {
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
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))
}

/// Picks the best macOS download URL for the current CPU architecture.
fn pick_mac_download_url<'a, I>(asset_names_and_urls: I, repo: &str) -> Result<String, String>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let current_arch = detect_mac_architecture();
    let assets: Vec<(&str, &str)> = asset_names_and_urls.into_iter().collect();

    let arch_match = assets.iter().find(|(name, _)| {
        let name_lower = name.to_lowercase();
        let is_mac_file = name_lower.ends_with(".dmg")
            || name_lower.contains("mac")
            || name_lower.contains("macos");

        let matches_arch = if current_arch == "arm64" {
            name_lower.contains("arm64")
                || name_lower.contains("aarch64")
                || name_lower.contains("apple-silicon")
        } else {
            name_lower.contains("x86_64")
                || name_lower.contains("intel")
                || name_lower.contains("x64")
        };

        let is_universal = name_lower.contains("universal");
        is_mac_file && (matches_arch || is_universal)
    });

    if let Some((_, url)) = arch_match {
        return Ok(url.to_string());
    }

    // Fallback: any .dmg if no arch-specific asset was found
    assets
        .iter()
        .find(|(name, _)| name.to_lowercase().ends_with(".dmg"))
        .map(|(_, url)| url.to_string())
        .ok_or_else(|| {
            format!(
                "Could not find a valid macOS binary asset in repo: {}",
                repo
            )
        })
}

/// Queries GitHub Releases for the latest tag and a matching macOS download URL.
pub fn fetch_github_release(repo: &str) -> Result<(String, String), String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
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

    let release: GitHubRelease = response
        .json()
        .map_err(|e| format!("Failed to parse JSON payload: {}", e))?;

    let version = normalise_version(&release.tag_name);
    let download_url = pick_mac_download_url(
        release
            .assets
            .iter()
            .map(|a| (a.name.as_str(), a.browser_download_url.as_str())),
        repo,
    )?;

    Ok((version, download_url))
}

/// Queries Codeberg (Gitea) for the latest tag and a matching macOS download URL.
pub fn fetch_codeberg_release(repo: &str) -> Result<(String, String), String> {
    let url = format!("https://codeberg.org/api/v1/repos/{}/releases/latest", repo);
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

    let release: CodebergRelease = response
        .json()
        .map_err(|e| format!("Failed to parse JSON payload: {}", e))?;

    let version = normalise_version(&release.tag_name);
    let download_url = pick_mac_download_url(
        release
            .assets
            .iter()
            .map(|a| (a.name.as_str(), a.browser_download_url.as_str())),
        repo,
    )?;

    Ok((version, download_url))
}

/// Version-only helpers (no download URL — used when the source has no asset list).
pub fn fetch_github_version(repo: &str) -> Result<String, String> {
    fetch_github_release(repo).map(|(version, _)| version)
}

pub fn fetch_codeberg_version(repo: &str) -> Result<String, String> {
    fetch_codeberg_release(repo).map(|(version, _)| version)
}

/// Queries the Chromium Dash release API for the current Stable version on macOS.
pub fn fetch_chromium_version(_locator: &str) -> Result<String, String> {
    let url = "https://chromiumdash.appspot.com/fetch_releases?channel=Stable&platform=Mac&num=1";
    let client = build_client()?;

    let response = client
        .get(url)
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
