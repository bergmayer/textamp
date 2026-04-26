#!/bin/bash
# Screenshot the running textamp-gui window into dev/out/.
#
# Uses the Win32 PrintWindow API with PW_RENDERFULLCONTENT, which captures
# the window's pixels even when the window is a Wayland/WSLg surface that
# standard CopyFromScreen can't read.
#
# Usage: dev/snap.sh [output_name]   # default: win.png

set -e
NAME="${1:-win.png}"
DEV_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT_DIR="$DEV_DIR/out"
mkdir -p "$OUT_DIR"

WIN_TMP="/mnt/c/Users/bergm/AppData/Local/Temp"

cat > "$WIN_TMP/tx_focus.ps1" <<'PS1'
param([string]$Match)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
}
"@
$p = Get-Process | Where-Object { $_.MainWindowTitle -like "*$Match*" } | Select-Object -First 1
if ($p) {
  [W]::ShowWindow($p.MainWindowHandle, 9) | Out-Null
  [W]::BringWindowToTop($p.MainWindowHandle) | Out-Null
  [W]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
}
PS1

cat > "$WIN_TMP/tx_pwin.ps1" <<'PS1'
param([string]$Match, [string]$OutPath)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class PW {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  public struct RECT { public int L,T,R,B; }
}
"@
$p = Get-Process | Where-Object { $_.MainWindowTitle -like "*$Match*" } | Select-Object -First 1
if (-not $p) { Write-Error "no window matching '$Match'"; exit 1 }
$r = New-Object PW+RECT
[PW]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
$w = $r.R - $r.L; $h = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$dc = $g.GetHdc()
$ok = [PW]::PrintWindow($p.MainWindowHandle, $dc, 2)  # PW_RENDERFULLCONTENT = 2
$g.ReleaseHdc($dc)
$bmp.Save($OutPath, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose(); $g.Dispose()
Write-Output "${w}x${h} ok=$ok"
PS1

PSEXE="/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"
"$PSEXE" -NoProfile -ExecutionPolicy Bypass -File 'C:\Users\bergm\AppData\Local\Temp\tx_focus.ps1' 'Textamp' >/dev/null
sleep 0.6
DIMS=$("$PSEXE" -NoProfile -ExecutionPolicy Bypass -File 'C:\Users\bergm\AppData\Local\Temp\tx_pwin.ps1' 'Textamp' 'C:\Users\bergm\AppData\Local\Temp\tx_win.png')
cp "$WIN_TMP/tx_win.png" "$OUT_DIR/$NAME"
echo "$OUT_DIR/$NAME  ($DIMS)"
