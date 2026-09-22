use std::path::Path;
use std::process::Command;

/// Checks if an app exists and extracts its version string using the macOS 'defaults' tool.
/// Returns Some(version_string) if successful, or None if the app isn't installed.
pub fn get_local_version(app_name: &str) -> Option<String> {
    // 1. Build the path string and check if the Info.plist actually exists
    let plist_path = format!("/Applications/{}.app/Contents/Info.plist", app_name);

    if !Path::new(&plist_path).exists() {
        return None;
    }

    // 2. Run the native macOS 'defaults' command: defaults read /Applications/... CFBundleShortVersionString
    let output = Command::new("defaults")
        .args(["read", &plist_path, "CFBundleShortVersionString"])
        .output()
        .ok()?; // The '?' operator exits early and returns None if the command fails to execute entirely

    // 3. Process the terminal output safely
    if output.status.success() {
        // Convert raw terminal bytes into a clean UTF-8 string
        let version_raw = String::from_utf8(output.stdout).ok()?;
        Some(version_raw.trim().to_string())
    } else {
        None
    }
}

/// Removes the Apple Gatekeeper quarantine flag from an app bundle.
/// This must be executed right AFTER an installation or upgrade finishes.
pub fn remove_quarantine(app_name: &str) -> Result<(), String> {
    let app_path = format!("/Applications/{}.app", app_name);

    if !Path::new(&app_path).exists() {
        return Err(format!(
            "Cannot remove quarantine: {} does not exist.",
            app_path
        ));
    }

    // Run: xattr -dr com.apple.quarantine /Applications/AppName.app
    // -d deletes the attribute, -r ensures it recursively strips nested binary frameworks inside the app
    let status = Command::new("xattr")
        .args(["-dr", "com.apple.quarantine", &app_path])
        .status()
        .map_err(|e| format!("Failed to execute xattr command: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "xattr command failed to strip quarantine from {}",
            app_path
        ))
    }
}
