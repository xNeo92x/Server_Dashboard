#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

command -v cargo >/dev/null || { echo "cargo fehlt" >&2; exit 1; }
command -v dpkg-deb >/dev/null || { echo "dpkg-deb fehlt" >&2; exit 1; }

VERSION=$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)
ARCH=$(dpkg --print-architecture)
PKGDIR="target/deb-root"

rm -rf "$PKGDIR"
cargo build --release
install -Dm0755 target/release/server-dashboard "$PKGDIR/opt/server-dashboard/server-dashboard"
install -Dm0644 packaging/server-dashboard.desktop "$PKGDIR/usr/share/applications/server-dashboard.desktop"

mkdir -p "$PKGDIR/DEBIAN"
cat > "$PKGDIR/DEBIAN/control" <<CONTROL
Package: server-dashboard
Version: $VERSION
Section: admin
Priority: optional
Architecture: $ARCH
Depends: libgtk-4-1 (>= 4.8), ca-certificates
Maintainer: xNeo92x
Description: Native N-able N-sight RMM server health dashboard
 Rust/GTK4 wallboard with session-only encrypted API-token handling.
CONTROL

OUT="target/server-dashboard_${VERSION}_${ARCH}.deb"
dpkg-deb --build --root-owner-group "$PKGDIR" "$OUT"
echo "$OUT"
