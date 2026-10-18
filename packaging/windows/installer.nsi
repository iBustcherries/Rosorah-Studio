Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"
!ifndef PAYLOAD
!error "Pass -DPAYLOAD=<prepared application folder>"
!endif
!ifndef OUTPUT
!define OUTPUT "Rosorah-Studio-0.32.0-preview.9-Setup.exe"
!endif
Name "Rosorah Studio"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Rosorah Studio"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "0.32.0.7"
VIAddVersionKey /LANG=1033 "ProductName" "Rosorah Studio"
VIAddVersionKey /LANG=1033 "FileDescription" "Rosorah Studio preview installer"
VIAddVersionKey /LANG=1033 "FileVersion" "0.32.0.7"
VIAddVersionKey /LANG=1033 "ProductVersion" "0.32.0-preview.9"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Rosorah Studio contributors"
!define MUI_ABORTWARNING
!define MUI_ICON "${PAYLOAD}/rosorah.ico"
!define MUI_UNICON "${PAYLOAD}/rosorah.ico"
!define MUI_WELCOMEPAGE_TEXT "This wizard installs Rosorah Studio preview.9 for your Windows account.$\r$\n$\r$\nGStreamer is included. Internet access is needed to download the remaining dependencies and Python for browser chat (about 140 MB). Official dependency installers may request administrator approval.$\r$\n$\r$\nClose any running Rosorah window before continuing."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_TEXT "Rosorah Studio is installed. Open it from the Start menu.$\r$\n$\r$\nThis is a preview build. Your projects and settings are stored separately from the application."
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
Section "Rosorah Studio (required)" Core
  SectionIn RO
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD}/*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "DisplayName" "Rosorah Studio"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "DisplayVersion" "0.32.0-preview.9"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "UninstallString" '"$INSTDIR\Uninstall.exe"' 
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "DisplayIcon" "$INSTDIR\rosorah.ico"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio" "NoRepair" 1
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
Section "Uninstall"
  SetShellVarContext current
  close_check:
  ClearErrors
  IfFileExists "$INSTDIR\rosorah-studio.exe" 0 closed
  Delete "$INSTDIR\rosorah-studio.exe"
  IfErrors 0 closed
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Close Rosorah Studio before uninstalling, then choose Retry." IDRETRY close_check
  Abort
  closed:
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
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RosorahStudio"
SectionEnd
