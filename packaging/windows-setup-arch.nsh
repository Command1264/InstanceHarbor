; Architecture rules for windows-setup.nsi (#66). tests/windows-setup-arch.nsi checks them.
; Setup itself is a 32-bit x86 program on every computer; ${ARCH} names the architecture of
; the files it installs: x64 or arm64.

; IMAGE_FILE_MACHINE_* values that IsWow64Process2 reports for the native machine.
!define MACHINE_AMD64 34404
!define MACHINE_ARM64 43620
; Windows 11 is the first Windows on ARM that runs x64 programs (emulation).
!define WINDOWS_11_BUILD 22000

; $R5: the architecture of the installed files (x64 or arm64).
; $R6: the native machine (IMAGE_FILE_MACHINE_*). $R7: the Windows build number.
; $R0: "native"     the files run natively;
;      "emulated"   x64 files on Windows 11 on ARM, which runs them through emulation;
;      "x64-needed" x64 files on a computer that cannot run them;
;      "arm64-needed" arm64 files on a computer that is not ARM64.
Function DecideArchitecture
  ${If} $R5 == "arm64"
    ${If} $R6 = ${MACHINE_ARM64}
      StrCpy $R0 "native"
    ${Else}
      StrCpy $R0 "arm64-needed"
    ${EndIf}
    Return
  ${EndIf}
  ${If} $R6 = ${MACHINE_AMD64}
    StrCpy $R0 "native"
  ${ElseIf} $R6 = ${MACHINE_ARM64}
  ${AndIf} $R7 >= ${WINDOWS_11_BUILD}
    StrCpy $R0 "emulated"
  ${Else}
    StrCpy $R0 "x64-needed"
  ${EndIf}
FunctionEnd

; $R5: the "Architecture" an installation registered; empty before #66, when every
; installation was x64. $R6: the architecture this Setup installs.
; $R0: the installed architecture, x64 or arm64; $R1: "same" or "switch".
Function CompareArchitecture
  StrCpy $R0 $R5
  StrCmp $R0 "" 0 +2
  StrCpy $R0 "x64"
  StrCpy $R1 "same"
  StrCmp $R0 $R6 +2
  StrCpy $R1 "switch"
FunctionEnd
