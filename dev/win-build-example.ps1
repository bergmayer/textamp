param([string]$Example = 'audio_smoke')

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot\..

# Load VS 2022 BuildTools dev environment (vcvars64.bat sets LIB/INCLUDE/PATH).
$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
$vsPath  = & $vswhere -products * -latest -property installationPath
$devCmd  = Join-Path $vsPath 'VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path $devCmd)) { throw "vcvars64.bat not found at $devCmd" }

cmd /c "`"$devCmd`" >NUL && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') {
        [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2])
    }
}

Write-Host "=== building example $Example (release) ==="
cargo build --release --example $Example
$rc = $LASTEXITCODE
Write-Host "cargo exit code: $rc"
$exe = ".\target\release\examples\$Example.exe"
if (Test-Path $exe) {
    Write-Host "=== artifact ==="
    Get-Item $exe | Format-List FullName, Length, LastWriteTime
}
exit $rc
