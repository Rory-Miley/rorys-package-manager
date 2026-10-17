# Rory's Package Manager — To-Do & Roadmap

This file tracks planned features, enhancements, and roadmap items for `rpm`. Work items should be driven from this list.

---

## High Priority

- [ ] **Install Output Transparency & Verbosity Overhaul (`rpm install <app>`)**
  - Explicitly display recipe origin: indicate whether an app is sourced from native `rpm` recipes or `homebrew` casks.
  - For Homebrew casks, display the active download track (`verified` vs `upstream`).
  - Standardize default vs. verbose (`-v`) output:
    - Default mode: clean, minimal, informative progress without raw URLs, hex digests, or intermediate extraction details.
    - Verbose mode (`-v` / `--verbose`): display detailed payload URLs, staging paths, calculated SHA-256 digests, and archive extraction steps.

- [ ] **Homebrew to RPM Migration Tool (`src/migrate.rs`)**
  - **Command**: `rpm migrate [homebrew]`
  - **Safety & Zero-Disruption Invariant**: Ultra-safe. Existing applications in `/Applications` are left completely intact and untouched (no re-downloading, no bundle swapping, zero risk of data loss).
  - **Homebrew Cask Research & Disowning**:
    - Investigate how Homebrew tracks casks (`/opt/homebrew/Caskroom/` on Apple Silicon, `/usr/local/Caskroom/` on Intel).
    - Safely modify or remove Homebrew Caskroom records/receipts to release Homebrew's ownership *without* triggering Homebrew to delete the `/Applications` bundle.
  - **Adoption & TOML Generation**:
    - Inspect local bundle (`Info.plist`) to record current installed version and bundle ID.
    - Query Homebrew Cask API to generate corresponding `recipes/homebrew/cask/<cask>.toml` manifests.
    - Register migrated applications in `cache.toml` with `managed = true`.
  - **Interactive Selection UI**:
    - Discover and present an enumerated list of all currently Homebrew-managed casks and their installed versions.
    - Provide interactive selection prompt allowing the user to select specific casks (e.g., `1, 3-5`), select all (`a` / `all`), or cancel.

- [ ] **Installation Progress Bar (`src/progress.rs` & `src/installer.rs`)**
  - Add a dedicated terminal progress bar for the archive extraction and `.app` bundle installation phase.
  - Match the exact layout, dynamic terminal width scaling, speed, and ETA metrics of the existing download progress bar, but styled in **green** instead of orange to visually distinguish the installation step from downloading.

---

## Medium Priority

- [ ] **Approved Binary Version Control & Pre-Built Repository Binaries**
  - Maintain version-controlled, approved `rpm` release binaries (and SHA-256 digests) inside a dedicated repository directory so users can install a known-good binary directly from the git repo without compiling from source (`cargo`).
  - Pair directly with the `install.sh` web installer and `rpm self-update` workflows to pull and verify approved binaries seamlessly.

- [ ] **One-Liner Web Installer (`install.sh`)**
  - Install command: `curl -fsSL https://raw.githubusercontent.com/rorymiley/rorys-package-manager/master/install.sh | sh`
  - Detect macOS architecture (`arm64` Apple Silicon vs `x86_64` Intel).
  - Fetch the latest approved binary from the repository's versioned binary directory (or GitHub Releases) and verify its SHA-256 digest.
  - Set up `~/.rpm/` persistent layout (`~/.rpm/recipes/rpm/`, `~/.rpm/cache.toml`, `~/.rpm/config.toml`).
  - Configure system `$PATH` via `/etc/paths.d/rpm` or shell profile fallback (`~/.zshrc`).

- [ ] **Native Self-Update (`rpm self-update`)**
  - Query upstream for new approved `rpm` binary versions.
  - Download binary, verify SHA-256 digest, and atomically replace the active executable at `/opt/rpm/bin/rpm` (or `~/.rpm/bin/rpm`).
  - Sync native recipes (`recipes/rpm/`) alongside binary update.
  - Display pip/brew-style banner during `rpm update` when a newer version of `rpm` itself is detected upstream.

---

## Future Ideas & Explorations

- [ ] **Homebrew Formula Support (CLI Packages)**
  - Extend on-demand resolution waterfall to Homebrew formulae (`recipes/homebrew/formula/`).
  - Manage command-line binaries in `/opt/rpm/bin/` or `~/.rpm/bin/`.
- [ ] **Parallel Update Checking**
  - Concurrently check remote livecheck endpoints across installed apps using tokio tasks with concurrency limits.
- [ ] **Shell Auto-Completion**
  - Generate shell completions for `zsh` and `bash` covering `rpm` commands, options, and installed app aliases.
