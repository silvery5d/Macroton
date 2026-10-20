#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
  echo 'macOS is required to build Macroton.app' >&2
  exit 1
fi

cargo build --release
app="${1:-dist/Macroton.app}"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/bin" "$app/Contents/Resources/compatibility" "$app/Contents/Resources/eligibility"
cp target/release/macroton "$app/Contents/Resources/bin/macroton"
for profile in compatibility/[0-9]*.conf; do
  case "$profile" in *.example.conf) continue ;; esac
  cp "$profile" "$app/Contents/Resources/compatibility/"
done
cp eligibility/[0-9]*.conf "$app/Contents/Resources/eligibility/"
mkdir -p target/swift-module-cache/clang target/swift-module-cache/swift
CLANG_MODULE_CACHE_PATH="$PWD/target/swift-module-cache/clang" \
SWIFT_MODULE_CACHE_PATH="$PWD/target/swift-module-cache/swift" \
  swiftc -parse-as-library -O -framework SwiftUI -framework AppKit macos/MacrotonApp.swift -o "$app/Contents/MacOS/MacrotonApp"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.macroton.preview</string>
  <key>CFBundleName</key><string>Macroton</string>
  <key>CFBundleExecutable</key><string>MacrotonApp</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
</dict></plist>
PLIST
plutil -lint "$app/Contents/Info.plist"
echo "Built unsigned development preview: $app"
