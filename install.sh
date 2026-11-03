#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -eq 0 ]]; then
  echo "Bitte als normaler Benutzer ausführen; sudo wird nur für die Installation benutzt." >&2
  exit 1
fi

for cmd in cargo pkg-config sudo; do
  command -v "$cmd" >/dev/null || { echo "Fehlt: $cmd" >&2; exit 1; }
done

pkg-config --atleast-version=4.8 gtk4 || {
  echo "GTK 4.8+ Entwicklungsdateien fehlen. Debian: sudo apt install libgtk-4-dev" >&2
  exit 1
}

cargo build --release
sudo install -d -m 0755 /opt/server-dashboard
sudo install -m 0755 target/release/server-dashboard /opt/server-dashboard/server-dashboard
sudo install -m 0644 packaging/server-dashboard.desktop /usr/share/applications/server-dashboard.desktop

echo
printf '%s\n' \
  "Installiert: /opt/server-dashboard/server-dashboard" \
  "Start über das App-Menü oder:" \
  "  /opt/server-dashboard/server-dashboard --fullscreen" \
  "Demo:" \
  "  /opt/server-dashboard/server-dashboard --demo"
