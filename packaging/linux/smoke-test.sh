#!/usr/bin/env bash
# Run the AppImage and the .deb in a clean container (as root).
# Usage: smoke-test.sh DIST_DIR APPVER
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 DIST_DIR APPVER" >&2
  exit 2
fi
DIST_DIR="$1" APPVER="$2"

one() {
  local files
  mapfile -t files < <(find "$DIST_DIR" -maxdepth 1 -type f -name "$1")
  if [[ ${#files[@]} -ne 1 ]]; then
    echo "error: expected exactly one $1 in $DIST_DIR" >&2
    exit 1
  fi
  readlink -f "${files[0]}"
}
A="$(one '*.AppImage')"
D="$(one '*.deb')"

expect_eq() {
  if [[ "$1" != "$2" ]]; then
    echo "error: $3: got '$1', expected '$2'" >&2
    exit 1
  fi
}

# Every FFmpeg library must resolve to the copy in LIBDIR.
check_ldd() {
  local bin="$1" libdir="$2" out name path
  out="$(ldd "$bin")"
  echo "$out"
  if grep -q 'not found' <<<"$out"; then
    echo "error: $bin has unresolved libraries" >&2
    exit 1
  fi
  # Every FFmpeg library ldd lists must resolve under LIBDIR; avcodec, avutil, swscale and swresample must be listed.
  local found=" " real
  while read -r name _ path _; do
    case "$name" in
      libavcodec.so* | libavformat.so* | libavutil.so* | libavfilter.so* | libavdevice.so* | libswscale.so* | libswresample.so* | libpostproc.so*)
        found+="$name "
        real="$(readlink -f "$path")"
        if [[ "$real" != "$(readlink -f "$libdir")"/* ]]; then
          echo "error: $bin $name resolves outside $libdir: $real" >&2
          exit 1
        fi
        ;;
    esac
  done <<<"$out"
  for name in libavcodec.so.63 libavutil.so.61 libswscale.so.10 libswresample.so.7; do
    if [[ "$found" != *" $name "* ]]; then
      echo "error: $bin does not list $name" >&2
      exit 1
    fi
  done
}

# AppImage
chmod +x "$A"
expect_eq "$("$A" --appimage-extract-and-run --version)" "transom $APPVER" "AppImage --version"
work="$(mktemp -d)"
(
  cd "$work"
  "$A" --appimage-extract >/dev/null
  expect_eq "$(squashfs-root/usr/bin/transom --version)" "transom $APPVER" "extracted transom --version"
  expect_eq "$(squashfs-root/usr/bin/transom-gui --version)" "transom-gui $APPVER" "extracted transom-gui --version"
  check_ldd squashfs-root/usr/bin/transom "$PWD/squashfs-root/usr/lib/transom"
  check_ldd squashfs-root/usr/bin/transom-gui "$PWD/squashfs-root/usr/lib/transom"
)

# .deb
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$D"
expect_eq "$(transom --version)" "transom $APPVER" "installed transom --version"
expect_eq "$(transom-gui --version)" "transom-gui $APPVER" "installed transom-gui --version"
check_ldd /usr/bin/transom /usr/lib/transom
check_ldd /usr/bin/transom-gui /usr/lib/transom
test -f /usr/share/applications/transom.desktop
test -f /usr/share/icons/hicolor/256x256/apps/transom.png
apt-get remove -y transom
test ! -e /usr/bin/transom

echo "smoke test passed"
