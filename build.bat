@echo off
setlocal

rem ============================================================
rem  build.bat - release build + versioned exe
rem
rem    Usage: double-click, or run  build.bat  from a terminal.
rem
rem    Output:
rem      target\release\my-excel.exe            (default name, cargo-managed; cargo run --release works)
rem      target\release\my-excel-<version>.exe  (versioned copy for distribution; version from Cargo.toml [package])
rem
rem    Notes:
rem      - Uses pushd (not cd /d) because %~dp0 ends with a backslash that silently breaks `cd /d`.
rem      - Keep this file ASCII-only: cmd.exe reads .bat under the OEM codepage, so non-ASCII
rem        comments get corrupted on non-UTF8 consoles.
rem ============================================================

pushd "%~dp0"

rem 1) parse [package].version from Cargo.toml (first line starting with "version")
set "raw="
for /f "tokens=2 delims==" %%a in ('findstr /b /i /c:"version" Cargo.toml') do (
    if not defined raw set "raw=%%a"
)
if not defined raw (
    echo [build.bat] ERROR: cannot parse version from Cargo.toml
    popd & exit /b 1
)
rem strip quotes and spaces
set "version=%raw:"=%"
set "version=%version: =%"
echo [build.bat] version = %version%

rem 2) release build
echo [build.bat] cargo build --release
cargo build --release
if errorlevel 1 ( echo [build.bat] ERROR: build failed & popd & exit /b 1 )

rem 3) copy versioned artifact (default name is kept so cargo run --release still works)
if not exist "target\release\my-excel.exe" (
    echo [build.bat] ERROR: not found target\release\my-excel.exe
    popd & exit /b 1
)
copy /Y "target\release\my-excel.exe" "target\release\my-excel-%version%.exe"
if errorlevel 1 ( echo [build.bat] ERROR: copy failed & popd & exit /b 1 )

echo.
echo [build.bat] default:   target\release\my-excel.exe
echo [build.bat] versioned: target\release\my-excel-%version%.exe

popd
endlocal
