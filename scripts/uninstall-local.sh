#!/usr/bin/env bash
set -e

# Rory's Package Manager (rpm) — Local Developer Uninstaller
# Removes /opt/rpm, /etc/paths.d/rpm, and prompts to remove ~/.rpm

OPT_DIR="/opt/rpm"
PATHS_FILE="/etc/paths.d/rpm"
USER_RPM_DIR="$HOME/.rpm"
CACHE_DIR="$HOME/Library/Caches/rpm"

echo -e "\033[1;34m==>\033[0m \033[1mUninstalling Rory's Package Manager (rpm)...\033[0m"

# 1. Remove binary and /opt/rpm
if [ -d "$OPT_DIR" ]; then
    echo -e "  • Removing \033[1m$OPT_DIR\033[0m..."
    rm -rf "$OPT_DIR"
fi

# 2. Remove /etc/paths.d/rpm (requires administrator password)
if [ -f "$PATHS_FILE" ]; then
    echo -e "  • Removing \033[1m$PATHS_FILE\033[0m (requires administrator password)..."
    sudo rm -f "$PATHS_FILE"
fi

# 3. Prompt specifically to remove ~/.rpm folder
if [ -d "$USER_RPM_DIR" ]; then
    echo ""
    read -r -p "Do you want to remove the ~/.rpm folder? [y/N] " response
    case "$response" in
        [yY][eE][sS]|[yY])
            echo -e "  • Removing \033[1m$USER_RPM_DIR\033[0m..."
            rm -rf "$USER_RPM_DIR"
            ;;
        *)
            echo -e "  • Kept \033[1m$USER_RPM_DIR\033[0m"
            ;;
    esac
fi

# 4. Prompt to remove download cache if present
if [ -d "$CACHE_DIR" ]; then
    read -r -p "Do you want to remove the download cache ($CACHE_DIR)? [y/N] " cache_resp
    case "$cache_resp" in
        [yY][eE][sS]|[yY])
            echo -e "  • Removing \033[1m$CACHE_DIR\033[0m..."
            rm -rf "$CACHE_DIR"
            ;;
        *)
            echo -e "  • Kept \033[1m$CACHE_DIR\033[0m"
            ;;
    esac
fi

echo ""
echo -e "\033[1;32m==>\033[0m \033[1mUninstallation complete!\033[0m"
echo -e "Rory's Package Manager binary and PATH registration have been removed."
echo -e "Note: Restart your terminal or open a new window to refresh your PATH."
