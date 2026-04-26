#!/bin/bash
# Sync WSL source → Windows staging dir, then run the MSVC release build.
#
# Usage: dev/win-build.sh [cargo args]
#
# Single source of truth: /home/bergmayer/software_projects/textamp/ (WSL).
# The Windows copy is a transient build staging directory — never edited
# directly. target/ is preserved between runs so incremental builds work.

set -euo pipefail

WSL_SRC="$(cd "$(dirname "$0")/.." && pwd)"
WIN_DIR="/mnt/c/Users/bergm/textamp-build"

if [ ! -d "$WIN_DIR" ]; then
    mkdir -p "$WIN_DIR"
fi

echo ">> syncing $WSL_SRC -> $WIN_DIR (target/ and worktrees excluded)"
rsync -a --delete \
    --exclude 'target/' \
    --exclude '.claude/worktrees/' \
    "$WSL_SRC/" "$WIN_DIR/"

echo ">> invoking dev/win-build.ps1 via powershell.exe"
# The PowerShell script sets up VS 2022 vcvars and runs cargo.
# It lives in dev/ inside the WSL repo so rsync keeps it in sync with
# the Windows staging tree.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File 'C:\Users\bergm\textamp-build\dev\win-build.ps1'
