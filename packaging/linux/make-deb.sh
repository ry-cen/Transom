#!/usr/bin/env bash
# Build the .deb from the staged usr/ tree.
# Usage: make-deb.sh STAGE_USR DEBVER OUT
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [[ $# -ne 3 ]]; then
  echo "usage: $0 STAGE_USR DEBVER OUT" >&2
  exit 2
fi
STAGE_USR="$1" DEBVER="$2" OUT="$3"

# mktemp makes the directory 0700, and that mode would be packaged as ./
root="$(mktemp -d)"
chmod 0755 "$root"
cp -a "$STAGE_USR" "$root/usr"
install -Dm0644 "$REPO/packaging/linux/deb/copyright" "$root/usr/share/doc/transom/copyright"
mkdir -m0755 "$root/DEBIAN"
size="$(du -sk --apparent-size "$root/usr" | cut -f1)"
sed -e "s|@VERSION@|$DEBVER|" -e "s|@INSTALLED_SIZE@|$size|" \
  "$REPO/packaging/linux/deb/control.in" >"$root/DEBIAN/control"
chmod 0644 "$root/DEBIAN/control"
(cd "$root" && find usr -type f -print0 | sort -z | xargs -0 md5sum) >"$root/DEBIAN/md5sums"
chmod 0644 "$root/DEBIAN/md5sums"
find "$root" -type d -exec chmod 0755 {} +

mkdir -p "$(dirname "$OUT")"
dpkg-deb --root-owner-group -Zxz --build "$root" "$OUT"
dpkg-deb --info "$OUT"
dpkg-deb --contents "$OUT"
