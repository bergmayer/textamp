#!/bin/bash
# Top-level Linux/macOS/WSL build entry point.
#
# Default (no args): build BOTH the TUI and the GUI for the current
# platform. Pass --gui or --tui to build just one.
#
# Usage:
#   ./build.sh           # build both
#   ./build.sh --tui     # textamp (TUI release) only
#   ./build.sh --gui     # textamp-gui (GUI release) only
#   ./build.sh --all     # alias of default
#   ./build.sh --windows # cross-build the Windows GUI from WSL
#                        # (sync to /mnt/c staging tree + run MSVC)
#   ./build.sh --help    # show this message
#
# On Windows, run build.bat instead (or call this script under WSL
# with --windows to build the Windows GUI).

set -euo pipefail

usage() {
    sed -n '2,18p' "$0"
}

WHAT="all"
case "${1:-}" in
    ""|--all)       WHAT="all" ;;
    --tui)          WHAT="tui" ;;
    --gui)          WHAT="gui" ;;
    --windows)      WHAT="windows" ;;
    -h|--help)      usage; exit 0 ;;
    *)
        echo "build.sh: unknown flag '$1'" >&2
        usage >&2
        exit 2
        ;;
esac

build_tui() {
    echo ">> cargo build --release --bin textamp"
    cargo build --release --bin textamp
    echo "   artifact: target/release/textamp"
}

build_gui() {
    # GUI requires the `gui` feature. On Linux we also enable
    # `native-menus` (muda needs GTK3). On macOS native menus work
    # without GTK; Windows uses the cross-build path below.
    local features="gui"
    case "$(uname -s)" in
        Linux*|Darwin*) features="gui,native-menus" ;;
    esac
    echo ">> cargo build --release --features \"$features\" --bin textamp-gui"
    cargo build --release --features "$features" --bin textamp-gui
    echo "   artifact: target/release/textamp-gui"
}

build_windows_gui() {
    # Cross-build path: rsync the source tree into a Windows-native
    # staging dir under /mnt/c, then invoke the existing dev/win-build
    # PowerShell helper so cargo runs against a real Windows path.
    if [ ! -d /mnt/c ]; then
        echo "build.sh --windows requires WSL with /mnt/c mounted" >&2
        exit 1
    fi
    bash dev/win-build.sh
}

case "$WHAT" in
    all)     build_tui; build_gui ;;
    tui)     build_tui ;;
    gui)     build_gui ;;
    windows) build_windows_gui ;;
esac
