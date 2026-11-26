#Requires -Version 7.0
<#
.SYNOPSIS
Checks that a development GUI stuck before its first window no longer blocks the next open (#117).

.DESCRIPTION
Uses only the given development workspace under %LOCALAPPDATA%/InstanceHarbor Dev/workspaces and
only GUIs this script starts. A GUI is started with IH_DEV_SIMULATE_WEBVIEW2 (development builds
only): `hang` keeps WebView2 environment creation from completing, `exit` ends the WebView2 browser
process right after the controller is created, `hang-deaf` hangs as `hang` and also ignores a
takeover's quit request. Then, per case:

- Deadline: the stuck GUI must end itself 20 s after it started and log `gui-startup-deadline`;
  the tray's Open then shows a GUI window.
- Takeover: 16 s after the start the tray's Open is used; the new GUI must end the stuck one
  (`gui-takeover`) and show its window.
- Young:    the tray's Open 3 s after the start must leave the stuck GUI alone (`gui-duplicate`).
- Shown:    with a normal GUI shown for 16 s, the tray's Open must focus it, not end it.

The tray's Open is the menu command posted to this workspace's own Manager tray window.
The workspace's Manager is asked to exit at the end.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Workspace,
    [ValidateSet('hang', 'hang-deaf', 'exit')][string[]]$Simulate = @('hang', 'hang-deaf', 'exit'),
    [ValidateSet('Deadline', 'Takeover', 'Young', 'Shown')][string[]]$Case = @('Deadline', 'Takeover', 'Young', 'Shown'),
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../src-tauri/target/x86_64-pc-windows-msvc/dev-channel'),
    # A GUI window must be shown this long after the tray's Open: the startup deadline (20 s).
    [ValidateRange(1000, 60000)][int]$OpenLimitMs = 20000
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

public static class IhStuck {
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

# muda numbers menu items in creation order from 1000: status (1000), open (1001), quit (1002).
$TrayOpenId = 1001

function Get-PlainPath([string]$path) { if ($path) { $path.Replace('\\?\', '') } }

function Get-Manager {
    Get-CimInstance Win32_Process -Filter "Name = 'ihctl.exe'" |
        Where-Object { (Get-PlainPath $_.ExecutablePath) -eq $cli -and (Get-PlainPath $_.CommandLine) -like "*$Workspace*manager-serve*" } | Select-Object -First 1
}

# GUI processes of this build (not native-console viewers).
function Get-Guis {
    @(Get-CimInstance Win32_Process -Filter "Name = 'InstanceHarborDev.exe'" |
        Where-Object { (Get-PlainPath $_.ExecutablePath) -eq $gui -and $_.CommandLine -notlike '*native-console*' })
}

function Wait-Until([scriptblock]$condition, [int]$timeoutMs, [string]$what) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($timeoutMs)
    while (!(& $condition)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "Timed out waiting for $what." }
        Start-Sleep -Milliseconds 50
    }
}

function Start-Gui([string]$simulation) {
    $start = [Diagnostics.ProcessStartInfo]::new($gui)
    $start.UseShellExecute = $false
    $start.Environment['IH_WORKSPACE'] = $Workspace
    if ($simulation) { $start.Environment['IH_DEV_SIMULATE_WEBVIEW2'] = $simulation }
    [Diagnostics.Process]::Start($start)
}

function Open-FromTray {
    $manager = Get-Manager
    if (!$manager) { throw 'The Manager is not running.' }
    $tray = [IhStuck]::Find($manager.ProcessId, 'tray_icon_app', $false)
    if (![IhStuck]::MenuCommand($tray, $TrayOpenId)) { throw 'Could not post the tray Open command.' }
}

# Waits for a GUI other than $except to show its window; returns its process id.
function Wait-ShownGui([int[]]$except, [int]$timeoutMs) {
    $script:shown = 0
    Wait-Until {
        foreach ($candidate in Get-Guis) {
            if ($except -notcontains $candidate.ProcessId -and [IhStuck]::Find($candidate.ProcessId, 'Tauri Window', $true) -ne [IntPtr]::Zero) {
                $script:shown = $candidate.ProcessId; return $true
            }
        }
        $false
    } $timeoutMs 'a GUI window'
    $script:shown
}

# Log files are named gui-<yyyyMMdd-HHmmss>-<pid>-<segment>.jsonl; 0 reads every GUI's.
function Get-Records([int]$processId, [DateTime]$since) {
    @(Get-ChildItem -LiteralPath $logs -Filter 'gui-*.jsonl' -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTimeUtc -ge $since -and (!$processId -or $_.Name -match "^gui-\d{8}-\d{6}-$processId-\d+\.jsonl$") } |
        ForEach-Object { Get-Content -LiteralPath $_.FullName } |
        ForEach-Object { try { ($_ | ConvertFrom-Json).event } catch { } })
}

function Close-Gui([int]$processId) {
    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    if (!$process) { return }
    $window = [IhStuck]::Find($processId, 'Tauri Window', $true)
    if ($window -ne [IntPtr]::Zero) { $null = [IhStuck]::CloseButton($window) }
    if (!$process.WaitForExit(15000)) { throw "GUI $processId did not close." }
}

# Every GUI this script started or caused, so the finally block only ends those.
$ours = [Collections.Generic.HashSet[int]]::new()
$results = [Collections.Generic.List[object]]::new()
try {
    if (@(Get-Guis).Count) { throw 'A GUI of this build is already running; close it first.' }
    # A Manager started by a normal GUI, so tray-opened GUIs inherit no simulation switch.
    $first = Start-Gui ''
    $null = $ours.Add($first.Id)
    $null = Wait-ShownGui @() 30000
    Wait-Until { $null -ne (Get-Manager) } 15000 'the Manager'
    Wait-Until { [IhStuck]::Find((Get-Manager).ProcessId, 'tray_icon_app', $false) -ne [IntPtr]::Zero } 15000 'the tray window'
    Close-Gui $first.Id

    foreach ($simulation in $Simulate) {
        foreach ($way in $Case) {
            $since = [DateTime]::UtcNow
            $stuck = Start-Gui $(if ($way -eq 'Shown') { '' } else { $simulation })
            $null = $ours.Add($stuck.Id)
            $result = [ordered]@{ simulate = $(if ($way -eq 'Shown') { 'none' } else { $simulation }); case = $way; ok = $false }
            try {
                switch ($way) {
                    'Deadline' {
                        if (!$stuck.WaitForExit(30000)) { throw 'The stuck GUI did not end itself.' }
                        $result.stuckEndedMs = [math]::Round(($stuck.ExitTime.ToUniversalTime() - $stuck.StartTime.ToUniversalTime()).TotalMilliseconds)
                        $deadline = @(Get-Records $stuck.Id $since | Where-Object type -eq 'gui-startup-deadline')
                        $result.stage = $deadline[0].stage
                        $result.webview = @($deadline[0].webview | ForEach-Object { "$($_.step):$($_.phase)$($_.kind)" }) -join ', '
                        $clock = [Diagnostics.Stopwatch]::StartNew()
                        Open-FromTray
                        $opened = Wait-ShownGui @($stuck.Id) 30000
                        $null = $ours.Add($opened)
                        $result.openMs = [math]::Round($clock.Elapsed.TotalMilliseconds)
                        $result.ok = $deadline.Count -eq 1 -and $result.stuckEndedMs -ge 20000 -and $result.stuckEndedMs -lt 23000 -and $result.openMs -le $OpenLimitMs
                        Close-Gui $opened
                    }
                    'Takeover' {
                        Start-Sleep -Milliseconds (16000 - [math]::Min(16000, ([DateTime]::Now - $stuck.StartTime).TotalMilliseconds))
                        $clock = [Diagnostics.Stopwatch]::StartNew()
                        Open-FromTray
                        $opened = Wait-ShownGui @($stuck.Id) 30000
                        $null = $ours.Add($opened)
                        $result.openMs = [math]::Round($clock.Elapsed.TotalMilliseconds)
                        $result.stuckEnded = $stuck.WaitForExit(1000)
                        $takeover = @(Get-Records $opened $since | Where-Object type -eq 'gui-takeover')
                        $result.ended = $takeover[0].ended
                        # A GUI that ignores the request is ended after the 2 s grace period.
                        $expected = if ($simulation -eq 'hang-deaf') { 'terminated' } else { 'quit' }
                        $result.ok = $result.stuckEnded -and $takeover.Count -eq 1 -and $takeover[0].lockTaken -and $takeover[0].ended -eq $expected -and $result.openMs -le $OpenLimitMs
                        Close-Gui $opened
                    }
                    'Young' {
                        Start-Sleep -Milliseconds 3000
                        $before = @(Get-Guis).ProcessId
                        Open-FromTray
                        Start-Sleep -Milliseconds 4000
                        $result.stuckAlive = !$stuck.HasExited
                        $duplicate = @(Get-Records 0 $since | Where-Object type -eq 'gui-duplicate')
                        $takeover = @(Get-Records 0 $since | Where-Object type -eq 'gui-takeover')
                        $result.ok = $result.stuckAlive -and $duplicate.Count -ge 1 -and $takeover.Count -eq 0 -and @(Get-Guis).Count -eq $before.Count
                        # It still ends itself at its deadline.
                        $result.stuckEndedItself = $stuck.WaitForExit(25000)
                        $result.ok = $result.ok -and $result.stuckEndedItself
                    }
                    'Shown' {
                        $null = Wait-ShownGui @() 30000
                        Start-Sleep -Milliseconds (16000 - [math]::Min(16000, ([DateTime]::Now - $stuck.StartTime).TotalMilliseconds))
                        Open-FromTray
                        Start-Sleep -Milliseconds 4000
                        $result.shownAlive = !$stuck.HasExited
                        $takeover = @(Get-Records 0 $since | Where-Object type -eq 'gui-takeover')
                        $duplicate = @(Get-Records 0 $since | Where-Object { $_.type -eq 'gui-duplicate' -and $_.existingWindowFound })
                        $result.ok = $result.shownAlive -and $takeover.Count -eq 0 -and $duplicate.Count -ge 1
                        Close-Gui $stuck.Id
                    }
                }
                if ($simulation -eq 'exit' -and $way -ne 'Shown') {
                    $failed = @(Get-Records $stuck.Id $since | Where-Object { $_.type -eq 'webview2' -and $_.step -eq 'process-failed' })
                    $result.processFailed = ($failed | ForEach-Object kind) -join ', '
                    $result.ok = $result.ok -and $failed.Count -ge 1
                }
            } catch {
                $result.error = $_.Exception.Message
            }
            $results.Add([pscustomobject]$result)
            Write-Host (($result.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join '  ')
            foreach ($id in @($ours)) {
                $process = Get-Process -Id $id -ErrorAction SilentlyContinue
                if ($process -and (Get-PlainPath $process.Path) -eq $gui) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
            }
        }
    }
} finally {
    foreach ($id in @($ours)) {
        $process = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($process -and (Get-PlainPath $process.Path) -eq $gui) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
    }
    if (Get-Manager) { & $cli $Workspace app-exit 2>$null | Out-Null }
}
$results | ConvertTo-Json -Depth 4
if (@($results | Where-Object { !$_.ok }).Count) { exit 1 }
