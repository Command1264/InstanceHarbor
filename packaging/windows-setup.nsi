Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
!include "windows-setup-version.nsh"
!include "windows-setup-webview2.nsh"
!include "windows-setup-close.nsh"
!include "windows-setup-scope.nsh"
!include "nsDialogs.nsh"

!define APP "InstanceHarbor"
; Default folders: only for the current user, or for all users (#38).
!define ROOT "$UserData\Programs\InstanceHarbor"
!define MACHINE_ROOT "$PROGRAMFILES64\InstanceHarbor"
; HKCU for the current user, HKLM for all users; SHCTX follows SetShellVarContext.
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\InstanceHarbor-Desktop"
!define SHORTCUT "$SMPROGRAMS\InstanceHarbor.lnk"
!define LEASE "instanceharbor.install"
; The default stable workspace: the only one whose servers Setup counts. It is always in
; the user's own %LOCALAPPDATA%, also for an installation for all users.
!define WORKSPACE "$UserData\InstanceHarbor\workspaces\default"
; The first version whose ihctl has app-exit; older ones only have manager-exit.
!define APP_EXIT_VERSION "1.1.0"
; Exit code of a silent update that stopped because servers may still be running.
!define EXIT_SERVERS_RUNNING 5
; Microsoft's permanent link to the WebView2 Evergreen Standalone Installer for x64.
!define WEBVIEW2_URL "https://go.microsoft.com/fwlink/?linkid=2124701"

Name "${APP} ${VERSION}"
OutFile "${OUTPUT}"
; .onInit replaces this placeholder unless /D= named a folder. Its last part is what the
; folder page's Browse adds to the chosen folder.
InstallDir "\InstanceHarbor"
; Setup starts without administrator rights and asks for them (UAC) only for all users.
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
!define MUI_PAGE_CUSTOMFUNCTION_PRE WelcomePre
!insertmacro MUI_PAGE_WELCOME
; A new installation chooses its scope and folder; an update keeps the registered ones.
Page custom ScopePage ScopeLeave
!define MUI_PAGE_CUSTOMFUNCTION_PRE FolderPre
!define MUI_PAGE_CUSTOMFUNCTION_SHOW FolderShow
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE FolderLeave
!define MUI_DIRECTORYPAGE_TEXT_TOP "$(FolderTop)"
!insertmacro MUI_PAGE_DIRECTORY
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
; "current" (HKCU, the user's Start menu) or "all" (HKLM, the Start menu of all users).
Var Scope
; The scope that the existing installation registered, or "" for a new installation.
Var Registered
; The scope that /AllUsers or /CurrentUser asked for, or "".
Var Wanted
; "1" when /D= named the folder.
Var FolderGiven
; "1" in the copy that this Setup started with administrator rights.
Var Elevated
; "1" when the elevated copy continues after the scope page of the first copy.
Var Continue
; The user's own %LOCALAPPDATA%, read before the shell context changes to all users.
Var UserData
Var ScopeCurrent
Var ScopeAll

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
LangString WaitAllUsers ${LANG_ENGLISH} "InstanceHarbor is installed for all users of this computer. If other users have InstanceHarbor or its servers open, their programs keep these files in use too. Ask them to stop their servers and quit InstanceHarbor."
LangString ScopeTitle ${LANG_ENGLISH} "Choose who can use InstanceHarbor"
LangString ScopeSubtitle ${LANG_ENGLISH} "Install InstanceHarbor only for you, or for every user of this computer."
LangString ScopeCurrent ${LANG_ENGLISH} "Only for me (recommended)"
LangString ScopeCurrentText ${LANG_ENGLISH} "Installs into your own user folder. Administrator rights are not needed, also for updates."
LangString ScopeAll ${LANG_ENGLISH} "For all users of this computer"
LangString ScopeAllText ${LANG_ENGLISH} "Installs into Program Files and asks for administrator approval (User Account Control). Updates ask for it too."
LangString ScopeNote ${LANG_ENGLISH} "Each user keeps a separate workspace in their own %LOCALAPPDATA%\InstanceHarbor. To change the scope or the folder later, uninstall InstanceHarbor first; workspaces are kept."
LangString FolderTop ${LANG_ENGLISH} "Setup installs InstanceHarbor in the following folder. Choose an empty or new folder: a folder that already contains other files cannot be used, so that uninstalling removes only what Setup installed."
LangString FolderInvalid ${LANG_ENGLISH} "Choose a full folder path on a local drive, for example D:\Apps\InstanceHarbor. A drive root, a relative path or a network folder cannot be used."
LangString FolderNotEmpty ${LANG_ENGLISH} "This folder already contains other files:$\r$\n$INSTDIR$\r$\n$\r$\nChoose an empty or new folder. Setup does not mix InstanceHarbor with other files, so that uninstalling removes only what Setup installed."
LangString FolderProtected ${LANG_ENGLISH} "This folder needs administrator rights:$\r$\n$INSTDIR$\r$\n$\r$\nAn installation only for you must stay writable without them, or updates would fail. Choose a folder in your own user folder, or go Back and install for all users."
LangString FolderNoAccess ${LANG_ENGLISH} "Setup cannot create files in this folder without administrator rights:$\r$\n$INSTDIR$\r$\n$\r$\nChoose a folder that you can write to, or go Back and install for all users."
LangString FolderProfile ${LANG_ENGLISH} "This folder is inside a user profile, where other users cannot use it:$\r$\n$INSTDIR$\r$\n$\r$\nChoose a folder outside the Users folder, such as Program Files."
LangString InstalledCurrent ${LANG_ENGLISH} "InstanceHarbor is already installed only for the current user, in:$\r$\n$INSTDIR$\r$\n$\r$\nSetup updates or repairs it there and does not install a second copy for all users. To install it for all users, uninstall it first. Your workspace is stored separately and is kept."
LangString InstalledAll ${LANG_ENGLISH} "InstanceHarbor is already installed for all users of this computer, in:$\r$\n$INSTDIR$\r$\n$\r$\nSetup updates or repairs it there and does not install a second copy for one user. To install it only for you, uninstall it first. Your workspace is stored separately and is kept."
LangString InstalledFolder ${LANG_ENGLISH} "InstanceHarbor is already installed in:$\r$\n$INSTDIR$\r$\n$\r$\nUpdates and repairs keep this folder. To install it in another folder, uninstall it first. Your workspace is stored separately and is kept."
LangString InstalledTwice ${LANG_ENGLISH} "InstanceHarbor is registered both for the current user and for all users. Uninstall both in Windows Settings > Apps > Installed apps, then run Setup again. Your workspace is stored separately and is kept."
LangString ScopeUsage ${LANG_ENGLISH} "Use only one of /AllUsers and /CurrentUser."
LangString ElevationDenied ${LANG_ENGLISH} "An installation for all users needs administrator approval, which was not given. Nothing was changed."
LangString ElevationFailed ${LANG_ENGLISH} "Setup could not start with administrator rights. Nothing was changed."
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
LangString WaitAllUsers ${LANG_TRADCHINESE} "InstanceHarbor 是為這台電腦的所有使用者安裝的。其他使用者開著 InstanceHarbor 或它的伺服器時，也會占用這些檔案；請他們停止伺服器並結束 InstanceHarbor。"
LangString ScopeTitle ${LANG_TRADCHINESE} "選擇安裝範圍"
LangString ScopeSubtitle ${LANG_TRADCHINESE} "只為您自己安裝 InstanceHarbor，或為這台電腦的所有使用者安裝。"
LangString ScopeCurrent ${LANG_TRADCHINESE} "只為目前使用者安裝（建議）"
LangString ScopeCurrentText ${LANG_TRADCHINESE} "安裝到您自己的使用者資料夾。安裝與更新都不需要系統管理員權限。"
LangString ScopeAll ${LANG_TRADCHINESE} "為這台電腦的所有使用者安裝"
LangString ScopeAllText ${LANG_TRADCHINESE} "安裝到 Program Files，需要系統管理員確認（使用者帳戶控制）；之後的更新也需要。"
LangString ScopeNote ${LANG_TRADCHINESE} "每位使用者的工作區仍分別存放在各自的 %LOCALAPPDATA%\InstanceHarbor。之後要更換範圍或資料夾，需要先解除安裝；工作區會保留。"
LangString FolderTop ${LANG_TRADCHINESE} "Setup 會把 InstanceHarbor 安裝到下列資料夾。請選擇空的或新的資料夾：已有其他檔案的資料夾不能使用，這樣解除安裝時只會刪除 Setup 安裝的檔案。"
LangString FolderInvalid ${LANG_TRADCHINESE} "請選擇本機磁碟上的完整資料夾路徑，例如 D:\Apps\InstanceHarbor。不能使用磁碟根目錄、相對路徑或網路資料夾。"
LangString FolderNotEmpty ${LANG_TRADCHINESE} "這個資料夾已有其他檔案：$\r$\n$INSTDIR$\r$\n$\r$\n請選擇空的或新的資料夾。Setup 不會把 InstanceHarbor 和其他檔案放在一起，這樣解除安裝時只會刪除 Setup 安裝的檔案。"
LangString FolderProtected ${LANG_TRADCHINESE} "這個資料夾需要系統管理員權限：$\r$\n$INSTDIR$\r$\n$\r$\n只為目前使用者安裝時，資料夾必須不需要系統管理員權限就能寫入，否則之後無法更新。請選擇您自己使用者資料夾內的位置，或按「上一步」改為所有使用者安裝。"
LangString FolderNoAccess ${LANG_TRADCHINESE} "沒有系統管理員權限時，Setup 無法在這個資料夾建立檔案：$\r$\n$INSTDIR$\r$\n$\r$\n請選擇您可以寫入的資料夾，或按「上一步」改為所有使用者安裝。"
LangString FolderProfile ${LANG_TRADCHINESE} "這個資料夾在某位使用者的設定檔內，其他使用者無法使用：$\r$\n$INSTDIR$\r$\n$\r$\n請選擇 Users 資料夾以外的位置，例如 Program Files。"
LangString InstalledCurrent ${LANG_TRADCHINESE} "InstanceHarbor 已經只為目前使用者安裝在：$\r$\n$INSTDIR$\r$\n$\r$\nSetup 只會在原處更新或修復，不會再為所有使用者另裝一份。要改為所有使用者安裝，請先解除安裝。工作區另外存放，會保留。"
LangString InstalledAll ${LANG_TRADCHINESE} "InstanceHarbor 已經為這台電腦的所有使用者安裝在：$\r$\n$INSTDIR$\r$\n$\r$\nSetup 只會在原處更新或修復，不會再為單一使用者另裝一份。要改為只為您自己安裝，請先解除安裝。工作區另外存放，會保留。"
LangString InstalledFolder ${LANG_TRADCHINESE} "InstanceHarbor 已安裝在：$\r$\n$INSTDIR$\r$\n$\r$\n更新與修復都沿用這個資料夾。要安裝到其他資料夾，請先解除安裝。工作區另外存放，會保留。"
LangString InstalledTwice ${LANG_TRADCHINESE} "InstanceHarbor 同時登記為目前使用者與所有使用者的安裝。請在 Windows「設定 > 應用程式 > 已安裝的應用程式」解除安裝這兩份，再重新執行 Setup。工作區另外存放，會保留。"
LangString ScopeUsage ${LANG_TRADCHINESE} "/AllUsers 與 /CurrentUser 只能擇一使用。"
LangString ElevationDenied ${LANG_TRADCHINESE} "為所有使用者安裝需要系統管理員確認，但沒有取得確認。沒有做任何變更。"
LangString ElevationFailed ${LANG_TRADCHINESE} "Setup 無法以系統管理員權限啟動。沒有做任何變更。"

; Starts this program again with administrator rights (UAC) and the arguments in $R9,
; and waits for it. $R0: its exit code, "denied" when the user (or the policy) refused
; the approval, or "failed".
!macro RunElevated PREFIX
Function ${PREFIX}RunElevated
  ; SHELLEXECUTEINFOW with SEE_MASK_NOCLOSEPROCESS, so that the process can be waited for,
  ; and SEE_MASK_FLAG_NO_UI: Setup explains a failure itself, without a system error box.
  System::Call '*(&l4, i 0x440,p $HWNDPARENT, w "runas", w "$EXEPATH", w "$R9", p 0, i 1, p 0, p 0, p 0, p 0, i 0, p 0, p 0) p .r1'
  System::Call 'shell32::ShellExecuteExW(p r1) i .r2 ?e'
  Pop $3
  IntCmp $2 0 refused
  System::Call '*$1(i, i, p, p, p, p, p, i, p, p, p, p, i, p, p .r4)'
  System::Free $1
  System::Call 'kernel32::WaitForSingleObject(p r4, i -1)'
  System::Call 'kernel32::GetExitCodeProcess(p r4, *i .r5)'
  System::Call 'kernel32::CloseHandle(p r4)'
  StrCpy $R0 $5
  Return
refused:
  System::Free $1
  StrCpy $R0 "failed"
  ; ERROR_CANCELLED
  IntCmp $3 1223 0 +2 +2
  StrCpy $R0 "denied"
FunctionEnd
!macroend
!insertmacro RunElevated ""
!insertmacro RunElevated "un."

; $0: 1 when this process has administrator rights.
!macro IsAdmin PREFIX
Function ${PREFIX}IsAdmin
  System::Call 'shell32::IsUserAnAdmin() i .r0'
FunctionEnd
!macroend
!insertmacro IsAdmin ""
!insertmacro IsAdmin "un."

; Elevates with the arguments in $R9 and ends this copy with the elevated copy's exit code.
; Nothing was changed when the approval was refused (exit code 4) or failed (2).
Function ElevateAndQuit
  Call RunElevated
  StrCmp $R0 "denied" denied
  StrCmp $R0 "failed" failed
  SetErrorLevel $R0
  Quit
denied:
  MessageBox MB_ICONSTOP "$(ElevationDenied)" /SD IDOK
  SetErrorLevel 4
  Quit
failed:
  MessageBox MB_ICONSTOP "$(ElevationFailed)" /SD IDOK
  SetErrorLevel 2
  Quit
FunctionEnd

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
  StrCpy $1 "$(WaitOther)"
  Goto show
servers:
  StrCpy $1 "$(WaitServers)"
  Goto show
unknown:
  StrCpy $1 "$(WaitUnknown)"
  Goto show
closing:
  StrCpy $1 "$(WaitClosing)"
  Goto show
legacy:
  StrCpy $1 "$(WaitLegacy)"
  Goto show
busy:
  StrCpy $1 "$(WaitBusy)"
show:
  ; Other users' programs also hold the marker of an installation for all users.
  StrCmp $Scope "all" 0 +2
  StrCpy $1 "$1$\r$\n$\r$\n$(WaitAllUsers)"
  ${NSD_SetText} $WaitText "$1"
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
  StrCmp $Elevated "1" shortcut
  Exec '"$INSTDIR\InstanceHarbor.exe" --stable-default'
  Return
shortcut:
  ; Never start InstanceHarbor with administrator rights: Explorer opens the Start menu
  ; shortcut as the signed-in user.
  Exec '"$WINDIR\explorer.exe" "${SHORTCUT}"'
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
  StrCpy $1 "$(InUse)"
  StrCmp $Scope "all" 0 +2
  StrCpy $1 "$1$\r$\n$\r$\n$(WaitAllUsers)"
  MessageBox MB_ICONSTOP "$1" /SD IDOK
  SetErrorLevel 2
  Abort
done:
FunctionEnd

; A new installation for all users outside Program Files gets the same protection as
; Program Files: owned by Administrators, writable only by administrators and SYSTEM,
; readable by users. The elevated Setup of a later update runs the installed ihctl, so no
; user may replace it. $R0: "" or "failed".
Function ProtectFolder
  StrCpy $R1 $INSTDIR
  !insertmacro IsUnderRoot "$PROGRAMFILES64" protected
  !insertmacro IsUnderRoot "$PROGRAMFILES32" protected
  ClearErrors
  CreateDirectory "$INSTDIR"
  IfErrors failed
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$INSTDIR" /setowner *S-1-5-32-544 /Q'
  Pop $0
  StrCmp $0 "0" 0 failed
  nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$INSTDIR" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX *S-1-15-2-1:(OI)(CI)RX /Q'
  Pop $0
  StrCmp $0 "0" 0 failed
protected:
  StrCpy $R0 ""
  Return
failed:
  StrCpy $R0 "failed"
FunctionEnd

Function DefaultFolder
  StrCmp $Scope "all" 0 +3
  StrCpy $INSTDIR "${MACHINE_ROOT}"
  Return
  StrCpy $INSTDIR "${ROOT}"
FunctionEnd

; Checks $INSTDIR for a new installation. $R0: "" or the message that explains the refusal.
Function CheckFolder
  StrCpy $R5 $Scope
  StrCpy $R6 $PROFILE
  Call CheckInstallFolder
  StrCmp $R0 "" done
  StrCmp $R0 "invalid" 0 +3
  StrCpy $R0 "$(FolderInvalid)"
  Return
  StrCmp $R0 "notempty" 0 +3
  StrCpy $R0 "$(FolderNotEmpty)"
  Return
  StrCmp $R0 "protected" 0 +3
  StrCpy $R0 "$(FolderProtected)"
  Return
  StrCmp $R0 "profile" 0 +3
  StrCpy $R0 "$(FolderProfile)"
  Return
  StrCpy $R0 "$(FolderNoAccess)"
done:
FunctionEnd

; The elevated copy that continues after the scope page starts at the folder page.
Function WelcomePre
  StrCmp $Continue "1" 0 +2
  Abort
FunctionEnd

; Only a new installation without /AllUsers or /CurrentUser asks for the scope.
Function ScopePage
  StrCmp $Registered "" 0 skip
  StrCmp $Wanted "" 0 skip
  StrCmp $Elevated "1" skip
  !insertmacro MUI_HEADER_TEXT "$(ScopeTitle)" "$(ScopeSubtitle)"
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateRadioButton} 0 0 100% 12u "$(ScopeCurrent)"
  Pop $ScopeCurrent
  ${NSD_CreateLabel} 12u 13u -12u 18u "$(ScopeCurrentText)"
  Pop $0
  ${NSD_CreateRadioButton} 0 36u 100% 12u "$(ScopeAll)"
  Pop $ScopeAll
  ${NSD_CreateLabel} 12u 49u -12u 26u "$(ScopeAllText)"
  Pop $0
  ${NSD_CreateLabel} 0 84u 100% 30u "$(ScopeNote)"
  Pop $0
  ${If} $Scope == "all"
    ${NSD_Check} $ScopeAll
  ${Else}
    ${NSD_Check} $ScopeCurrent
  ${EndIf}
  ${NSD_OnClick} $ScopeCurrent ScopeShield
  ${NSD_OnClick} $ScopeAll ScopeShield
  Push $ScopeAll
  Call ScopeShield
  nsDialogs::Show
  Return
skip:
  Abort
FunctionEnd

; Shows the UAC shield on Next while "for all users" is chosen without administrator rights.
Function ScopeShield
  Pop $0
  ${NSD_GetState} $ScopeAll $1
  Call IsAdmin
  StrCpy $3 0
  ${If} $1 == ${BST_CHECKED}
  ${AndIf} $0 = 0
    StrCpy $3 1
  ${EndIf}
  GetDlgItem $2 $HWNDPARENT 1
  ; BCM_SETSHIELD
  SendMessage $2 0x160C 0 $3
FunctionEnd

Function ScopeLeave
  ${NSD_GetState} $ScopeAll $0
  StrCpy $Scope "current"
  ${If} $0 == ${BST_CHECKED}
    StrCpy $Scope "all"
  ${EndIf}
  GetDlgItem $2 $HWNDPARENT 1
  SendMessage $2 0x160C 0 0
  ${If} $Scope == "all"
    Call IsAdmin
    ${If} $0 = 0
      ; The elevated copy continues at the folder page; this copy ends with its exit code.
      StrCpy $R9 "/AllUsers /Elevated /Continue"
      ${If} $FolderGiven == "1"
        StrCpy $R9 "$R9 /D=$INSTDIR"
      ${EndIf}
      HideWindow
      Call RunElevated
      ${If} $R0 == "denied"
      ${OrIf} $R0 == "failed"
        ShowWindow $HWNDPARENT ${SW_SHOW}
        BringToFront
        StrCmp $R0 "denied" 0 +3
        MessageBox MB_ICONEXCLAMATION "$(ElevationDenied)"
        Goto +2
        MessageBox MB_ICONEXCLAMATION "$(ElevationFailed)"
        Push $ScopeAll
        Call ScopeShield
        Abort
      ${EndIf}
      SetErrorLevel $R0
      Quit
    ${EndIf}
    SetShellVarContext all
  ${Else}
    SetShellVarContext current
  ${EndIf}
  ${If} $FolderGiven != "1"
    Call DefaultFolder
  ${EndIf}
FunctionEnd

Function FolderPre
  StrCmp $Registered "" +2
  Abort
FunctionEnd

Function FolderShow
  ; The elevated copy has no earlier page to go back to.
  StrCmp $Continue "1" 0 +3
  GetDlgItem $0 $HWNDPARENT 3
  EnableWindow $0 0
FunctionEnd

Function FolderLeave
  Call CheckFolder
  StrCmp $R0 "" +3
  MessageBox MB_ICONEXCLAMATION "$R0"
  Abort
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
  StrCpy $UserData $LOCALAPPDATA
  StrCpy $Installed ""
  StrCpy $Lease -1
  StrCpy $Scope "current"
  ${GetParameters} $R9
  ClearErrors
  ${GetOptions} $R9 "/Elevated" $0
  ${IfNot} ${Errors}
    StrCpy $Elevated "1"
  ${EndIf}
  ClearErrors
  ${GetOptions} $R9 "/Continue" $0
  ${IfNot} ${Errors}
    StrCpy $Continue "1"
  ${EndIf}
  ClearErrors
  ${GetOptions} $R9 "/AllUsers" $0
  ${IfNot} ${Errors}
    StrCpy $Wanted "all"
  ${EndIf}
  ClearErrors
  ${GetOptions} $R9 "/CurrentUser" $0
  ${IfNot} ${Errors}
    ${If} $Wanted == "all"
      StrCpy $Wanted "both"
    ${Else}
      StrCpy $Wanted "current"
    ${EndIf}
  ${EndIf}
  ; NSIS sets $INSTDIR from /D= before .onInit; otherwise it is the InstallDir placeholder.
  ${If} $INSTDIR != "\InstanceHarbor"
    StrCpy $FolderGiven "1"
  ${EndIf}
  ${If} $Elevated == "1"
    ; Never ask again from the elevated copy.
    Call IsAdmin
    ${If} $0 = 0
      MessageBox MB_ICONSTOP "$(ElevationFailed)" /SD IDOK
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${Else}
    ; Checked on every run, updates included, in case WebView2 was removed later. The
    ; elevated copy relies on this check: its own per-user runtime may differ.
    Call RequireWebView2
  ${EndIf}
  ; Which installation is registered: HKCU for the current user, HKLM for all users.
  ReadRegStr $1 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  ReadRegStr $2 HKLM "${UNINSTALL_KEY}" "InstallLocation"
  StrCpy $3 ""
  ${If} $1 != ""
  ${AndIf} $2 != ""
    StrCpy $R1 "both"
  ${ElseIf} $1 != ""
    StrCpy $R1 "current"
    StrCpy $3 $1
  ${ElseIf} $2 != ""
    StrCpy $R1 "all"
    StrCpy $3 $2
  ${Else}
    StrCpy $R1 ""
  ${EndIf}
  StrCpy $R2 $Wanted
  StrCpy $R3 ""
  ${If} $3 != ""
  ${AndIf} $FolderGiven == "1"
    StrCpy $0 $INSTDIR 1 -1
    ${If} $0 == "\"
      StrCpy $INSTDIR $INSTDIR -1
    ${EndIf}
    ${If} $INSTDIR != $3
      StrCpy $R3 "1"
    ${EndIf}
  ${EndIf}
  Call DecideScope
  StrCmp $R0 "usage" usage
  StrCmp $R0 "two" two
  StrCmp $R0 "fresh-current" fresh
  StrCmp $R0 "fresh-all" fresh
  ; An update, repair or downgrade keeps the registered scope and folder.
  StrCpy $INSTDIR $3
  StrCmp $R0 "moved" moved
  StrCmp $R0 "other-scope" otherscope
  StrCpy $Registered $R1
  StrCpy $Scope $R1
  ${If} $Scope == "all"
    Call IsAdmin
    ${If} $0 = 0
      StrCpy $R9 "/Elevated"
      IfSilent 0 +2
      StrCpy $R9 "/S /Elevated"
      Call ElevateAndQuit
    ${EndIf}
    SetShellVarContext all
  ${EndIf}
  ; Replace only an installation this Setup can identify: its version and marker.
  ReadRegStr $Installed SHCTX "${UNINSTALL_KEY}" "DisplayVersion"
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
  ; Never overwrite an earlier installation that is not registered.
  IfFileExists "${ROOT}\${LEASE}" unverified
  IfFileExists "${ROOT}\InstanceHarbor.exe" unverified
  IfFileExists "${MACHINE_ROOT}\${LEASE}" unverified
  IfFileExists "${MACHINE_ROOT}\InstanceHarbor.exe" unverified
  IfFileExists "${SHORTCUT}" unverified
  SetShellVarContext all
  IfFileExists "${SHORTCUT}" unverified
  SetShellVarContext current
  StrCmp $R0 "fresh-all" 0 folder
  StrCpy $Scope "all"
  Call IsAdmin
  ${If} $0 = 0
    StrCpy $R9 "/AllUsers /Elevated"
    IfSilent 0 +2
    StrCpy $R9 "/S $R9"
    ; /D= must be the last argument, without quotes.
    ${If} $FolderGiven == "1"
      StrCpy $R9 "$R9 /D=$INSTDIR"
    ${EndIf}
    Call ElevateAndQuit
  ${EndIf}
  SetShellVarContext all
folder:
  ${If} $FolderGiven != "1"
    Call DefaultFolder
  ${EndIf}
  ; Without the folder page, a silent installation checks the folder here.
  IfSilent 0 +6
  Call CheckFolder
  StrCmp $R0 "" +4
  MessageBox MB_ICONSTOP "$R0" /SD IDOK
  SetErrorLevel 2
  Abort
  Return
usage:
  MessageBox MB_ICONSTOP "$(ScopeUsage)" /SD IDOK
  SetErrorLevel 2
  Abort
two:
  MessageBox MB_ICONSTOP "$(InstalledTwice)" /SD IDOK
  SetErrorLevel 2
  Abort
moved:
  MessageBox MB_ICONSTOP "$(InstalledFolder)" /SD IDOK
  SetErrorLevel 2
  Abort
otherscope:
  StrCmp $R1 "all" 0 +3
  MessageBox MB_ICONSTOP "$(InstalledAll)" /SD IDOK
  Goto +2
  MessageBox MB_ICONSTOP "$(InstalledCurrent)" /SD IDOK
  SetErrorLevel 2
  Abort
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
  StrCmp $Installed "" 0 files
  StrCmp $Scope "all" 0 files
  Call ProtectFolder
  StrCmp $R0 "" 0 failed
files:
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
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "Publisher" "${APP}"
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\InstanceHarbor.exe,0"
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegDWORD SHCTX "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD SHCTX "${UNINSTALL_KEY}" "NoRepair" 1
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
  DeleteRegKey SHCTX "${UNINSTALL_KEY}"
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
  ; Remove only the registered installation that this uninstaller belongs to.
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  StrCmp $0 "" machine
  StrCmp $INSTDIR $0 0 machine
  Return
machine:
  ReadRegStr $0 HKLM "${UNINSTALL_KEY}" "InstallLocation"
  StrCmp $0 "" invalid
  StrCmp $INSTDIR $0 0 invalid
  Call un.IsAdmin
  StrCmp $0 0 0 elevated
  ; An installation for all users needs administrator rights (UAC) to remove. This copy
  ; already runs from the temporary folder, so the elevated copy can delete Uninstall.exe.
  StrCpy $R9 "_?=$INSTDIR"
  IfSilent 0 +2
  StrCpy $R9 "/S $R9"
  Call un.RunElevated
  StrCmp $R0 "denied" 0 +4
  MessageBox MB_ICONSTOP "Removing InstanceHarbor for all users needs administrator approval, which was not given. Nothing was changed." /SD IDOK
  SetErrorLevel 4
  Quit
  StrCmp $R0 "failed" 0 +4
  MessageBox MB_ICONSTOP "The uninstaller could not start with administrator rights. Nothing was changed." /SD IDOK
  SetErrorLevel 2
  Quit
  SetErrorLevel $R0
  Quit
elevated:
  SetShellVarContext all
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
  DeleteRegKey SHCTX "${UNINSTALL_KEY}"
  SetOutPath "$TEMP"
  RMDir "$INSTDIR"
  Goto done
failed:
  System::Call 'kernel32::CloseHandle(p r0)'
  SetErrorLevel 1
  Abort "Program files could not be removed. Workspace data was kept."
busy:
  MessageBox MB_ICONSTOP "InstanceHarbor is still in use, or its installation marker could not be opened. Close InstanceHarbor, including its Manager and Hosts, before retrying. For an installation for all users, this includes InstanceHarbor and servers that other users run." /SD IDOK
  SetErrorLevel 2
  Abort
done:
SectionEnd
