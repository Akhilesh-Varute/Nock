#!/usr/bin/env bash
# Installs Nock for the current user (no root needed). Run it from the unpacked folder.
#   ./install.sh               install into ~/.local
#   ./install.sh --autostart   also start Nock when you log in
#   ./install.sh --uninstall   remove everything this script added
set -euo pipefail
cd "$(dirname "$0")"

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin/nock"
DESKTOP="$PREFIX/share/applications/nock.desktop"
ICON="$PREFIX/share/icons/hicolor/256x256/apps/nock.png"
AUTOSTART="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/nock.desktop"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN" "$DESKTOP" "$ICON" "$AUTOSTART"
    echo "Nock removed. Your settings stay in ${XDG_DATA_HOME:-$HOME/.local/share}/Nock"
    exit 0
fi

install -Dm755 nock "$BIN"
install -Dm644 nock.png "$ICON"
# the menu entry points at the installed copy, so it works even when ~/.local/bin is not on PATH
sed "s|^Exec=.*|Exec=$BIN|" nock.desktop | install -Dm644 /dev/stdin "$DESKTOP"
if [ "${1:-}" = "--autostart" ]; then
    sed "s|^Exec=.*|Exec=$BIN|" nock.desktop | install -Dm644 /dev/stdin "$AUTOSTART"
    echo "Nock will start at login."
fi
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true

echo "Installed to $BIN"
echo "Needs an X11 session (or XWayland) and the DejaVu Sans font (package fonts-dejavu-core)."
echo "Optional helpers: xdotool, network-manager, bluez, brightnessctl, curl."
