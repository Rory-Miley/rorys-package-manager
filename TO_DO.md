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

---

## Medium Priority

- [ ] **Native Self-Update (`rpm self-update`)**
  - Query GitHub Releases for new `rpm` releases.
  - Download binary, verify SHA-256 digest, and atomically replace the active executable at `/opt/rpm/bin/rpm` (or `~/.rpm/bin/rpm`).
  - Sync native recipes (`recipes/rpm/`) alongside binary update.
  - Display pip/brew-style banner during `rpm update` when a newer version of `rpm` itself is detected upstream.

- [ ] **One-Liner Web Installer (`install.sh`)**
  - Install command: `curl -fsSL https://raw.githubusercontent.com/rorymiley/rorys-package-manager/master/install.sh | sh`
  - Detect macOS architecture (`arm64` Apple Silicon vs `x86_64` Intel).
  - Download latest release binary from GitHub releases.
  - Set up `~/.rpm/` persistent layout (`~/.rpm/recipes/rpm/`, `~/.rpm/cache.toml`, `~/.rpm/config.toml`).
  - Configure system `$PATH` via `/etc/paths.d/rpm` or shell profile fallback (`~/.zshrc`).

---

## Future Ideas & Explorations

- [ ] **Homebrew Formula Support (CLI Packages)**
  - Extend on-demand resolution waterfall to Homebrew formulae (`recipes/homebrew/formula/`).
  - Manage command-line binaries in `/opt/rpm/bin/` or `~/.rpm/bin/`.
- [ ] **Parallel Update Checking**
  - Concurrently check remote livecheck endpoints across installed apps using tokio tasks with concurrency limits.
- [ ] **Shell Auto-Completion**
  - Generate shell completions for `zsh` and `bash` covering `rpm` commands, options, and installed app aliases.
