@echo off
REM Build script for the textamp TUI binary on Windows.
REM Usage: build-tui.bat [--makepackage | --clean | --help]

setlocal enabledelayedexpansion

set "PKG_NAME=textamp"
set "MODE=build"

:parse
if "%~1"=="" goto :parsed
if /I "%~1"=="--help" goto :help
if /I "%~1"=="-h" goto :help
if /I "%~1"=="--clean" ( set "MODE=clean" & shift & goto :parse )
if /I "%~1"=="--makepackage" ( set "MODE=package" & shift & goto :parse )
echo Unknown option: %~1
goto :help

:parsed

if "%MODE%"=="clean" (
    echo Cleaning build artifacts...
    cargo clean
    if exist dist rmdir /S /Q dist
    echo Clean complete.
    exit /b 0
)

echo Building %PKG_NAME% TUI (release)...
cargo build --release --no-default-features --features tui --bin textamp
if errorlevel 1 exit /b 1

echo.
echo Build complete!
if exist "target\release\%PKG_NAME%.exe" echo   TUI: %CD%\target\release\%PKG_NAME%.exe

if "%MODE%"=="package" (
    echo.
    echo Packaging on Windows: please ship the .exe directly,
    echo or install cargo-wix for MSI generation.
)

exit /b 0

:help
echo Usage: build-tui.bat [OPTIONS]
echo.
echo Builds the textamp TUI binary (no GUI).
echo.
echo Options:
echo   --makepackage       Placeholder (use cargo-wix or zip the binary)
echo   --clean             Remove build artifacts
echo   --help              Show this help message
exit /b 0
