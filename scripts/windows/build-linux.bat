@echo off
rem Builds the Linux version of Audio Graph from Windows, in WSL, into release\<version>\
rem (AppImage, deb, rpm). It builds on Ubuntu 22.04 so the packages run on Ubuntu 22.04+,
rem Debian 12+, Fedora and other distributions of that age or newer.
rem The first run creates a WSL distribution of its own, "audio-graph-build", from the official
rem Ubuntu image (about 330 MB download) and installs the build tools into it. It doesn't touch
rem your other WSL distributions. Remove it with: wsl --unregister audio-graph-build
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
setlocal
cd /d "%~dp0..\.."
set "DISTRO=audio-graph-build"
set "IMAGE=ubuntu-jammy-wsl-amd64-ubuntu22.04lts.rootfs.tar.gz"
set "IMAGE_URL=https://cloud-images.ubuntu.com/wsl/jammy/current"

where wsl >nul 2>nul || (
    echo WSL is not installed. Install it with "wsl --install" and run this again.
    goto :error
)

rem wsl.exe fails with -1, which "if errorlevel 1" doesn't catch
wsl -d %DISTRO% -u root -- true >nul 2>nul
if %errorlevel% equ 0 goto :build

echo Creating the WSL distribution %DISTRO% (Ubuntu 22.04)...
set "DOWNLOAD=%TEMP%\audio-graph-build"
if not exist "%DOWNLOAD%" mkdir "%DOWNLOAD%"
curl.exe -fL -o "%DOWNLOAD%\%IMAGE%" "%IMAGE_URL%/%IMAGE%" || goto :error
curl.exe -fsSL -o "%DOWNLOAD%\SHA256SUMS" "%IMAGE_URL%/SHA256SUMS" || goto :error
powershell -NoProfile -Command "$sum = (Select-String -Path '%DOWNLOAD%\SHA256SUMS' -Pattern '%IMAGE%' -SimpleMatch).Line.Split(' ')[0]; if ((Get-FileHash '%DOWNLOAD%\%IMAGE%' -Algorithm SHA256).Hash -ne $sum) { Write-Host 'The downloaded image is damaged (checksum mismatch)'; exit 1 }" || goto :error
wsl --import %DISTRO% "%LOCALAPPDATA%\%DISTRO%" "%DOWNLOAD%\%IMAGE%" --version 2 || goto :error
rmdir /s /q "%DOWNLOAD%"

:build
wsl -d %DISTRO% -u root --cd "%CD%" -- bash scripts/windows/build-linux-wsl.sh || goto :error
call :pause_if_double_clicked
exit /b 0

:error
echo.
echo Linux build failed, see the output above.
pause
exit /b 1

:pause_if_double_clicked
echo %cmdcmdline% | "%SystemRoot%\System32\find.exe" /i "%~nx0" >nul && pause
exit /b 0
