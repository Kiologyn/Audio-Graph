@echo off
rem Builds the portable version: a single portable\Audio Graph <version>.exe, no installer.
rem The version comes from src-tauri\tauri.conf.json. Data stays in %APPDATA%\dev.audiograph.app.
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
cd /d "%~dp0"

for /f "usebackq delims=" %%v in (`powershell -NoProfile -Command "(Get-Content 'src-tauri\tauri.conf.json' -Raw | ConvertFrom-Json).version"`) do set VERSION=%%v
if "%VERSION%"=="" goto :error
set "OUT=portable\Audio Graph %VERSION%.exe"

if not exist node_modules (
    echo Installing dependencies...
    call npm install || goto :error
)

call npm run tauri build -- --no-bundle || goto :error

if not exist portable mkdir portable
copy /y "src-tauri\target\release\audio-graph.exe" "%OUT%" >nul || goto :error

echo.
echo Done: %OUT%
exit /b 0

:error
echo.
echo Build failed, see the output above.
pause
exit /b 1
