@echo off
rem Installs what building Audio Graph on Windows needs, skipping what is already there:
rem Node.js, Rust, the Microsoft C++ Build Tools and WebView2, then the npm packages.
rem Uses winget, which comes with Windows 10 and 11 (the "App Installer" from the Store).
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
setlocal
cd /d "%~dp0..\.."
set "RESTART="

where winget >nul 2>nul || (
    echo winget is not available. Install "App Installer" from the Microsoft Store and run this again.
    goto :error
)

rem Node.js 22 or later: the interface build (Vite) needs it
node -e "process.exit(+process.versions.node.split('.')[0] >= 22 ? 0 : 1)" >nul 2>nul
if errorlevel 1 (
    echo Installing Node.js...
    winget install -e --id OpenJS.NodeJS.LTS --accept-package-agreements --accept-source-agreements || goto :error
    set "RESTART=1"
) else (
    echo Node.js: found
)

rem Rust through rustup, with the default MSVC toolchain
where cargo >nul 2>nul
if errorlevel 1 if not exist "%USERPROFILE%\.cargo\bin\cargo.exe" (
    echo Installing Rust...
    winget install -e --id Rustlang.Rustup --accept-package-agreements --accept-source-agreements || goto :error
    set "RESTART=1"
    goto :rust_done
)
echo Rust: found
:rust_done

rem The C++ compiler and Windows SDK that Rust uses on Windows
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VCTOOLS="
if exist "%VSWHERE%" (
    for /f "usebackq delims=" %%p in (`"%VSWHERE%" -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VCTOOLS=%%p"
)
if defined VCTOOLS (
    echo C++ Build Tools: found
) else (
    echo Installing the Microsoft C++ Build Tools, this takes a while...
    winget install -e --id Microsoft.VisualStudio.2022.BuildTools --accept-package-agreements --accept-source-agreements --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" || goto :error
)

rem WebView2 draws the window; Windows 11 and up-to-date Windows 10 already have it
reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv >nul 2>nul || reg query "HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv >nul 2>nul
if errorlevel 1 (
    echo Installing WebView2...
    winget install -e --id Microsoft.EdgeWebView2Runtime --accept-package-agreements --accept-source-agreements || goto :error
) else (
    echo WebView2: found
)

if defined RESTART (
    echo.
    echo Node.js or Rust was just installed: open a new terminal and run setup.bat again to finish.
    goto :done
)

echo Installing npm packages...
call npm install || goto :error

echo.
echo Ready. Run scripts\windows\dev.bat to start the app, scripts\windows\build.bat to build it.
:done
call :pause_if_double_clicked
exit /b 0

:error
echo.
echo Setup failed, see the output above.
pause
exit /b 1

:pause_if_double_clicked
echo %cmdcmdline% | "%SystemRoot%\System32\find.exe" /i "%~nx0" >nul && pause
exit /b 0
