; Compiled and run by tests/verify-windows-setup-version.ps1. It installs nothing: it writes
; one line per case, "case expected actual", to ${RESULTS}. Folder cases only create and
; remove folders inside ${WORK}; the other folders are refused before anything is written.
Unicode true
!addincludedir "..\packaging"
!include "windows-setup-scope.nsh"
Name "InstanceHarbor Setup install scope"
OutFile "${OUTPUT}"
RequestExecutionLevel user
SilentInstall silent

; Registered scope, requested scope, /D= names another folder -> what Setup does.
!macro Decide LABEL REGISTERED WANTED MOVED EXPECTED
  StrCpy $R1 "${REGISTERED}"
  StrCpy $R2 "${WANTED}"
  StrCpy $R3 "${MOVED}"
  Call DecideScope
  FileWrite $0 "decide:${LABEL} ${EXPECTED} $R0$\r$\n"
!macroend

!macro Under LABEL PATH ROOT EXPECTED
  StrCpy $R1 "${PATH}"
  StrCpy $R2 "${ROOT}"
  Call IsUnder
  FileWrite $0 "under:${LABEL} ${EXPECTED} $R0$\r$\n"
!macroend

; An empty EXPECTED means the folder can be used; it is written as "ok".
!macro Folder LABEL SCOPE PATH EXPECTED
  StrCpy $INSTDIR "${PATH}"
  StrCpy $R5 "${SCOPE}"
  StrCpy $R6 "$PROFILE"
  Call CheckInstallFolder
  StrCpy $1 "${EXPECTED}"
  StrCmp $1 "" 0 +2
  StrCpy $1 "ok"
  StrCmp $R0 "" 0 +2
  StrCpy $R0 "ok"
  FileWrite $0 "folder:${LABEL} $1 $R0$\r$\n"
!macroend

Section
  FileOpen $0 "${RESULTS}" w
  !insertmacro Decide new-default "" "" "" fresh-current
  !insertmacro Decide new-current "" current "" fresh-current
  !insertmacro Decide new-all "" all "" fresh-all
  !insertmacro Decide new-both-flags "" both "" usage
  !insertmacro Decide update-current current "" "" update-current
  !insertmacro Decide update-all all "" "" update-all
  !insertmacro Decide update-current-flag current current "" update-current
  !insertmacro Decide update-all-flag all all "" update-all
  !insertmacro Decide current-asks-all current all "" other-scope
  !insertmacro Decide all-asks-current all current "" other-scope
  !insertmacro Decide update-elsewhere current "" 1 moved
  !insertmacro Decide update-all-elsewhere all "" 1 moved
  !insertmacro Decide registered-twice both "" "" two
  !insertmacro Decide flags-before-two both both "" usage

  !insertmacro Under same "C:\Program Files" "C:\Program Files" 1
  !insertmacro Under inside "C:\Program Files\InstanceHarbor" "C:\Program Files" 1
  !insertmacro Under case "c:\program files\InstanceHarbor" "C:\Program Files" 1
  !insertmacro Under sibling "C:\Program Files (x86)\InstanceHarbor" "C:\Program Files" 0
  !insertmacro Under outside "D:\Apps\InstanceHarbor" "C:\Program Files" 0
  !insertmacro Under no-root "D:\Apps" "" 0

  ; Only for the current user.
  !insertmacro Folder new-nested current "${WORK}\new\nested\InstanceHarbor" ""
  !insertmacro Folder empty current "${WORK}\empty" ""
  !insertmacro Folder trailing-slash current "${WORK}\empty\" ""
  !insertmacro Folder dot-dot current "${WORK}\new\..\empty" ""
  !insertmacro Folder not-empty current "${WORK}\used" notempty
  !insertmacro Folder hidden-file current "${WORK}\hidden" notempty
  !insertmacro Folder a-file current "${WORK}\used\other.txt" notempty
  !insertmacro Folder denied current "${WORK}\denied\InstanceHarbor" noaccess
  !insertmacro Folder program-files current "$PROGRAMFILES64\InstanceHarbor-scope-test" protected
  !insertmacro Folder program-files-x86 current "$PROGRAMFILES32\InstanceHarbor-scope-test" protected
  !insertmacro Folder windows current "$WINDIR\InstanceHarbor-scope-test" protected
  !insertmacro Folder relative current "InstanceHarbor" invalid
  !insertmacro Folder rooted current "\InstanceHarbor" invalid
  !insertmacro Folder network current "\\localhost\share\InstanceHarbor-scope-test" invalid
  !insertmacro Folder drive-root current "C:\" invalid
  !insertmacro Folder no-drive current "${MISSING_DRIVE}:\InstanceHarbor-scope-test" invalid
  ; For all users: nothing is written while checking.
  !insertmacro Folder all-program-files all "$PROGRAMFILES64\InstanceHarbor-scope-test" ""
  !insertmacro Folder all-other-drive all "${OUTSIDE}" ""
  !insertmacro Folder all-profile all "${WORK}\new\InstanceHarbor" profile
  !insertmacro Folder all-not-empty all "${WORK}\used" notempty
  FileClose $0
SectionEnd
