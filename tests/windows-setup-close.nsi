; Compiled and run by tests/verify-windows-setup-version.ps1. It installs nothing:
; it writes one line per case, "servers close expected actual", to ${RESULTS}.
Unicode true
!addincludedir ".."
!include "windows-setup-close.nsh"
Name "InstanceHarbor Setup wait status"
OutFile "${OUTPUT}"
RequestExecutionLevel user
SilentInstall silent

!macro Case SERVERS CLOSE EXPECTED
  StrCpy $R5 "${SERVERS}"
  StrCpy $R6 "${CLOSE}"
  Call WaitStatus
  FileWrite $0 "${SERVERS} ${CLOSE}- ${EXPECTED} $R0$\r$\n"
!macroend

Section
  FileOpen $0 "${RESULTS}" w
  ; Setup asks to close only when no server runs.
  !insertmacro Case "2" "" servers
  !insertmacro Case "1" "" servers
  !insertmacro Case "unknown" "" unknown
  !insertmacro Case "0" "closed" closing
  !insertmacro Case "0" "legacy" legacy
  !insertmacro Case "0" "legacy-none" legacy
  !insertmacro Case "0" "busy" busy
  !insertmacro Case "0" "none" other
  !insertmacro Case "0" "failed" other
  FileClose $0
SectionEnd
