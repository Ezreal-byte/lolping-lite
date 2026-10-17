Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
Name "LOLPing Lite"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\LOLPing Lite"
InstallDirRegKey HKCU "Software\LOLPing Lite" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 32
BrandingText "LOLPing Lite ${VERSION}"
Icon "${ICON}"
UninstallIcon "${ICON}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "LOLPing Lite"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "LOLPing Lite per-user setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT; third-party artwork has separate licensing"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\lolping-lite.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${LICENSE_FILE}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

Section "LOLPing Lite" SEC_MAIN
  SetShellVarContext current
  IfFileExists "$INSTDIR\lolping-lite.exe" 0 install_ready
  ExecWait '"$INSTDIR\lolping-lite.exe" --quit' $0
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "Please exit LOLPing Lite before updating. / 请先退出 LOLPing Lite 再更新。" /SD IDOK
    Abort
  ${EndIf}
  install_ready:
  SetOutPath "$INSTDIR"
  File /r "${STAGE}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\LOLPing Lite"
  CreateShortcut "$SMPROGRAMS\LOLPing Lite\LOLPing Lite.lnk" "$INSTDIR\lolping-lite.exe"
  CreateShortcut "$SMPROGRAMS\LOLPing Lite\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\LOLPing Lite" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "DisplayName" "LOLPing Lite"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "Publisher" "Ezreal-byte"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "URLInfoAbout" "https://github.com/Ezreal-byte/lolping-lite"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "DisplayIcon" "$INSTDIR\app.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "NoRepair" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite" "EstimatedSize" ${INSTALLED_KIB}
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  IfFileExists "$INSTDIR\lolping-lite.exe" 0 uninstall_ready
  ExecWait '"$INSTDIR\lolping-lite.exe" --quit' $0
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "Please exit LOLPing Lite before uninstalling. / 请先退出 LOLPing Lite 再卸载。" /SD IDOK
    Abort
  ${EndIf}
  uninstall_ready:
  ; Remove only packaged files, preserving user data and anything the user added.
  !include "${UNINSTALL_FILES}"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\LOLPing Lite\LOLPing Lite.lnk"
  Delete "$SMPROGRAMS\LOLPing Lite\Uninstall.lnk"
  RMDir "$SMPROGRAMS\LOLPing Lite"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "LOLPing Lite"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite"
  DeleteRegKey HKCU "Software\LOLPing Lite"
SectionEnd
