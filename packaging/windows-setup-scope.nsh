; Install scope and folder rules for windows-setup.nsi. tests/windows-setup-scope.nsi checks them.
!include "FileFunc.nsh"

; Decides what Setup does from the registered installation and the command line.
; $R1: the registered scope: "", "current", "all" or "both" (one in HKCU and one in HKLM).
; $R2: the requested scope: "", "current", "all" or "both" (/CurrentUser and /AllUsers).
; $R3: "1" when /D= names a folder other than the registered one.
; $R0: fresh-current, fresh-all, update-current, update-all, usage, two, other-scope or moved.
Function DecideScope
  StrCmp $R2 "both" usage
  StrCmp $R1 "both" two
  StrCmp $R1 "" fresh
  ; An update, repair or downgrade keeps the registered scope and folder.
  StrCmp $R2 "" same
  StrCmp $R2 $R1 same
  StrCpy $R0 "other-scope"
  Return
same:
  StrCmp $R3 "1" 0 +3
  StrCpy $R0 "moved"
  Return
  StrCpy $R0 "update-$R1"
  Return
fresh:
  StrCmp $R2 "all" 0 +3
  StrCpy $R0 "fresh-all"
  Return
  StrCpy $R0 "fresh-current"
  Return
usage:
  StrCpy $R0 "usage"
  Return
two:
  StrCpy $R0 "two"
FunctionEnd

; $R0: "1" when the folder $R1 is $R2 or inside it, otherwise "0". Not case-sensitive.
Function IsUnder
  StrCpy $R0 "0"
  StrCmp $R2 "" done
  StrLen $R3 $R2
  StrCpy $R4 $R1 $R3
  StrCmp $R4 $R2 0 done
  StrCpy $R4 $R1 1 $R3
  StrCmp $R4 "" under
  StrCmp $R4 "\" 0 done
under:
  StrCpy $R0 "1"
done:
FunctionEnd

!macro IsUnderRoot ROOT LABEL
  StrCpy $R2 "${ROOT}"
  Call IsUnder
  StrCmp $R0 "1" ${LABEL}
!macroend

; Checks the folder for a new installation and makes $INSTDIR a full path.
; $R5: "current" or "all". $R6: the user profile folder ($PROFILE).
; $R0: "" when the folder can be used, otherwise invalid, notempty, protected, noaccess or profile.
; For the current user, the folder must be writable without administrator rights: Setup
; refuses system folders and tries to create the folder and a file in it, then removes
; only what the try created.
Function CheckInstallFolder
  Push $R1
  Push $R2
  Push $R3
  Push $R4
  Push $R7
  Push $R8
  StrCpy $R7 ""
  ; Only a folder on a local drive, such as D:\Apps\InstanceHarbor; never a relative path,
  ; a network path or a drive root.
  StrCpy $R2 $INSTDIR 2 1
  StrCmp $R2 ":\" 0 invalid
  System::Call 'kernel32::GetFullPathNameW(w "$INSTDIR", i ${NSIS_MAX_STRLEN}, w .R1, p 0) i .R2'
  IntCmp $R2 0 invalid
  IntCmp $R2 ${NSIS_MAX_STRLEN} invalid 0 invalid
  StrCpy $R2 $R1 2 1
  StrCmp $R2 ":\" 0 invalid
  StrCpy $R2 $R1 1 -1
  StrCmp $R2 "\" 0 +2
  StrCpy $R1 $R1 -1
  StrLen $R2 $R1
  IntCmp $R2 3 invalid invalid
  StrCpy $R2 $R1 3
  System::Call 'kernel32::GetDriveTypeW(w "$R2") i .R3'
  IntCmp $R3 3 local
  IntCmp $R3 2 local
  Goto invalid
local:
  StrCpy $INSTDIR $R1
  ; An existing file, or a folder with anything in it, is never used.
  IfFileExists "$R1\*.*" folder
  IfFileExists "$R1" notempty
  Goto scope
folder:
  FindFirst $R3 $R4 "$R1\*"
next:
  StrCmp $R4 "" empty
  StrCmp $R4 "." skip
  StrCmp $R4 ".." skip
  FindClose $R3
  Goto notempty
skip:
  FindNext $R3 $R4
  Goto next
empty:
  FindClose $R3
scope:
  StrCmp $R5 "all" all
  !insertmacro IsUnderRoot "$PROGRAMFILES64" protected
  !insertmacro IsUnderRoot "$PROGRAMFILES32" protected
  !insertmacro IsUnderRoot "$WINDIR" protected
  ; The nearest folder that already exists; Setup removes only what it creates below it.
  StrCpy $R8 $R1
find:
  IfFileExists "$R8\*.*" found
  ${GetParent} $R8 $R8
  StrCmp $R8 "" noaccess
  Goto find
found:
  ClearErrors
  CreateDirectory "$R1"
  IfErrors cleanup
  FileOpen $R3 "$R1\instanceharbor.write-test" w
  IfErrors cleanup
  FileClose $R3
  Delete "$R1\instanceharbor.write-test"
  StrCpy $R7 "ok"
cleanup:
  StrCpy $R2 $R1
remove:
  StrCmp $R2 $R8 removed
  StrCmp $R2 "" removed
  RMDir "$R2"
  ${GetParent} $R2 $R2
  Goto remove
removed:
  StrCmp $R7 "ok" usable noaccess
all:
  ; Every user must be able to reach the program, so not inside a user profile.
  ${GetParent} $R6 $R2
  Call IsUnder
  StrCmp $R0 "1" profile
usable:
  StrCpy $R0 ""
  Goto done
invalid:
  StrCpy $R0 "invalid"
  Goto done
notempty:
  StrCpy $R0 "notempty"
  Goto done
protected:
  StrCpy $R0 "protected"
  Goto done
noaccess:
  StrCpy $R0 "noaccess"
  Goto done
profile:
  StrCpy $R0 "profile"
done:
  Pop $R8
  Pop $R7
  Pop $R4
  Pop $R3
  Pop $R2
  Pop $R1
FunctionEnd
