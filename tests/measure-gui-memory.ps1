#Requires -Version 7.0
<#
.SYNOPSIS
Measures the memory and CPU of a development GUI and its WebView2 processes, shown and hidden (#56).

.DESCRIPTION
Uses only the given development workspace under %LOCALAPPDATA%/InstanceHarbor Dev/workspaces and
only GUIs this script starts. Each run starts the GUI, waits until its window is shown and the
first list is read, lets it settle, then samples the GUI process and every process below it
except the Manager (the WebView2 browser, renderer, GPU and utility processes): working set and private bytes, and CPU time over `SampleSeconds`.

With -Hide (the workspace must have fast open turned on), the window is then closed with the
title bar's X, which only hides it in fast open mode, and the same values are sampled again
after `SettleSeconds`. -Reopen then uses the tray's Open and times how long the window takes to
show again. The GUI is closed with `ihctl app-exit` at the end of each run.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Workspace,
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../src-tauri/target/x86_64-pc-windows-msvc/dev-channel'),
    [ValidateRange(1, 50)][int]$Runs = 3,
    [ValidateRange(1, 300)][int]$SettleSeconds = 10,
    [ValidateRange(1, 300)][int]$SampleSeconds = 20,
    [switch]$Hide,
    [switch]$Reopen
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

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class IhMemory {
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
    public static bool CloseButton(IntPtr window) { return PostMessageW(window, 0x0112, (IntPtr)0xF060, IntPtr.Zero); }
    public static bool MenuCommand(IntPtr tray, int id) { return PostMessageW(tray, 0x0111, (IntPtr)id, IntPtr.Zero); }
}
'@
$TrayOpenId = 1001

function Get-PlainPath([string]$path) { if ($path) { $path.Replace('\\?\', '') } }

function Wait-Until([scriptblock]$condition, [int]$timeoutMs, [string]$what) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($timeoutMs)
    while (!(& $condition)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "Timed out waiting for $what." }
        Start-Sleep -Milliseconds 50
    }
}

# The GUI and all processes below it (WebView2 browser, renderer, GPU, utility). The Manager
# (ihctl.exe) the GUI may have started is left out: it runs whether or not a GUI is open.
function Get-Tree([int]$root) {
    $all = @(Get-CimInstance Win32_Process | Where-Object Name -ne 'ihctl.exe')
    $ids = [Collections.Generic.HashSet[int]]::new(); $null = $ids.Add($root)
    do {
        $added = $false
        foreach ($process in $all) {
            if ($ids.Contains([int]$process.ParentProcessId) -and $ids.Add([int]$process.ProcessId)) { $added = $true }
        }
    } while ($added)
    @($ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
}

function Measure-Tree([int]$root, [int]$seconds) {
    $first = Get-Tree $root
    $cpuStart = [TimeSpan]::Zero
    foreach ($process in $first) { $cpuStart += $process.TotalProcessorTime }
    Start-Sleep -Seconds $seconds
    $last = Get-Tree $root
    $cpuEnd = [TimeSpan]::Zero
    foreach ($process in $last) { $process.Refresh(); $cpuEnd += $process.TotalProcessorTime }
    [pscustomobject]@{
        processes = $last.Count
        workingSetMB = [math]::Round(($last | Measure-Object -Property WorkingSet64 -Sum).Sum / 1MB, 1)
        privateMB = [math]::Round(($last | Measure-Object -Property PrivateMemorySize64 -Sum).Sum / 1MB, 1)
        guiPrivateMB = [math]::Round(($last | Where-Object Id -eq $root).PrivateMemorySize64 / 1MB, 1)
        # Share of one CPU core over the sample.
        cpuPercent = [math]::Round(($cpuEnd - $cpuStart).TotalMilliseconds / ($seconds * 10), 2)
    }
}

function Get-Manager {
    Get-CimInstance Win32_Process -Filter "Name = 'ihctl.exe'" |
        Where-Object { (Get-PlainPath $_.ExecutablePath) -eq $cli -and (Get-PlainPath $_.CommandLine) -like "*$Workspace*manager-serve*" } | Select-Object -First 1
}

$results = [Collections.Generic.List[object]]::new()
for ($run = 1; $run -le $Runs; $run++) {
    $start = [Diagnostics.ProcessStartInfo]::new($gui)
    $start.UseShellExecute = $false
    $start.Environment['IH_WORKSPACE'] = $Workspace
    $process = [Diagnostics.Process]::Start($start)
    try {
        Wait-Until { [IhMemory]::Find($process.Id, 'Tauri Window', $true) -ne [IntPtr]::Zero } 30000 'the GUI window'
        Start-Sleep -Seconds $SettleSeconds
        $row = [ordered]@{ run = $run; shown = Measure-Tree $process.Id $SampleSeconds }
        if ($Hide) {
            $null = [IhMemory]::CloseButton([IhMemory]::Find($process.Id, 'Tauri Window', $true))
            Wait-Until { [IhMemory]::Find($process.Id, 'Tauri Window', $true) -eq [IntPtr]::Zero } 10000 'the window to hide'
            if ($process.HasExited) { throw 'The GUI exited on X; is fast open turned on for this workspace?' }
            Start-Sleep -Seconds $SettleSeconds
            $row.hidden = Measure-Tree $process.Id $SampleSeconds
            if ($Reopen) {
                Wait-Until { $null -ne (Get-Manager) } 15000 'the Manager'
                $tray = [IhMemory]::Find((Get-Manager).ProcessId, 'tray_icon_app', $false)
                $clock = [Diagnostics.Stopwatch]::StartNew()
                $null = [IhMemory]::MenuCommand($tray, $TrayOpenId)
                Wait-Until { [IhMemory]::Find($process.Id, 'Tauri Window', $true) -ne [IntPtr]::Zero } 10000 'the window to show again'
                $row.reopenMs = [math]::Round($clock.Elapsed.TotalMilliseconds)
            }
        }
        $results.Add([pscustomobject]$row)
        Write-Host ($row | ConvertTo-Json -Compress -Depth 3)
    } finally {
        if (!$process.HasExited) {
            & $cli $Workspace app-exit 2>$null | Out-Null
            if (!$process.WaitForExit(15000)) { $process.Kill($true) }
        }
    }
    Start-Sleep -Seconds 3
}
$results | ConvertTo-Json -Depth 4
