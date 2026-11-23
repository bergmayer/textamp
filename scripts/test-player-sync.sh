#!/bin/bash
# Uses a disposable disk image and public loopback credentials, never a music server.
set -euo pipefail
cd "$(dirname "$0")/.."
if pgrep -x cargo >/dev/null; then
    echo "A Cargo build is running; wait for it to finish." >&2
    exit 1
fi
player_fixture=$(mktemp -d)
player_mount="/Volumes/TextampPlayerValidation-$(uuidgen)"
player_server_pid=""
cleanup() {
    if [[ -n "$player_server_pid" ]]; then kill "$player_server_pid" 2>/dev/null || true; fi
    if [[ -d "$player_mount" ]]; then
        hdiutil detach "$player_mount" >/dev/null 2>&1 || hdiutil detach -force "$player_mount" >/dev/null 2>&1 || { echo "Could not detach test image; retained $player_fixture" >&2; return 1; }
    fi
    rm -rf "$player_fixture"
}
trap cleanup EXIT
hdiutil create -size 64m -fs "MS-DOS FAT32" -layout MBRSPUD -volname TEXTAMPTEST -type UDIF -nospotlight "$player_fixture/player.dmg" >/dev/null
hdiutil attach -nobrowse -mountpoint "$player_mount" "$player_fixture/player.dmg" >/dev/null
python3 tests/fixtures/desktop_ui_server.py --catalog --host 127.0.0.1 --port 0 --ready-file "$player_fixture/ready" >"$player_fixture/server.log" 2>&1 &
player_server_pid=$!
for attempt in {1..100}; do
    [[ -s "$player_fixture/ready" ]] && break
    kill -0 "$player_server_pid"
    sleep 0.1
done
[[ -s "$player_fixture/ready" ]]
TEXTAMP_PLAYER_TEST_VOLUME="$player_mount" \
TEXTAMP_PLAYER_TEST_SERVER="$(cat "$player_fixture/ready")" \
cargo test --features desktop-tests,mobile --test player_sync_mounted mounted_fat32_player_sync_roundtrip -- --ignored --nocapture
