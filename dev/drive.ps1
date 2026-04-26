param(
    [Parameter(Mandatory=$true)] [string]$Action,   # 'key' | 'click' | 'focus'
    [string]$Title = 'Textamp',
    [string]$Keys  = '',       # for -Action key (SendKeys syntax)
    [int]$X = 0,               # for -Action click (relative to window)
    [int]$Y = 0
)

Add-Type -AssemblyName System.Windows.Forms

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win32Drive {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, int cButtons, int dwExtraInfo);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public const uint MOUSEEVENTF_LEFTDOWN  = 0x0002;
    public const uint MOUSEEVENTF_LEFTUP    = 0x0004;
    public const uint MOUSEEVENTF_RIGHTDOWN = 0x0008;
    public const uint MOUSEEVENTF_RIGHTUP   = 0x0010;
}
'@

$proc = Get-Process | Where-Object { $_.MainWindowTitle -like "*$Title*" -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Error "No window matching '*$Title*' found."; exit 1 }
$hwnd = $proc.MainWindowHandle
[Win32Drive]::ShowWindow($hwnd, 9) | Out-Null  # SW_RESTORE
[Win32Drive]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds 120

switch ($Action) {
    'focus' {
        Write-Host "focused window '$($proc.MainWindowTitle)' (pid=$($proc.Id))"
    }
    'key' {
        [System.Windows.Forms.SendKeys]::SendWait($Keys)
        Write-Host "sent keys: $Keys"
    }
    'click' {
        $rect = New-Object Win32Drive+RECT
        [Win32Drive]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
        $px = $rect.Left + $X
        $py = $rect.Top + $Y
        [Win32Drive]::SetCursorPos($px, $py) | Out-Null
        Start-Sleep -Milliseconds 60
        [Win32Drive]::mouse_event([Win32Drive]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
        [Win32Drive]::mouse_event([Win32Drive]::MOUSEEVENTF_LEFTUP,   0, 0, 0, 0)
        Write-Host "clicked at (${px}, ${py})"
    }
    'rclick' {
        $rect = New-Object Win32Drive+RECT
        [Win32Drive]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
        $px = $rect.Left + $X
        $py = $rect.Top + $Y
        [Win32Drive]::SetCursorPos($px, $py) | Out-Null
        Start-Sleep -Milliseconds 60
        [Win32Drive]::mouse_event([Win32Drive]::MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, 0)
        [Win32Drive]::mouse_event([Win32Drive]::MOUSEEVENTF_RIGHTUP,   0, 0, 0, 0)
        Write-Host "right-clicked at (${px}, ${py})"
    }
    default { Write-Error "unknown action: $Action" }
}
