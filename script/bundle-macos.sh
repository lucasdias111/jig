#!/bin/sh
# Build target/Jig.app so macOS shows "Jig" in the menu bar and Dock.
#
# Run the bundled binary from a terminal so it keeps your environment
# (API keys); apps opened from Finder don't read your shell profile:
#
#     target/Jig.app/Contents/MacOS/Jig path/to/file.rs
set -eu

cd "$(dirname "$0")/.."
cargo build --release -p jig-app

app=target/Jig.app
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/Jig "$app/Contents/MacOS/Jig"
cp assets/app-icon/Jig.icns "$app/Contents/Resources/Jig.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Jig</string>
  <key>CFBundleDisplayName</key><string>Jig</string>
  <key>CFBundleIdentifier</key><string>dev.jig.editor</string>
  <key>CFBundleExecutable</key><string>Jig</string>
  <key>CFBundleIconFile</key><string>Jig</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
echo "Built $app"
