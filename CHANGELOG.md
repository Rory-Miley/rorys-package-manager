# Changelog

All notable changes to Rory's Package Manager (`rpm`) will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)

---

## [Unreleased]

---

## [0.4.5] - 2026-10-20

### Added
- **Multi-Package Batch Installation**: `rpm install [-v] [-c] [--upstream | --verified] <app1> [app2 ...]` now accepts multiple packages in a single command, resolving all targets and verifying cross-package conflict compatibility (`recipes_conflict_with_each_other`) before downloading.
- **Two-Phase Connected Tree Pipeline (`install` & `upgrade`)**: Unified `rpm install` and `rpm upgrade` into a connected `├─` / `│` / `└─` tree workflow that downloads and verifies all packages in Phase 1 (`==> Downloading N package(s)...`) and then extracts and installs all packages in Phase 2 (`==> Installing N package(s)...`).
- **Slate-Cyan Installation Progress Bar**: Added `InstallProgress` and `render_install_progress_line` in `src/progress.rs` (`\x1b[38;2;90;145;165m`) tracking real-time archive extraction (`DMG`, `ZIP`, `TAR.GZ`, `TAR.XZ`) and `.app` bundle installation via uncompressed size estimation (`hdiutil imageinfo`, `unzip -Zt`, TAR heuristic) and symlink-aware directory byte polling (`compute_dir_size`).
- **Proactive Multi-Bar Terminal Resize Sync (`TreeTranscript`)**: Resizing the terminal window wider or narrower during multi-stage or multi-package operations dynamically redraws all previously completed bars in the active tree to match the new window width and clears soft-wrapped ghost lines.
- **Unit Tests**: Added `test_render_install_progress_line_green_color_and_bounds`, `test_render_install_progress_line_completion`, `test_download_and_install_bars_always_equal_length`, `test_compute_dir_size_skips_symlinks`, `test_parse_unzip_zt_total`, `test_format_tree_branch_headers`, and `test_recipes_conflict_with_each_other`.

### Changed
- **Tree-Aligned `rpm update` Output**: Updated `rpm update` and `rpm update -v` to render package checks, version comparisons, download URLs, and SHA-256 digests inside the unified `├─` / `│` / `└─` tree hierarchy.
- **Locked Equal Progress Bar Widths**: Standardized right-hand progress metrics (`MB`, speed, elapsed/ETA) to a fixed 36-character column width (`{:>5.1}/{:>5.1} MB • {:>10} • {:>6}`) so Download and Install progress bars always share identical `[` and `]` bracket positions regardless of payload size or duration.

---

## [0.4.4] - 2026-10-19

### Added
- **Install & Upgrade Recipe Origin Badges**: `rpm install` and `rpm upgrade` now display color-coded recipe origin and track badges (`[recipe: rpm]`, `[recipe: homebrew (verified)]`, `[recipe: homebrew (upstream)]`) directly in the installation/upgrade header and up-to-date notices.
- **Expanded Verbose Diagnostics (`-v` / `--verbose`)**: Added explicit staging path, expected vs. calculated SHA-256 digests, and source-to-destination archive extraction paths (`DMG`, `ZIP`, `TAR.GZ`, `TAR.XZ`) when running with `-v`.
- Unit test `test_format_install_and_upgrade_headers` verifying install and upgrade header formatting across native and Homebrew tracks.

### Changed
- **Minimalist Default Install Output**: Gated Homebrew Cask catalog discovery notices strictly behind `-v` / `--verbose` so default `rpm install` output remains clean and pip-like.

---

## [0.4.3] - 2026-10-15

### Added
- **Homebrew Recipe Cleanup on Uninstall**: `recipe::remove_cask_recipe` automatically removes generated manifests in `recipes/homebrew/cask/<token>.toml` when an app is uninstalled via `rpm uninstall`. Native curated recipes in `recipes/rpm/` are strictly protected.
- **Track-Aware Track Switching in `install`**: Added `Recipe::matches_homebrew_track` and track mismatch detection in `cmd_install`. Installing with `--upstream` or `--verified` against a mismatched recipe automatically regenerates the recipe for the requested track.
- Unit tests `test_remove_cask_recipe` and `test_matches_homebrew_track`.

### Changed
- Standardized default install output: removed redundant `🔒 SHA-256 Verified` line when installing without `-v`.

---

## [0.4.2] - 2026-10-15

### Added
- **Recipe Origin & Track Badges in `rpm list -v`**: Modeled `RecipeOrigin` enum (`Rpm` vs `Homebrew { track }`) and displayed color-coded badges (`[recipe: rpm]`, `[recipe: homebrew (verified)]`, `[recipe: homebrew (upstream)]`) in verbose list output.
- Stored explicit `origin` and `track` metadata in generated Homebrew Cask manifests.
- Added `-list` and `--list` aliases to CLI dispatch.

---

## [0.4.1] - 2026-10-15

### Added
- **Instant System Registration**: Installed applications are registered with Launch Services (`lsregister -f`), Spotlight (`mdimport`), and touch-refreshed for instant visibility in Launchpad and Spotlight without opening Finder.
- **System Deregistration on Uninstall**: Removed bundles are unregistered from Launch Services via `lsregister -u`.
- Two-phase Gatekeeper quarantine stripping on both staged and destination `/Applications/<bundle>.app` paths.

### Changed
- **Directory Layout Isolation**: Moved native curated recipes to `recipes/rpm/` to isolate them from user-imported `recipes/homebrew/cask/` manifests.
- **Fast Filesystem Relocation**: Replaced recursive `ditto` bundle copy with fast atomic filesystem move (`fs::rename`, $O(1)$) with copy fallback on cross-device boundary (`EXDEV`).

---

## [0.4.0] - 2026-10-15

### Added
- **Index-Free On-Demand Resolution**: `rpm install <query>` checks local recipes, then native GitHub raw manifests, and falls back to the Homebrew Cask public API (`formulae.brew.sh/api/cask/<query>.json`).
- **Dual-Track Homebrew Cask Importer**:
  - Track 1 (`default` / `verified`): Homebrew verified SHA-256 and tested download URL.
  - Track 2 (`upstream`): Direct developer upstream source with dynamic checksum calculation on download.
- **Dual-Mode Configuration Engine (`src/config.rs`)**:
  - Config file at `local/config.toml` (dev) or `~/.rpm/config.toml` (installed).
  - CLI command `rpm homebrew [-default | -upstream | -request]`.
- Recursive recipe loader scanning subdirectories in `recipes/`.

### Removed
- Deprecated and removed `rpm list-all` in favor of on-demand catalog queries.

---

## [0.3.1] - 2026-10-01

### Fixed
- Progress bar layout jitter: fixed-width metrics, stable bracket positions, and dynamic terminal resize handling.
- Installer script permissions detecting `$SUDO_USER` to guarantee user ownership of `~/.rpm` and `/opt/rpm`.

---

## [0.3.0] - 2026-10-01

### Added
- **Standalone CLI Binary**: Defined `[[bin]] name = "rpm"` in `Cargo.toml`.
- **Dual-Mode Paths Engine (`src/paths.rs`)**: Automatic switching between dev mode (`./recipes/`, `./local/`) and installed system mode (`/opt/rpm/bin/rpm`, `~/.rpm/`, `~/Library/Caches/rpm/staging/`).
- Local installer and uninstaller scripts (`scripts/install-local.sh`, `scripts/uninstall-local.sh`).

---

## [0.2.1] - 2026-10-01

### Changed
- Migrated declarative recipe manifest format from JSON (`recipes/*.json`) to TOML (`recipes/*.toml`).
- Migrated state cache from `local/cache.json` to `local/cache.toml`.
- Baked binary version via compile-time `env!("CARGO_PKG_VERSION")`.

---

## [0.2.0] - 2026-09-29

### Added
- **Declarative Recipe System**: Replaced hardcoded package catalog with extensible manifests.
- **Semantic Versioning Engine (`src/version.rs`)**: Multi-segment comparison with pre-release and build revision handling.
- **Pip-Style Dynamic Progress UI (`src/progress.rs`)**: Real-time progress bar with download speed, downloaded/total MB, and ETA.
- **Mutual Conflict Protection**: Rejection of incompatible applications declared in `conflicting_applications`.
- **Unmanaged Package Protection**: Refusal to overwrite or update apps not installed by `rpm`.
- **Cached Payload Reuse**: Verified staging archives are reused on interrupted installs or re-runs.
- **Staging Cleanup**: Added `rpm cleanup [app]` (alias `clean`) and `-c` / `--clean` install flag.
- Extended attribute version tagging (`user.rpm.version`) preserving package build revisions.

---

## [0.1.0] - 2026-09-25

### Added
- Initial project implementation of macOS CLI package manager (`rpm`).
- Baseline commands: `update`, `upgrade`, `install`, `uninstall`, `list`, `help`.
- Atomic directory swap protocol with backup and automatic rollback.
- Active process detection via `pgrep` with graceful quit requests.
- Path-based Gatekeeper quarantine stripping (`xattr -dr com.apple.quarantine`).
