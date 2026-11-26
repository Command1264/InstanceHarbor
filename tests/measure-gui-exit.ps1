#Requires -Version 7.0
<#
.SYNOPSIS
Opens and closes the development GUI repeatedly and measures how long the GUI process takes to end.

.DESCRIPTION
Uses only the given development workspace under %LOCALAPPDATA%/InstanceHarbor Dev/workspaces.
Each run starts the GUI, waits until its window is shown and it is attached to the workspace's
Manager, then closes it in one of three ways and times the GUI process until it ends:

- AppExit:     `ihctl <workspace> app-exit` (what Setup uses).
- TrayQuit:    the tray menu's Quit command, posted to this workspace's own Manager tray window.
- WindowClose: SC_CLOSE posted to the main window, as the title bar's X button sends it.

Afterwards the GUI logs written during the runs are searched for `gui-exit-watchdog`.
Only GUIs started by this script are timed; nothing else is stopped. The workspace's Manager is
asked to exit at the end.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Workspace,
    [ValidateSet('AppExit', 'TrayQuit', 'WindowClose')][string[]]$Method = @('AppExit', 'TrayQuit', 'WindowClose'),
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../src-tauri/target/x86_64-pc-windows-msvc/dev-channel'),
    [ValidateRange(1, 100)][int]$Runs = 20,
    [ValidateRange(100, 60000)][int]$LimitMs = 2000,
    [string]$BrowserArguments
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$devRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'InstanceHarbor Dev'))
$Workspace = [IO.Path]::GetFullPath($Workspace)
if ([IO.Path]::GetDirectoryName($Workspace) -ne (Join-Path $devRoot 'workspaces')) {
    throw "Workspace must be a direct child of $(Join-Path $devRoot 'workspaces')."
}
$gui = (Resolve-Path -LiteralPath (Join-Path $BinaryDirectory 'InstanceHarborDev.exe')).Path
$cli = (Resolve-Path -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe')).Path
if (!(Test-Path -LiteralPath (Join-Path $Workspace 'workspace.json'))) { & $cli $Workspace init | Out-Null }
$logs = Join-Path $devRoot (Join-Path 'logs' (Split-Path -Leaf $Workspace))

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class IhExit {
    delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int size);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);

    public static IntPtr Find(int pid, string prefix, bool visibleOnly) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint owner; GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid) return true;
            var name = new StringBuilder(256); GetClassNameW(hwnd, name, 256);
            if (!name.ToString().StartsWith(prefix)) return true;
            if (visibleOnly && !IsWindowVisible(hwnd)) return true;
            found = hwnd; return false;
        }, IntPtr.Zero);
        return found;
    }

    // WM_SYSCOMMAND SC_CLOSE: what the title bar's close button sends.
    public static bool CloseButton(IntPtr window) { return PostMessageW(window, 0x0112, (IntPtr)0xF060, IntPtr.Zero); }
    // WM_COMMAND with a muda menu item's id, as TrackPopupMenu delivers a selection.
    public static bool MenuCommand(IntPtr tray, int id) { return PostMessageW(tray, 0x0111, (IntPtr)id, IntPtr.Zero); }
}
'@

# muda numbers menu items in creation order from 1000; manager_tray::run creates
# status (1000), open (1001), quit (1002) first.
$TrayQuitId = 1002

# Windows reports these processes' paths with the \\?\ prefix.
function Get-PlainPath([string]$path) { if ($path) { $path.Replace('\\?\', '') } }

function Get-Manager {
    Get-CimInstance Win32_Process -Filter "Name = 'ihctl.exe'" |
        Where-Object { (Get-PlainPath $_.ExecutablePath) -eq $cli -and (Get-PlainPath $_.CommandLine) -like "*$Workspace*manager-serve*" } | Select-Object -First 1
}

function Wait-Until([scriptblock]$condition, [int]$timeoutMs, [string]$what) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($timeoutMs)
    while (!(& $condition)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "Timed out waiting for $what." }
        Start-Sleep -Milliseconds 50
    }
}

$started = [DateTime]::UtcNow
$results = [Collections.Generic.List[object]]::new()
try {
    foreach ($way in $Method) {
        for ($run = 1; $run -le $Runs; $run++) {
            $start = [Diagnostics.ProcessStartInfo]::new($gui)
            $start.UseShellExecute = $false
            $start.Environment['IH_WORKSPACE'] = $Workspace
            if ($BrowserArguments) { $start.Environment['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = $BrowserArguments }
            $process = [Diagnostics.Process]::Start($start)
            try {
                Wait-Until { [IhExit]::Find($process.Id, 'Tauri Window', $true) -ne [IntPtr]::Zero } 30000 'the GUI window'
                Wait-Until { $null -ne (Get-Manager) } 15000 'the Manager'
                $manager = Get-Manager
                Wait-Until { [IhExit]::Find($manager.ProcessId, 'tray_icon_app', $false) -ne [IntPtr]::Zero } 15000 'the tray window'
                # Let the first attach and the first list read finish, as a user would.
                Start-Sleep -Milliseconds 2500
                $clock = [Diagnostics.Stopwatch]::StartNew()
                switch ($way) {
                    'AppExit' { & $cli $Workspace app-exit | Out-Null }
                    'TrayQuit' {
                        $tray = [IhExit]::Find($manager.ProcessId, 'tray_icon_app', $false)
                        if (![IhExit]::MenuCommand($tray, $TrayQuitId)) { throw 'Could not post the tray Quit command.' }
                    }
                    'WindowClose' {
                        $window = [IhExit]::Find($process.Id, 'Tauri Window', $true)
                        if (![IhExit]::CloseButton($window)) { throw 'Could not post the close command.' }
                    }
                }
                $exited = $process.WaitForExit(20000)
                $elapsed = $clock.Elapsed.TotalMilliseconds
                if (!$exited) { throw "Run $run ($way): GUI $($process.Id) did not exit within 20 s." }
                $results.Add([pscustomobject]@{ method = $way; run = $run; pid = $process.Id; exitMs = [math]::Round($elapsed); ok = $elapsed -le $LimitMs })
                Write-Host ("{0,-11} run {1,2}: {2,6:0} ms" -f $way, $run, $elapsed)
                if ($way -ne 'WindowClose') {
                    # The Manager ends too; wait so the next run starts a fresh one.
                    Wait-Until { $null -eq (Get-Manager) } 15000 'the Manager to exit'
                }
            } finally {
                if (!$process.HasExited) { & $cli $Workspace app-exit 2>$null | Out-Null; $null = $process.WaitForExit(15000) }
            }
        }
    }
} finally {
    if (Get-Manager) { & $cli $Workspace app-exit 2>$null | Out-Null }
}

$watchdog = @(Get-ChildItem -LiteralPath $logs -Filter 'gui-*.jsonl' -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -ge $started } |
    Select-String -SimpleMatch 'gui-exit-watchdog')
$summary = foreach ($group in $results | Group-Object method) {
    $sorted = @($group.Group.exitMs | Sort-Object)
    [pscustomobject]@{
        method = $group.Name; runs = $sorted.Count; overLimit = @($group.Group | Where-Object { !$_.ok }).Count
        minMs = $sorted[0]; medianMs = $sorted[[int][math]::Floor(($sorted.Count - 1) / 2)]; maxMs = $sorted[-1]
    }
}
$summary | Format-Table -AutoSize | Out-String | Write-Host
Write-Host "gui-exit-watchdog records since start: $($watchdog.Count)"
[pscustomobject]@{ limitMs = $LimitMs; summary = $summary; watchdogRecords = $watchdog.Count; runs = $results } | ConvertTo-Json -Depth 4
if (@($results | Where-Object { !$_.ok }).Count -or $watchdog.Count) { exit 1 }
