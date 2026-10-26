#!/usr/bin/env bash
# Build assets/icon/AppIcon.icns from assets/icon/AppIcon.png (1024x1024) using
# only sips and iconutil, so CI needs no image tooling.
#
#   scripts/make-icon.sh
#
# AppIcon.png is the already-cleaned, Apple-grid-fitted master (824px rounded
# square centred on a 1024px transparent canvas). Re-run this after replacing it.
set -euo pipefail

cd "$(dirname "$0")/.."
src=assets/icon/AppIcon.png
iconset=$(mktemp -d)/AppIcon.iconset
mkdir -p "$iconset"

for base in 16 32 128 256 512; do
  sips -z "$base" "$base" "$src" --out "$iconset/icon_${base}x${base}.png" >/dev/null
  sips -z $((base * 2)) $((base * 2)) "$src" --out "$iconset/icon_${base}x${base}@2x.png" >/dev/null
done

iconutil -c icns "$iconset" -o assets/icon/AppIcon.icns
rm -rf "$(dirname "$iconset")"
echo "wrote assets/icon/AppIcon.icns"
