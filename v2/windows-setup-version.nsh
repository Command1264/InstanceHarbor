; Version order for windows-setup.nsi. tests/windows-setup-version.nsi checks it.
!include "WordFunc.nsh"

Var VersionCore
Var VersionPre

; $R9 "1.2.3" or "1.2.3-rc2" -> $VersionCore "1.2.3" and $VersionPre "" or "rc2".
Function SplitVersion
  StrCpy $R8 0
loop:
  StrCpy $R7 $R9 1 $R8
  StrCmp $R7 "" whole
  StrCmp $R7 "-" found
  IntOp $R8 $R8 + 1
  Goto loop
found:
  StrCpy $VersionCore $R9 $R8
  IntOp $R8 $R8 + 1
  StrCpy $VersionPre $R9 "" $R8
  Return
whole:
  StrCpy $VersionCore $R9
  StrCpy $VersionPre ""
FunctionEnd

; Compares the installed version $R5 with this Setup's version $R6.
; $R0: 0 same, 1 installed is older (update), 2 installed is newer (downgrade).
Function CompareVersions
  StrCpy $R9 $R5
  Call SplitVersion
  StrCpy $R1 $VersionCore
  StrCpy $R2 $VersionPre
  StrCpy $R9 $R6
  Call SplitVersion
  ; VersionCompare: 0 equal, 1 the installed version is newer, 2 this Setup is newer.
  ${VersionCompare} $R1 $VersionCore $R0
  StrCmp $R0 "1" newer
  StrCmp $R0 "2" older
  ; Same x.y.z: a release is newer than its release candidates (1.0.0 > 1.0.0-rc2).
  StrCmpS $R2 $VersionPre done
  StrCmp $R2 "" newer
  StrCmp $VersionPre "" older
  StrCpy $R3 $R2 2
  StrCpy $R4 $VersionPre 2
  ; An unknown pre-release order is confirmed like a downgrade.
  StrCmpS "$R3$R4" "rcrc" 0 newer
  StrCpy $R3 $R2 "" 2
  StrCpy $R4 $VersionPre "" 2
  IntCmp $R3 $R4 newer older newer
older:
  StrCpy $R0 1
  Return
newer:
  StrCpy $R0 2
  Return
done:
FunctionEnd
