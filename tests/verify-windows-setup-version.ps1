# Checks the Setup's version order (update, repair, downgrade), architecture rules and other
# decisions with the NSIS 3.13 compiler.
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

    # What the update wait page says for each probe result.
    $wait = Join-Path $work 'wait-status.exe'
    $statuses = Join-Path $work 'wait-status.txt'
    Push-Location $PSScriptRoot
    try {
        & $Compiler /NOCONFIG /V2 "/DOUTPUT=$wait" "/DRESULTS=$statuses" windows-setup-close.nsi
        if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
    $run = Start-Process -FilePath $wait -ArgumentList '/S' -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "The wait status test installer exited with $($run.ExitCode)." }
    $failures = @(Get-Content -LiteralPath $statuses | Where-Object {
        $parts = $_ -split ' '
        $parts.Count -ne 4 -or $parts[2] -cne $parts[3]
    })
    if ($failures.Count) { throw "Wait status mismatch (servers close expected actual):`n$($failures -join "`n")" }
    "Setup wait status: $(@(Get-Content -LiteralPath $statuses).Count) cases passed."

    # Install scope decisions and folder checks (#38). The folder cases write only below
    # $folders; Program Files, Windows and the folder outside the profile are refused or
    # checked without writing.
    $scope = Join-Path $work 'install-scope.exe'
    $decisions = Join-Path $work 'install-scope.txt'
    $folders = Join-Path $work 'folders'
    foreach ($name in 'empty', 'used', 'hidden', 'denied') { New-Item -ItemType Directory -Path (Join-Path $folders $name) | Out-Null }
    Set-Content -LiteralPath (Join-Path $folders 'used\other.txt') -Value 'not InstanceHarbor'
    $hidden = New-Item -ItemType File -Path (Join-Path $folders 'hidden\desktop.ini')
    $hidden.Attributes = 'Hidden'
    # Without administrator rights the current user cannot create anything in "denied".
    $denied = Join-Path $folders 'denied'
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    & icacls.exe $denied /deny "*${sid}:(OI)(CI)(WD,AD)" /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls could not deny writing to $denied." }
    $outside = Join-Path "$env:SystemDrive\" "InstanceHarbor-scope-test-$PID\InstanceHarbor"
    $missing = [char[]](90..68) | Where-Object { !(Test-Path -LiteralPath "${_}:\") } | Select-Object -First 1
    if (!$missing) { throw 'No unused drive letter for the missing-drive case.' }
    try {
        Push-Location $PSScriptRoot
        try {
            & $Compiler /NOCONFIG /V2 "/DOUTPUT=$scope" "/DRESULTS=$decisions" "/DWORK=$folders" "/DOUTSIDE=$outside" "/DMISSING_DRIVE=$missing" windows-setup-scope.nsi
            if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE." }
        } finally { Pop-Location }
        $run = Start-Process -FilePath $scope -ArgumentList '/S' -Wait -PassThru
        if ($run.ExitCode -ne 0) { throw "The install scope test installer exited with $($run.ExitCode)." }
    } finally {
        & icacls.exe $denied /remove:d "*$sid" /Q | Out-Null
    }
    $failures = @(Get-Content -LiteralPath $decisions | Where-Object {
        $parts = $_ -split ' '
        $parts.Count -ne 3 -or $parts[1] -cne $parts[2]
    })
    if ($failures.Count) { throw "Install scope mismatch (case expected actual):`n$($failures -join "`n")" }
    # A check removes only what it created, and never writes for all users.
    $left = @(Get-ChildItem -LiteralPath $folders -Force -Recurse | ForEach-Object { $_.FullName.Substring($folders.Length + 1) } | Sort-Object)
    $expectedLeft = @('denied', 'empty', 'hidden', 'hidden\desktop.ini', 'used', 'used\other.txt')
    if (Compare-Object $left $expectedLeft) { throw "The folder checks left: $($left -join ', ')" }
    if (Test-Path -LiteralPath (Split-Path -Parent $outside)) { throw "The check for all users created $outside." }
    "Setup install scope: $(@(Get-Content -LiteralPath $decisions).Count) cases passed."

    # Architecture rules (#66): which computers run the files, and switching an installation.
    $arch = Join-Path $work 'architecture.exe'
    $archResults = Join-Path $work 'architecture.txt'
    Push-Location $PSScriptRoot
    try {
        & $Compiler /NOCONFIG /V2 "/DOUTPUT=$arch" "/DRESULTS=$archResults" windows-setup-arch.nsi
        if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
    $run = Start-Process -FilePath $arch -ArgumentList '/S' -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "The architecture test installer exited with $($run.ExitCode)." }
    $failures = @(Get-Content -LiteralPath $archResults | Where-Object {
        $parts = $_ -split ' '
        $parts.Count -ne 3 -or $parts[1] -cne $parts[2]
    })
    if ($failures.Count) { throw "Architecture rule mismatch (case expected actual):`n$($failures -join "`n")" }
    "Setup architecture rules: $(@(Get-Content -LiteralPath $archResults).Count) cases passed."

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
