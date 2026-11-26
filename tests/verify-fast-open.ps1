#Requires -Version 7.0
<#
.SYNOPSIS
Checks fast open (#56) on a development GUI: X hides, the window comes back, and every exit path
ends a hidden GUI.

.DESCRIPTION
Uses only the given development workspace under %LOCALAPPDATA%/InstanceHarbor Dev/workspaces and
only GUIs this script starts. It turns fast open on in that workspace with `ihctl save-preferences`
(and off for the Off case), and puts the previous preferences file back at the end. Cases:

- Reopen:    X hides the window (the process stays); 17 s later, past #117's 15 s takeover age,
             the tray's Open shows the same process again. The page's Manager calls are counted
             over WebView2's local debugging port (this case only): none while hidden, some after.
- HideLimit: with IH_DEV_FAST_OPEN_HIDE_MS (development builds only) set to `HideLimitMs`, a hidden
             GUI ends itself after that time (`gui-hidden-exit`).
- TrayQuit:  the tray's Quit ends a hidden GUI (`gui-exit-requested`).
- AppExit:   `ihctl app-exit`, which Setup and the in-app update use, ends a hidden GUI.
- Off:       with fast open off, X ends the GUI as before.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Workspace,
    [ValidateSet('Reopen', 'HideLimit', 'TrayQuit', 'AppExit', 'Off')][string[]]$Case = @('Reopen', 'HideLimit', 'TrayQuit', 'AppExit', 'Off'),
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../src-tauri/target/x86_64-pc-windows-msvc/dev-channel'),
    [ValidateRange(1000, 600000)][int]$HideLimitMs = 5000,
    # How long an exit may take from the request until the GUI process has ended.
    [ValidateRange(500, 60000)][int]$ExitLimitMs = 3000
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
$preferences = Join-Path $Workspace 'preferences.json'
$savedPreferences = if (Test-Path -LiteralPath $preferences) { [IO.File]::ReadAllBytes($preferences) }

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class IhFast {
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
$TrayQuitId = 1002

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

# Cleanup only: the Manager may already be gone (ManagerUnavailable).
function Stop-App {
    $PSNativeCommandUseErrorActionPreference = $false
    & $cli $Workspace app-exit 2>$null | Out-Null
}

function Set-FastOpen([bool]$on) {
    $value = if ($on) { 'true' } else { 'false' }
    & $cli $Workspace save-preferences "{`"theme`":`"system`",`"pins`":[],`"fastOpen`":$value}" | Out-Null
}

# Records of one GUI process; log files are gui-<yyyyMMdd-HHmmss>-<pid>-<segment>.jsonl.
function Get-Records([int]$processId) {
    @(Get-ChildItem -LiteralPath $logs -Filter "gui-*-$processId-*.jsonl" -ErrorAction SilentlyContinue |
        ForEach-Object {
            $stream = [IO.FileStream]::new($_.FullName, 'Open', 'Read', 'ReadWrite')
            try { [IO.StreamReader]::new($stream).ReadToEnd() -split "`n" } finally { $stream.Dispose() }
        } |
        Where-Object { $_.Trim() } | ForEach-Object { ($_ | ConvertFrom-Json).event })
}

function Start-Gui([hashtable]$environment = @{}) {
    $start = [Diagnostics.ProcessStartInfo]::new($gui)
    $start.UseShellExecute = $false
    $start.Environment['IH_WORKSPACE'] = $Workspace
    foreach ($key in $environment.Keys) { $start.Environment[$key] = $environment[$key] }
    $process = [Diagnostics.Process]::Start($start)
    Wait-Until { [IhFast]::Find($process.Id, 'Tauri Window', $true) -ne [IntPtr]::Zero } 30000 'the GUI window'
    Wait-Until { $null -ne (Get-Manager) } 15000 'the Manager'
    Wait-Until { [IhFast]::Find((Get-Manager).ProcessId, 'tray_icon_app', $false) -ne [IntPtr]::Zero } 15000 'the tray window'
    # Let the first list read finish, as a user would.
    Start-Sleep -Milliseconds 2500
    $process
}

function Hide-Gui($process) {
    $null = [IhFast]::CloseButton([IhFast]::Find($process.Id, 'Tauri Window', $true))
    Wait-Until { [IhFast]::Find($process.Id, 'Tauri Window', $true) -eq [IntPtr]::Zero -or $process.HasExited } 10000 'the window to hide'
}

$DebugPort = 9347
# Evaluates one expression in the GUI page over the Chrome DevTools protocol.
function Invoke-Page([string]$expression) {
    $page = @(Invoke-RestMethod "http://127.0.0.1:$DebugPort/json" | Write-Output) | Where-Object type -eq 'page' | Select-Object -First 1
    if (!$page) { throw "No page on the debugging port: $(Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$DebugPort/json" | Select-Object -ExpandProperty Content)" }
    $socket = [Net.WebSockets.ClientWebSocket]::new()
    try {
        $socket.ConnectAsync([Uri]$page.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()
        $message = @{ id = 1; method = 'Runtime.evaluate'; params = @{ expression = $expression; returnByValue = $true } } | ConvertTo-Json -Compress -Depth 5
        $bytes = [Text.Encoding]::UTF8.GetBytes($message)
        $socket.SendAsync([ArraySegment[byte]]$bytes, 'Text', $true, [Threading.CancellationToken]::None).Wait()
        $buffer = [byte[]]::new(1MB); $text = ''
        do {
            $received = $socket.ReceiveAsync([ArraySegment[byte]]$buffer, [Threading.CancellationToken]::None).Result
            $text += [Text.Encoding]::UTF8.GetString($buffer, 0, $received.Count)
        } while (!$received.EndOfMessage)
        ($text | ConvertFrom-Json).result.result.value
    } finally { $socket.Dispose() }
}

function Send-Tray([int]$id) {
    $tray = [IhFast]::Find((Get-Manager).ProcessId, 'tray_icon_app', $false)
    if (![IhFast]::MenuCommand($tray, $id)) { throw "Could not post tray command $id." }
}

$results = [Collections.Generic.List[object]]::new()
$ours = [Collections.Generic.HashSet[int]]::new()
try {
    foreach ($way in $Case) {
        $result = [ordered]@{ case = $way; ok = $false }
        try {
            Set-FastOpen ($way -ne 'Off')
            $environment = switch ($way) {
                'HideLimit' { @{ IH_DEV_FAST_OPEN_HIDE_MS = "$HideLimitMs" } }
                # The page is inspected over WebView2's local debugging port, for this test only.
                'Reopen' { @{ WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$DebugPort --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection" } }
                default { @{} }
            }
            $process = Start-Gui $environment
            $null = $ours.Add($process.Id)
            if ($way -eq 'Reopen') {
                # Counts the page's Manager calls with their times; the page keeps polling while shown.
                # Tauri sends each command as a fetch to ipc.localhost; count the `manage` ones.
                $installed = Invoke-Page "(()=>{if(!window.__ihCalls){const o=window.fetch.bind(window);window.__ihCalls=[];window.fetch=(u,...r)=>{if(String(u?.url??u).includes('ipc.localhost/manage'))window.__ihCalls.push(Date.now());return o(u,...r)}}return Array.isArray(window.__ihCalls)})()"
                if ($installed -ne $true) { throw 'Could not count the page calls.' }
                Start-Sleep -Seconds 4
            }
            $clock = [Diagnostics.Stopwatch]::StartNew()
            Hide-Gui $process
            switch ($way) {
                'Off' {
                    $result.exited = $process.WaitForExit($ExitLimitMs)
                    $result.ok = $result.exited
                }
                'Reopen' {
                    $hiddenAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    Start-Sleep -Seconds 17
                    $result.stillRunning = !$process.HasExited
                    $clock.Restart()
                    $openedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    Send-Tray $TrayOpenId
                    Wait-Until { [IhFast]::Find($process.Id, 'Tauri Window', $true) -ne [IntPtr]::Zero } 10000 'the same GUI to show again'
                    $result.reopenMs = [math]::Round($clock.Elapsed.TotalMilliseconds)
                    Start-Sleep -Seconds 3
                    $calls = @(Invoke-Page 'JSON.stringify(window.__ihCalls)' | ConvertFrom-Json)
                    # From X until the tray's Open; a poll already in flight when X was pressed may still
                    # finish just after it. The page reads everything once as it resumes, before the
                    # window is drawn, so calls between Open and the window showing are expected.
                    $result.pollsBefore = @($calls | Where-Object { $_ -lt $hiddenAt }).Count
                    $result.pollsWhileHidden = @($calls | Where-Object { $_ -ge $hiddenAt + 500 -and $_ -lt $openedAt }).Count
                    $result.pollsAfterOpen = @($calls | Where-Object { $_ -ge $openedAt }).Count
                    $result.hiddenMs = $openedAt - $hiddenAt
                    $takeover = @(Get-ChildItem -LiteralPath $logs -Filter 'gui-*.jsonl' |
                        Where-Object { $_.LastWriteTimeUtc -ge [DateTimeOffset]::FromUnixTimeMilliseconds($hiddenAt).UtcDateTime } |
                        Select-String -SimpleMatch '"gui-takeover"')
                    $result.takeover = $takeover.Count
                    $result.ok = $result.stillRunning -and $result.reopenMs -le 2000 -and $result.pollsBefore -gt 0 -and
                        $result.pollsWhileHidden -eq 0 -and $result.pollsAfterOpen -gt 0 -and $takeover.Count -eq 0
                }
                'HideLimit' {
                    $result.exited = $process.WaitForExit($HideLimitMs + 5000)
                    $result.endedMs = [math]::Round($clock.Elapsed.TotalMilliseconds)
                    $result.recorded = @(Get-Records $process.Id | Where-Object type -eq 'gui-hidden-exit').Count
                    $result.ok = $result.exited -and $result.endedMs -ge $HideLimitMs -and $result.recorded -eq 1
                }
                'TrayQuit' {
                    Start-Sleep -Seconds 2
                    $clock.Restart()
                    Send-Tray $TrayQuitId
                    $result.exited = $process.WaitForExit($ExitLimitMs)
                    $result.exitMs = [math]::Round($clock.Elapsed.TotalMilliseconds)
                    $result.requested = @(Get-Records $process.Id | Where-Object { $_.type -eq 'gui-exit-requested' -and $_.hidden }).Count
                    $result.ok = $result.exited -and $result.requested -eq 1
                }
                'AppExit' {
                    Start-Sleep -Seconds 2
                    $clock.Restart()
                    & $cli $Workspace app-exit | Out-Null
                    $result.exited = $process.WaitForExit($ExitLimitMs)
                    $result.exitMs = [math]::Round($clock.Elapsed.TotalMilliseconds)
                    $result.requested = @(Get-Records $process.Id | Where-Object { $_.type -eq 'gui-exit-requested' -and $_.hidden }).Count
                    $result.ok = $result.exited -and $result.requested -eq 1
                }
            }
        } catch {
            $result.error = $_.Exception.Message
        } finally {
            if ($process -and !$process.HasExited) {
                Stop-App
                if (!$process.WaitForExit(15000)) { $process.Kill($true) }
            }
            if (Get-Manager) { Stop-App }
            Wait-Until { $null -eq (Get-Manager) } 15000 'the Manager to exit'
        }
        $results.Add([pscustomobject]$result)
        Write-Host (($result.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join '  ')
    }
} finally {
    if (Get-Manager) { Stop-App }
    if ($savedPreferences) { [IO.File]::WriteAllBytes($preferences, $savedPreferences) }
}
$results | ConvertTo-Json -Depth 4
if (@($results | Where-Object { !$_.ok }).Count) { exit 1 }
