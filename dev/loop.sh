#!/bin/bash
# Dev loop for textamp-gui on WSLg.
#   dev/loop.sh             # rebuild, relaunch, wait, screenshot
#   dev/loop.sh --no-build  # just relaunch + screenshot
#   dev/loop.sh --only-snap # screenshot whatever's running
# Output screenshot: dev/out/win.png

set -e
cd "$(dirname "$0")/.."

DO_BUILD=1
DO_LAUNCH=1
WAIT_SECS=5

for arg in "$@"; do
    case "$arg" in
        --no-build)  DO_BUILD=0 ;;
        --only-snap) DO_BUILD=0; DO_LAUNCH=0 ;;
        --wait=*)    WAIT_SECS="${arg#--wait=}" ;;
        -h|--help)
            grep '^#' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
    esac
done

if [ $DO_BUILD -eq 1 ]; then
    echo ">> cargo build --release --no-default-features --features tui,gui"
    cargo build --release --no-default-features --features tui,gui
fi

if [ $DO_LAUNCH -eq 1 ]; then
    # NOTE: use `-x <basename>` not `-f <path>` — `pkill -f` matches against
    # the full command line of every process, including our own shell if the
    # path appears in its args, and killing our own shell aborts the script.
    pkill -x textamp-gui 2>/dev/null || true
    sleep 0.5
    rm -f "${XDG_STATE_HOME:-$HOME/.local/state}/textamp/textamp.lock"
    echo ">> launching textamp-gui (WSLg: ICED_BACKEND=tiny-skia, X11 winit path)"
    env -u WAYLAND_DISPLAY ICED_BACKEND=tiny-skia \
        ./target/release/textamp-gui > dev/out/run.log 2>&1 < /dev/null &
    disown
    echo "PID=$!"
    sleep "$WAIT_SECS"
    if ! pgrep -x textamp-gui >/dev/null; then
        echo "!! textamp-gui exited immediately; see dev/out/run.log"
        tail -20 dev/out/run.log
        exit 1
    fi
fi

./dev/snap.sh
