#!/usr/bin/env bash
# Build the macOS .app bundle. CI and local builds both use this script.
#
#   scripts/bundle.sh [--universal] [--out DIR]
#
#   --universal   build arm64 + x86_64 and lipo them into one binary
#   --out DIR     where the .app goes (default: dist)
#
# Environment:
#   SIGN_IDENTITY   codesign identity, e.g. "Developer ID Application: Name (TEAMID)".
#                   Unset or empty: ad-hoc signing ("-"), which is enough to run locally.
#   KEYCHAIN        keychain holding the identity (optional)
#   BUNDLE_ID       CFBundleIdentifier (default to.nathancurtis.jellyfast)
#   APP_NAME        display name and .app name (default: Jellyfast)
#   MIN_MACOS       LSMinimumSystemVersion (default 14.0)
#   CARGO_TARGET_DIR  honoured as usual
#
# The binary name and version come from `cargo metadata`, so the package
# rename needs no change here.
set -euo pipefail

universal=0
out=dist
while [ $# -gt 0 ]; do
  case "$1" in
    --universal) universal=1 ;;
    --out) out=$2; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

cd "$(dirname "$0")/.."

meta=$(cargo metadata --no-deps --format-version 1)
bin=$(jq -r '[.packages[0].targets[] | select(.kind | index("bin")) | .name][0]' <<<"$meta")
version=$(jq -r '.packages[0].version' <<<"$meta")
target_dir=$(jq -r '.target_directory' <<<"$meta")
app_name=${APP_NAME:-Jellyfast}
bundle_id=${BUNDLE_ID:-to.nathancurtis.jellyfast}
min_macos=${MIN_MACOS:-14.0}
identity=${SIGN_IDENTITY:--}

app="$out/$app_name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

if [ "$universal" = 1 ]; then
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --target "$t"
  done
  lipo -create -output "$app/Contents/MacOS/$bin" \
    "$target_dir/aarch64-apple-darwin/release/$bin" \
    "$target_dir/x86_64-apple-darwin/release/$bin"
else
  cargo build --release --locked
  cp "$target_dir/release/$bin" "$app/Contents/MacOS/$bin"
fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$app_name</string>
  <key>CFBundleDisplayName</key><string>$app_name</string>
  <key>CFBundleExecutable</key><string>$bin</string>
  <key>CFBundleIdentifier</key><string>$bundle_id</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>$min_macos</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSAppTransportSecurity</key>
  <dict><key>NSAllowsArbitraryLoads</key><true/></dict>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

sign=(codesign --force --sign "$identity" --identifier "$bundle_id")
[ -n "${KEYCHAIN:-}" ] && sign+=(--keychain "$KEYCHAIN")
if [ "$identity" != - ]; then
  sign+=(--options runtime --timestamp)   # hardened runtime, needed for notarization
fi
"${sign[@]}" "$app"
codesign --verify --strict --verbose=2 "$app"

echo "bundle: $app"
lipo -archs "$app/Contents/MacOS/$bin"
