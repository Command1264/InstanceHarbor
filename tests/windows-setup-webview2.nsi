; Compiled and run by tests/verify-windows-setup-version.ps1. It installs nothing:
; it writes the WebView2 runtime that Setup would find on this computer to ${RESULTS}.
Unicode true
!addincludedir ".."
!include "windows-setup-webview2.nsh"
Name "InstanceHarbor Setup WebView2 detection"
OutFile "${OUTPUT}"
RequestExecutionLevel user
SilentInstall silent

Section
  SetRegView 64
  Call FindWebView2
  FileOpen $1 "${RESULTS}" w
  FileWrite $1 "$0"
  FileClose $1
SectionEnd
