@echo off
REM Top-level Windows build entry point.
REM
REM Default (no args): build BOTH the TUI and the GUI release.
REM Pass --gui or --tui for one only.
REM
REM Usage:
REM   build.bat            # build both
REM   build.bat --tui      # textamp (TUI release) only
REM   build.bat --gui      # textamp-gui (GUI release) only
REM   build.bat --all      # alias of default
REM   build.bat --help     # show this message

setlocal
set WHAT=all
if "%~1"=="" goto run
if /I "%~1"=="--all" goto run
if /I "%~1"=="--tui" set WHAT=tui & goto run
if /I "%~1"=="--gui" set WHAT=gui & goto run
if /I "%~1"=="-h" goto help
if /I "%~1"=="--help" goto help
echo build.bat: unknown flag "%~1" 1>&2
goto help

:help
findstr /B /C:"REM" "%~f0"
exit /b 2

:run
if /I "%WHAT%"=="all" goto build_all
if /I "%WHAT%"=="tui" goto build_tui
if /I "%WHAT%"=="gui" goto build_gui
exit /b 0

:build_all
call :build_tui || exit /b %ERRORLEVEL%
call :build_gui || exit /b %ERRORLEVEL%
exit /b 0

:build_tui
echo ^>^> cargo build --release --bin textamp
cargo build --release --bin textamp
exit /b %ERRORLEVEL%

:build_gui
echo ^>^> cargo build --release --features "gui,native-menus" --bin textamp-gui
cargo build --release --features "gui,native-menus" --bin textamp-gui
exit /b %ERRORLEVEL%
