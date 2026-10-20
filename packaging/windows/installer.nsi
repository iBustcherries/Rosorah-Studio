Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!ifndef PAYLOAD
!error "Pass -DPAYLOAD=<prepared application folder>"
!endif
!ifndef VERSION
!error "Pass -DVERSION=<version from Cargo.toml>"
!endif
!ifndef OUTPUT
!define OUTPUT "Rosorah-Studio-${VERSION}-Setup.exe"
!endif
Name "Rosorah Studio"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Rosorah Studio"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "Rosorah Studio"
VIAddVersionKey /LANG=1033 "FileDescription" "Rosorah Studio installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Rosorah Studio contributors"
!define MUI_ABORTWARNING
!define MUI_ICON "${PAYLOAD}/rosorah.ico"
!define MUI_UNICON "${PAYLOAD}/rosorah.ico"
!define MUI_WELCOMEPAGE_TEXT "This wizard installs Rosorah Studio ${VERSION} for your Windows account.$\r$\n$\r$\nGStreamer is included. Internet access is needed to download the remaining dependencies and Python for browser chat (about 140 MB). Official dependency installers may request administrator approval.$\r$\n$\r$\nClose any running Rosorah window before continuing."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_TEXT "Rosorah Studio is installed. Open it any time from the Start menu.$\r$\n$\r$\nYour projects and settings are stored separately from the application, so updating or uninstalling keeps them."
!define MUI_FINISHPAGE_RUN "$INSTDIR\Rosorah.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Rosorah Studio"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "This build requires 64-bit Windows."
    Abort
  ${EndIf}
  SetShellVarContext current
FunctionEnd
; Updating while Rosorah runs would fail on its locked files, so ask for it to be closed first.
!macro WaitForClosed
  ${Do}
    ClearErrors
    ${IfNot} ${FileExists} "$INSTDIR\rosorah-studio.exe"
      ${Break}
    ${EndIf}
    Delete "$INSTDIR\rosorah-studio.exe"
    ${IfNot} ${Errors}
      ${Break}
    ${EndIf}
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Close Rosorah Studio, then choose Retry." IDRETRY +2
    Abort
  ${Loop}
!macroend
Section "Rosorah Studio (required)" Core
  SectionIn RO
  !insertmacro WaitForClosed
  ; Files earlier versions shipped and this one doesn't.
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\run.cmd"
  Delete "$INSTDIR\release-0.31-preview6.md"
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD}/*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "DisplayName" "Rosorah Studio"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "Publisher" "Rosorah Studio"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "URLInfoAbout" "https://github.com/iBustcherries/Rosorah-Studio"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "UninstallString" '"$INSTDIR\Uninstall.exe"' 
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "DisplayIcon" "$INSTDIR\rosorah.ico"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "EstimatedSize" $0
  DetailPrint "Installing media dependencies. This may take several minutes."
  nsExec::Exec '"$SYSDIR\cmd.exe" /D /S /C ""$INSTDIR\setup.cmd" /installer > "$INSTDIR\setup.log" 2>&1"'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "Media setup did not finish (code $0). Check $INSTDIR\setup.log, then run this installer again to retry. The partial installation can be removed from Windows Settings > Apps."
    SetErrorLevel 1
    Abort
  ${EndIf}
  CreateDirectory "$SMPROGRAMS\Rosorah Studio"
  CreateShortCut "$SMPROGRAMS\Rosorah Studio\Rosorah Studio.lnk" "$INSTDIR\Rosorah.exe" "" "$INSTDIR\rosorah.ico"
  CreateShortCut "$SMPROGRAMS\Rosorah Studio\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
SectionEnd
Section /o "Desktop shortcut" Desktop
  CreateShortCut "$DESKTOP\Rosorah Studio.lnk" "$INSTDIR\Rosorah.exe" "" "$INSTDIR\rosorah.ico"
SectionEnd
Section "-Refresh icons"
  ; Windows caches shortcut icons by file path, so a new logo at the same path would not show.
  ; Tell the shell its icons changed (SHCNE_ASSOCCHANGED, flushed) and rebuild the icon cache.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x1000, p 0, p 0)'
  nsExec::Exec '"$SYSDIR\ie4uinit.exe" -show'
  Pop $0
SectionEnd
Section "Uninstall"
  SetShellVarContext current
  !insertmacro WaitForClosed
  Delete "$DESKTOP\Rosorah Studio.lnk"
  Delete "$SMPROGRAMS\Rosorah Studio\Rosorah Studio.lnk"
  Delete "$SMPROGRAMS\Rosorah Studio\Uninstall.lnk"
  RMDir "$SMPROGRAMS\Rosorah Studio"
  !include "uninstall-files.nsh"
  ; Only installer-owned media files are removed. Never recursively delete the
  ; application root, project folders or account settings.
  Delete "$INSTDIR\runtime\vc_redist.x64.exe"
  Delete "$INSTDIR\runtime\gstreamer-1.26.10.msi"
  Delete "$INSTDIR\runtime\ffmpeg-9.0.2.zip"
  Delete "$INSTDIR\runtime\python-3.13.16-amd64.exe"
  RMDir /r "$INSTDIR\runtime\ffmpeg-9.0.2-essentials_build"
  RMDir "$INSTDIR\runtime"
  Delete "$INSTDIR\setup.log"
  ; The virtual camera, if it was set up, lives in Program Files; removing it needs permission.
  SetRegView 64
  ReadRegStr $0 HKLM "SOFTWARE\Classes\CLSID\{b6ac0fba-5752-439c-ad7b-9bbc8d093265}\InprocServer32" ""
  SetRegView lastused
  ${If} $0 != ""
    ExecShellWait "runas" "$SYSDIR\cmd.exe" '/s /c "reg delete "HKLM\SOFTWARE\Classes\CLSID\{b6ac0fba-5752-439c-ad7b-9bbc8d093265}" /f /reg:64 & del /f /q "$0" & rmdir "$PROGRAMFILES64\Rosorah Studio Camera""' SW_HIDE
  ${EndIf}
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio"
SectionEnd
