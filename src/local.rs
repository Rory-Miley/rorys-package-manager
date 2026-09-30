use crate::recipe::{Recipe, SourceConfig};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Reads the custom rpm version tag from extended attributes on the app bundle.
pub fn get_bundle_rpm_version(bundle_name: &str) -> Option<String> {
    let clean_bundle = if bundle_name.ends_with(".app") {
        bundle_name.to_string()
    } else {
        format!("{}.app", bundle_name)
    };
    let path = format!("/Applications/{}", clean_bundle);
    if !Path::new(&path).exists() {
        return None;
    }

    let output = Command::new("xattr")
        .args(["-p", "user.rpm.version", &path])
        .output()
        .ok()?;

    if output.status.success() {
        let text = String::from_utf8(output.stdout).ok()?;
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    None
}

/// Stores the custom rpm version tag in extended attributes on the app bundle.
pub fn set_bundle_rpm_version(bundle_name: &str, version: &str) {
    let clean_bundle = if bundle_name.ends_with(".app") {
        bundle_name.to_string()
    } else {
        format!("{}.app", bundle_name)
    };
    let path = format!("/Applications/{}", clean_bundle);
    if Path::new(&path).exists() {
        let _ = Command::new("xattr")
            .args(["-w", "user.rpm.version", version, &path])
            .output();
    }
}

/// Checks if an app exists in /Applications and extracts its version string.
pub fn get_local_version(bundle_name: &str) -> Option<String> {
    get_bundle_local_version(bundle_name, false)
}

/// Checks the local version of an app bundle, taking into account whether it is a snapshot.
pub fn get_bundle_local_version(bundle_name: &str, is_snapshot: bool) -> Option<String> {
    let clean_bundle = if bundle_name.ends_with(".app") {
        bundle_name.to_string()
    } else {
        format!("{}.app", bundle_name)
    };

    let plist_path = format!("/Applications/{}/Contents/Info.plist", clean_bundle);
    if !Path::new(&plist_path).exists() {
        return None;
    }

    // 1. Check for xattr user.rpm.version (if rpm stamped the full package version)
    if !is_snapshot {
        if let Some(xattr_ver) = get_bundle_rpm_version(&clean_bundle) {
            return Some(xattr_ver);
        }
    }

    // 2. For Chromium snapshot builds, CFBundleShortVersionString is a static milestone placeholder
    // (e.g. "157.0.8079.0") shared across thousands of commits. The real sequential revision
    // is stored in SCMRevision (e.g. "...@{#1707229}").
    if is_snapshot {
        if let Some(scm) = read_plist_key(&plist_path, "SCMRevision") {
            if let Some(start) = scm.find("@{#") {
                let rest = &scm[start + 3..];
                if let Some(end) = rest.find('}') {
                    let rev = &rest[..end];
                    if !rev.is_empty() && rev.chars().all(|c| c.is_ascii_digit()) {
                        return Some(rev.to_string());
                    }
                }
            }
        }
    }

    // Try CFBundleShortVersionString
    if let Some(v) = read_plist_key(&plist_path, "CFBundleShortVersionString") {
        if !v.is_empty() {
            return Some(v);
        }
    }

    // Fallback to CFBundleVersion
    read_plist_key(&plist_path, "CFBundleVersion")
}

/// Helper extracting local version based on recipe configuration (e.g. snapshots).
pub fn get_recipe_local_version(recipe: &Recipe) -> Option<String> {
    let clean_bundle = if recipe.bundle_name.ends_with(".app") {
        recipe.bundle_name.clone()
    } else {
        format!("{}.app", recipe.bundle_name)
    };

    let app_path = PathBuf::from(format!("/Applications/{}", clean_bundle));
    if !app_path.exists() {
        return None;
    }

    let is_snapshot = matches!(recipe.source, SourceConfig::ChromiumSnapshot { .. });
    if is_snapshot {
        return get_bundle_local_version(&recipe.bundle_name, true);
    }

    // 1. Check bundle extended attribute
    if let Some(xattr_ver) = get_bundle_rpm_version(&clean_bundle) {
        return Some(xattr_ver);
    }

    let base_plist_ver = get_bundle_local_version(&recipe.bundle_name, false);

    // 2. Check if cache has a full package version matching the bundle's base plist version
    if let Ok(cache) = crate::cache::load_cache_or_empty() {
        if let Some(cached) = cache.find(&recipe.name) {
            if cached.managed {
                if let Some(ref base) = base_plist_ver {
                    if (cached.remote_version.starts_with(base) && cached.remote_version.contains(['-', '_']))
                        || (cached.local_version.starts_with(base) && cached.local_version.contains(['-', '_']))
                    {
                        let full = if cached.local_version.contains(['-', '_']) {
                            cached.local_version.clone()
                        } else {
                            cached.remote_version.clone()
                        };
                        set_bundle_rpm_version(&clean_bundle, &full);
                        return Some(full);
                    }
                }
            }
        }
    }

    base_plist_ver
}

fn read_plist_key(plist_path: &str, key: &str) -> Option<String> {
    let output = Command::new("defaults")
        .args(["read", plist_path, key])
        .output()
        .ok()?;

    if output.status.success() {
        let text = String::from_utf8(output.stdout).ok()?;
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    None
}

/// Checks whether an app is currently running using macOS `pgrep`.
pub fn is_app_running(bundle_name: &str) -> bool {
    let proc_name = bundle_name.strip_suffix(".app").unwrap_or(bundle_name);
    let output = Command::new("pgrep")
        .args(["-ix", proc_name])
        .output();

    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

/// Attempts to gracefully request an application to quit via AppleScript.
pub fn request_quit(bundle_name: &str) -> Result<(), String> {
    let app_name = bundle_name.strip_suffix(".app").unwrap_or(bundle_name);
    let script = format!("tell application \"{}\" to quit", app_name);
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("Failed to execute osascript: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("Could not quit {}: {}", app_name, err.trim()))
    }
}

/// Removes the Apple Gatekeeper quarantine flag from an app bundle path.
/// Can be applied to a staged bundle before installation or in `/Applications`.
pub fn remove_quarantine(bundle_path: &Path) -> Result<(), String> {
    if !bundle_path.exists() {
        return Err(format!(
            "Cannot remove quarantine: {} does not exist.",
            bundle_path.display()
        ));
    }

    // xattr -dr com.apple.quarantine <bundle_path>
    let status = Command::new("xattr")
        .args(["-dr", "com.apple.quarantine", bundle_path.to_str().unwrap_or_default()])
        .status()
        .map_err(|e| format!("Failed to execute xattr command: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "xattr command failed to strip quarantine from {}",
            bundle_path.display()
        ))
    }
}

/// Backward compatibility helper taking an app name string.
#[allow(dead_code)]
pub fn remove_quarantine_by_name(app_name: &str) -> Result<(), String> {
    let clean_bundle = if app_name.ends_with(".app") {
        app_name.to_string()
    } else {
        format!("{}.app", app_name)
    };
    let path = Path::new("/Applications").join(clean_bundle);
    remove_quarantine(&path)
}

/// Validates that an extracted directory is a structurally valid macOS .app bundle.
pub fn validate_app_bundle(bundle_path: &Path) -> Result<(), String> {
    if !bundle_path.is_dir() {
        return Err(format!("{} is not a directory", bundle_path.display()));
    }

    let plist = bundle_path.join("Contents/Info.plist");
    if !plist.exists() {
        return Err(format!("Missing Contents/Info.plist in {}", bundle_path.display()));
    }

    let macos_dir = bundle_path.join("Contents/MacOS");
    if !macos_dir.exists() {
        return Err(format!("Missing Contents/MacOS in {}", bundle_path.display()));
    }

    Ok(())
}
