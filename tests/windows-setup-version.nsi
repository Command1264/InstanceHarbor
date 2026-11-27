; Compiled and run by tests/verify-windows-setup-version.ps1. It installs nothing:
; it writes one line per case, "installed setup expected actual", to ${RESULTS}.
Unicode true
!addincludedir "..\packaging"
!include "windows-setup-version.nsh"
Name "InstanceHarbor Setup version order"
OutFile "${OUTPUT}"
RequestExecutionLevel user
SilentInstall silent

!macro Case INSTALLED SETUP EXPECTED
  StrCpy $R5 "${INSTALLED}"
  StrCpy $R6 "${SETUP}"
  Call CompareVersions
  FileWrite $0 "${INSTALLED} ${SETUP} ${EXPECTED} $R0$\r$\n"
!macroend

Section
  FileOpen $0 "${RESULTS}" w
  !insertmacro Case "1.0.0" "1.0.0" 0
  !insertmacro Case "1.0.0-rc2" "1.0.0-rc2" 0
  !insertmacro Case "1.0.0-rc2" "1.0.0" 1
  !insertmacro Case "1.0.0" "1.0.0-rc2" 2
  !insertmacro Case "1.0.0-rc2" "1.0.0-rc10" 1
  !insertmacro Case "1.0.0-rc10" "1.0.0-rc2" 2
  !insertmacro Case "1.0.0" "1.0.1" 1
  !insertmacro Case "1.0.1" "1.0.0" 2
  !insertmacro Case "1.9.0" "1.10.0" 1
  !insertmacro Case "2.0.0" "1.10.0" 2
  !insertmacro Case "1.0.0-rc2" "1.1.0-rc1" 1
  !insertmacro Case "1.0.0-beta" "1.0.0-rc1" 2
  FileClose $0
SectionEnd
