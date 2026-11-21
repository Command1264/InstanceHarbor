Unicode true
!include "MUI2.nsh"
!include "x64.nsh"

!define APP "InstanceHarbor"
!define ROOT "$LOCALAPPDATA\Programs\InstanceHarbor"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\InstanceHarbor-Desktop"
!define SHORTCUT "$SMPROGRAMS\InstanceHarbor.lnk"
!define LEASE "instanceharbor.install"

Name "${APP} ${VERSION}"
OutFile "${OUTPUT}"
InstallDir "${ROOT}"
RequestExecutionLevel user
ShowInstDetails show
ShowUninstDetails show
!define MUI_ICON "src-tauri\icons\icon.ico"
!define MUI_UNICON "src-tauri\icons\icon.ico"
VIProductVersion "${FILE_VERSION}"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${FILE_VERSION}"
VIAddVersionKey "FileDescription" "InstanceHarbor Setup"
VIAddVersionKey "LegalCopyright" "InstanceHarbor contributors"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "TradChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "A Windows x64 system is required." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  SetShellVarContext current
  SetRegView 64
  StrCpy $INSTDIR "${ROOT}"
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  StrCmp $0 "" 0 occupied
  IfFileExists "$INSTDIR" occupied 0
  IfFileExists "${SHORTCUT}" occupied 0
  Return
occupied:
  MessageBox MB_ICONSTOP "InstanceHarbor installation files or a shortcut already exist. Close InstanceHarbor and uninstall the previous version before reinstalling. Your workspace is stored separately." /SD IDOK
  SetErrorLevel 2
  Abort
FunctionEnd

Section
  ClearErrors
  SetOutPath "$INSTDIR"
  File "${PAYLOAD}\InstanceHarbor.exe"
  File "${PAYLOAD}\ihctl.exe"
  File "${PAYLOAD}\conpty.dll"
  File "${PAYLOAD}\OpenConsole.exe"
  File "${PAYLOAD}\vcruntime140.dll"
  File "${PAYLOAD}\vcruntime140_1.dll"
  IfErrors failed
  ClearErrors
  FileOpen $0 "$INSTDIR\${LEASE}" w
  IfErrors failed
  FileWrite $0 "InstanceHarbor Windows desktop installation"
  FileClose $0
  IfErrors failed
  ClearErrors
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  IfErrors failed
  CreateShortCut "${SHORTCUT}" "$INSTDIR\InstanceHarbor.exe" "--stable-default"
  IfErrors failed
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\InstanceHarbor.exe,0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  IfErrors failed
  Goto done
failed:
  Delete "${SHORTCUT}"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$INSTDIR\${LEASE}"
  Delete "$INSTDIR\InstanceHarbor.exe"
  Delete "$INSTDIR\ihctl.exe"
  Delete "$INSTDIR\conpty.dll"
  Delete "$INSTDIR\OpenConsole.exe"
  Delete "$INSTDIR\vcruntime140.dll"
  Delete "$INSTDIR\vcruntime140_1.dll"
  SetOutPath "$TEMP"
  RMDir "$INSTDIR"
  SetErrorLevel 1
  Abort "InstanceHarbor installation did not complete."
done:
SectionEnd

Function un.onInit
  SetShellVarContext current
  SetRegView 64
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  StrCmp $0 "${ROOT}" 0 invalid
  StrCmp $INSTDIR $0 0 invalid
  Return
invalid:
  SetErrorLevel 2
  Abort "InstanceHarbor installation path cannot be verified."
FunctionEnd

Section "Uninstall"
  ; OPEN_EXISTING, GENERIC_READ and no sharing. Every installed process keeps a
  ; shared read handle to the same marker until it exits.
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\${LEASE}", i 0x80000000, i 0, p 0, i 3, i 0x00200080, p 0) p.r0'
  IntCmp $0 -1 busy
  ClearErrors
  Delete "$INSTDIR\InstanceHarbor.exe"
  Delete "$INSTDIR\ihctl.exe"
  Delete "$INSTDIR\conpty.dll"
  Delete "$INSTDIR\OpenConsole.exe"
  Delete "$INSTDIR\vcruntime140.dll"
  Delete "$INSTDIR\vcruntime140_1.dll"
  IfErrors failed
  System::Call 'kernel32::CloseHandle(p r0)'
  Delete "${SHORTCUT}"
  Delete "$INSTDIR\${LEASE}"
  Delete "$INSTDIR\Uninstall.exe"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  SetOutPath "$TEMP"
  RMDir "$INSTDIR"
  Goto done
failed:
  System::Call 'kernel32::CloseHandle(p r0)'
  SetErrorLevel 1
  Abort "Program files could not be removed. Workspace data was kept."
busy:
  MessageBox MB_ICONSTOP "InstanceHarbor is still in use, or its installation marker could not be opened. Close InstanceHarbor, including its Manager and Hosts, before retrying." /SD IDOK
  SetErrorLevel 2
  Abort
done:
SectionEnd
