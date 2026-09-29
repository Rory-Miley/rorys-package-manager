use std::path::Path;
use std::process::Command;

/// Checks if an app exists in /Applications and extracts its version string.
/// Checks `CFBundleShortVersionString` first, falling back to `CFBundleVersion`.
pub fn get_local_version(bundle_name: &str) -> Option<String> {
    let clean_bundle = if bundle_name.ends_with(".app") {
        bundle_name.to_string()
    } else {
        format!("{}.app", bundle_name)
    };

    let plist_path = format!("/Applications/{}/Contents/Info.plist", clean_bundle);
    if !Path::new(&plist_path).exists() {
        return None;
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
