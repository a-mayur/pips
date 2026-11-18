#!/bin/bash
# Builds a universal Pips.app, Pips.dmg, Pips.zip and SHA256SUMS.txt into build/.
# Usage: ./build.sh [version]   (default 0.0.0)
set -euo pipefail
cd "$(dirname "$0")"

VERSION="${1:-0.0.0}"
APP=build/Pips.app

rm -rf build
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

for arch in arm64 x86_64; do
  swiftc -O -swift-version 5 -target "$arch-apple-macos13.0" Sources/*.swift -o "build/Pips-$arch"
done
lipo -create build/Pips-arm64 build/Pips-x86_64 -output "$APP/Contents/MacOS/Pips"

cp Resources/Info.plist "$APP/Contents/Info.plist"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
plutil -replace CFBundleShortVersionString -string "$VERSION" "$APP/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$VERSION" "$APP/Contents/Info.plist"

# ad-hoc signature: not notarized
codesign --force --sign - "$APP"

mkdir build/dmg
cp -R "$APP" build/dmg/
ln -s /Applications build/dmg/Applications
hdiutil create -volname Pips -srcfolder build/dmg -ov -format UDZO build/Pips.dmg
rm -rf build/dmg build/Pips-arm64 build/Pips-x86_64

ditto -c -k --keepParent "$APP" build/Pips.zip
(cd build && shasum -a 256 Pips.dmg Pips.zip > SHA256SUMS.txt)

echo "Built $APP, build/Pips.dmg, build/Pips.zip (version $VERSION)"
