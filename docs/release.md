# Releasing Memento (macOS)

## Why signing matters here

macOS stores Accessibility, Microphone and Screen Recording permissions against
the app's **code-signing identity**. The development build is signed ad-hoc
(`"signingIdentity": "-"`), which produces a different identity on every build —
fine for development, but a release signed that way would make every user
re-grant all three permissions after every update. Releases must use a stable
Developer ID certificate, and must be notarized so Gatekeeper opens them.

## One-time setup

1. Enrol in the Apple Developer Program and create a **Developer ID Application**
   certificate; install it in your login keychain.
2. Create an **app-specific password** for your Apple ID (appleid.apple.com).
3. Note your 10-character **Team ID**.

## Build

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx"   # app-specific password
export APPLE_TEAM_ID="TEAMID"
./scripts/release-macos.sh
```

The script refuses to run without those variables or if the certificate is not
in the keychain, builds with the identity overridden, then verifies the
signature, the `audio-input` entitlement, Gatekeeper acceptance and the
notarization ticket. Tauri notarizes automatically when the `APPLE_*` variables
are present.

## What is configured

- `src-tauri/entitlements.plist` — hardened-runtime entitlements. Hardened
  runtime blocks the microphone unless `com.apple.security.device.audio-input`
  is declared.
- `src-tauri/Info.plist` — `NSMicrophoneUsageDescription`.
- `tauri.conf.json` → `bundle.macOS` — `hardenedRuntime`, `entitlements`,
  `minimumSystemVersion`.
- The app is **not** sandboxed: the Accessibility APIs Memento relies on are
  incompatible with App Sandbox.

## Things to check on a real release build

- First launch prompts for Accessibility, Microphone and Screen Recording, and
  the grants survive an update to the next version.
- Recording a meeting works (microphone entitlement) and system audio works
  after granting Screen Recording and relaunching.
- `spctl --assess --type execute -v Memento.app` reports `accepted` with source
  `Notarized Developer ID`.
