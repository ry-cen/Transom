#!/usr/bin/env bash
# Build the usr/ tree shared by the AppImage and the .deb.
# Usage: stage-usr.sh TARGET_DIR FFMPEG_DIR FFMPEG_NAME FFMPEG_TAG DEST_USR
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [[ $# -ne 5 ]]; then
  echo "usage: $0 TARGET_DIR FFMPEG_DIR FFMPEG_NAME FFMPEG_TAG DEST_USR" >&2
  exit 2
fi
TARGET_DIR="$1" FFMPEG_DIR="$2" FFMPEG_NAME="$3" FFMPEG_TAG="$4" DEST_USR="$5"

if [[ -e "$DEST_USR" ]]; then
  echo "error: $DEST_USR already exists" >&2
  exit 1
fi

for b in transom transom-gui; do
  install -Dm0755 "$TARGET_DIR/$b" "$DEST_USR/bin/$b"
done

# The binaries must carry DT_RPATH (not DT_RUNPATH) so the rpath also covers the
# dependencies of the FFmpeg libraries.
for b in transom transom-gui; do
  dyn="$(readelf -d "$DEST_USR/bin/$b")"
  # shellcheck disable=SC2016 # $ORIGIN is literal
  if ! grep -F '(RPATH)' <<<"$dyn" | grep -qF '[$ORIGIN/../lib/transom]' || grep -qF '(RUNPATH)' <<<"$dyn"; then
    echo "error: $b has the wrong rpath" >&2
    grep -E 'RPATH|RUNPATH' <<<"$dyn" >&2 || true
    exit 1
  fi
  grep -E 'RPATH|RUNPATH' <<<"$dyn"
done

# Copy the FFmpeg libraries the binaries need, and what those need in turn.
needed() { readelf -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\]$/\1/p'; }
libdir="$DEST_USR/lib/transom"
mkdir -p "$libdir"
# libavformat is not referenced by Transom itself, so the linker may drop it from
# NEEDED; bundle all five FFmpeg libraries unconditionally.
ffmpeg_libs=(libavcodec.so.63 libavformat.so.63 libavutil.so.61 libswresample.so.7 libswscale.so.10)
for n in "${ffmpeg_libs[@]}"; do
  [[ -e "$FFMPEG_DIR/lib/$n" ]] || {
    echo "error: $FFMPEG_DIR/lib/$n not found" >&2
    exit 1
  }
  install -m0644 "$(readlink -f "$FFMPEG_DIR/lib/$n")" "$libdir/$n"
done
bin_seen=" "
for b in transom transom-gui; do
  while read -r n; do
    bin_seen+="$n "
  done < <(needed "$DEST_USR/bin/$b")
done
queue=("$DEST_USR/bin/transom" "$DEST_USR/bin/transom-gui" "${ffmpeg_libs[@]/#/$libdir/}")
seen=" "
while ((${#queue[@]})); do
  f="${queue[0]}"
  queue=("${queue[@]:1}")
  while read -r n; do
    seen+="$n "
    [[ -e "$FFMPEG_DIR/lib/$n" && ! -e "$libdir/$n" ]] || continue
    install -m0644 "$(readlink -f "$FFMPEG_DIR/lib/$n")" "$libdir/$n"
    queue+=("$libdir/$n")
  done < <(needed "$f")
done
# Every FFmpeg library the walk saw must be one we bundle, and the core ones must be present.
for n in $seen; do
  case "$n" in
    libavcodec.so* | libavformat.so* | libavutil.so* | libavfilter.so* | libavdevice.so* | libswscale.so* | libswresample.so* | libpostproc.so*)
      [[ " ${ffmpeg_libs[*]} " == *" $n "* ]] || {
        echo "error: unexpected FFmpeg dependency $n" >&2
        exit 1
      }
      ;;
  esac
done
for n in libavcodec.so.63 libavutil.so.61 libswscale.so.10; do
  [[ "$bin_seen" == *" $n "* ]] || {
    echo "error: binaries do not NEED $n" >&2
    exit 1
  }
done
[[ "$seen" == *" libswresample.so.7 "* ]] || {
  echo "error: dependency walk did not find libswresample.so.7" >&2
  exit 1
}
got=""
for p in "$libdir"/*; do got+="${p##*/} "; done
want="libavcodec.so.63 libavformat.so.63 libavutil.so.61 libswresample.so.7 libswscale.so.10 "
[[ "$got" == "$want" ]] || {
  echo "error: bundled FFmpeg set is '$got', expected '$want'" >&2
  exit 1
}

install -Dm0644 "$REPO/packaging/linux/transom.desktop" "$DEST_USR/share/applications/transom.desktop"

for s in 16 32 48 64 128 256 512; do
  install -Dm0644 "$REPO/assets/icons/transom-$s.png" "$DEST_USR/share/icons/hicolor/${s}x${s}/apps/transom.png"
done
install -Dm0644 "$REPO/assets/icons/transom.svg" "$DEST_USR/share/icons/hicolor/scalable/apps/transom.svg"

doc="$DEST_USR/share/doc/transom"
for f in README.md LICENSE NOTICE; do
  install -Dm0644 "$REPO/$f" "$doc/$f"
done
install -Dm0644 "$FFMPEG_DIR/LICENSE.txt" "$doc/FFmpeg-LICENSE.txt"
sed -e "s|@FFMPEG_NAME@|$FFMPEG_NAME|g" -e "s|@FFMPEG_TAG@|$FFMPEG_TAG|g" \
  "$REPO/packaging/linux/FFmpeg-NOTICE.txt.in" >"$doc/FFmpeg-NOTICE.txt"
chmod 0644 "$doc/FFmpeg-NOTICE.txt"
