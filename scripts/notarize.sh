#!/usr/bin/env bash
# Notarize and staple a Developer ID signed .app.
#   scripts/notarize.sh path/to/App.app
# Environment: ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_P8 (App Store Connect API key, the .p8 text).
set -euo pipefail
app=$1
tmp=${RUNNER_TEMP:-$(mktemp -d)}
printf '%s' "$ASC_KEY_P8" > "$tmp/asc.p8"
trap 'rm -f "$tmp/asc.p8"' EXIT
ditto -c -k --keepParent "$app" "$tmp/notarize.zip"
xcrun notarytool submit "$tmp/notarize.zip" --key "$tmp/asc.p8" --key-id "$ASC_KEY_ID" \
  --issuer "$ASC_ISSUER_ID" --wait --timeout 30m --output-format json > "$tmp/notary.json"
status=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["status"])' "$tmp/notary.json")
echo "Notarization: $status"
if [ "$status" != Accepted ]; then
  id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$tmp/notary.json")
  xcrun notarytool log "$id" --key "$tmp/asc.p8" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" || true
  exit 1
fi
xcrun stapler staple "$app"
xcrun stapler validate "$app"
