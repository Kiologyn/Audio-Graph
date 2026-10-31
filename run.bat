@echo off
rem Audio Graph in development mode: UI and Rust changes are picked up while it runs.
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
cd /d "%~dp0"

if not exist node_modules (
    echo Installing dependencies...
    call npm install || goto :error
)

call npm run tauri dev || goto :error
exit /b 0

:error
echo.
echo Audio Graph failed to start, see the output above.
pause
exit /b 1
