#!/usr/bin/env bash
# Builds RUSTMAP.app for macOS: one universal binary (Apple Silicon + Intel),
# app icon, Info.plist, ad-hoc signature, and a zip next to it in dist/.
#
#   scripts/macos-app.sh
#
# Needs Xcode command line tools and rustup. Video uses AVFoundation, so
# FFmpeg is not needed.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DIST=dist
APP="$DIST/RUSTMAP.app"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

echo "Building RUSTMAP $VERSION for ${TARGETS[*]}"
rustup target add "${TARGETS[@]}"
for t in "${TARGETS[@]}"; do
    cargo build --release --locked -p rustmap --target "$t"
done

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/rustmap" \
    target/aarch64-apple-darwin/release/rustmap \
    target/x86_64-apple-darwin/release/rustmap

# Icon: every size macOS asks for, from assets/icon.png (1024 x 1024).
ICONSET="$(mktemp -d)/RUSTMAP.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z "$s" "$s" assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/RUSTMAP.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>RUSTMAP</string>
    <key>CFBundleDisplayName</key><string>RUSTMAP</string>
    <key>CFBundleIdentifier</key><string>io.github.everaertjan.rustmap</string>
    <key>CFBundleExecutable</key><string>rustmap</string>
    <key>CFBundleIconFile</key><string>RUSTMAP</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
    <key>NSHighResolutionCapable</key><true/>
    <!-- Art-Net, sACN and NDI talk to the local network; macOS asks the user once. -->
    <key>NSLocalNetworkUsageDescription</key>
    <string>RUSTMAP sends to LED and DMX controllers (Art-Net, sACN) and sends and receives NDI video on your local network.</string>
    <key>NSBonjourServices</key>
    <array><string>_ndi._tcp</string></array>
</dict>
</plist>
PLIST

# Ad-hoc signature: required for Apple Silicon, not a Developer ID. First
# launch on another Mac: right-click > Open (or remove the quarantine flag).
codesign --force --deep --sign - "$APP"

ZIP="$DIST/RUSTMAP-$VERSION-macos-universal.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "Built $APP and $ZIP"
