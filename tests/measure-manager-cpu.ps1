# Samples the CPU use of a workspace's resident Manager and of each of its threads (#118).
# Read-only: it never attaches to, suspends or signals the process. Output is in percent
# of one CPU core over the sample, so a thread that never sleeps shows close to 100.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Workspace,
    [ValidateRange(1, 600)][int]$Seconds = 10,
    [ValidateRange(1, 64)][int]$Top = 3
)
$ErrorActionPreference = 'Stop'

$descriptor = Join-Path $Workspace 'resident-manager.json'
if (!(Test-Path -LiteralPath $descriptor)) { throw "No Manager is published for this workspace: $descriptor" }
$managerPid = [int](Get-Content -LiteralPath $descriptor -Raw | ConvertFrom-Json).pid
$process = Get-Process -Id $managerPid -ErrorAction Stop
if ($process.ProcessName -ne 'ihctl') { throw "PID $managerPid is $($process.ProcessName), not the Manager." }

function Read-Times($process) {
    $process.Refresh()
    $threads = @{}
    foreach ($thread in $process.Threads) {
        try { $threads[$thread.Id] = $thread.TotalProcessorTime } catch { }
    }
    [pscustomobject]@{ total = $process.TotalProcessorTime; threads = $threads; at = [Diagnostics.Stopwatch]::GetTimestamp() }
}

$before = Read-Times $process
Start-Sleep -Seconds $Seconds
$after = Read-Times $process
$elapsed = ($after.at - $before.at) / [Diagnostics.Stopwatch]::Frequency
$percent = { param($span) [Math]::Round(100 * $span.TotalSeconds / $elapsed, 1) }
$threads = foreach ($id in $after.threads.Keys) {
    if ($before.threads.ContainsKey($id)) {
        [pscustomobject]@{ thread = $id; cpu = & $percent ($after.threads[$id] - $before.threads[$id]) }
    }
}
[pscustomobject]@{
    pid = $managerPid
    seconds = [Math]::Round($elapsed, 2)
    processCpu = & $percent ($after.total - $before.total)
    threadCount = $after.threads.Count
    busiest = @($threads | Sort-Object cpu -Descending | Select-Object -First $Top)
}
