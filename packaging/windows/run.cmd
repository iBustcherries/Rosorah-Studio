@echo off
setlocal
cd /d "%~dp0"
if not defined GSTREAMER_1_0_ROOT_MSVC_X86_64 set "GSTREAMER_1_0_ROOT_MSVC_X86_64=C:\gstreamer\1.0\msvc_x86_64"
if not exist "%GSTREAMER_1_0_ROOT_MSVC_X86_64%\bin\gstreamer-1.0-0.dll" (
  echo Run setup.cmd once before starting this Windows preview.
  pause
  exit /b 1
)
set "PATH=%~dp0runtime\ffmpeg-9.0.2-essentials_build\bin;%GSTREAMER_1_0_ROOT_MSVC_X86_64%\bin;%PATH%"
set "GST_PLUGIN_PATH=%GSTREAMER_1_0_ROOT_MSVC_X86_64%\lib\gstreamer-1.0"
"%~dp0rosorah-studio.exe" %*
if errorlevel 1 pause
