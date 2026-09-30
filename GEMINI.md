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
| `rpm update [-v]` | Scan locally installed apps, check upstream versions; skip updates for unmanaged apps; write `local/cache.json` (`-v` for verbose output including download URLs, expected SHA-256 hashes, and architecture) |
| `rpm upgrade [-v] [-c]` | Read cache, download, verify SHA-256, and install pending **managed** apps (`-v` for verbose output, `-c` to bypass cached payloads and force clean download) |
| `rpm install [-v] [-c] <app>` | Resolve recipe; check compatibility conflicts; refuse if target exists but is unmanaged; install atomically and mark `managed` (`-v` for verbose output, `-c` for clean install) |
| `rpm uninstall <app>` | Safely remove a **managed** app from `/Applications` and clear `managed` in cache |
| `rpm cleanup [app] [-v]` | Remove cached download payloads from `local/staging/` to free disk space or clean specific apps (alias: `rpm clean`) |
| `rpm list [-v]` | List locally installed packages and their versions (`-v` or `-verbose` for full bundle details and descriptions) |
| `rpm list-all [-v]` | List all available recipes in catalog and installation status (`-v` or `-verbose` for full details) |
| `rpm help` | Show usage, available commands, and tracked packages |

App names and aliases are matched case-insensitively (`qbt` → `qBittorrent`, `chrome` → `Chromium`, `librewolf` → `LibreWolf`). All version outputs across all commands are standardized in square brackets `[<version>]`.

### Unmanaged Package Security Policy (Homebrew-Inspired)

To preserve user data and system integrity, `rpm` treats any application in `/Applications` not installed by `rpm` as **unmanaged**:
1. **`rpm install <app>`**: If `/Applications/<bundle_name>` exists and is not managed by `rpm`, the installation aborts immediately with an explanatory error instructing the user to remove/uninstall the existing application first if they wish to manage it via `rpm`.
2. **`rpm update`**: Remote checks run on installed packages, but unmanaged installed applications are never flagged with `update_available: true`. Guidance is printed to the console explaining how to switch to `rpm`.
3. **`rpm upgrade`**: Only upgrades apps where `managed: true`.
4. **Installer pipelines**: `install_recipe`, `install_bundle_pipeline`, and `install_app_bundle_atomic` perform defense-in-depth checks preventing overwriting unmanaged bundles.

### Mutual Package Conflict & Compatibility Protection

Certain applications cannot coexist safely on macOS (for example, stable `Chromium` and bleeding-edge continuous `Chromium Snapshot`):
1. **Recipe Declarations**: Packages declare mutually exclusive software via the optional `conflicting_applications` array (accepts aliases `conflicts_with`, `conflicts`, `conflicting_apps`).
2. **Pre-Flight Conflict Inspection**: `cmd_install` and `install_recipe` check whether any conflicting recipe or bundle is currently installed before querying upstreams or downloading payloads.
3. **Bidirectional Resolution**: Protects against conflicts whether declared directly on the target recipe or declared by an existing installed recipe against the incoming target.
4. **Friendly Migration Guidance**: Aborts with clear actionable instructions showing how to uninstall the conflicting package first via `cargo run -- uninstall <app>`.

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
  "conflicting_applications": ["ConflictingAppOrAlias"],
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
8. **`conflicting_applications`** *(Array of Strings, optional)*: Canonical names, aliases, or bundle names of applications that cannot be installed alongside this package (e.g. `["Chromium Snapshot"]`). Supports aliases: `conflicts_with`, `conflicts`, `conflicting_apps`.
9. **`source`** *(Object, required)*:
   - **`github_release`**: GitHub repository (`locator: "owner/repo"`). Automatically scans recent releases for valid macOS containers (`.dmg`, `.zip`, `.tar.gz`, `.tar.xz`). Supports optional regex `asset_filter`.
   - **`codeberg_release`**: Codeberg/Gitea repository (`locator: "owner/repo"`).
   - **`chromium_snapshot`**: Downloads snapshot zip from Google storage buckets (`channel: "auto"` selects `Mac_Arm` or `Mac`).
   - **`direct_url`**: Formats a download URL using `{version}` and `{arch}` tokens (e.g. `url_template: "https://example.com/v{version}/App-{arch}.dmg"`).
10. **`livecheck`** *(Object, required)*:
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
- Supports `conflicting_applications` with serde aliases (`conflicts_with`, `conflicts`, `conflicting_apps`).
- Resolves aliases case-insensitively.

### 2. Version Engine — `src/version.rs`
- Parses version strings into numeric segments and pre-release/revision identifiers.
- Implements `Ord`, `PartialOrd`, and semantic `PartialEq` (`5.0.1 > 5.0.0`, `1.2.10 > 1.2.9`, `2.0.0 > 2.0.0-rc1`).
- Differentiates true pre-release tags (`rc`, `beta`, `alpha`, `dev`) from package build revisions (`-1`, `-1.1`). Numeric package revisions are evaluated properly (e.g. `156.0.1-1 == 156.0.1-1`, `156.0.1-2 > 156.0.1-1`), avoiding false "Ahead of upstream" warnings.
- Defines `VersionComparison` (`UpToDate`, `UpdateAvailable`, `AheadOfUpstream`, `NotInstalled`).

### 3. Network & Livecheck Engine — `src/network.rs`
- Single-pass download streaming with SHA-256 calculation (`sha2` crate).
- Atomic destination staging: writes streaming downloads to temporary `<dest>.part` before verifying SHA-256 and atomically promoting to `<dest>`, preventing incomplete transfers from corrupting staging.
- Pluggable livecheck resolver (GitHub, Codeberg, Chromium Dash, raw URLs, regex scraping).
- Architecture detection (`arm64` vs `x86_64`) and multi-container asset filtering (`.dmg`, `.zip`, `.tar.gz`, `.tar.xz`).
- Robust scoring prioritizing `.dmg` > `.zip` > `.tar.*` while strictly filtering out Linux, Windows, AppImage, and Debian packages.
- Scans recent releases to find the latest valid macOS build even if a release tag omitted macOS binaries.

### 4. Progress & Download UI — `src/progress.rs`
- Pip-style dynamic terminal progress bar with classic solid orange blocks (`█`) and dim grey shaded background track (`░`).
- Left-aligned 6-character percentage (`  0%` to `✓ 100%`) keeping bracket positions rock-steady.
- Real-time downloaded/total MB counter, smoothed download speed (EMA), and remaining ETA countdown.
- Dynamic terminal scaling via `terminal_size` crate: strictly prevents line wraps on window resizing with `\r\x1b[K` carriage return and line clearing.
- 100% completion rendering with green checkmark, total duration, and average speed.
- Automatic TTY detection falling back to clean summary output in non-interactive environments (CI, pipes).

### 5. Local Core & Process Management — `src/local.rs`
- Reads version from `/Applications/<bundle_name>/Contents/Info.plist` (supporting `CFBundleShortVersionString`, `CFBundleVersion`, and snapshot `SCMRevision` commit positions).
- Extended attribute version tagging (`user.rpm.version`): stamps full package version onto managed bundles and reads it back via `get_bundle_rpm_version`, preserving packager revision suffixes (such as `-1` or `-1.1`) that `CFBundleShortVersionString` in `Info.plist` omits.
- Active process detection via `pgrep` to prevent modifying active executables in place.
- Graceful shutdown requests via AppleScript (`osascript`).
- Structural bundle validation (`validate_app_bundle` checking `Contents/Info.plist` and `Contents/MacOS`).
- Path-based quarantine removal (`xattr -dr com.apple.quarantine`).

### 6. Installer Engine — `src/installer.rs`
- Archive extractors:
  - Unattended DMG mounting (`hdiutil attach` with `-nobrowse`, `-readonly`, `-noverify`, and piped license acceptance).
  - ZIP extraction (`ditto -x -k`).
  - TAR extraction (`tar -xzf`, `tar -xJf`).
- **Cached Payload Reuse**:
  - `download_for_app` checks whether a non-empty payload already exists in `local/staging/`.
  - Verifies local file SHA-256 using `calculate_file_sha256`; reuses verified downloads without re-downloading if interrupted or re-run.
  - Automatically purges and redownloads corrupt or mismatched files.
- **Staging Cleanup**:
  - `cleanup_staging`: Purges all cached payload archives or targets files matching a specific app/alias, calculating and reporting total freed disk space.
  - `format_bytes`: Formats byte counts into human-readable B, KB, MB, and GB units.
- **Configurable Verbosity**:
  - Functions accept `verbose: bool` to selectively display or suppress intermediate extraction, staging, DMG mounting, Gatekeeper quarantine stripping, and backup steps while keeping active process warnings visible.
- **Compatibility & Conflict Handler**:
  - `check_conflicting_apps`: Verifies that no mutually exclusive applications or bundles are currently installed before installation begins.
- **Atomic Swap Protocol**:
  1. Detects running app and warns/prompts.
  2. Renames existing `/Applications/<bundle_name>` to `.rpm-backup-<timestamp>`.
  3. Strips quarantine while still staged.
  4. Copies staged files to `/Applications/<bundle_name>`.
  5. On success: deletes backup.
  6. On failure: restores backup immediately (rollback).

### 7. Update Cache — `src/cache.rs`
- Manages `local/cache.json`.
- Records `name`, `local_version`, `remote_version`, `download_url`, `update_available`, `managed`, and `sha256`.
- `pending_upgrades`: Strictly filters for `managed && update_available && !download_url.is_empty()`.
- `was_managed`: Helper checking if an app was previously installed by `rpm`.

### 8. CLI Dispatch — `src/cli.rs` + thin `src/main.rs`
- Commands: `update`, `upgrade`, `install`, `uninstall`, `cleanup` (alias `clean`), `list`, `list-all`, `help`.
- Standardized bracketed version formatting `[<version>]` across all commands.
- `list` only lists installed packages; `list-all` lists the complete recipe catalog.
- Supports `-v` / `--verbose` / `-verbose` across `update`, `install`, `upgrade`, `cleanup`, `list`, and `list-all` for detailed inspection (suppressing payload URLs, full SHA-256 digests, status tags, and intermediate extraction steps in default mode).
- Clean install flag (`-c` / `--clean`): purges existing cached payloads in staging and forces fresh download and installation even if currently up to date.
- Pre-flight conflict rejection in `cmd_install` aborting incompatible installations before network operations.
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
  - Cleaned up compiler warnings, verified all unit tests pass, and pushed changes to `origin/master`.
- **Unmanaged Package Protection (Homebrew Cask Integrity Pattern)**:
  - Analyzed Homebrew Cask collision handling (`moved.rb`).
  - Added multi-layer collision protection across CLI (`cmd_install`), installer (`install_recipe`, `install_bundle_pipeline`, `install_app_bundle_atomic`), and cache (`pending_upgrades`).
  - Enforced refusal to overwrite or automatically update unmanaged existing applications in `/Applications`.
  - Provided user-friendly error messages instructing how to uninstall outside `rpm` and reinstall via `cargo run -- install <app>`.
  - Enhanced `rpm list` to show `[unmanaged]` vs `[managed]` status badges.
- **LibreWolf Recipe Addition & macOS Binary Resolution Fix**:
  - Added recipe for LibreWolf browser (`recipes/librewolf.json`) tracking releases from Codeberg (`librewolf/bsys6`).
  - Enhanced `pick_mac_download_url` in `src/network.rs` with strict non-macOS filtering (excluding AppImage, deb, rpm, and Windows packages) and scoring prioritizing `.dmg` > `.zip` > `.tar.*`. Added unit tests.
- **CLI Clean-up & List/List-all Separation**:
  - `rpm list` now lists only installed packages in clean `• Name [version]` format, with `-v` or `-verbose` for full details.
  - Added `rpm list-all` to inspect the entire available recipe catalog.
  - `rpm update` scans only locally installed packages instead of uninstalled catalog recipes.
  - Standardized all version numbers inside square brackets `[...]` across `list`, `list-all`, `update`, `upgrade`, and `install`.
- **Chromium Snapshot vs Stable Chromium Separation**:
  - Separated continuous snapshot builds (`recipes/chromium-snapshot.json`, targeting `Chromium Snapshot.app`) from stable Ungoogled Chromium (`recipes/chromium.json`, targeting `Chromium.app`).
  - Added continuous commit revision parsing (`SCMRevision` containing `@{#<rev>}`) in `src/local.rs` for snapshot builds while preserving semantic versioning for stable builds.
- **Mutual Package Conflict & Compatibility Protection**:
  - Added `conflicting_applications` to `Recipe` schema (`src/recipe.rs`) with serde aliases (`conflicts_with`, `conflicts`, `conflicting_apps`).
  - Added mutual conflicts between `recipes/chromium.json` and `recipes/chromium-snapshot.json`.
  - Implemented `check_conflicting_apps` in `src/installer.rs` verifying conflicts against installed applications, aliases, and bundle paths with bidirectional resolution.
  - Integrated pre-flight check into `cmd_install` in `src/cli.rs` and defense-in-depth verification into `install_recipe` in `src/installer.rs`.
- **Pip-Style Dynamic Download UI (`src/progress.rs`)**:
  - Implemented dynamic terminal progress bar featuring solid orange blocks (`█`) and dim grey shaded track (`░`).
  - Includes real-time downloaded/total MB counter, EMA-smoothed transfer speed, and remaining ETA countdown timer.
  - Dynamically resizes to the CLI window width on each frame via `terminal_size`, maintaining strict bounds `<= terminal_width - 1` to prevent unwanted line wrapping.
  - Leaves 100% completed bar visible with a green checkmark (`✓ 100%`) before printing the SHA-256 verification line.
  - Hooked into `download_file_with_hash` in `src/network.rs` for both `install` and `upgrade` commands.
  - Added unit tests covering speed, ETA, bounds, indeterminate streams, and terminal width scaling (40-140 columns).
- **Verbose Output Filtering (`-v` / `--verbose`) & Cached Payload Reuse**:
  - Added `-v`, `--verbose`, and `-verbose` flag parsing across `rpm install` and `rpm upgrade`.
  - Filtered output to be clean and minimal by default: showing pip progress bar and `🔒 SHA-256 Verified` without raw URLs, full hash hex digests, or intermediate extraction/staging logs unless verbose mode is requested.
  - Implemented cached payload reuse in `src/installer.rs`: checks `local/staging/` for complete files matching the expected SHA-256 hash before downloading, avoiding repeated downloads when installs are delayed or interrupted (e.g. when waiting for a running app to close).
  - Added atomic `.part` download staging in `src/network.rs` ensuring broken downloads never leave corrupt files at destination paths.
  - Preserved active running process warning (`⚠️ Notice: <app> is currently running`) across both standard and verbose modes.
  - Added unit tests for file SHA-256 calculation and verified both modes live on disk.
- **Staging Cleanup, Clean Installs (`-c`), and Package Revision (`-X`) Version Tracking**:
  - Implemented `rpm cleanup [app]` command (with alias `clean`): cleans all archives from `local/staging/` or filters by specific app/alias and reports freed disk space.
  - Added `-c` / `--clean` flag to `rpm install` and `rpm upgrade`: purges cached payloads and forces a fresh download and installation even if currently up to date.
  - Fixed local version omission of package revisions (`-1` on LibreWolf, `-1.1` on Chromium): implemented `user.rpm.version` macOS extended attribute stamping (`set_bundle_rpm_version`, `get_bundle_rpm_version`) and cache fallback, ensuring the full release version string is preserved.
  - Enhanced `version.rs` to differentiate package build revisions (numeric suffixes like `-1`, `-1.1`) from pre-releases (`rc`, `beta`, `alpha`), eliminating spurious `Ahead of upstream` warnings.
  - Expanded unit test suite to 20 tests covering `format_bytes`, `cleanup_dir`, and build revision ordering.
- **Clean Standard Update Output & Verbose Mode (`update [-v]`)**:
  - Differentiated standard vs verbose output for `rpm update`:
    - Standard mode (`cargo run -- update`) prints a minimal check displaying `🔍 Checking <App>...` and `Local: [<ver>] ==/>/< Upstream: [<ver>]` while omitting `(Current)`, `[managed]`, `[unmanaged]`, remote download URLs, expected SHA-256 digests, and system architecture headers.
    - Verbose mode (`cargo run -- update -v` / `--verbose`) preserves full diagnostic output with system architecture, emoji badges (`✨`, `🚀`, `💎`, `⚠️`), status labels, payload URLs, expected SHA-256 hashes, and cache metadata.
    - Added immediate stdout flushing during version checks for real-time progress feedback.

