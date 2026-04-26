@echo off
REM Build script for the textamp GUI binary on Windows.
REM Usage: build-gui.bat [--makepackage | --clean | --no-native-menus | --help]
REM
REM The Windows GUI binary links as the `windows` subsystem so launching
REM it from Explorer does not flash a console window.

setlocal enabledelayedexpansion

set "PKG_NAME=textamp"
set "NATIVE_MENUS=auto"
set "MODE=build"

:parse
if "%~1"=="" goto :parsed
if /I "%~1"=="--help" goto :help
if /I "%~1"=="-h" goto :help
if /I "%~1"=="--clean" ( set "MODE=clean" & shift & goto :parse )
if /I "%~1"=="--makepackage" ( set "MODE=package" & shift & goto :parse )
if /I "%~1"=="--native-menus" ( set "NATIVE_MENUS=yes" & shift & goto :parse )
if /I "%~1"=="--no-native-menus" ( set "NATIVE_MENUS=no" & shift & goto :parse )
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

set "FEATS=gui"
if "%NATIVE_MENUS%"=="yes" set "FEATS=!FEATS!,native-menus"
if "%NATIVE_MENUS%"=="auto" set "FEATS=!FEATS!,native-menus"

echo Building %PKG_NAME% GUI (release, features: %FEATS%)...
cargo build --release --no-default-features --features %FEATS% --bin textamp-gui
if errorlevel 1 exit /b 1

echo.
echo Build complete!
if exist "target\release\%PKG_NAME%-gui.exe" echo   GUI: %CD%\target\release\%PKG_NAME%-gui.exe

if "%MODE%"=="package" (
    echo.
    echo Packaging on Windows: please ship the .exe directly,
    echo or install cargo-wix for MSI generation.
)

exit /b 0

:help
echo Usage: build-gui.bat [OPTIONS]
echo.
echo Builds the textamp GUI binary (Iced desktop app).
echo.
echo Options:
echo   --makepackage       Placeholder (use cargo-wix or zip the binary)
echo   --no-native-menus   Build without muda
echo   --native-menus      Force muda native menus on
echo   --clean             Remove build artifacts
echo   --help              Show this help message
exit /b 0
