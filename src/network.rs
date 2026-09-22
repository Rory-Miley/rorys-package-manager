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

// Chromium Dash returns an array of release objects; we only need "version".
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

/// Queries public GitHub Release endpoints to discover the newest tagged deployment.
pub fn fetch_github_version(repo: &str) -> Result<String, String> {
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

    let download_url = release
        .assets
        .iter()
        .find(|asset| {
            let asset_name_lower = asset.name.to_lowercase();
            let is_mac_file = asset_name_lower.ends_with(".dmg")
                || asset_name_lower.contains("mac")
                || asset_name_lower.contains("macos");

            let matches_arch = if current_arch == "arm64" {
                // Keywords common for Apple Silicon binaries
                asset_name_lower.contains("arm64")
                    || asset_name_lower.contains("aarch64")
                    || asset_name_lower.contains("apple-silicon")
            } else {
                // Keywords common for Intel binaries
                asset_name_lower.contains("x86_64")
                    || asset_name_lower.contains("intel")
                    || asset_name_lower.contains("x64")
            };

            // Exceptional fallback: If an app distributes a "universal" binary, it works on both architectures
            let is_universal = asset_name_lower.contains("universal");

            is_mac_file && (matches_arch || is_universal)
        })
        .map(|asset| asset.browser_download_url.clone())
        // Fallback: If no arch-specific binary was found, grab any valid mac asset as a last resort
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|asset| asset.name.ends_with(".dmg"))
                .map(|asset| asset.browser_download_url.clone())
        })
        .ok_or_else(|| {
            format!(
                "Could not find a valid macOS binary asset in repo: {}",
                repo
            )
        })?;

    Ok((version, download_url))
}

/// Queries a Codeberg (Gitea-based) repository's latest tagged release.
pub fn fetch_codeberg_version(repo: &str) -> Result<String, String> {
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

    // Codeberg's Gitea API returns the same "tag_name" shape as GitHub's.
    let release: CodebergRelease = response
        .json()
        .map_err(|e| format!("Failed to parse JSON payload: {}", e))?;

    let version = normalise_version(&release.tag_name);

    let download_url = release
        .assets
        .iter()
        .find(|asset| {
            let asset_name_lower = asset.name.to_lowercase();
            let is_mac_file = asset_name_lower.ends_with(".dmg")
                || asset_name_lower.contains("mac")
                || asset_name_lower.contains("macos");

            let matches_arch = if current_arch == "arm64" {
                // Keywords common for Apple Silicon binaries
                asset_name_lower.contains("arm64")
                    || asset_name_lower.contains("aarch64")
                    || asset_name_lower.contains("apple-silicon")
            } else {
                // Keywords common for Intel binaries
                asset_name_lower.contains("x86_64")
                    || asset_name_lower.contains("intel")
                    || asset_name_lower.contains("x64")
            };

            // Exceptional fallback: If an app distributes a "universal" binary, it works on both architectures
            let is_universal = asset_name_lower.contains("universal");

            is_mac_file && (matches_arch || is_universal)
        })
        .map(|asset| asset.browser_download_url.clone())
        // Fallback: If no arch-specific binary was found, grab any valid mac asset as a last resort
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|asset| asset.name.ends_with(".dmg"))
                .map(|asset| asset.browser_download_url.clone())
        })
        .ok_or_else(|| {
            format!(
                "Could not find a valid macOS binary asset in repo: {}",
                repo
            )
        })?;

    Ok((version, download_url))
}

/// Queries the Chromium Dash release API for the current Stable version on macOS.
/// This mirrors what Homebrew/other package managers use in place of the
/// old (now defunct) omahaproxy.appspot.com endpoint.
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

/// Downloads a raw plain-text payload directly from a file link.
/// Used for unstructured trackers like snapshot 'LAST_CHANGE' manifests.
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
