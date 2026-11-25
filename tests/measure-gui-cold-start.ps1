#Requires -Version 7.0
<#
.SYNOPSIS
Cold-starts the development GUI several times and prints its `gui-startup` timing marks.

.DESCRIPTION
Uses only the given development workspace under %LOCALAPPDATA%/InstanceHarbor Dev/workspaces.
Starts that workspace's Manager first, so each launch matches a tray-click cold start.
Every GUI this script starts is ended by its own PID, and the workspace's Manager is asked to
exit at the end, so the next run can use binaries from another folder. Nothing else is stopped.
Stops when another development GUI already holds the single-instance lock.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Workspace,
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../src-tauri/target/x86_64-pc-windows-msvc/dev-channel'),
    [ValidateRange(1, 50)][int]$Runs = 7,
    [ValidateRange(5, 120)][int]$TimeoutSeconds = 30,
    [string]$Output,
    # Sets WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS for the GUI, to compare WebView2 flags without a rebuild.
    [string]$BrowserArguments
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$devRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'InstanceHarbor Dev'))
$workspaceRoot = Join-Path $devRoot 'workspaces'
$Workspace = [IO.Path]::GetFullPath($Workspace)
if ([IO.Path]::GetDirectoryName($Workspace) -ne $workspaceRoot) {
    throw "Workspace must be a direct child of $workspaceRoot."
}
$gui = (Resolve-Path -LiteralPath (Join-Path $BinaryDirectory 'InstanceHarborDev.exe')).Path
$cli = (Resolve-Path -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe')).Path
if (!(Test-Path -LiteralPath (Join-Path $Workspace 'workspace.json'))) {
    & $cli $Workspace init | Out-Null
}
# A Manager from another binary folder may still be exiting; retry briefly.
$connectUntil = [DateTime]::UtcNow.AddSeconds(15)
while ($true) {
    $PSNativeCommandUseErrorActionPreference = $false
    & $cli $Workspace manager-connect 2>$null | Out-Null
    $PSNativeCommandUseErrorActionPreference = $true
    if ($LASTEXITCODE -eq 0) { break }
    if ([DateTime]::UtcNow -gt $connectUntil) { throw 'Could not connect to the workspace Manager.' }
    Start-Sleep -Milliseconds 500
}
# Each development workspace writes its GUI logs into its own folder.
$logs = Join-Path $devRoot (Join-Path 'logs' (Split-Path -Leaf $Workspace))

function Read-Startup([string]$path) {
    # The GUI keeps its log open for writing.
    $stream = [IO.FileStream]::new($path, 'Open', 'Read', 'ReadWrite')
    try { $text = [IO.StreamReader]::new($stream).ReadToEnd() } finally { $stream.Dispose() }
    foreach ($line in $text -split "`n" | Where-Object { $_.Trim() }) {
        $entry = $line | ConvertFrom-Json -AsHashtable
        if ($entry.event.type -eq 'gui-startup') { return $entry.event }
        if ($entry.event.type -eq 'gui-duplicate') { throw 'Another development GUI is running; close it and retry.' }
    }
    $null
}

$results = [Collections.Generic.List[object]]::new()
try {
for ($run = 1; $run -le $Runs; $run++) {
    # The development single-instance lock is per channel: wait for other development GUIs, never stop them.
    $waitUntil = [DateTime]::UtcNow.AddMinutes(15)
    while (Get-Process -Name 'InstanceHarborDev' -ErrorAction SilentlyContinue) {
        if ([DateTime]::UtcNow -gt $waitUntil) { throw 'Another development GUI stayed open for 15 minutes.' }
        Start-Sleep -Seconds 2
    }
    $before =@(Get-ChildItem -LiteralPath $logs -Filter 'gui-*.jsonl' -ErrorAction SilentlyContinue).FullName
    $start = [Diagnostics.ProcessStartInfo]::new($gui)
    $start.UseShellExecute = $false
    $start.Environment['IH_WORKSPACE'] = $Workspace
    if ($BrowserArguments) { $start.Environment['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = $BrowserArguments }
    $process = [Diagnostics.Process]::Start($start)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        $startup = $null
        while (!$startup -and [DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 200
            if ($process.HasExited) { throw "GUI exited with code $($process.ExitCode) before reporting its startup." }
            $log = Get-ChildItem -LiteralPath $logs -Filter "gui-$($process.Id)-*.jsonl" -ErrorAction SilentlyContinue |
                Where-Object FullName -notin $before | Select-Object -First 1
            if ($log) { $startup = Read-Startup $log.FullName }
        }
        if (!$startup) { throw "Run ${run}: no gui-startup event within $TimeoutSeconds s." }
        $row = [ordered]@{ run = $run }
        foreach ($set in 'native', 'page') {
            foreach ($mark in $startup[$set].GetEnumerator() | Sort-Object Value) { $row[$mark.Key] = $mark.Value }
        }
        $results.Add([pscustomobject]$row)
        Write-Host ("run {0}: window-shown {1} ms, first list painted {2} ms" -f $run, $row['window-shown'], $row['page-first-list-painted'])
    } finally {
        if (!$process.HasExited) {
            $process.Kill($true)
            $process.WaitForExit(10000) | Out-Null
        }
    }
    # Let the WebView2 browser processes of this GUI exit before the next cold start.
    Start-Sleep -Seconds 3
}
} finally {
    & $cli $Workspace manager-exit | Out-Null
    $exitUntil = [DateTime]::UtcNow.AddSeconds(15)
    while ((Get-Process -Name 'ihctl' -ErrorAction SilentlyContinue | Where-Object Path -eq $cli) -and [DateTime]::UtcNow -lt $exitUntil) {
        Start-Sleep -Milliseconds 200
    }
}

$names = $results | ForEach-Object { $_.PSObject.Properties.Name } | Select-Object -Unique | Where-Object { $_ -ne 'run' }
$summary = foreach ($name in $names) {
    $values = @($results | ForEach-Object { $_.$name } | Where-Object { $null -ne $_ } | Sort-Object)
    if (!$values.Count) { continue }
    $median = if ($values.Count % 2) { $values[[int][Math]::Floor($values.Count / 2)] } else { ($values[$values.Count / 2 - 1] + $values[$values.Count / 2]) / 2 }
    [pscustomobject]@{ mark = $name; median = $median; min = $values[0]; max = $values[-1] }
}
$summary | Sort-Object median | Format-Table -AutoSize | Out-Host
if ($Output) {
    @{ runs = $results; summary = $summary } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Output -Encoding utf8
}
