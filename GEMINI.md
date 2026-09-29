# AI.md — session context for Rory's Package Manager

This is the AI.md file for tracking context across sessions and different AI models. It can be updated by any AI model in use, but **only after asking the user first**.

When developing this project, **goals and tasks should be focused on this file** to keep work in scope. If work starts drifting off scope, remind the user and either:

- steer back to what's listed here, or
- offer to **add new scope to this file** (with user approval) before implementing it.

---

## Project goal

A macOS CLI package manager (`rpm`) for **unsigned** and custom macOS apps: check updates, download/install, strip Apple quarantine after upgrades, and safely replace bundles without breaking running apps.

**Motivations:** learn Rust · pip-like CLI · don't break installs · be fast · easy to install · Homebrew-inspired recipe decoupling.

---

## Design principles

| Principle | Meaning |
|---|---|
| Learn Rust | Clear, idiomatic, functional Rust |
| Good CLI | pip-like: clear commands, colors, friendly errors |
| Don't break | Atomic directory swap, backup/rollback, running app checks, unmanaged app protection |
| Be fast | Reasonable timeouts, stream downloads with single-pass hashing |
| Easy to install | Low friction to get `rpm` onto a machine |
| Recipe-driven | Declarative JSON recipes decoupled from host APIs |
| Stay in scope | Drive work from this file; expand only with user OK |

---

## Decided product rules

### CLI commands

| Command | Behavior |
|---|---|
| `rpm update` | Scan recipes in `recipes/`, check versions; skip updates for unmanaged apps; write `local/cache.json` |
| `rpm upgrade` | Read cache, download, verify SHA-256, and install pending **managed** apps |
| `rpm install <app>` | Resolve recipe; refuse if target exists but is unmanaged; install atomically and mark `managed` |
| `rpm uninstall <app>` | Safely remove a **managed** app from `/Applications` and clear `managed` in cache |
| `rpm list` | List all tracked recipes, aliases, and status (`Installed [managed]`, `Installed [unmanaged]`, `Not Installed`) |
| `rpm help` | Show usage, available commands, and tracked packages |

App names and aliases are matched case-insensitively (`qbt` → `qBittorrent`, `chrome` → `Chromium`).

### Unmanaged Package Security Policy (Homebrew-Inspired)

To preserve user data and system integrity, `rpm` treats any application in `/Applications` not installed by `rpm` as **unmanaged**:
1. **`rpm install <app>`**: If `/Applications/<bundle_name>` exists and is not managed by `rpm`, the installation aborts immediately with an explanatory error instructing the user to remove/uninstall the existing application first if they wish to manage it via `rpm`.
2. **`rpm update`**: Remote checks run, but unmanaged installed applications are never flagged with `update_available: true`. Guidance is printed to the console explaining how to switch to `rpm`.
3. **`rpm upgrade`**: Only upgrades apps where `managed: true`.
4. **Installer pipelines**: `install_recipe`, `install_bundle_pipeline`, and `install_app_bundle_atomic` perform defense-in-depth checks preventing overwriting unmanaged bundles.

---

## How to Add New Packages (JSON Recipe Guide)

Package recipes live in the `recipes/` directory as individual JSON files (`recipes/<app_name>.json`). Adding a new package requires **zero Rust recompilation**.

### Recipe JSON Schema

```json
{
  "name": "CanonicalDisplayName",
  "aliases": ["alias1", "alias2"],
  "bundle_name": "ApplicationName.app",
  "homepage": "https://example.com",
  "description": "Short description of the software",
  "quarantine_strip": true,
  "sha256": "optional_expected_sha256_hash",
  "source": {
    "type": "github_release | codeberg_release | chromium_snapshot | direct_url",
    "locator": "owner/repository",
    "asset_filter": {
      "arm64": "regex_pattern_for_apple_silicon",
      "x86_64": "regex_pattern_for_intel",
      "fallback": "regex_pattern_fallback"
    }
  },
  "livecheck": {
    "strategy": "github_latest_release | codeberg_latest_release | chromium_dash | raw_url | regex_scrape",
    "channel": "Stable",
    "url": "https://example.com/downloads",
    "regex": "(?P<version>[0-9]+(\\.[0-9]+)+)"
  }
}
```

### Fields Reference

1. **`name`** *(String, required)*: The canonical display name of the package.
2. **`aliases`** *(Array of Strings, optional)*: Shorthand names accepted by `install`, `uninstall`, and `list` (e.g. `["qbt"]`, `["chrome"]`).
3. **`bundle_name`** *(String, required)*: The exact `.app` folder name installed into `/Applications` (e.g. `qBittorrent.app`, `Chromium.app`).
4. **`homepage`** *(String, optional)*: Website URL.
5. **`description`** *(String, optional)*: Description printed in `rpm list`.
6. **`quarantine_strip`** *(Boolean, optional, default: `true`)*:
   - `true`: Runs `xattr -dr com.apple.quarantine` on the staged bundle (essential for unsigned third-party software).
   - `false`: Leaves quarantine untouched (useful for signed/notarized packages).
7. **`sha256`** *(String, optional)*: Expected SHA-256 hash. If provided, the installer strictly verifies the download against this hash before mounting or staging.
8. **`source`** *(Object, required)*:
   - **`github_release`**: GitHub repository (`locator: "owner/repo"`). Automatically scans recent releases for valid macOS containers (`.dmg`, `.zip`, `.tar.gz`, `.tar.xz`). Supports optional regex `asset_filter`.
   - **`codeberg_release`**: Codeberg/Gitea repository (`locator: "owner/repo"`).
   - **`chromium_snapshot`**: Downloads snapshot zip from Google storage buckets (`channel: "auto"` selects `Mac_Arm` or `Mac`).
   - **`direct_url`**: Formats a download URL using `{version}` and `{arch}` tokens (e.g. `url_template: "https://example.com/v{version}/App-{arch}.dmg"`).
9. **`livecheck`** *(Object, required)*:
   - **`github_latest_release`**: Queries GitHub API.
   - **`codeberg_latest_release`**: Queries Codeberg API.
   - **`chromium_dash`**: Queries Chromium Dash release endpoint with `channel` (`"Stable"`, `"Beta"`, `"Dev"`, `"Canary"`).
   - **`raw_url`**: Fetches a plain-text manifest (e.g. `LAST_CHANGE`).
   - **`regex_scrape`**: Fetches an HTML page and extracts the version via regex capture group `(?P<version>...)`.

### Step-by-Step: Adding a New Package

1. **Create the Recipe File**:
   Create `recipes/<app_id>.json`.
   
   *Example: GitHub-hosted App (`recipes/vlc.json`)*:
   ```json
   {
     "name": "VLC",
     "aliases": ["vlc"],
     "bundle_name": "VLC.app",
     "homepage": "https://www.videolan.org/vlc/",
     "quarantine_strip": true,
     "source": {
       "type": "github_release",
       "locator": "videolan/vlc"
     },
     "livecheck": {
       "strategy": "github_latest_release"
     }
   }
   ```

   *Example: Webpage Scraped App (`recipes/custom-app.json`)*:
   ```json
   {
     "name": "CustomApp",
     "aliases": ["capp"],
     "bundle_name": "CustomApp.app",
     "quarantine_strip": true,
     "source": {
       "type": "direct_url",
       "url_template": "https://example.com/downloads/CustomApp-{version}.dmg"
     },
     "livecheck": {
       "strategy": "regex_scrape",
       "url": "https://example.com/downloads",
       "regex": "CustomApp-([0-9.]+)\\.dmg"
     }
   }
   ```

2. **Verify Detection**:
   Run:
   ```sh
   cargo run -- list
   ```
   Confirm that your new recipe appears in the list with its aliases and local installation status.

3. **Check Version & Upstream Resolution**:
   Run:
   ```sh
   cargo run -- update
   ```
   Confirm that the livecheck strategy resolves the remote version and displays the correct status badge (`Current`, `Update available`, or `Ahead of upstream`).

4. **Test Installation**:
   Run:
   ```sh
   cargo run -- install <alias>
   ```
   Verify that the archive is downloaded, SHA-256 is computed/checked, atomic directory swap occurs, and quarantine is stripped.

---

## Architecture (Implemented V2 Modules)

### 1. Recipe Loader — `src/recipe.rs`
- Defines `Recipe`, `SourceConfig`, `LivecheckConfig`.
- Loads and parses all JSON manifests under `recipes/`.
- Resolves aliases case-insensitively.

### 2. Version Engine — `src/version.rs`
- Parses version strings into numeric segments and pre-release identifiers.
- Implements `Ord`, `PartialOrd`, and semantic `PartialEq` (`5.0.1 > 5.0.0`, `1.2.10 > 1.2.9`, `2.0.0 > 2.0.0-rc1`).
- Defines `VersionComparison` (`UpToDate`, `UpdateAvailable`, `AheadOfUpstream`, `NotInstalled`).

### 3. Network & Livecheck Engine — `src/network.rs`
- Single-pass download streaming with SHA-256 calculation (`sha2` crate).
- Pluggable livecheck resolver (GitHub, Codeberg, Chromium Dash, raw URLs, regex scraping).
- Architecture detection (`arm64` vs `x86_64`) and multi-container asset filtering (`.dmg`, `.zip`, `.tar.gz`, `.tar.xz`).
- Scans recent releases to find the latest valid macOS build even if a release tag omitted macOS binaries.

### 4. Local Core & Process Management — `src/local.rs`
- Reads version from `/Applications/<bundle_name>/Contents/Info.plist` (supporting `CFBundleShortVersionString` and `CFBundleVersion`).
- Active process detection via `pgrep` to prevent modifying active executables in place.
- Graceful shutdown requests via AppleScript (`osascript`).
- Structural bundle validation (`validate_app_bundle` checking `Contents/Info.plist` and `Contents/MacOS`).
- Path-based quarantine removal (`xattr -dr com.apple.quarantine`).

### 5. Installer Engine — `src/installer.rs`
- Archive extractors:
  - Unattended DMG mounting (`hdiutil attach` with `-nobrowse`, `-readonly`, `-noverify`, and piped license acceptance).
  - ZIP extraction (`ditto -x -k`).
  - TAR extraction (`tar -xzf`, `tar -xJf`).
- **Atomic Swap Protocol**:
  1. Detects running app and warns/prompts.
  2. Renames existing `/Applications/<bundle_name>` to `.rpm-backup-<timestamp>`.
  3. Strips quarantine while still staged.
  4. Copies staged files to `/Applications/<bundle_name>`.
  5. On success: deletes backup.
  6. On failure: restores backup immediately (rollback).

### 6. Update Cache — `src/cache.rs`
- Manages `local/cache.json`.
- Records `name`, `local_version`, `remote_version`, `download_url`, `update_available`, `managed`, and `sha256`.
- `pending_upgrades`: Strictly filters for `managed && update_available && !download_url.is_empty()`.
- `was_managed`: Helper checking if an app was previously installed by `rpm`.

### 7. CLI Dispatch — `src/cli.rs` + thin `src/main.rs`
- Commands: `update`, `upgrade`, `install`, `uninstall`, `list`, `help`.
- Formatted output with color badges for update states, download URLs, and verified checksums.
- Clear distinction between `Installed [managed]` and `Installed [unmanaged]`.
- Rejection of installs targeting unmanaged existing applications, presenting clear migration steps.

---

## Session History

### 2026-09-25
- Initial project creation and scoping for v0.1.
- Documented Homebrew/apt/pacman-style safe install norms.
- Implemented baseline `local.rs`, `network.rs`, `cache.rs`, `installer.rs`, and `cli.rs`.

### 2026-09-29
- **V2 Architecture Overhaul (Homebrew Modernization)**:
  - Replaced hardcoded catalog with declarative JSON recipes (`recipes/*.json`) and loader (`src/recipe.rs`).
  - Implemented semantic versioning and ordering (`src/version.rs`) replacing naive `!=` string matching.
  - Added single-pass SHA-256 verification to `src/network.rs`.
  - Added pluggable livecheck engine supporting GitHub, Codeberg, Chromium Dash, and regex scraping.
  - Completed Chromium handler natively via `recipes/chromium.json` and snapshot archives.
  - Implemented active process detection (`pgrep`), graceful quit, and atomic directory swap in `src/installer.rs`.
  - Added support for `.tar.gz` and `.tar.xz` archives, and automated DMG license agreement bypassing.
  - Added `rpm list` command.
  - Cleaned up compiler warnings, verified all unit tests pass, and pushed changes to `origin/master`.
- **Unmanaged Package Protection (Homebrew Cask Integrity Pattern)**:
  - Analyzed Homebrew Cask collision handling (`moved.rb`).
  - Added multi-layer collision protection across CLI (`cmd_install`), installer (`install_recipe`, `install_bundle_pipeline`, `install_app_bundle_atomic`), and cache (`pending_upgrades`).
  - Enforced refusal to overwrite or automatically update unmanaged existing applications in `/Applications`.
  - Provided user-friendly error messages instructing how to uninstall outside `rpm` and reinstall via `cargo run -- install <app>`.
  - Enhanced `rpm list` to show `[unmanaged]` vs `[managed]` status badges.
