#!/bin/zsh
# Build a signed, notarized Memento.app / .dmg.
#
# macOS ties Accessibility, Microphone and Screen Recording grants to the app's
# code-signing identity. Ad-hoc builds ("-") get a new identity on every build,
# so users would have to re-grant permissions after each update. Release builds
# must be signed with a stable Developer ID certificate.
#
# Required environment (never commit these):
#   APPLE_SIGNING_IDENTITY  e.g. "Developer ID Application: Your Name (TEAMID)"
#   APPLE_ID                Apple ID used for notarization
#   APPLE_PASSWORD          app-specific password for that Apple ID
#   APPLE_TEAM_ID           10-character team id
set -euo pipefail
cd "${0:A:h}/.."

missing=()
for name in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  [[ -n "${(P)name:-}" ]] || missing+=("$name")
done
if (( ${#missing} )); then
  echo "Missing environment: ${missing[*]}" >&2
  echo "See docs/release.md." >&2
  exit 1
fi

if ! security find-identity -v -p codesigning | grep -qF "$APPLE_SIGNING_IDENTITY"; then
  echo "Signing identity not found in the keychain: $APPLE_SIGNING_IDENTITY" >&2
  security find-identity -v -p codesigning >&2
  exit 1
fi

# The config ships ad-hoc ("-") for local builds; override it for release.
npm run tauri build -- --config "{\"bundle\":{\"macOS\":{\"signingIdentity\":\"$APPLE_SIGNING_IDENTITY\"}}}"

app="src-tauri/target/release/bundle/macos/Memento.app"
echo "Verifying signature, hardened runtime and notarization ticket…"
codesign --verify --deep --strict --verbose=2 "$app"
codesign -d --entitlements - "$app" 2>/dev/null | grep -q audio-input \
  || { echo "audio-input entitlement missing from the signed app" >&2; exit 1; }
spctl --assess --type execute --verbose=2 "$app"
xcrun stapler validate "$app"
echo "OK: $app"
