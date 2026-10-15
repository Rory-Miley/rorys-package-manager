use std::fs;
use std::path::{Path, PathBuf};

/// Environment variable overrides for development and testing.
pub const ENV_DEV: &str = "RPM_DEV";
pub const ENV_DIR: &str = "RPM_DIR";

/// Installed system-wide binary location.
pub const INSTALLED_BINARY_PATH: &str = "/opt/rpm/bin/rpm";

/// Returns the command invocation string to display in user messages.
pub fn runner_cmd() -> &'static str {
    if is_dev_mode() {
        "cargo run --"
    } else {
        "rpm"
    }
}

/// Returns true if rpm is running in local repository dev mode.
///
/// Priority:
/// 1. `RPM_DEV` environment variable (1/true/yes vs 0/false/no).
/// 2. `RPM_DIR` environment variable (if set, dev mode is active pointing to that directory).
/// 3. Auto-detection: `Cargo.toml` file exists AND `./recipes` directory exists in the current working directory.
pub fn is_dev_mode() -> bool {
    if let Ok(val) = std::env::var(ENV_DEV) {
        let v = val.trim().to_ascii_lowercase();
        if v == "1" || v == "true" || v == "yes" {
            return true;
        }
        if v == "0" || v == "false" || v == "no" {
            return false;
        }
    }

    if std::env::var(ENV_DIR).is_ok() {
        return true;
    }

    Path::new("Cargo.toml").is_file() && Path::new("recipes").is_dir()
}

fn dev_root() -> PathBuf {
    if let Ok(dir) = std::env::var(ENV_DIR) {
        PathBuf::from(dir)
    } else {
        PathBuf::from(".")
    }
}

pub fn user_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
    } else {
        PathBuf::from("/tmp")
    }
}

/// Directory where declarative TOML recipes are stored.
/// - Dev mode: `<repo>/recipes`
/// - Installed mode: `~/.rpm/recipes`
pub fn recipes_dir() -> PathBuf {
    if is_dev_mode() {
        dev_root().join("recipes")
    } else {
        user_home_dir().join(".rpm").join("recipes")
    }
}

/// Path to the update cache file.
/// - Dev mode: `<repo>/local/cache.toml`
/// - Installed mode: `~/.rpm/cache.toml`
pub fn cache_file_path() -> PathBuf {
    if is_dev_mode() {
        dev_root().join("local").join("cache.toml")
    } else {
        user_home_dir().join(".rpm").join("cache.toml")
    }
}

/// Path to the configuration file.
/// - Dev mode: `<repo>/local/config.toml`
/// - Installed mode: `~/.rpm/config.toml`
pub fn config_file_path() -> PathBuf {
    if is_dev_mode() {
        dev_root().join("local").join("config.toml")
    } else {
        user_home_dir().join(".rpm").join("config.toml")
    }
}

/// Directory where download archives and payloads are staged.
/// - Dev mode: `<repo>/local/staging`
/// - Installed mode: `~/Library/Caches/rpm/staging` (macOS purgeable cache)
pub fn staging_dir() -> PathBuf {
    if is_dev_mode() {
        dev_root().join("local").join("staging")
    } else {
        user_home_dir()
            .join("Library")
            .join("Caches")
            .join("rpm")
            .join("staging")
    }
}

/// Ensures that the recipe directory, cache parent directory, and staging directory exist.
pub fn ensure_dirs() -> Result<(), String> {
    let r_dir = recipes_dir();
    fs::create_dir_all(&r_dir)
        .map_err(|e| format!("Failed to create recipes directory {}: {}", r_dir.display(), e))?;

    let c_path = cache_file_path();
    if let Some(parent) = c_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create cache directory {}: {}", parent.display(), e))?;
    }

    let cfg_path = config_file_path();
    if let Some(parent) = cfg_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory {}: {}", parent.display(), e))?;
    }

    let s_dir = staging_dir();
    fs::create_dir_all(&s_dir)
        .map_err(|e| format!("Failed to create staging directory {}: {}", s_dir.display(), e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex to serialize tests modifying environment variables
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_dev_mode_env_override() {
        let _guard = ENV_MUTEX.lock().unwrap();

        unsafe {
            std::env::set_var(ENV_DEV, "1");
        }
        assert!(is_dev_mode());

        unsafe {
            std::env::set_var(ENV_DEV, "true");
        }
        assert!(is_dev_mode());

        unsafe {
            std::env::set_var(ENV_DEV, "0");
        }
        assert!(!is_dev_mode());

        unsafe {
            std::env::set_var(ENV_DEV, "false");
        }
        assert!(!is_dev_mode());

        unsafe {
            std::env::remove_var(ENV_DEV);
        }
    }

    #[test]
    fn test_installed_paths_resolution() {
        let _guard = ENV_MUTEX.lock().unwrap();

        unsafe {
            std::env::set_var(ENV_DEV, "0");
            std::env::set_var("HOME", "/Users/tester");
        }

        assert_eq!(
            recipes_dir(),
            PathBuf::from("/Users/tester/.rpm/recipes")
        );
        assert_eq!(
            cache_file_path(),
            PathBuf::from("/Users/tester/.rpm/cache.toml")
        );
        assert_eq!(
            staging_dir(),
            PathBuf::from("/Users/tester/Library/Caches/rpm/staging")
        );

        unsafe {
            std::env::remove_var(ENV_DEV);
        }
    }

    #[test]
    fn test_dev_paths_with_custom_dir() {
        let _guard = ENV_MUTEX.lock().unwrap();

        unsafe {
            std::env::set_var(ENV_DIR, "/tmp/custom_rpm_dev");
        }

        assert_eq!(
            recipes_dir(),
            PathBuf::from("/tmp/custom_rpm_dev/recipes")
        );
        assert_eq!(
            cache_file_path(),
            PathBuf::from("/tmp/custom_rpm_dev/local/cache.toml")
        );
        assert_eq!(
            staging_dir(),
            PathBuf::from("/tmp/custom_rpm_dev/local/staging")
        );

        unsafe {
            std::env::remove_var(ENV_DIR);
        }
    }
}
