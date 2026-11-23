#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
if pgrep -x cargo >/dev/null; then
  echo "A Cargo build is already running; wait for it to finish." >&2
  exit 1
fi
platform="${1:-simulator}"
configuration="${2:-Debug}"
case "$platform" in
  simulator) target=aarch64-apple-ios-sim; sdk=iphonesimulator ;;
  device) target=aarch64-apple-ios; sdk=iphoneos ;;
  *) echo "Usage: ios/build.sh [simulator|device] [Debug|Release]" >&2; exit 1 ;;
esac
case "$configuration" in
  Debug) cargo build -p textamp-ios --target "$target" ;;
  Release) cargo build -p textamp-ios --target "$target" --release ;;
  *) echo "Configuration must be Debug or Release" >&2; exit 1 ;;
esac
xcodegen generate --spec ios/project.yml
xcodebuild -project ios/Textamp.xcodeproj -scheme Textamp -configuration "$configuration" -sdk "$sdk" -derivedDataPath target/ios-xcode CODE_SIGNING_ALLOWED=NO build
