@echo off
setlocal enabledelayedexpansion

echo ===================================================
echo  Building Bolt NSIS Installer
echo ===================================================

:: 1. Ensure release binary is compiled
echo [*] Building bolt release binary...
cargo build --release
if errorlevel 1 (
    echo [!] Cargo build failed!
    exit /b 1
)

:: 2. Locate makensis.exe
set "MAKENSIS="
where makensis.exe >nul 2>&1
if not errorlevel 1 (
    set "MAKENSIS=makensis.exe"
) else (
    if exist "%ProgramFiles(x86)%\NSIS\makensis.exe" (
        set "MAKENSIS=%ProgramFiles(x86)%\NSIS\makensis.exe"
    ) else if exist "%ProgramFiles%\NSIS\makensis.exe" (
        set "MAKENSIS=%ProgramFiles%\NSIS\makensis.exe"
    ) else if exist "%LOCALAPPDATA%\Programs\NSIS\makensis.exe" (
        set "MAKENSIS=%LOCALAPPDATA%\Programs\NSIS\makensis.exe"
    )
)

if "%MAKENSIS%"=="" (
    echo [!] makensis.exe not found on system PATH or Program Files.
    echo     Please install NSIS:
    echo       winget install NSIS.NSIS
    echo     or download from: https://nsis.sourceforge.io/
    exit /b 1
)

echo [*] Found makensis: %MAKENSIS%
echo [*] Compiling installer...

cd /d "%~dp0"
"%MAKENSIS%" bolt.nsi
if errorlevel 1 (
    echo [!] NSIS compilation failed!
    exit /b 1
)

echo.
echo [OK] Installer created at: target\release\bolt-installer-x64.exe
