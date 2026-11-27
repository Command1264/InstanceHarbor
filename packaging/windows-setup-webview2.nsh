; WebView2 detection for windows-setup.nsi. tests/windows-setup-webview2.nsi checks it.
!define WEBVIEW2_CLIENT "Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

; $0: the found msedgewebview2.exe, or "" when the runtime is missing.
; Microsoft's documented detection is a non-empty pv other than 0.0.0.0, per machine
; or per user. A registry entry alone is not enough: that version's runtime must also
; be on disk, which is what InstanceHarbor's own start-up check finds.
Function FindWebView2
  ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\${WEBVIEW2_CLIENT}" "pv"
  StrCmp $0 "" user
  StrCmp $0 "0.0.0.0" user
  StrCpy $0 "$PROGRAMFILES32\Microsoft\EdgeWebView\Application\$0\msedgewebview2.exe"
  IfFileExists $0 0 user
  Return
user:
  ReadRegStr $0 HKCU "Software\${WEBVIEW2_CLIENT}" "pv"
  StrCmp $0 "" missing
  StrCmp $0 "0.0.0.0" missing
  StrCpy $0 "$LOCALAPPDATA\Microsoft\EdgeWebView\Application\$0\msedgewebview2.exe"
  IfFileExists $0 0 missing
  Return
missing:
  StrCpy $0 ""
FunctionEnd
