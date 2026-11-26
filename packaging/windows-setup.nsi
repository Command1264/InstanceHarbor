Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
!include "windows-setup-version.nsh"
!include "windows-setup-webview2.nsh"
!include "windows-setup-close.nsh"
!include "nsDialogs.nsh"

!define APP "InstanceHarbor"
!define ROOT "$LOCALAPPDATA\Programs\InstanceHarbor"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\InstanceHarbor-Desktop"
!define SHORTCUT "$SMPROGRAMS\InstanceHarbor.lnk"
!define LEASE "instanceharbor.install"
; The default stable workspace: the only one whose servers Setup counts.
!define WORKSPACE "$LOCALAPPDATA\InstanceHarbor\workspaces\default"
; The first version whose ihctl has app-exit; older ones only have manager-exit.
!define APP_EXIT_VERSION "1.1.0"
; Exit code of a silent update that stopped because servers may still be running.
!define EXIT_SERVERS_RUNNING 5
; Microsoft's permanent link to the WebView2 Evergreen Standalone Installer for x64.
!define WEBVIEW2_URL "https://go.microsoft.com/fwlink/?linkid=2124701"

Name "${APP} ${VERSION}"
OutFile "${OUTPUT}"
InstallDir "${ROOT}"
RequestExecutionLevel user
ShowInstDetails show
ShowUninstDetails show
!define MUI_ICON "..\src-tauri\icons\icon.ico"
!define MUI_UNICON "..\src-tauri\icons\icon.ico"
VIProductVersion "${FILE_VERSION}"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${FILE_VERSION}"
VIAddVersionKey "FileDescription" "InstanceHarbor Setup"
VIAddVersionKey "LegalCopyright" "InstanceHarbor contributors"

; Cancel on any page, the wait page included, changes nothing and exits with 4.
!define MUI_CUSTOMFUNCTION_ABORT CancelCode
!insertmacro MUI_PAGE_WELCOME
Page custom WaitPage WaitLeave
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
; The first language is the default for a Windows display language Setup does not include.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "TradChinese"

; The version recorded by the installation that this Setup replaces, or empty.
Var Installed
; Handle of the installation marker held without sharing, or -1 while it is in use.
Var Lease
; "current" when the installed ihctl has app-exit, "legacy" for 1.0.x.
Var Generation
Var ServerCount
Var WaitText

; The binaries link the Visual C++ runtime statically; only WebView2 comes from the system.
LangString NeedX64 ${LANG_ENGLISH} "A Windows x64 system is required."
LangString WebView2Missing ${LANG_ENGLISH} "Microsoft Edge WebView2 Runtime was not found. InstanceHarbor needs it to show its window.$\r$\n$\r$\nInstall Microsoft's Evergreen Standalone Installer for x64, then run InstanceHarbor Setup again.$\r$\n$\r$\nDownload that installer from Microsoft now (about 280 MB)?"
LangString Unverified ${LANG_ENGLISH} "InstanceHarbor files or a shortcut already exist, but the installed version cannot be verified. Close InstanceHarbor and uninstall it before installing. Your workspace is stored separately."
LangString UpdateAsk ${LANG_ENGLISH} "InstanceHarbor $Installed is installed.$\r$\nUpdate it to ${VERSION}?$\r$\n$\r$\nYour workspace is kept."
LangString RepairAsk ${LANG_ENGLISH} "InstanceHarbor ${VERSION} is already installed.$\r$\nReinstall its program files (repair)?$\r$\n$\r$\nYour workspace is kept."
LangString DowngradeAsk ${LANG_ENGLISH} "A newer InstanceHarbor $Installed is installed.$\r$\nReplace it with the older ${VERSION}?$\r$\n$\r$\nYour workspace is kept, but the older version may not read settings or data that the newer version wrote."
LangString InUse ${LANG_ENGLISH} "InstanceHarbor is still running. Close InstanceHarbor, including its Manager and Hosts, then run Setup again."
LangString ReplaceFailed ${LANG_ENGLISH} "Some program files could not be replaced. Close InstanceHarbor and run Setup again to repair it. Your workspace is kept."
LangString ServersRunning ${LANG_ENGLISH} "InstanceHarbor servers are still running, or their states could not be read. Setup does not stop servers. Stop them in InstanceHarbor, then run Setup again."
LangString WaitTitle ${LANG_ENGLISH} "Closing InstanceHarbor"
LangString WaitSubtitle ${LANG_ENGLISH} "Setup continues by itself once InstanceHarbor has closed."
LangString WaitChecking ${LANG_ENGLISH} "Checking InstanceHarbor..."
LangString WaitServers ${LANG_ENGLISH} "$ServerCount server(s) are not stopped (including starting or undecided).$\r$\n$\r$\nSetup does not stop servers, so players are not interrupted. Stop them in InstanceHarbor. Setup then closes InstanceHarbor and continues by itself."
LangString WaitUnknown ${LANG_ENGLISH} "Server states could not be read, so some servers may still be running.$\r$\n$\r$\nCheck them in InstanceHarbor and stop any that run. Setup continues by itself once InstanceHarbor and its servers have closed."
LangString WaitClosing ${LANG_ENGLISH} "Closing the InstanceHarbor windows and Manager..."
LangString WaitLegacy ${LANG_ENGLISH} "InstanceHarbor $Installed cannot close its windows for Setup.$\r$\n$\r$\nClose every InstanceHarbor window yourself. If it stays in the notification area, quit it from there. Setup continues by itself."
LangString WaitBusy ${LANG_ENGLISH} "InstanceHarbor is finishing a batch or a core update. Setup continues by itself once it finishes and InstanceHarbor closes."
LangString WaitOther ${LANG_ENGLISH} "Other InstanceHarbor processes are still running. They may belong to another workspace, or be servers that are no longer managed.$\r$\n$\r$\nClose them. Setup continues by itself."
LangString WaitOpen ${LANG_ENGLISH} "Open InstanceHarbor"
LangString NeedX64 ${LANG_TRADCHINESE} "需要 Windows x64 系統。"
LangString WebView2Missing ${LANG_TRADCHINESE} "找不到 Microsoft Edge WebView2 Runtime，InstanceHarbor 需要它才能顯示視窗。$\r$\n$\r$\n請安裝 Microsoft 官方的「Evergreen 獨立安裝程式」x64 版（Evergreen Standalone Installer），再重新執行 InstanceHarbor Setup。$\r$\n$\r$\n要現在從 Microsoft 下載這個安裝程式嗎？（約 280 MB）"
LangString Unverified ${LANG_TRADCHINESE} "已有 InstanceHarbor 的檔案或捷徑，但無法確認已安裝的版本。請先關閉並解除安裝 InstanceHarbor，再進行安裝。工作區另外存放，不受影響。"
LangString UpdateAsk ${LANG_TRADCHINESE} "已安裝 InstanceHarbor $Installed。$\r$\n要更新為 ${VERSION} 嗎？$\r$\n$\r$\n工作區資料會保留。"
LangString RepairAsk ${LANG_TRADCHINESE} "已安裝相同版本的 InstanceHarbor ${VERSION}。$\r$\n要重新安裝程式檔案（修復）嗎？$\r$\n$\r$\n工作區資料會保留。"
LangString DowngradeAsk ${LANG_TRADCHINESE} "已安裝較新的 InstanceHarbor $Installed。$\r$\n確定要退回舊版 ${VERSION} 嗎？$\r$\n$\r$\n工作區資料會保留，但舊版可能無法讀取新版寫入的設定或資料。"
LangString InUse ${LANG_TRADCHINESE} "InstanceHarbor 仍在執行。請關閉 InstanceHarbor（包含 Manager 與 Host）後，再重新執行 Setup。"
LangString ReplaceFailed ${LANG_TRADCHINESE} "部分程式檔案無法替換。請關閉 InstanceHarbor 後重新執行 Setup 進行修復。工作區資料會保留。"
LangString ServersRunning ${LANG_TRADCHINESE} "仍有 InstanceHarbor 伺服器在執行，或無法讀取伺服器狀態。Setup 不會替您停止伺服器。請在 InstanceHarbor 停止伺服器後，再重新執行 Setup。"
LangString WaitTitle ${LANG_TRADCHINESE} "正在關閉 InstanceHarbor"
LangString WaitSubtitle ${LANG_TRADCHINESE} "InstanceHarbor 結束後，Setup 會自動繼續。"
LangString WaitChecking ${LANG_TRADCHINESE} "正在檢查 InstanceHarbor…"
LangString WaitServers ${LANG_TRADCHINESE} "還有 $ServerCount 台伺服器沒有停止（包含啟動中或狀態未定）。$\r$\n$\r$\nSetup 不會替您停止伺服器，以免中斷遊戲中的玩家。請在 InstanceHarbor 停止它們，Setup 隨後會關閉 InstanceHarbor 並自動繼續。"
LangString WaitUnknown ${LANG_TRADCHINESE} "無法讀取伺服器狀態，可能還有伺服器在執行。$\r$\n$\r$\n請在 InstanceHarbor 確認並停止執行中的伺服器。InstanceHarbor 與伺服器都結束後，Setup 會自動繼續。"
LangString WaitClosing ${LANG_TRADCHINESE} "正在關閉 InstanceHarbor 視窗與 Manager…"
LangString WaitLegacy ${LANG_TRADCHINESE} "InstanceHarbor $Installed 無法由 Setup 關閉視窗。$\r$\n$\r$\n請自行關閉所有 InstanceHarbor 視窗；如果它縮到系統匣，請從系統匣圖示結束。Setup 會自動繼續。"
LangString WaitBusy ${LANG_TRADCHINESE} "InstanceHarbor 還有批次工作或核心更新在進行。完成並關閉後，Setup 會自動繼續。"
LangString WaitOther ${LANG_TRADCHINESE} "還有其他 InstanceHarbor 程序在執行，可能是其他工作區，或已不在管理範圍的伺服器。$\r$\n$\r$\n請關閉它們，Setup 會自動繼續。"
LangString WaitOpen ${LANG_TRADCHINESE} "開啟 InstanceHarbor"

Function RequireWebView2
  Call FindWebView2
  StrCmp $0 "" 0 found
  MessageBox MB_YESNO|MB_ICONEXCLAMATION "$(WebView2Missing)" /SD IDNO IDNO abort
  ExecShell "open" "${WEBVIEW2_URL}"
abort:
  SetErrorLevel 3
  Abort
found:
FunctionEnd

; Every installed GUI, CLI, Manager and Host keeps a shared read handle to the marker
; until it exits. Opening it without sharing succeeds only once all of them have ended;
; the handle is then held while the program files are replaced.
Function TryLease
  IntCmp $Lease -1 0 held held
  System::Call 'kernel32::CreateFileW(w "$INSTDIR\${LEASE}", i 0x80000000, i 0, p 0, i 3, i 0x00200080, p 0) p.r0'
  StrCpy $Lease $0
held:
FunctionEnd

; This Setup's ihctl, run from the temporary folder. It reads and closes the installed
; version only through the installed ihctl (src-tauri/src/setup_probe.rs).
Function PrepareProbe
  IfFileExists "$PLUGINSDIR\ihctl.exe" ready
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File "${PAYLOAD}\ihctl.exe"
  SetOutPath "$TEMP"
ready:
FunctionEnd

; $R5: how many servers of the default workspace are not stopped, or "unknown".
Function ProbeServers
  nsExec::ExecToStack /TIMEOUT=20000 '"$PLUGINSDIR\ihctl.exe" setup-probe servers "$INSTDIR" "${WORKSPACE}"'
  Pop $R1
  Pop $R5
  StrCmp $R1 "0" +2
  StrCpy $R5 "unknown"
FunctionEnd

; $R6: what the installed version did when asked to close its GUI and Manager.
Function ProbeClose
  nsExec::ExecToStack /TIMEOUT=20000 '"$PLUGINSDIR\ihctl.exe" setup-probe close "$INSTDIR" "${WORKSPACE}" $Generation'
  Pop $R1
  Pop $R6
  StrCmp $R1 "0" +2
  StrCpy $R6 "failed"
FunctionEnd

; Counts servers, asks to close once none runs, and leaves the page status in $R0.
Function Survey
  Call ProbeServers
  StrCpy $R6 ""
  StrCmp $R5 "0" 0 +2
  Call ProbeClose
  StrCpy $ServerCount $R5
  Call WaitStatus
FunctionEnd

Function WaitPage
  StrCmp $Installed "" skip
  Call TryLease
  IntCmp $Lease -1 0 skip skip
  Call PrepareProbe
  !insertmacro MUI_HEADER_TEXT "$(WaitTitle)" "$(WaitSubtitle)"
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 70% "$(WaitChecking)"
  Pop $WaitText
  ${NSD_CreateButton} 0 76% 45% 15u "$(WaitOpen)"
  Pop $0
  ${NSD_OnClick} $0 OpenInstalled
  GetDlgItem $0 $HWNDPARENT 1
  EnableWindow $0 0
  ${NSD_CreateTimer} WaitTick 500
  nsDialogs::Show
  Return
skip:
  Abort
FunctionEnd

Function WaitTick
  ; A probe takes a moment; no second tick may start meanwhile.
  ${NSD_KillTimer} WaitTick
  Call TryLease
  IntCmp $Lease -1 wait
  GetDlgItem $0 $HWNDPARENT 1
  EnableWindow $0 1
  ; WM_NOTIFY_OUTER_NEXT: continue to the installation page.
  SendMessage $HWNDPARENT 0x408 1 0
  Return
wait:
  Call Survey
  StrCmp $R0 "servers" servers
  StrCmp $R0 "unknown" unknown
  StrCmp $R0 "closing" closing
  StrCmp $R0 "legacy" legacy
  StrCmp $R0 "busy" busy
  ${NSD_SetText} $WaitText "$(WaitOther)"
  Goto again
servers:
  ${NSD_SetText} $WaitText "$(WaitServers)"
  Goto again
unknown:
  ${NSD_SetText} $WaitText "$(WaitUnknown)"
  Goto again
closing:
  ${NSD_SetText} $WaitText "$(WaitClosing)"
  Goto again
legacy:
  ${NSD_SetText} $WaitText "$(WaitLegacy)"
  Goto again
busy:
  ${NSD_SetText} $WaitText "$(WaitBusy)"
again:
  ${NSD_CreateTimer} WaitTick 1500
FunctionEnd

Function WaitLeave
  IntCmp $Lease -1 0 done done
  Abort
done:
FunctionEnd

; The user stops the servers in the installed version's GUI; Setup keeps waiting.
Function OpenInstalled
  Pop $0
  Exec '"$INSTDIR\InstanceHarbor.exe" --stable-default'
FunctionEnd

; Without the wait page (silent install): close only when no server runs; never wait
; for servers.
Function CloseForUpdate
  Call PrepareProbe
  Call ProbeServers
  StrCmp $R5 "0" close
  MessageBox MB_ICONSTOP "$(ServersRunning)" /SD IDOK
  SetErrorLevel ${EXIT_SERVERS_RUNNING}
  Abort
close:
  StrCpy $R4 0
retry:
  Call ProbeClose
  Sleep 1000
  Call TryLease
  IntCmp $Lease -1 0 done done
  IntOp $R4 $R4 + 1
  IntCmp $R4 30 0 retry
  MessageBox MB_ICONSTOP "$(InUse)" /SD IDOK
  SetErrorLevel 2
  Abort
done:
FunctionEnd

Function CancelCode
  ; Cancel before the installation page changes nothing.
  SetErrorLevel 4
FunctionEnd

Function .onInit
  ; Same rule as InstanceHarbor's WebView2 prompt: Traditional Chinese for a
  ; zh-TW, zh-HK or zh-MO display language, English for every other one.
  StrCpy $LANGUAGE ${LANG_ENGLISH}
  System::Call 'kernel32::GetUserDefaultUILanguage() i.r0'
  ${If} $0 = 0x0404
  ${OrIf} $0 = 0x0C04
  ${OrIf} $0 = 0x1404
    StrCpy $LANGUAGE ${LANG_TRADCHINESE}
  ${EndIf}
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "$(NeedX64)" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  SetShellVarContext current
  SetRegView 64
  StrCpy $INSTDIR "${ROOT}"
  StrCpy $Installed ""
  StrCpy $Lease -1
  ; Checked on every run, updates included, in case WebView2 was removed later.
  Call RequireWebView2
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  StrCmp $0 "" fresh
  ; Replace only an installation this Setup can identify: its folder, version and marker.
  StrCmp $0 "${ROOT}" 0 unverified
  ReadRegStr $Installed HKCU "${UNINSTALL_KEY}" "DisplayVersion"
  StrCmp $Installed "" unverified
  IfFileExists "$INSTDIR\${LEASE}" 0 unverified
  StrCpy $Generation "current"
  StrCpy $R5 $Installed
  StrCpy $R6 "${APP_EXIT_VERSION}"
  Call CompareVersions
  StrCmp $R0 "1" 0 +2
  StrCpy $Generation "legacy"
  StrCpy $R5 $Installed
  StrCpy $R6 "${VERSION}"
  Call CompareVersions
  StrCmp $R0 "1" update
  StrCmp $R0 "2" downgrade
  MessageBox MB_YESNO|MB_ICONQUESTION "$(RepairAsk)" /SD IDYES IDYES proceed
  Goto cancel
update:
  MessageBox MB_YESNO|MB_ICONQUESTION "$(UpdateAsk)" /SD IDYES IDYES proceed
  Goto cancel
downgrade:
  MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "$(DowngradeAsk)" /SD IDNO IDYES proceed
cancel:
  SetErrorLevel 4
  Abort
proceed:
  Return
fresh:
  IfFileExists "$INSTDIR" unverified 0
  IfFileExists "${SHORTCUT}" unverified 0
  Return
unverified:
  StrCpy $Installed ""
  MessageBox MB_ICONSTOP "$(Unverified)" /SD IDOK
  SetErrorLevel 2
  Abort
FunctionEnd

Section
  StrCmp $Installed "" copy
  ; The wait page normally holds the marker already; a silent install closes here.
  Call TryLease
  IntCmp $Lease -1 0 copy copy
  Call CloseForUpdate
copy:
  ClearErrors
  SetOutPath "$INSTDIR"
  File "${PAYLOAD}\InstanceHarbor.exe"
  File "${PAYLOAD}\ihctl.exe"
  File "${PAYLOAD}\conpty.dll"
  File "${PAYLOAD}\OpenConsole.exe"
  File "${PAYLOAD}\WINDOWS-GETTING-STARTED.txt"
  File "${PAYLOAD}\LICENSE"
  File "${PAYLOAD}\THIRD-PARTY-NOTICES.txt"
  File "${PAYLOAD}\MPL-SOURCES.zip"
  ; 1.0.0-rc2 installed app-local Visual C++ runtime DLLs; 1.0 links the runtime
  ; statically, so remove them on update and uninstall.
  IfFileExists "$INSTDIR\vcruntime140.dll" 0 +2
  Delete "$INSTDIR\vcruntime140.dll"
  IfFileExists "$INSTDIR\vcruntime140_1.dll" 0 +2
  Delete "$INSTDIR\vcruntime140_1.dll"
  StrCmp $Installed "" +2
  System::Call 'kernel32::CloseHandle(p $Lease)'
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
  ; Never delete an existing installation; running Setup again repairs it.
  StrCmp $Installed "" remove
  MessageBox MB_ICONSTOP "$(ReplaceFailed)" /SD IDOK
  SetErrorLevel 1
  Abort
remove:
  Delete "${SHORTCUT}"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$INSTDIR\${LEASE}"
  Delete "$INSTDIR\InstanceHarbor.exe"
  Delete "$INSTDIR\ihctl.exe"
  Delete "$INSTDIR\conpty.dll"
  Delete "$INSTDIR\OpenConsole.exe"
  Delete "$INSTDIR\WINDOWS-GETTING-STARTED.txt"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\MPL-SOURCES.zip"
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
  Delete "$INSTDIR\WINDOWS-GETTING-STARTED.txt"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\MPL-SOURCES.zip"
  ; 1.0.0-rc2 installed app-local Visual C++ runtime DLLs; 1.0 links the runtime
  ; statically, so remove them on update and uninstall.
  IfFileExists "$INSTDIR\vcruntime140.dll" 0 +2
  Delete "$INSTDIR\vcruntime140.dll"
  IfFileExists "$INSTDIR\vcruntime140_1.dll" 0 +2
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
