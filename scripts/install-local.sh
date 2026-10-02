#!/usr/bin/env bash
set -e

# Rory's Package Manager (rpm) — Local Developer Installer
# Builds rpm from source and installs it into /opt/rpm/bin/rpm with system PATH integration.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SCRIPT_DIR"

echo -e "\033[1;34m==>\033[0m \033[1mBuilding Rory's Package Manager (rpm) in release mode...\033[0m"
cargo build --release

RELEASE_BIN="target/release/rpm"
if [ ! -f "$RELEASE_BIN" ]; then
    echo -e "\033[31mError: Release binary not found at $RELEASE_BIN\033[0m" >&2
    exit 1
fi

# Determine the real user and home directory (even if invoked with sudo)
if [ -n "$SUDO_USER" ] && [ "$SUDO_USER" != "root" ]; then
    REAL_USER="$SUDO_USER"
    USER_HOME=$(eval echo "~$SUDO_USER")
else
    REAL_USER=$(whoami)
    USER_HOME="$HOME"
fi

INSTALL_DIR="/opt/rpm/bin"
PATHS_FILE="/etc/paths.d/rpm"
USER_RPM_DIR="$USER_HOME/.rpm"
USER_RECIPES_DIR="$USER_RPM_DIR/recipes"
CACHE_DIR="$USER_HOME/Library/Caches/rpm/staging"

echo -e "\033[1;34m==>\033[0m \033[1mInstalling to /opt/rpm/bin (requires administrator password)...\033[0m"
sudo mkdir -p "$INSTALL_DIR"
sudo cp "$RELEASE_BIN" "$INSTALL_DIR/rpm"
sudo chmod 755 "$INSTALL_DIR/rpm"

# Give the real user ownership of /opt/rpm so future updates/builds don't require sudo
sudo chown -R "${REAL_USER}:admin" /opt/rpm

echo -e "\033[1;34m==>\033[0m \033[1mRegistering /opt/rpm/bin in /etc/paths.d/rpm...\033[0m"
echo "$INSTALL_DIR" | sudo tee "$PATHS_FILE" > /dev/null

echo -e "\033[1;34m==>\033[0m \033[1mSeeding recipes to $USER_RECIPES_DIR...\033[0m"
sudo mkdir -p "$USER_RECIPES_DIR"
if [ -d "recipes" ]; then
    sudo cp recipes/*.toml "$USER_RECIPES_DIR/"
fi

# Ensure user owns ~/.rpm and download cache
sudo chown -R "$REAL_USER" "$USER_RPM_DIR"
sudo mkdir -p "$CACHE_DIR"
sudo chown -R "$REAL_USER" "$USER_HOME/Library/Caches/rpm"

echo -e "\033[1;32m==>\033[0m \033[1mInstallation complete!\033[0m"
echo -e "Binary installed to: \033[1m$INSTALL_DIR/rpm\033[0m"
echo -e "Recipes seeded to:   \033[1m$USER_RECIPES_DIR\033[0m"
echo -e "Cache directory:     \033[1m$CACHE_DIR\033[0m"
echo ""
echo -e "To verify installation, open a new terminal window or run:"
echo -e "  \033[1m$INSTALL_DIR/rpm help\033[0m"
