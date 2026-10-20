#!/usr/bin/env bash
# Build the AppImage from the staged usr/ tree.
# Usage: make-appimage.sh STAGE_USR APPIMAGETOOL RUNTIME OUT
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [[ $# -ne 4 ]]; then
  echo "usage: $0 STAGE_USR APPIMAGETOOL RUNTIME OUT" >&2
  exit 2
fi
STAGE_USR="$1" APPIMAGETOOL="$2" RUNTIME="$3" OUT="$4"

appdir="$(mktemp -d)/Transom.AppDir"
mkdir -m0755 "$appdir"
cp -a "$STAGE_USR" "$appdir/usr"
install -m0755 "$REPO/packaging/linux/appimage/AppRun" "$appdir/AppRun"
install -m0644 "$REPO/packaging/linux/transom.desktop" "$appdir/transom.desktop"
install -m0644 "$REPO/assets/icons/transom-256.png" "$appdir/transom.png"

mkdir -p "$(dirname "$OUT")"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" --no-appstream --runtime-file "$RUNTIME" "$appdir" "$OUT"
test -s "$OUT"
chmod 0755 "$OUT"
