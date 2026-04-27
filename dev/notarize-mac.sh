#!/usr/bin/env bash
# End-to-end macOS packaging for textamp-gui — build, bundle, sign,
# notarize, staple, ship. No Xcode, no Swift, no Apple developer
# tools beyond `codesign`, `xcrun notarytool`, and `xcrun stapler`
# (all installed by the Command Line Tools `xcode-select --install`).
#
# What it does, in order:
#   1.  cargo build --release the GUI binary (skipped if up-to-date).
#   2.  Generate a multi-resolution AppIcon.icns from icon.jpg.
#   3.  Stage Textamp.app under /tmp (NOT the Desktop — iCloud Desktop
#       sync injects a `com.apple.fileprovider.fpfs#P` xattr that
#       codesign rejects as "detritus").
#   4.  Sign the inner binary with Developer ID + hardened runtime +
#       timestamp + entitlements, then sign the bundle wrapper.
#   5.  Verify signature, run a pre-notary spctl check (will say
#       "Unnotarized" — expected at this stage).
#   6.  ditto-zip the bundle and submit to notarytool with --wait.
#       Bails out if the submission fails review.
#   7.  staple the ticket so Gatekeeper can validate offline.
#   8.  Re-zip the stapled bundle for distribution.
#   9.  Move the signed+stapled .app onto ~/Desktop (overwriting any
#       prior copy) and leave the distribution zip alongside it.
#
# Required prerequisites (one-time):
#   • Developer ID Application certificate installed in login keychain.
#       security find-identity -v -p codesigning
#     should show a `Developer ID Application: <name> (TEAMID)` line.
#   • Developer ID G2 intermediate CA installed:
#       curl -sSLO https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer
#       security import DeveloperIDG2CA.cer -k ~/Library/Keychains/login.keychain-db
#   • Keychain pre-authorised for codesign (otherwise the first sign
#     blocks on a SecurityAgent dialog):
#       security set-key-partition-list -S apple-tool:,apple: -s -k '' \
#         ~/Library/Keychains/login.keychain-db
#   • notarytool keychain profile created once via app-specific
#     password from appleid.apple.com:
#       xcrun notarytool store-credentials TEXTAMP_NOTARY \
#         --apple-id <your-apple-id> --team-id <TEAMID> \
#         --password <xxxx-xxxx-xxxx-xxxx>
#
# Usage:
#   dev/notarize-mac.sh              # full pipeline
#   dev/notarize-mac.sh --no-notary  # sign + staple skipped, ad-hoc-style
#   dev/notarize-mac.sh --build-only # build binary, skip everything else
#   dev/notarize-mac.sh --clean      # rm -rf staging + dist artifacts
#
# Override defaults via env vars:
#   SIGN_IDENTITY    — full Developer ID Application identity string.
#                      Auto-detected when exactly one is in keychain.
#   NOTARY_PROFILE   — keychain profile name (default: TEXTAMP_NOTARY).
#   APP_NAME         — bundle display name (default: Textamp).
#   BUNDLE_ID        — reverse-DNS bundle id (default: com.bergmayer.textamp).
#   APP_VERSION      — Info.plist version (default: from Cargo.toml).
#   ICON_SOURCE      — path to source PNG/JPG (default: icon.jpg).
#   OUTPUT_DIR       — where final .app and .zip land (default: ~/Desktop).

set -euo pipefail

# ── 0. Repo paths ───────────────────────────────────────────────────────────
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

ENTITLEMENTS="$REPO_ROOT/dev/entitlements.plist"
BIN_PATH="$REPO_ROOT/target/release/textamp-gui"

APP_NAME="${APP_NAME:-Textamp}"
BUNDLE_ID="${BUNDLE_ID:-com.bergmayer.textamp}"
ICON_SOURCE="${ICON_SOURCE:-$REPO_ROOT/icon.jpg}"
OUTPUT_DIR="${OUTPUT_DIR:-$HOME/Desktop}"
NOTARY_PROFILE="${NOTARY_PROFILE:-TEXTAMP_NOTARY}"

# Pull the version straight from Cargo.toml so a `cargo set-version`
# bump propagates without a parallel edit here.
APP_VERSION="${APP_VERSION:-$(awk -F'"' '/^version/ { print $2; exit }' Cargo.toml)}"

WORK_DIR="/tmp/textamp_pkg_workspace"
APP_BUNDLE="$WORK_DIR/${APP_NAME}.app"
ICONSET_DIR="$WORK_DIR/AppIcon.iconset"
ICNS_PATH="$WORK_DIR/AppIcon.icns"
SUBMIT_ZIP="$WORK_DIR/${APP_NAME}-submit.zip"
DIST_ZIP="$OUTPUT_DIR/${APP_NAME}.zip"
DEST_APP="$OUTPUT_DIR/${APP_NAME}.app"

# ── 1. Argument parsing ────────────────────────────────────────────────────
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

# ── 2. Auto-detect signing identity ────────────────────────────────────────
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

# ── 3. Build release binary ────────────────────────────────────────────────
echo "==> Building textamp-gui (release, gui+native-menus)"
cargo build --release \
    --no-default-features \
    --features gui,native-menus \
    --bin textamp-gui

if [ "$MODE" = build_only ]; then
    echo "Done. Binary at: $BIN_PATH"
    exit 0
fi

[ -x "$BIN_PATH" ] || { echo "Error: $BIN_PATH not found after build." >&2; exit 1; }

# ── 4. Build AppIcon.icns from a source image ──────────────────────────────
echo "==> Generating AppIcon.icns from $ICON_SOURCE"
[ -f "$ICON_SOURCE" ] || { echo "Error: ICON_SOURCE '$ICON_SOURCE' not found." >&2; exit 1; }
rm -rf "$ICONSET_DIR" "$ICNS_PATH"
mkdir -p "$ICONSET_DIR"
# `sips -s format png` is critical — without it, sips writes JPEG
# bytes into a .png file and iconutil chokes on the iconset.
for spec in "16 16x16" "32 16x16@2x" "32 32x32" "64 32x32@2x" \
            "128 128x128" "256 128x128@2x" "256 256x256" \
            "512 256x256@2x" "512 512x512" "1024 512x512@2x"; do
    read -r sz name <<<"$spec"
    sips -z "$sz" "$sz" -s format png "$ICON_SOURCE" \
         --out "$ICONSET_DIR/icon_${name}.png" >/dev/null
done
iconutil -c icns "$ICONSET_DIR"
[ -f "$ICNS_PATH" ] || { echo "Error: iconutil produced no .icns." >&2; exit 1; }

# ── 5. Stage the bundle in /tmp ────────────────────────────────────────────
# We assemble entirely outside iCloud-synced folders to avoid the
# `com.apple.fileprovider.fpfs#P` xattr that codesign rejects. Once
# signed, we ditto the bundle to OUTPUT_DIR; codesign-validated
# bundles tolerate the resulting iCloud xattrs.
echo "==> Staging $APP_BUNDLE"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"
# Bytes-only write — `cp` would inherit `com.apple.provenance` which
# also makes codesign unhappy on stricter macOS releases.
python3 - "$BIN_PATH" "$APP_BUNDLE/Contents/MacOS/textamp-gui" <<'PY'
import sys
src, dst = sys.argv[1], sys.argv[2]
with open(src, 'rb') as r, open(dst, 'wb') as w:
    w.write(r.read())
PY
chmod +x "$APP_BUNDLE/Contents/MacOS/textamp-gui"
cp "$ICNS_PATH" "$APP_BUNDLE/Contents/Resources/AppIcon.icns"

cat > "$APP_BUNDLE/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
    <key>CFBundleExecutable</key><string>textamp-gui</string>
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

# ── 6. Sign inner binary, then bundle ──────────────────────────────────────
echo "==> Signing inner binary (this hits Apple's TSP, ~10–60s)"
codesign --force --options runtime --timestamp \
         --entitlements "$ENTITLEMENTS" \
         --sign "$SIGN_IDENTITY" \
         "$APP_BUNDLE/Contents/MacOS/textamp-gui"

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

# ── 7. Submit & wait ───────────────────────────────────────────────────────
echo
echo "==> Packaging $SUBMIT_ZIP for upload"
rm -f "$SUBMIT_ZIP"
ditto -c -k --keepParent "$APP_BUNDLE" "$SUBMIT_ZIP"

echo "==> Submitting to notarytool (typically 1–5 min)"
xcrun notarytool submit "$SUBMIT_ZIP" \
      --keychain-profile "$NOTARY_PROFILE" \
      --wait

# ── 8. Staple ──────────────────────────────────────────────────────────────
echo
echo "==> Stapling ticket"
xcrun stapler staple "$APP_BUNDLE"
xcrun stapler validate "$APP_BUNDLE"

echo "==> spctl post-notary (should now accept)"
spctl --assess --type execute --verbose=2 "$APP_BUNDLE"

# ── 9. Ship ────────────────────────────────────────────────────────────────
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
