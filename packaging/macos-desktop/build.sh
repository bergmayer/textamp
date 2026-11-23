#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
mode=""
optimized=false
for argument in "$@"; do
    case "$argument" in
        --preview) mode="--preview" ;;
        --release) optimized=true ;;
        --help|-h)
            echo "Usage: $0 [--preview] [--release]"
            echo "Preview uses the incremental development build; --release requests optimization."
            exit 0 ;;
        *) echo "Unknown option: $argument" >&2; exit 2 ;;
    esac
done
if [[ "$mode" == "--preview" && "$optimized" == false ]]; then
    cargo build --features desktop-tests --bin textamp
    executable="target/debug/textamp"
else
    cargo build --release --features desktop --bin textamp
    executable="target/release/textamp"
fi
bundle="target/desktop/Textamp.app"
if [[ "$mode" == "--preview" ]]; then
    bundle="target/desktop/Textamp Preview.app"
fi
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
# Finder's runtime custom icon is not a bundled resource.
rm -f "$bundle/"$'Icon\r'
# A new inode avoids macOS retaining the replaced executable's signing cache.
staged_binary=$(mktemp "$bundle/Contents/MacOS/.textamp.XXXXXX")
trap 'rm -f "$staged_binary"' EXIT
cp -p "$executable" "$staged_binary"
mv -f "$staged_binary" "$bundle/Contents/MacOS/textamp"
cp packaging/macos-desktop/assets/Textamp.icns "$bundle/Contents/Resources/Textamp.icns"
mkdir -p "$bundle/Contents/Resources/licenses/ghostty-vt"
cp vendor/ghostty-vt/LICENSE vendor/ghostty-vt/SIMDUTF-LICENSE-* "$bundle/Contents/Resources/licenses/ghostty-vt/"
mkdir -p "$bundle/Contents/Resources/licenses/gpui-component"
cp vendor/gpui-component/LICENSE-APACHE "$bundle/Contents/Resources/licenses/gpui-component/"
cp assets/artwork/LICENSE-LUCIDE "$bundle/Contents/Resources/licenses/LICENSE-LUCIDE"
rm -f "$bundle/Contents/MacOS/textamp-gui"
cp packaging/macos-desktop/Info.plist "$bundle/Contents/Info.plist"
python3 - "$bundle/Contents/Info.plist" "$mode" <<'PY'
import os
import pathlib
import plistlib
import re
import sys
path = pathlib.Path(sys.argv[1])
plist = plistlib.loads(path.read_bytes())
plist['CFBundleShortVersionString'] = re.search(r'(?m)^version = "([^"]+)"', pathlib.Path('Cargo.toml').read_text()).group(1)
if sys.argv[2] == '--preview':
    plist['CFBundleIdentifier'] = 'org.textamp.desktop.preview'
    plist['CFBundleName'] = plist['CFBundleDisplayName'] = 'Textamp Preview'
    profile = pathlib.Path.home() / 'Library/Application Support/textamp-desktop-preview'
    profile.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(profile, 0o700)
    cache = pathlib.Path.home() / 'Library/Caches/textamp-desktop-preview'
    paths = dict(XDG_CONFIG_HOME=profile / 'config', XDG_DATA_HOME=profile / 'data',
                 XDG_STATE_HOME=profile / 'state', XDG_CACHE_HOME=cache)
    for directory in paths.values():
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(directory, 0o700)
    plist['LSEnvironment'] = {key: str(value) for key, value in paths.items()}
path.write_bytes(plistlib.dumps(plist))
PY
# A certificate-backed identity keeps macOS privacy grants valid across builds.
signing_identity="${TEXTAMP_CODESIGN_IDENTITY:-}"
if [[ -z "$signing_identity" ]]; then
    signing_identity=$(security find-identity -v -p codesigning | awk '/"Apple Development:/ {print $2; exit}')
fi
if [[ -n "$signing_identity" ]]; then
    xattr -dr com.apple.FinderInfo "$bundle" 2>/dev/null || true
    xattr -dr com.apple.ResourceFork "$bundle" 2>/dev/null || true
    codesign --force --sign "$signing_identity" --timestamp=none "$bundle"
fi
touch "$bundle"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$bundle"
printf '%s\n' "$bundle"
