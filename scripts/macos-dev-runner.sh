#!/bin/zsh
set -euo pipefail

binary_path="${1:A}"
shift

# Cargo also applies this runner to test executables. Only the actual desktop
# application belongs in the app wrapper; tests must remain attached to the
# invoking terminal so their output and exit status are preserved.
if [[ "${binary_path:t}" != "poppy" ]]; then
  /usr/bin/codesign --force --sign - "$binary_path"
  exec "$binary_path" "$@"
fi

# TCC's user-managed privacy panels expect a real application bundle. Running
# Cargo's raw executable makes Memento impossible to add reliably and gives
# each rebuild a different code identity. Wrap every dev binary at one stable
# path, include the production privacy metadata, then sign the whole bundle.
bundle_dir="${binary_path:h}/Memento Dev.app"
contents_dir="$bundle_dir/Contents"
macos_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"
bundle_binary="$macos_dir/poppy"
plist_path="$contents_dir/Info.plist"

/bin/mkdir -p "$macos_dir" "$resources_dir"
/usr/bin/install -m 755 "$binary_path" "$bundle_binary"
/bin/cp "${0:A:h}/../src-tauri/Info.plist" "$plist_path"

/usr/libexec/PlistBuddy -c "Add :CFBundleDevelopmentRegion string en" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :CFBundleDisplayName string Memento Dev" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :CFBundleExecutable string poppy" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :CFBundleIdentifier string dev.poppy.memento" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :CFBundleInfoDictionaryVersion string 6.0" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :CFBundleName string Memento Dev" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :CFBundlePackageType string APPL" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :CFBundleShortVersionString string 0.1.0" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :CFBundleVersion string 1" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :LSMinimumSystemVersion string 13.0" "$plist_path"
/usr/libexec/PlistBuddy -c "Add :NSHighResolutionCapable bool true" "$plist_path"

/usr/bin/codesign --force --deep --sign - --identifier dev.poppy.memento "$bundle_dir"
# Launch the bundle through Launch Services. Executing Contents/MacOS/poppy
# directly makes Accessibility attribute the process differently from the app
# the user enabled in System Settings.
exec /usr/bin/open --wait-apps --new "$bundle_dir" --args "$@"
