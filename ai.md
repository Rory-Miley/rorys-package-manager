# AI.md — session context for Rory's Package Manager

This is the AI.md file for tracking context across sessions and different AI models. It can be updated by any AI model in use, but **only after asking the user first**.

When developing this project, **goals and tasks should be focused on this file** to keep work in scope. If work starts drifting off scope, remind the user and either:

- steer back to what's listed here, or
- offer to **add new scope to this file** (with user approval) before implementing it.

---

## Project goal

A macOS CLI package manager (`rpm`) for **unsigned** apps: check updates, download/install, strip Apple quarantine after upgrades, and optionally roll back from a local cache (one prior package, per-app opt-in).

**Motivations:** learn Rust · pip-like CLI · don't break installs · be fast · easy to install.

**v0.1 success criteria:** successfully **upgrade one app** (qBittorrent — GitHub, most developed path) and **strip its quarantine**. That is the near-term focus.

---

## Design principles

| Principle | Meaning |
|-----------|---------|
| Learn Rust | Clear, idiomatic Rust |
| Good CLI | pip-like: clear commands, colors, friendly errors |
| Don't break | Follow package-manager norms for safe replace (see below) |
| Be fast | Reasonable timeouts; stream downloads |
| Easy to install | Low friction to get `rpm` onto a machine |
| Stay in scope | Drive work from this file; expand only with user OK |

---

## Decided product rules

### CLI commands (planned; `cli.rs` not built yet)

| Command | Behavior |
|---------|----------|
| `rpm update` | Check remotes, print status, **write** `local/cache.json` |
| `rpm upgrade` | **Read** `local/cache.json`, then download + install pending apps |
| `rpm rollback [app]` | Revert using cached prior package (exact flag names TBD) |

Other commands (`list`, `install`, etc.) are out of scope until added here.

### Update cache (`local/cache.json`)

- Project-local file (for now): `local/cache.json` — written by update, read by upgrade/installer.
- Fields per app: `name`, `local_version`, `remote_version`, `download_url`, `update_available`; plus top-level `updated_at` (unix seconds).
- **No hashing/encryption** for now — contents are public release metadata on your machine; revisit later only if needed.
- **Stale cache (planned, not implemented yet):** if cache is older than **1 day**, notify the user (e.g. suggest re-running `update`). Do not block other work on this yet.
- Distinct from rollback package cache under `~/.config/.../cache/` (binary `.dmg`/`.zip` files).

### Safe install (“don't break”) — follow package-manager norms

How common tools do it (what we should mirror):

1. **Fetch completely first** (apt/pacman/Homebrew): download the full artifact into a cache/staging area *before* touching the live install.
2. **Stage, then commit** (Homebrew Cask): unpack/prepare the new `.app` under a private staging dir (Homebrew uses Caskroom), then move/replace into `/Applications`. Never half-write into the live bundle mid-download.
3. **Prefer in-place content replace on upgrade** (modern Homebrew Cask): keep the existing `.app` directory and replace its contents so macOS preserves Dock, Launchpad, and related state; only delete-and-recreate the whole `.app` if in-place replace isn't possible.
4. **On failure before commit**: leave the previous live app untouched.
5. **Optional prior artifact** (our addition): if rollback is enabled for that app, keep **one** previous `.dmg`/`.zip` (or staged package) in cache so `rollback` can restore it.

**Our installer rule of thumb:** download → stage/extract → (if rollback enabled: retain previous package) → replace live `.app` safely → strip quarantine → update state. If any step before the live replace fails, abort with the old app still working.

### Rollback

- Keep **exactly one** prior package per app (when enabled).
- **Per-app flag**, not global-only: enable at install time, or later on an already-tracked app so the *next* upgrade caches a rollback copy.
- Exact CLI flag names TBD later.
- Rollback restores from that single cached prior package.

### App catalog vs handlers

| Hardcoded in Rust | Stored in local JSON (user-editable later) |
|-------------------|--------------------------------------------|
| Source handlers: GitHub, Codeberg, Chromium Dash, raw URL, etc. | App **name**, **repo type**, and **URL/locator** |

Stop hardcoding the app list in source eventually; handlers stay in code. Near-term hardcoding is OK while building toward v0.1.

### Quarantine

- Strip quarantine **only after a successful upgrade** of that app (`xattr -dr com.apple.quarantine`).
- Do **not** strip on check-only (`update`) or if upgrade failed.

### Version comparison

- For now: normalize tags with the shared stripper (`v` / `release-` prefixes, etc.), then **string-compare** to local `CFBundleShortVersionString`.
- If a false “update available” appears for equal versions, fix by improving the normaliser later — no per-app mapping yet.

### Install formats

- **Now / v0.1:** `.dmg` and `.zip` that contain a `.app`.
- **Later (in scope to add when ready):** `.pkg` via macOS `installer` (typically needs elevated privileges) — confirm when implementing; not required for v0.1.

---

## Architecture (planned modules)

### 1. Local core — `src/local.rs`
- Detect `{Name}.app` under `/Applications`
- Read version via `defaults` → `CFBundleShortVersionString`
- Strip quarantine after successful upgrade

### 2. Network client — `src/network.rs`
- GitHub / Codeberg / Chromium Dash / raw URL wrappers
- Arch-aware asset picking (arm64 / x86_64 / universal)
- Friendly network errors

### 3. Update cache — `src/cache.rs` + `local/cache.json`
- Serialise/deserialise the update index between `update` and `upgrade`
- Stale (>1 day) user notification — planned later

### 4. State manager — `src/states.rs` *(not created yet)*
- `~/.config/rorys-package-manager/`
- `state.json` — update history, previous version, **per-app rollback enabled?**
- `cache/` — prior **package binaries** when rollback is on (max one)

### 5. Installer engine — `src/installer.rs`
- Read pending apps from `local/cache.json`
- Download into staging
- Extract `.dmg`/`.zip`
- Safe replace into `/Applications` (norms above)
- Hook quarantine strip after success

### 6. CLI — planned `src/cli.rs` (or equivalent) + `main.rs`
- Parse `update` / `upgrade` / `rollback`
- pip-like colors and errors

---

## Roadmap (scoped)

### v0.1 (current focus)
1. Stabilize GitHub path + write `local/cache.json` for qBittorrent
2. Download + stage + install qBittorrent into `/Applications` (from cache)
3. Strip quarantine after that upgrade
4. Minimal CLI (`update` / `upgrade`) enough to run that one upgrade flow

### Next after v0.1
- Notify when update cache is older than 1 day
- State + optional per-app rollback (one prior package)
- Move app list into JSON (name, type, URL); keep handlers in Rust
- Broader update checker across catalog

### Later / wish list (do not start unless added as active scope)
- Per-app normaliser overrides
- `.pkg` install support
- Hash/encrypt cache or catalog if ever needed
- Custom source directories / platforms via CLI
- Interactive “pick default apps” install flow

---

## Implementation snapshot

| Area | Status |
|------|--------|
| `local.rs` | Version read + quarantine strip implemented |
| `network.rs` | Stabilized: arch detect, shared asset picker, `(version, url)` releases |
| `cache.rs` | Read/write `local/cache.json` |
| `installer.rs` | Can load pending upgrades from cache; download-to-path only; no extract/replace yet |
| `states.rs` | Missing |
| CLI | `cli.rs` + thin `main`; `update` works; `upgrade` stubbed |

---

## Session history

### 2026-09-25
- Created `ai.md` from README + code pass; asked clarifying questions.
- User answered CLI, rollback, catalog, quarantine, versions, formats, v0.1 target.
- Documented Homebrew/apt/pacman-style safe install norms for “don't break.”
- Added header: cross-model context file; update only with user request; keep work scoped to this file.
- Chose update→`local/cache.json`→upgrade handoff (no encryption); stale >1 day notify planned later.
- Stabilized `network.rs`; added `cache.rs` + installer cache read; `main` writes cache for qBittorrent.
- Added `cli.rs`: thin `main` → `cli::run`; implemented `update`; stubbed `upgrade`.
