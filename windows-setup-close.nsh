; What the update wait page of windows-setup.nsi says. tests/windows-setup-close.nsi checks it.
; Setup continues only once the installation marker is free; this only explains the wait.

; $R5: the probe's server count, or "unknown".
; $R6: the probe's close outcome, or "" when Setup did not ask to close.
; $R0: servers, unknown, closing, legacy, busy or other.
Function WaitStatus
  StrCmp $R5 "unknown" unknown
  StrCmp $R5 "0" 0 servers
  StrCmp $R6 "closed" closing
  StrCmp $R6 "legacy" legacy
  StrCmp $R6 "legacy-none" legacy
  StrCmp $R6 "busy" busy
  ; No Manager answers, or it failed: the marker is held by something else.
  StrCpy $R0 "other"
  Return
servers:
  StrCpy $R0 "servers"
  Return
unknown:
  StrCpy $R0 "unknown"
  Return
closing:
  StrCpy $R0 "closing"
  Return
legacy:
  StrCpy $R0 "legacy"
  Return
busy:
  StrCpy $R0 "busy"
FunctionEnd
