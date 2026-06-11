#!/usr/bin/env bash
# Build "Companion Cube.app" — a signed (ad-hoc) macOS bundle around
# ccube-daemon. Bundle identity is what gives notifications our name and
# icon (UNUserNotificationCenter refuses to work without it), enables
# login-item autostart later, and keeps Gatekeeper calmer than a bare binary.
#
# Usage: scripts/make-bundle.sh [--debug]
# Output: dist/Companion Cube.app
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE=release
CARGO_FLAGS=(--release)
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE=debug
  CARGO_FLAGS=()
fi

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP="dist/Companion Cube.app"

echo "==> Frontend (the daemon embeds build/ at compile time)"
npm run build

echo "==> Daemon ($PROFILE)"
# (the +" expansion keeps macOS bash 3.2's set -u happy when the array is empty)
cargo build -p ccube-daemon ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}

echo "==> Bundle structure"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$PROFILE/ccube-daemon" "$APP/Contents/MacOS/ccube-daemon"

echo "==> Icon (icns from design/icon-1024.png)"
ICONSET=$(mktemp -d)/ccube.iconset
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z $size $size design/icon-1024.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -z $((size*2)) $((size*2)) design/icon-1024.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/ccube.icns"

echo "==> Info.plist"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>      <string>ccube-daemon</string>
    <key>CFBundleIdentifier</key>      <string>com.companioncube.daemon</string>
    <key>CFBundleName</key>            <string>Companion Cube</string>
    <key>CFBundleDisplayName</key>     <string>Companion Cube</string>
    <key>CFBundleVersion</key>         <string>$VERSION</string>
    <key>CFBundleShortVersionString</key> <string>$VERSION</string>
    <key>CFBundlePackageType</key>     <string>APPL</string>
    <key>CFBundleIconFile</key>        <string>ccube</string>
    <key>LSMinimumSystemVersion</key>  <string>12.0</string>
    <!-- Menu-bar app: no Dock icon, no app switcher entry -->
    <key>LSUIElement</key>             <true/>
    <key>NSHighResolutionCapable</key> <true/>
</dict>
</plist>
PLIST

echo "==> Ad-hoc codesign"
codesign --force --deep --sign - "$APP"

echo "==> Done: $APP (v$VERSION)"
echo "    Verify: open '$APP' && curl -s http://127.0.0.1:7431/api/health"
