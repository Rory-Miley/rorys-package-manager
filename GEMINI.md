# AI.md — session context for Rory's Package Manager

This is the context and instructions file for Rory's Package Manager (`rpm`). It can be updated by any AI model in use, but **only after asking the user first**.

When developing this project, **goals and tasks should be focused on this file and `TO_DO.md`** to keep work in scope. If work starts drifting off scope, remind the user and either:
- steer back to what's listed here or in `TO_DO.md`, or
- offer to **add new scope to `TO_DO.md` or this file** (with user approval) before implementing it.

---

## AI & Developer Workflow Rules

### Commit & Release Protocol
When the user requests to **commit and push** (or wrap up a feature/release):
1. **Commit Intent Differentiation**:
   - **Feature / Bugfix Commits**: Trigger the interactive release prompts below.
   - **Documentation / Chore / Refactor Commits**: Default to staging, committing, and pushing directly to `master` without prompting for version bump or tag, unless explicitly requested.
2. **Interactive Release Prompts**:
   - Read `Cargo.toml` for the current version (e.g. `0.4.3`).
   - Ask if the user wants to increment the version number in `Cargo.toml` (state the current version and propose next semver increment, e.g. `Current version is 0.4.3. Increment to 0.4.4?`).
   - Ask if the user wants to tag the release with that version (e.g. `git tag 0.4.4`).
   - Ask if the user wants to update `README.md` to align with the current status and feature set of the project.
3. **Changelog Maintenance**: Ensure notable user-facing changes are documented under the corresponding version heading in [`CHANGELOG.md`](./CHANGELOG.md).
4. **README Alignment**: If requested by the user, update `README.md` to reflect new commands, flags, or architecture.
5. **Execute**: Stage changed files, commit, create the git tag if requested, and push to origin.

### CLI Output Philosophy: Minimalist by Default
Command output must be clean, human-friendly, and pip-like by default (e.g., progress bars, checkmarks, `• App [version]`). Never add raw download URLs, hex checksum digests, staging paths, or intermediate extraction details to the standard output. All diagnostic, transport, and debugging information belongs strictly behind the `-v` / `--verbose` flag.

### Architecture & Bug Triage Protocol
When diagnosing non-trivial bugs, performance issues, or evaluating design alternatives, explain the root cause and present viable approaches (with pros, cons, and a recommended solution) before touching code, unless the user explicitly commands immediate implementation.

### Core Invariants & Mandatory Unit Testing
Every code change affecting resolution, installation, or cache must preserve the four core project invariants:
1. **Unmanaged Application Protection**: Refuse to overwrite or update bundles in `/Applications` not installed by `rpm`.
2. **Cache Freshness**: Never reuse an older version payload from staging for an upstream install.
3. **Quarantine Removal**: Strip `com.apple.quarantine` on unsigned packages after install/upgrade.
4. **Atomic Swaps**: Safeguard running apps with atomic directory swaps, backups, and automatic rollback on failure.
Any new installer, resolver, or recipe logic must include unit tests, and the full test suite (`cargo test`) must pass before concluding work.

### Dev Environment Hygiene
When testing local commands in dev mode, clean up any temporary scratch files or dummy test recipes created during verification, leaving `./local/staging/` and `./recipes/homebrew/` clean and predictable. **Important**: If an existing package recipe was already present in `recipes/homebrew/` prior to AI intervention, it must be preserved and not deleted.

### File Separation
- **`README.md`**: Public user-facing project documentation, install instructions, CLI usage guide, and feature showcase.
- **`GEMINI.md`** (this file): Architectural context, invariant design principles, decided product rules, recipe schema, and assistant protocols.
- **`TO_DO.md`**: Feature backlog, prioritized to-dos, and future roadmap items.
- **`CHANGELOG.md`**: Full historical release notes following the [Keep a Changelog](https://keepachangelog.com/) standard.

---

## Project Goal

A macOS CLI package manager (`rpm`) for **unsigned** and custom macOS apps: check updates, download/install, strip Apple quarantine after upgrades, and safely replace bundles without breaking running apps.

**Motivations:** learn Rust · pip-like CLI · don't break installs · be fast · easy to install · Homebrew-inspired recipe decoupling.

---

## Design Principles

| Principle | Meaning |
|---|---|
| **Learn Rust** | Clear, idiomatic, functional Rust |
| **Good CLI** | pip-like: clear commands, colors, friendly errors |
| **Don't break** | Atomic directory swap, backup/rollback, running app checks, unmanaged app protection |
| **Be fast** | Reasonable timeouts, stream downloads with single-pass hashing |
| **Easy to install** | Low friction to get `rpm` onto a machine |
| **Recipe-driven** | Declarative TOML recipes decoupled from host APIs |
| **Stay in scope** | Drive work from this file & `TO_DO.md`; expand only with user OK |

---

## Decided Product Rules

### CLI Commands

| Command | Behavior |
|---|---|
| `rpm update [-v]` | Scan locally installed apps, check upstream versions; skip updates for unmanaged apps; write `local/cache.toml` (`-v` for verbose output including download URLs, expected SHA-256 hashes, and architecture) |
| `rpm upgrade [-v] [-c]` | Read cache, download, verify SHA-256, and install pending **managed** apps (`-v` for verbose output, `-c` to bypass cached payloads and force clean download) |
| `rpm install [-v] [-c] [--upstream \| --verified] <app>` | Resolve recipe on demand (local -> native GitHub raw -> Homebrew Cask API); check compatibility conflicts; refuse if target exists but is unmanaged; install atomically and mark `managed` (`-v` for verbose output, `-c` for clean install) |
| `rpm uninstall <app>` | Safely remove a **managed** app from `/Applications`, remove auto-generated Homebrew recipe (`recipes/homebrew/cask/<cask>.toml`), and clear `managed` in cache |
| `rpm cleanup [app] [-v]` | Remove cached download payloads from `local/staging/` to free disk space or clean specific apps (alias: `rpm clean`) |
| `rpm list [-v]` | List locally installed packages and their versions (`-v` or `-verbose` for full bundle details, descriptions, and origin badges `[recipe: rpm]`, `[recipe: homebrew (verified)]`, `[recipe: homebrew (upstream)]`) |
| `rpm homebrew [-default \| -upstream \| -request]` | View or set default download verification track for Homebrew casks (`default` = Homebrew verified SHA-256, `upstream` = direct developer release, `request` = prompt every time) |
| `rpm self-update [-v]` | Check upstream release of `rpm`, download binary, verify SHA-256, atomically swap binary in `/opt/rpm/bin/rpm`, and sync recipes |
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

### Dual-Mode Paths Engine (`src/paths.rs`)
- **Local Dev Mode**: Automatically detected when `./recipes/` and `Cargo.toml` exist in the current working directory (or overridden by `RPM_DEV=1`/`RPM_DIR`). Uses `./recipes/`, `./local/cache.toml`, and `./local/staging/`.
- **Installed System Mode**: When run outside the repository, uses:
  - Binary: `/opt/rpm/bin/rpm` (added to system `$PATH` via `/etc/paths.d/rpm`).
  - Persistent Recipes: `~/.rpm/recipes/*.toml`.
  - Persistent State: `~/.rpm/cache.toml`.
  - Download Cache: `~/Library/Caches/rpm/staging/` (macOS-compliant purgeable cache; auto-cleaned on disk pressure without breaking `rpm` state).

### Index-Free On-Demand Resolution Waterfall (`rpm install <query>`)
1. Checks local recipes in `recipes/rpm/*.toml` and `recipes/homebrew/cask/*.toml`.
2. If found locally as a Homebrew recipe, checks if the user passed an explicit track override (`--upstream` or `--verified`). If the track differs, regenerates the recipe from Homebrew API.
3. If not found locally, checks native GitHub raw repository: `raw.githubusercontent.com/rorymiley/rorys-package-manager/master/recipes/rpm/<query>.toml`. If 200 OK, saves to `recipes/rpm/<query>.toml` and proceeds.
4. If not found natively, checks Homebrew Cask public API: `formulae.brew.sh/api/cask/<query>.json`. If 200 OK, triggers the Dual-Track Homebrew Cask Importer and saves to `recipes/homebrew/cask/<cask>.toml`.
5. If neither exists (both 404), aborts with a friendly error: `Package '<query>' not found in rpm recipes or Homebrew casks.`

---

## How to Add New Packages (TOML Recipe Guide)

Package recipes live in `recipes/rpm/<app_name>.toml`. Adding a new package requires **zero Rust recompilation**.

### Recipe TOML Schema

```toml
name = "CanonicalDisplayName"
aliases = ["alias1", "alias2"]
bundle_name = "ApplicationName.app"
homepage = "https://example.com"
description = "Short description of the software"
quarantine_strip = true
sha256 = "optional_expected_sha256_hash"
conflicting_applications = ["ConflictingAppOrAlias"]

[source]
type = "github_release | codeberg_release | chromium_snapshot | direct_url | homebrew_cask"
locator = "owner/repository"

[source.asset_filter]
arm64 = "regex_pattern_for_apple_silicon"
x86_64 = "regex_pattern_for_intel"
fallback = "regex_pattern_fallback"

[livecheck]
strategy = "github_latest_release | codeberg_latest_release | chromium_dash | raw_url | regex_scrape | homebrew_cask"
channel = "Stable"
url = "https://example.com/downloads"
regex = "(?P<version>[0-9]+(\\.[0-9]+)+)"
```

---

## Core Architecture Overview

- **`src/recipe.rs`**: Recipe manifest definitions, recursive loader, origin/track resolution (`RecipeOrigin`), track matching (`matches_homebrew_track`), and safe removal (`remove_cask_recipe`).
- **`src/version.rs`**: Version parsing into numeric segments and build revisions; implements semantic `Ord` and `VersionComparison`.
- **`src/network.rs`**: Single-pass streaming download with SHA-256 verification and atomic `.part` promotion; livecheck resolvers (GitHub, Codeberg, Chromium Dash, Homebrew Cask API).
- **`src/progress.rs`**: Dynamic terminal progress bar (`█░`) with EMA speed, remaining ETA, and terminal resize scaling.
- **`src/local.rs`**: Bundle inspection (`Info.plist`), extended attribute version stamping (`user.rpm.version`), process detection (`pgrep`), quarantine removal (`xattr`), and system registration (`lsregister`, `mdimport`, `touch`).
- **`src/installer.rs`**: Archive extractors (DMG, ZIP, TAR), cached staging payload reuse, staging cleanup (`cleanup_staging`), compatibility checker (`check_conflicting_apps`), and atomic bundle swap protocol.
- **`src/cache.rs`**: State persistence in `local/cache.toml` (or `~/.rpm/cache.toml`).
- **`src/config.rs`**: User preferences in `local/config.toml` (or `~/.rpm/config.toml`), managing `[homebrew] track`.
- **`src/cli.rs` + `src/main.rs`**: CLI command dispatch, interactive track prompting, and argument handling.
