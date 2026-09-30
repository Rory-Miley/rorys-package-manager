# Rory's Package Manager (`rpm`) MacOS

NOT TO BE CONFUSED WITH "red hat package manager (rpm) for linux", it is annoying and unfortunate that they have the same name as me, but I don't use Fedora (Arch obviously) and this package manager is for MacOS. I will consider changing the name... never.

A fast, lightweight, and safe macOS package manager written in Rust, designed for managing, updating, and installing unsigned and custom macOS software. In addition, it removes the apple "quarantine" attribute from the package.

---

## Compatibility

Honestly, no idea, but I can tell you what I have tested it on:

Macbook M3 Pro
- MacOS version: 26.6.1
- MacOS version: 26.6.2

---

## Key Features

- **Declarative JSON Recipes (`recipes/*.json`)**: Packages are defined in simple JSON files with zero recompilation needed to add or modify apps.
- **Smart Alias Resolution**: Install or manage apps using intuitive shortcuts (e.g. `qbt` for `qBittorrent`, `chrome` for `Chromium`).
- **Cryptographic SHA-256 Verification**: Streams incoming archives through a SHA-256 hasher during download to ensure file integrity.
- **Running App Protection & Atomic Swaps**: Detects running applications before upgrading, requests graceful quit, and uses an atomic directory swap with automatic rollback to protect active apps from crashes or partial overwrite corruption.
- **Pluggable Version Detection (`Livecheck`)**: Supports GitHub releases, Codeberg, Chromium Dash, raw URL manifests, and regex webpage scrapers.
- **Multi-Container Support**: Handles `.dmg` (with automatic software license agreement acceptance), `.zip`, `.tar.gz`, and `.tar.xz` archives.
- **Gatekeeper Quarantine Stripping**: Automatically strips Apple's `com.apple.quarantine` extended attribute on unsigned apps, with a per-package `quarantine_strip` toggle.
- **Package Conflict & Compatibility Protection**: Prevents installing incompatible or mutually exclusive applications (e.g. stable `Chromium` vs bleeding-edge `Chromium Snapshot`) via `conflicting_applications` recipe tags.
- **Unmanaged Application Protection (Homebrew-Inspired)**: Refuses to overwrite or automatically update existing applications in `/Applications` that were not installed through `rpm`. Protects external or manual installs from unintended modification and provides clear migration instructions.

---

## Usage

### 1. Check for Updates
Scan currently installed applications, query upstreams, compare versions, and refresh `local/cache.json`:
```sh
cargo run -- update
```
*(Only checks packages currently installed on your machine. Unmanaged applications installed outside `rpm` are flagged with a notice and skipped from automatic update queues).*

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
cargo run -- install librewolf
```
> **Security Note:** If an application is already present under `/Applications` but was **not installed by `rpm`** (unmanaged), `rpm` refuses to overwrite it to preserve system integrity. To manage it with `rpm`, uninstall or remove the existing `.app` bundle first, then run `install`.

### 4. List Installed Applications
View currently installed packages and their versions:
```sh
cargo run -- list
```
Pass `-v` or `-verbose` to view detailed verbose information (aliases, bundle name, management status badge, and description):
```sh
cargo run -- list -v
```

### 5. List All Available Packages
View all available recipes in the catalog that you can install:
```sh
cargo run -- list-all
```
Pass `-v` or `-verbose` for detailed package information:
```sh
cargo run -- list-all -v
```

### 6. Uninstall an Application
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

### Example: Conflicting Packages (`recipes/chromium.json` & `recipes/chromium-snapshot.json`)

```json
{
  "name": "Chromium",
  "aliases": ["chromium", "chrome"],
  "bundle_name": "Chromium.app",
  "homepage": "https://github.com/ungoogled-software/ungoogled-chromium-macos",
  "description": "Privacy-focused stable open-source Chromium browser",
  "quarantine_strip": true,
  "conflicting_applications": [
    "Chromium Snapshot"
  ],
  "source": {
    "type": "github_release",
    "locator": "ungoogled-software/ungoogled-chromium-macos"
  },
  "livecheck": {
    "strategy": "github_latest_release"
  }
}
```

```json
{
  "name": "Chromium Snapshot",
  "aliases": ["chromium-snapshot", "chrome-snapshot"],
  "bundle_name": "Chromium Snapshot.app",
  "homepage": "https://www.chromium.org",
  "description": "Continuous bleeding-edge snapshot build of Chromium",
  "quarantine_strip": true,
  "conflicting_applications": [
    "Chromium"
  ],
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

- **`src/recipe.rs`**: JSON recipe parsing, schema validation, `conflicting_applications` parsing, and alias matching.
- **`src/version.rs`**: Semantic version parsing and ordering (`Ord`, `PartialEq`).
- **`src/network.rs`**: Streaming SHA-256 download client and pluggable Livecheck strategies.
- **`src/local.rs`**: `Info.plist` version inspection, `pgrep` process checking, and quarantine removal.
- **`src/installer.rs`**: Container extraction (.dmg, .zip, .tar), bundle validation, atomic directory swap, unmanaged bundle guards, and conflict prevention.
- **`src/cache.rs`**: Local state tracking (`local/cache.json`) and `pending_upgrades` filtering for managed apps.
- **`src/cli.rs`**: CLI command dispatch, unmanaged app protection warnings, conflict checking, and pip-style formatted output.

---

## To Do

- [x] ~~**Fixed**: LibreWolf recipe & network.rs macOS binary resolution~~ (Resolved: strict non-macOS binary filtering and preference for `.dmg` > `.zip` > `.tar.*`).

- Improve the download UI. Make the download UI (when updating apps and downloading that update, or installing an app for the first time) show a pip style download bar. This bar should scale with the the size of the CLI window so that it doesn't go to a new line when the window is dynamically resized. The bar will be a grey outline with a 50% opacity so that it roughly matches the background colour of the CLI it is open in. I want the progress on this bar to represented by an Orange bar. On the left of the bar it should have the % that has so far be downloaded. Additionally, to the right of the bar, I want a downloaded/total MB counter to represent the progress.

- **'Further down the line'** I would like the final stages for distribution to be done. I.E. I shouldn't ahve to run "$ cargo run -- [commadn]". I want an installer script to install the rpm application so that i can use "$ rpm -- [command]" instead. Additionally, a selfupdater is required for this implimentation also. For this we will need to decide on local config directory to store the cache files and i guess the receipe json files also. I would recommend checking the homebrew package manager to see what best practice is in that regard.

- Ensure that we are crediting Homebrew in our repo in the correct file maybe a lisence.md file or something that shows we coppied a lot of the style from Homebrew and i guess credit pip because we plan on using their download bar. 

## License

MIT
