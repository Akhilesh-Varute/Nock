#!/usr/bin/env bash
# Builds, from target/release/nock:
#   packaging/linux/out/nock_<version>_amd64.deb
#   packaging/linux/out/nock-<version>-x86_64-linux.tar.gz   (user-level install, no root)
# Usage: packaging/linux/package.sh [version]     (default: the version in Cargo.toml)
set -euo pipefail
cd "$(dirname "$0")/../.."

VER="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
VER="${VER#v}"
BIN=target/release/nock
[ -x "$BIN" ] || { echo "build first: cargo build --release" >&2; exit 1; }
OUT=packaging/linux/out
mkdir -p "$OUT"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

# ---- .deb
D="$STAGE/deb"
install -Dm755 "$BIN" "$D/usr/bin/nock"
install -Dm644 packaging/linux/nock.desktop "$D/usr/share/applications/nock.desktop"
install -Dm644 assets/icon256.png "$D/usr/share/icons/hicolor/256x256/apps/nock.png"
install -Dm644 LICENSE "$D/usr/share/doc/nock/copyright"
mkdir -p "$D/DEBIAN"
SIZE="$(du -sk "$D/usr" | cut -f1)"
cat > "$D/DEBIAN/control" <<EOF
Package: nock
Version: $VER
Section: utils
Priority: optional
Architecture: amd64
Installed-Size: $SIZE
Maintainer: Akhilesh Varute <akhil.ecc@gmail.com>
Homepage: https://github.com/Akhilesh-Varute/Nock
Depends: libc6, libx11-6, libxcb1, libxcursor1, libxi6, libxrandr2, libxkbcommon-x11-0, fonts-dejavu-core
Recommends: xdotool, network-manager, bluez, brightnessctl, curl
Description: tiny notch and top bar for the desktop
 A small native notch and top bar written in Rust: now playing, focus timer,
 to-dos, clipboard history, file shelf, app launcher, small games, and a top
 bar with Wi-Fi, Bluetooth, sound, brightness and battery dropdowns.
 Runs on X11 (or Wayland through XWayland). Preview release.
EOF
dpkg-deb --root-owner-group --build "$D" "$OUT/nock_${VER}_amd64.deb" >/dev/null

# ---- tarball
T="$STAGE/nock-$VER-x86_64-linux"
install -Dm755 "$BIN" "$T/nock"
install -Dm644 packaging/linux/nock.desktop "$T/nock.desktop"
install -Dm644 assets/icon256.png "$T/nock.png"
install -Dm755 packaging/linux/install.sh "$T/install.sh"
install -Dm644 LICENSE "$T/LICENSE"
tar -C "$STAGE" -czf "$OUT/nock-$VER-x86_64-linux.tar.gz" "nock-$VER-x86_64-linux"

ls -l "$OUT"
