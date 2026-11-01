@echo off
rem Builds Audio Graph for Windows into release\<version>\:
rem   Audio Graph <version> portable.exe  - a single file, no installation
rem   Audio Graph <version> setup.msi     - the installer
rem The version comes from package.json; change it with "npm version patch" (or minor, major).
rem App data is always in %APPDATA%\dev.audiograph.app, never next to the exe.
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
setlocal
cd /d "%~dp0..\.."

for /f "usebackq delims=" %%v in (`node -p "require('./package.json').version"`) do set "VERSION=%%v"
if "%VERSION%"=="" goto :error
set "OUT=release\%VERSION%"
set "EXE=%OUT%\Audio Graph %VERSION% portable.exe"
set "MSI=%OUT%\Audio Graph %VERSION% setup.msi"

rem one version, one build: a changed app gets a new version number
if exist "%EXE%" (
    echo Version %VERSION% is already built: %EXE%
    echo Change the version first: npm version patch
    goto :error
)

call npm install --no-audit --no-fund || goto :error
call npm run tauri build -- --bundles msi || goto :error

if not exist "%OUT%" mkdir "%OUT%"
copy /y "src-tauri\target\release\audio-graph.exe" "%EXE%" >nul || goto :error
copy /y "src-tauri\target\release\bundle\msi\Audio Graph_%VERSION%_x64_en-US.msi" "%MSI%" >nul || goto :error

echo.
echo Done:
echo   %EXE%
echo   %MSI%
call :pause_if_double_clicked
exit /b 0

:error
echo.
echo Build failed, see the output above.
pause
exit /b 1

:pause_if_double_clicked
echo %cmdcmdline% | "%SystemRoot%\System32\find.exe" /i "%~nx0" >nul && pause
exit /b 0
