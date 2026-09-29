# Rory's Package Manager (`rpm`)

A fast, lightweight, and safe macOS package manager written in Rust, designed for managing, updating, and installing unsigned and custom macOS software.

---

## Key Features

- **Declarative JSON Recipes (`recipes/*.json`)**: Packages are defined in simple JSON files with zero recompilation needed to add or modify apps.
- **Smart Alias Resolution**: Install or manage apps using intuitive shortcuts (e.g. `qbt` for `qBittorrent`, `chrome` for `Chromium`).
- **Semantic Version Checking**: Compares versions using natural SemVer ordering (`>`), distinguishing between *Up to date*, *Update available*, and *Ahead of upstream / Dev build*.
- **Cryptographic SHA-256 Verification**: Streams incoming archives through a SHA-256 hasher during download to ensure file integrity.
- **Running App Protection & Atomic Swaps**: Detects running applications before upgrading, requests graceful quit, and uses an atomic directory swap with automatic rollback to protect active apps from crashes or partial overwrite corruption.
- **Pluggable Version Detection (`Livecheck`)**: Supports GitHub releases, Codeberg, Chromium Dash, raw URL manifests, and regex webpage scrapers.
- **Multi-Container Support**: Handles `.dmg` (with automatic software license agreement acceptance), `.zip`, `.tar.gz`, and `.tar.xz` archives.
- **Gatekeeper Quarantine Stripping**: Automatically strips Apple's `com.apple.quarantine` extended attribute on unsigned apps, with a per-package `quarantine_strip` toggle.
- **Unmanaged Application Protection (Homebrew-Inspired)**: Refuses to overwrite or automatically update existing applications in `/Applications` that were not installed through `rpm`. Protects external or manual installs from unintended modification and provides clear migration instructions.

---

## Usage

### 1. Check for Updates
Scan all tracked recipes, query upstreams, compare versions, and refresh `local/cache.json`:
```sh
cargo run -- update
```
*(Unmanaged applications installed outside `rpm` are flagged with a notice and skipped from automatic update queues).*

### 2. Upgrade Pending Applications
Download, verify SHA-256, and install pending updates from the cache (only manages `rpm`-tracked apps):
```sh
cargo run -- upgrade
```

### 3. Install an Application
Install or reinstall an app by its canonical name or alias:
```sh
cargo run -- install qbt
cargo run -- install chromium
```
> **Security Note:** If an application is already present under `/Applications` but was **not installed by `rpm`** (unmanaged), `rpm` refuses to overwrite it to preserve system integrity. To manage it with `rpm`, uninstall or remove the existing `.app` bundle first, then run `install`.

### 4. List Tracked Recipes
View all available recipes, aliases, and their local installation status (`Installed [managed]`, `Installed [unmanaged]`, or `Not Installed`):
```sh
cargo run -- list
```

### 5. Uninstall an Application
Safely remove a managed application from `/Applications` and update cache:
```sh
cargo run -- uninstall qbt
```

---

## Adding New Packages (JSON Recipes)

To add a new application, drop a JSON recipe file into the `recipes/` directory.

### Example: GitHub-Hosted App (`recipes/qbittorrent.json`)

```json
{
  "name": "qBittorrent",
  "aliases": ["qbittorrent", "qbt"],
  "bundle_name": "qBittorrent.app",
  "homepage": "https://www.qbittorrent.org",
  "description": "BitTorrent client with a Qt user interface",
  "quarantine_strip": true,
  "source": {
    "type": "github_release",
    "locator": "qbittorrent/qBittorrent"
  },
  "livecheck": {
    "strategy": "github_latest_release"
  }
}
```

### Example: Chromium Snapshot (`recipes/chromium.json`)

```json
{
  "name": "Chromium",
  "aliases": ["chromium", "chrome"],
  "bundle_name": "Chromium.app",
  "homepage": "https://www.chromium.org",
  "description": "Open-source browser project",
  "quarantine_strip": true,
  "source": {
    "type": "chromium_snapshot",
    "channel": "auto"
  },
  "livecheck": {
    "strategy": "raw_url",
    "url": "https://commondatastorage.googleapis.com/chromium-browser-snapshots/Mac_Arm/LAST_CHANGE"
  }
}
```

For full recipe documentation and advanced options (such as regex web scrapers and custom asset filters), see [GEMINI.md](GEMINI.md).

---

## Architecture Overview

- **`src/recipe.rs`**: JSON recipe parsing, schema validation, and alias matching.
- **`src/version.rs`**: Semantic version parsing and ordering (`Ord`, `PartialEq`).
- **`src/network.rs`**: Streaming SHA-256 download client and pluggable Livecheck strategies.
- **`src/local.rs`**: `Info.plist` version inspection, `pgrep` process checking, and quarantine removal.
- **`src/installer.rs`**: Container extraction (.dmg, .zip, .tar), bundle validation, atomic directory swap, and unmanaged bundle guards.
- **`src/cache.rs`**: Local state tracking (`local/cache.json`) and `pending_upgrades` filtering for managed apps.
- **`src/cli.rs`**: CLI command dispatch, unmanaged app protection warnings, and pip-style formatted output.

---

## To Do

- Improve the download UI. Make the download UI (when updating apps and downloading that update, or installing an app for the first time) show a pip style download bar. This bar should scale with the the size of the CLI window so that it doesn't go to a new line when the window is dynamically resized. The bar will be a grey outline with a 50% opacity so that it roughly matches the background colour of the CLI it is open in. I want the progress on this bar to represented by an Orange bar. On the left of the bar it should have the % that has so far be downloaded. Additionally, to the right of the bar, I want a downloaded/total MB counter to represent the progress.

- **'Further down the line'** I would like the final stages for distribution to be done. I.E. I shouldn't ahve to run "$ cargo run -- [commadn]". I want an installer script to install the rpm application so that i can use "$ rpm -- [command]" instead. Additionally, a selfupdater is required for this implimentation also. For this we will need to decide on local config directory to store the cache files and i guess the receipe json files also. I would recommend checking the homebrew package manager to see what best practice is in that regard.

- Ensure that we are crediting Homebrew in our repo in the correct file maybe a lisence.md file or something that shows we coppied a lot of the style from Homebrew and i guess credit pip because we plan on using their download bar. 

## License

MIT
