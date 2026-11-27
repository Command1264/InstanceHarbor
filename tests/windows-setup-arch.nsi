; Compiled and run by tests/verify-windows-setup-version.ps1. It installs nothing:
; it writes one line per case, "case expected actual", to ${RESULTS}.
Unicode true
!addincludedir "..\packaging"
!include "LogicLib.nsh"
!include "windows-setup-arch.nsh"
Name "InstanceHarbor Setup architecture rules"
OutFile "${OUTPUT}"
RequestExecutionLevel user
SilentInstall silent

!macro Decide CASE FILES MACHINE BUILD EXPECTED
  StrCpy $R5 "${FILES}"
  StrCpy $R6 "${MACHINE}"
  StrCpy $R7 "${BUILD}"
  Call DecideArchitecture
  FileWrite $0 "${CASE} ${EXPECTED} $R0$\r$\n"
!macroend

!macro Compare CASE INSTALLED SETUP EXPECTED
  StrCpy $R5 "${INSTALLED}"
  StrCpy $R6 "${SETUP}"
  Call CompareArchitecture
  FileWrite $0 "${CASE} ${EXPECTED} $R0/$R1$\r$\n"
!macroend

Section
  FileOpen $0 "${RESULTS}" w
  !insertmacro Decide x64-on-x64 x64 ${MACHINE_AMD64} 19045 native
  !insertmacro Decide x64-on-arm64-win11 x64 ${MACHINE_ARM64} 22000 emulated
  !insertmacro Decide x64-on-arm64-win10 x64 ${MACHINE_ARM64} 19045 x64-needed
  !insertmacro Decide x64-on-x86 x64 332 19045 x64-needed
  !insertmacro Decide arm64-on-arm64 arm64 ${MACHINE_ARM64} 22631 native
  !insertmacro Decide arm64-on-x64 arm64 ${MACHINE_AMD64} 26100 arm64-needed
  !insertmacro Decide arm64-on-x86 arm64 332 19045 arm64-needed
  !insertmacro Compare unrecorded-x64 "" x64 x64/same
  !insertmacro Compare unrecorded-arm64 "" arm64 x64/switch
  !insertmacro Compare x64-x64 x64 x64 x64/same
  !insertmacro Compare x64-arm64 x64 arm64 x64/switch
  !insertmacro Compare arm64-x64 arm64 x64 arm64/switch
  !insertmacro Compare arm64-arm64 arm64 arm64 arm64/same
  FileClose $0
SectionEnd
