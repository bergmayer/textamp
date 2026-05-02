#!/usr/bin/env bash
# End-to-end macOS packaging for the textamp TUI binary as a
# Finder-launchable .app bundle. Mirrors `notarize-mac.sh` (the GUI
# pipeline) but the bundle's entry point is a launcher shell script
# that opens Terminal.app and runs the embedded binary, since a TUI
# needs a terminal host to display.
#
# What it does:
#   1. cargo build --release the TUI binary.
#   2. Generate a multi-resolution AppIcon.icns from icon.jpg.
#   3. Stage Textamp-TUI.app under /tmp:
#        Contents/MacOS/launch    — bash launcher (CFBundleExecutable)
#        Contents/MacOS/textamp   — the actual TUI binary
#        Contents/Resources/      — AppIcon.icns
#        Contents/Info.plist
#   4. Sign launcher + binary, then the bundle. Hardened runtime +
#      timestamp + entitlements.
#   5. Submit to notarytool and wait.
#   6. Staple the ticket so Gatekeeper validates offline.
#   7. Zip the stapled bundle for distribution.
#
# Recipient experience:
#   • Unzip → drag .app to /Applications (or Desktop).
#   • Double-click → Terminal opens and the TUI runs in it.
#   • No "unidentified developer" warning, no `xattr -d` step.
#
# Prerequisites (one-time, same as notarize-mac.sh):
#   • Developer ID Application certificate in login keychain.
#   • Developer ID G2 intermediate CA installed.
#   • Keychain pre-authorised for codesign.
#   • notarytool keychain profile created (default name: TEXTAMP_NOTARY).
#
# Usage:
#   dev/notarize-tui-mac.sh              # full pipeline
#   dev/notarize-tui-mac.sh --no-notary  # build + sign + skip notary
#   dev/notarize-tui-mac.sh --build-only # build binary, skip everything else
#   dev/notarize-tui-mac.sh --clean      # rm staging + dist artifacts
#
# Override defaults via env vars:
#   SIGN_IDENTITY    — full Developer ID Application identity string.
#   NOTARY_PROFILE   — keychain profile name (default: TEXTAMP_NOTARY).
#   APP_NAME         — bundle display name (default: Textamp-TUI).
#   BUNDLE_ID        — reverse-DNS bundle id (default:
#                      com.bergmayer.textamp-tui).
#   APP_VERSION      — Info.plist version (default: from Cargo.toml).
#   ICON_SOURCE      — path to source PNG/JPG (default: icon.jpg).
#   OUTPUT_DIR       — where final .app and .zip land (default: ~/Desktop).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

ENTITLEMENTS="$REPO_ROOT/dev/entitlements.plist"
BIN_PATH="$REPO_ROOT/target/release/textamp"

APP_NAME="${APP_NAME:-Textamp-TUI}"
BUNDLE_ID="${BUNDLE_ID:-com.bergmayer.textamp-tui}"
ICON_SOURCE="${ICON_SOURCE:-$REPO_ROOT/icon.jpg}"
OUTPUT_DIR="${OUTPUT_DIR:-$HOME/Desktop}"
NOTARY_PROFILE="${NOTARY_PROFILE:-TEXTAMP_NOTARY}"

APP_VERSION="${APP_VERSION:-$(awk -F'"' '/^version/ { print $2; exit }' Cargo.toml)}"

WORK_DIR="/tmp/textamp_tui_pkg_workspace"
APP_BUNDLE="$WORK_DIR/${APP_NAME}.app"
ICONSET_DIR="$WORK_DIR/AppIcon.iconset"
ICNS_PATH="$WORK_DIR/AppIcon.icns"
SUBMIT_ZIP="$WORK_DIR/${APP_NAME}-submit.zip"
DIST_ZIP="$OUTPUT_DIR/${APP_NAME}.zip"
DEST_APP="$OUTPUT_DIR/${APP_NAME}.app"

MODE=full
while [ $# -gt 0 ]; do
    case "$1" in
        --build-only)  MODE=build_only ;;
        --no-notary)   MODE=sign_only ;;
        --clean)       MODE=clean ;;
        --help|-h)     sed -n '2,/^$/p' "$0" | sed 's/^# *//'; exit 0 ;;
        *) echo "Unknown flag: $1" >&2; exit 1 ;;
    esac
    shift
done

if [ "$MODE" = clean ]; then
    rm -rf "$WORK_DIR" "$DIST_ZIP" "$DEST_APP"
    echo "Cleaned $WORK_DIR, $DEST_APP, $DIST_ZIP."
    exit 0
fi

AUTO_IDENT="$(security find-identity -v -p codesigning \
    | awk -F'"' '/Developer ID Application:/ {print $2; exit}' || true)"
SIGN_IDENTITY="${SIGN_IDENTITY:-${AUTO_IDENT:-}}"

if [ "$MODE" != build_only ] && [ -z "$SIGN_IDENTITY" ]; then
    cat >&2 <<MSG
Error: no 'Developer ID Application' code-signing identity in your
login keychain. Install one from developer.apple.com → Certificates
and re-run, or set SIGN_IDENTITY=... explicitly.
MSG
    exit 1
fi

echo "==> Repo:        $REPO_ROOT"
echo "==> Bundle:      $APP_BUNDLE"
echo "==> App version: $APP_VERSION"
echo "==> Bundle ID:   $BUNDLE_ID"
[ -n "$SIGN_IDENTITY" ] && echo "==> Identity:    $SIGN_IDENTITY"
echo "==> Profile:     $NOTARY_PROFILE  (mode=$MODE)"
echo

echo "==> Building textamp (TUI release)"
cargo build --release --bin textamp

if [ "$MODE" = build_only ]; then
    echo "Done. Binary at: $BIN_PATH"
    exit 0
fi

[ -x "$BIN_PATH" ] || { echo "Error: $BIN_PATH not found after build." >&2; exit 1; }

echo "==> Generating AppIcon.icns from $ICON_SOURCE"
[ -f "$ICON_SOURCE" ] || { echo "Error: ICON_SOURCE '$ICON_SOURCE' not found." >&2; exit 1; }
rm -rf "$ICONSET_DIR" "$ICNS_PATH"
mkdir -p "$ICONSET_DIR"
for spec in "16 16x16" "32 16x16@2x" "32 32x32" "64 32x32@2x" \
            "128 128x128" "256 128x128@2x" "256 256x256" \
            "512 256x256@2x" "512 512x512" "1024 512x512@2x"; do
    read -r sz name <<<"$spec"
    sips -z "$sz" "$sz" -s format png "$ICON_SOURCE" \
         --out "$ICONSET_DIR/icon_${name}.png" >/dev/null
done
iconutil -c icns "$ICONSET_DIR"
[ -f "$ICNS_PATH" ] || { echo "Error: iconutil produced no .icns." >&2; exit 1; }

echo "==> Staging $APP_BUNDLE"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

# Copy the TUI binary in via a clean byte-copy to drop iCloud/file-
# provider xattrs that codesign would later reject as "detritus".
python3 - "$BIN_PATH" "$APP_BUNDLE/Contents/MacOS/textamp" <<'PY'
import sys
src, dst = sys.argv[1], sys.argv[2]
with open(src, 'rb') as r, open(dst, 'wb') as w:
    w.write(r.read())
PY
chmod +x "$APP_BUNDLE/Contents/MacOS/textamp"

# Launcher script — the bundle's CFBundleExecutable. Finder runs
# this; it opens a terminal host and execs the actual TUI binary
# inside it. Prefers Ghostty when present (truecolor and sixel
# defaults) and falls back to Terminal.app otherwise.
cat > "$APP_BUNDLE/Contents/MacOS/launch" <<'LAUNCH'
#!/bin/bash
# Resolve the bundle's MacOS dir from the running script — works
# regardless of where the user dragged the .app to (/Applications,
# Desktop, ~/Downloads, etc.).
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/textamp"

# Prefer Ghostty if installed. Spotlight finds it wherever the user
# placed it; the standard install paths are checked as a fallback in
# case Spotlight is disabled.
GHOSTTY_APP="$(mdfind \
    "kMDItemCFBundleIdentifier == 'com.mitchellh.ghostty'" 2>/dev/null \
    | head -n1)"
if [ -z "$GHOSTTY_APP" ]; then
    for cand in "/Applications/Ghostty.app" "$HOME/Applications/Ghostty.app"; do
        if [ -d "$cand" ]; then
            GHOSTTY_APP="$cand"
            break
        fi
    done
fi

if [ -n "$GHOSTTY_APP" ] && [ -d "$GHOSTTY_APP" ]; then
    # `open -na` opens a new Ghostty instance and detaches from our
    # process, so the launcher exits cleanly while Ghostty stays
    # running. `--args -e <bin>` is passed through to ghostty(1),
    # which runs the binary in a new window.
    open -na "$GHOSTTY_APP" --args -e "$BIN"
    exit 0
fi

# Fall back to Terminal.app. `exec` replaces the host shell so the
# tab closes (or stays open per user preference) when textamp
# exits, instead of leaving an idle bash prompt.
osascript >/dev/null <<EOF
tell application "Terminal"
    activate
    do script "exec '${BIN}'"
end tell
EOF
LAUNCH
chmod +x "$APP_BUNDLE/Contents/MacOS/launch"

cp "$ICNS_PATH" "$APP_BUNDLE/Contents/Resources/AppIcon.icns"

cat > "$APP_BUNDLE/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
    <key>CFBundleExecutable</key><string>launch</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>${APP_NAME}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${APP_VERSION}</string>
    <key>CFBundleVersion</key><string>${APP_VERSION}</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>Copyright © 2026 John Bergmayer.</string>
</dict>
</plist>
EOF

# Sign the inner binary and the launcher script first, then wrap-
# sign the bundle. Both must carry a signature for notarization.
echo "==> Signing inner TUI binary"
codesign --force --options runtime --timestamp \
         --entitlements "$ENTITLEMENTS" \
         --sign "$SIGN_IDENTITY" \
         "$APP_BUNDLE/Contents/MacOS/textamp"

echo "==> Signing launcher script"
codesign --force --options runtime --timestamp \
         --sign "$SIGN_IDENTITY" \
         "$APP_BUNDLE/Contents/MacOS/launch"

echo "==> Signing app bundle"
codesign --force --options runtime --timestamp \
         --entitlements "$ENTITLEMENTS" \
         --sign "$SIGN_IDENTITY" \
         "$APP_BUNDLE"

echo "==> Verifying signature"
codesign --verify --strict --verbose=2 "$APP_BUNDLE"
echo "==> spctl pre-notary (Unnotarized = expected)"
spctl --assess --type execute --verbose=2 "$APP_BUNDLE" || true

if [ "$MODE" = sign_only ]; then
    echo
    echo "==> Skipping notarization (--no-notary). Copying signed bundle:"
    rm -rf "$DEST_APP"
    ditto "$APP_BUNDLE" "$DEST_APP"
    echo "    $DEST_APP"
    exit 0
fi

echo
echo "==> Packaging $SUBMIT_ZIP for upload"
rm -f "$SUBMIT_ZIP"
ditto -c -k --keepParent "$APP_BUNDLE" "$SUBMIT_ZIP"

echo "==> Submitting to notarytool (typically 1–5 min)"
xcrun notarytool submit "$SUBMIT_ZIP" \
      --keychain-profile "$NOTARY_PROFILE" \
      --wait

echo
echo "==> Stapling ticket"
xcrun stapler staple "$APP_BUNDLE"
xcrun stapler validate "$APP_BUNDLE"

echo "==> spctl post-notary (should now accept)"
spctl --assess --type execute --verbose=2 "$APP_BUNDLE"

echo
echo "==> Copying signed+stapled bundle to $DEST_APP"
rm -rf "$DEST_APP"
ditto "$APP_BUNDLE" "$DEST_APP"

echo "==> Building distribution zip $DIST_ZIP"
rm -f "$DIST_ZIP"
ditto -c -k --keepParent "$DEST_APP" "$DIST_ZIP"

echo
echo "==================================================================="
echo " Done."
echo "   App:  $DEST_APP"
echo "   Zip:  $DIST_ZIP"
echo "==================================================================="
codesign -dvv "$DEST_APP" 2>&1 | sed 's/^/  /'
