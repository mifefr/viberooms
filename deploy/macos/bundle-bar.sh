#!/bin/zsh
# Builds the menu bar app and assembles "VibeRooms.app" in dist/ (ad-hoc signed).
set -euo pipefail
export PATH=$HOME/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin
ROOT=${0:A:h:h:h}
cd "$ROOT"
cargo build --release -p viberooms-bar
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP="$ROOT/dist/VibeRooms.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/viberooms-bar "$APP/Contents/MacOS/viberooms-bar"
cp deploy/macos/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>VibeRooms</string>
  <key>CFBundleDisplayName</key><string>VibeRooms</string>
  <key>CFBundleIdentifier</key><string>com.allanbruyere.viberooms</string>
  <key>CFBundleExecutable</key><string>viberooms-bar</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
echo "$APP"
