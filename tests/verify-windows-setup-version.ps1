# Checks the Setup's version order (update, repair, downgrade) with the NSIS 3.13 compiler.
# The test installer writes its results to a temporary folder and installs nothing.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Compiler)
$ErrorActionPreference = 'Stop'
$work = Join-Path ([IO.Path]::GetTempPath()) "ih-setup-version-$PID"
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $exe = Join-Path $work 'version-order.exe'
    $results = Join-Path $work 'results.txt'
    Push-Location $PSScriptRoot
    try {
        & $Compiler /NOCONFIG /V2 "/DOUTPUT=$exe" "/DRESULTS=$results" windows-setup-version.nsi
        if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
    $run = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "The version test installer exited with $($run.ExitCode)." }
    $failures = @(Get-Content -LiteralPath $results | Where-Object {
        $parts = $_ -split ' '
        $parts.Count -ne 4 -or $parts[2] -cne $parts[3]
    })
    if ($failures.Count) { throw "Version order mismatch (installed setup expected actual):`n$($failures -join "`n")" }
    "Setup version order: $(@(Get-Content -LiteralPath $results).Count) cases passed."

    # Setup must find the same runtime that InstanceHarbor uses on this computer.
    $detect = Join-Path $work 'webview2.exe'
    $found = Join-Path $work 'webview2.txt'
    Push-Location $PSScriptRoot
    try {
        & $Compiler /NOCONFIG /V2 "/DOUTPUT=$detect" "/DRESULTS=$found" windows-setup-webview2.nsi
        if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
    $run = Start-Process -FilePath $detect -ArgumentList '/S' -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "The WebView2 test installer exited with $($run.ExitCode)." }
    $client = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    $expected = ''
    $machine = (Get-ItemProperty -LiteralPath "HKLM:\SOFTWARE\WOW6432Node\$client" -ErrorAction SilentlyContinue).pv
    $user = (Get-ItemProperty -LiteralPath "HKCU:\Software\$client" -ErrorAction SilentlyContinue).pv
    if ($machine -and $machine -ne '0.0.0.0' -and (Test-Path -LiteralPath "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application\$machine\msedgewebview2.exe")) {
        $expected = "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application\$machine\msedgewebview2.exe"
    } elseif ($user -and $user -ne '0.0.0.0' -and (Test-Path -LiteralPath "$env:LOCALAPPDATA\Microsoft\EdgeWebView\Application\$user\msedgewebview2.exe")) {
        $expected = "$env:LOCALAPPDATA\Microsoft\EdgeWebView\Application\$user\msedgewebview2.exe"
    }
    $actual = [IO.File]::ReadAllText($found)
    if ($actual -ne $expected) { throw "Setup found WebView2 '$actual'; expected '$expected'." }
    "Setup WebView2 detection: $(if ($actual) { $actual } else { 'missing' })"
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force
}
