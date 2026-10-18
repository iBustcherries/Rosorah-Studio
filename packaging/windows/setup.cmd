@echo off
setlocal
cd /d "%~dp0"
echo Rosorah Windows preview - media dependency setup
echo Installs Microsoft Visual C++ Runtime, GStreamer 1.26.10, FFmpeg 9.0.2 and Python 3.13 if they are missing.
echo The official installers may request administrator approval.
echo This does not change your graphics drivers.
if /I not "%~1"=="/installer" pause
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup.ps1"
if errorlevel 1 goto failed
echo Setup finished. Start Rosorah from the Start menu.
if /I not "%~1"=="/installer" pause
exit /b 0
:failed
echo Setup did not finish. Please save the error above for troubleshooting.
if /I not "%~1"=="/installer" pause
exit /b 1
