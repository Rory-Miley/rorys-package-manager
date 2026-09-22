# Creating a package manager to update unsigned packages #
## Objectives ##
- Learn some rust
- have good info on the CLI, I like pip for example
- dont break
- be fast
- easy to install

## Current steps ##
- Build the checker for update check
- Build the tool to remove apple cuaratine from unsigned apps
- Download update, install update, and cleanup after.
- Offer an option to revert to last update if enabled,
	will offer to install revert to last update that is cashed.

### Section 1. The Local Core (src/local.rs) ###
- Task 1: Check if an .app actually exists in the /Applications
- Task 2: Run the macOS defaults tool via Rust to extract the version string
- Task 3: Create a custom fucntion to strip the quarantine attribute
	$ xattr -d com.apple.quarantine

### Section 2. The Network Client (src/networks.rs) for local tracking data ###
- Task 1: Create a clean wrapper for Githubs release API to fetch the latest tag
- Task 2: Create a clean wrapper for the raw URL text files
- Task 3: Add error bounds so for example lost internet prints friendly message instead of crashing

### Section 3. The State Manager (src/states.rs) ###
- Task 1: Check for and create a dedicated confi directo at ~/.config/rorys-package-manager/
- Task 2: Read and write a state.json file to log when an app was updated and what version to it used to be
- Task 3: Define a cahce folder ~/.config/rorys-package-manager/cache/ to hold old .dmg or .zip files to have roll backs if enabled by user

### Section 4. The installer Engine (src/installer.rs) ###
- Task 1: Download teh remote file (.dmg or .zip) into local cache
- Task 2: Extract cleanly using rusts file handler
- Task 3: Safety copy the new .app bundle into /Applications overwriting the old one without leaving corrupted files 

### Section 5. The CLI interface (src/main.rs) ###
- Task 1: Parse user commands, example rpm update, rpm upgrade, rpm rollback [app]
- Task 2: Add clean terminal color coding and error outputs that are similar to pip


## Wishfull thinking ##
- Cool CLI to add your own source directories for certain platforms
- Offer custom install option where you manually select what apps you,
	are looking to have by default.

