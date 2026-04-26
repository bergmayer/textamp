$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot\..

# Load VS 2022 BuildTools dev environment (vcvars64.bat sets LIB/INCLUDE/PATH
# for the MSVC toolchain used by cargo).
$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
$vsPath  = & $vswhere -products * -latest -property installationPath
$devCmd  = Join-Path $vsPath 'VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path $devCmd)) { throw "vcvars64.bat not found at $devCmd" }

cmd /c "`"$devCmd`" >/dev/null && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') {
        [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2])
    }
}

Write-Host '=== cargo/rustc ==='
cargo --version
rustc --version

Write-Host '=== building textamp-gui (release, features=gui,native-menus) ==='
cargo build --release --features "gui,native-menus" --bin textamp-gui
$rc = $LASTEXITCODE
Write-Host "cargo exit code: $rc"
if (Test-Path '.\target\release\textamp-gui.exe') {
    Write-Host '=== artifact ==='
    Get-Item '.\target\release\textamp-gui.exe' | Format-List FullName, Length, LastWriteTime
}
exit $rc
