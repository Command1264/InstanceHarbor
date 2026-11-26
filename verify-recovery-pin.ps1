[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Evidence,
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot 'src-tauri/target/debug'),
    # A build from before recovery pins were retired (1.1.0 or earlier). It starts
    # a pinned run so the current build can show that it still controls one.
    [string]$LegacyBinaryDirectory,
    # A development build only opens workspaces under InstanceHarbor Dev\workspaces.
    [string]$Workspace
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Evidence = [IO.Path]::GetFullPath($Evidence)
if (Test-Path -LiteralPath $Evidence) { throw 'Choose a new evidence directory.' }
New-Item -ItemType Directory -Path $Evidence | Out-Null
$checks = [Collections.Generic.List[string]]::new()
$managers = [Collections.Generic.List[Diagnostics.Process]]::new()
$root = if ($Workspace) { [IO.Path]::GetFullPath($Workspace) } else { Join-Path $Evidence 'workspace' }
if ($Workspace -and (Test-Path -LiteralPath $root)) { throw 'Choose a new workspace directory.' }
$java = Join-Path $Evidence 'java.exe'

function Install-Copy([string]$from, [string]$folder) {
    $target = Join-Path $Evidence $folder
    New-Item -ItemType Directory -Path $target | Out-Null
    Copy-Item -LiteralPath (Join-Path $from 'ihctl.exe') -Destination $target
    # Host recovery hashes the CLI together with the GUI that sits beside it.
    $gui = @('InstanceHarborDev.exe', 'InstanceHarbor.exe') | Where-Object { Test-Path -LiteralPath (Join-Path $from $_) } | Select-Object -First 1
    if ($gui) {
        Copy-Item -LiteralPath (Join-Path $from $gui) -Destination $target
    } else {
        Copy-Item -LiteralPath (Join-Path $from 'instanceharbor-acceptance.exe') -Destination (Join-Path $target 'InstanceHarbor.exe')
    }
    Join-Path $target 'ihctl.exe'
}
function Check($condition, [string]$label) {
    if (!$condition) { throw "Check failed: $label" }
    $checks.Add($label)
}
function Ih([string]$cli, [string[]]$argv) {
    $reply = & $cli $root @argv 2>&1
    if ($LASTEXITCODE -ne 0) { throw "$reply" }
    ConvertFrom-Json -InputObject ($reply -join "`n") -NoEnumerate
}
function Ih-Error([string]$cli, [string[]]$argv) {
    $reply = & $cli $root @argv 2>&1
    if ($LASTEXITCODE -eq 0) { return $null }
    "$reply"
}
function Wait-For([scriptblock]$probe) {
    $until = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $result = & $probe
        if ($result) { return $result }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $until)
    throw 'Fixture observation timed out.'
}
function Published-Manager {
    $descriptor = Join-Path $root 'resident-manager.json'
    if (!(Test-Path -LiteralPath $descriptor)) { return $null }
    (Get-Content -LiteralPath $descriptor -Raw | ConvertFrom-Json).pid
}
function Start-Manager([string]$cli) {
    $attempt = $managers.Count + 1
    $process = Start-Process -FilePath $cli -ArgumentList @(('"' + $root + '"'), 'manager-serve') -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $Evidence "manager-$attempt.out") `
        -RedirectStandardError (Join-Path $Evidence "manager-$attempt.err")
    $managers.Add($process)
    Wait-For { (Published-Manager) -eq $process.Id } | Out-Null
    Start-Sleep -Milliseconds 250
    $process
}
function Stop-Manager {
    $id = Published-Manager
    if (!$id) { return }
    $process = Get-Process -Id $id -ErrorAction SilentlyContinue
    if (!$process) { return }
    # Ending the Manager process is what a crash or an update does to it.
    $process.Kill()
    if (!$process.WaitForExit(5000)) { throw 'Own Manager did not exit' }
}
function Pin([string]$name) { Join-Path $root "runs/$name/recovery-pin.json" }
function Start-Fixture([string]$cli, [string]$name) {
    New-Item -ItemType File -Path (Join-Path $root "instances/$name/fixture-ready") -Force | Out-Null
    Ih $cli @('start', $name, "start-$name") | Out-Null
    Wait-For { $s = Ih $cli @('status', $name); if ($s.state -eq 'Listening' -and $s.run.id) { $s } }
}
function Stop-Fixture([string]$cli, [string]$name, [string]$run) {
    Ih $cli @('stop', $name, ('stop-' + [Guid]::NewGuid().ToString('N')), $run) | Out-Null
    Wait-For { (Ih $cli @('status', $name)).state -eq 'Stopped' } | Out-Null
}

$cli = Install-Copy $BinaryDirectory 'current'
$legacy = if ($LegacyBinaryDirectory) { Install-Copy $LegacyBinaryDirectory 'legacy' }
& rustc --edition 2021 (Join-Path $PSScriptRoot 'tests/controlled-server.rs') -o $java
if ($LASTEXITCODE -ne 0) { throw 'Fixture compilation failed.' }
Ih $cli @('init') | Out-Null
$names = @('fixture-plain', 'fixture-keyed')
if ($legacy) { $names += 'fixture-pinned' }
foreach ($name in $names) {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try { $port = $listener.LocalEndpoint.Port } finally { $listener.Stop() }
    $instance = Join-Path $root "instances/$name"
    New-Item -ItemType Directory -Path $instance | Out-Null
    Set-Content -LiteralPath (Join-Path $instance 'server.properties') -Value @("server-port=$port", 'server-ip=127.0.0.1')
    New-Item -ItemType File -Path (Join-Path $instance 'server.jar') | Out-Null
    @{name=$name; java=$java; port=$port; removed=$false} | ConvertTo-Json |
        Set-Content -LiteralPath (Join-Path $root "registry/$name.json")
}

$result = @{}
try {
    # 1. A workspace without a key behaves as before: no pin, and a restarted
    #    Manager stops the run without any recovery step.
    Check ($null -eq (Ih $cli @('recovery-key-status')).key) 'Workspace starts without a recovery key'
    Start-Manager $cli | Out-Null
    $plain = Start-Fixture $cli 'fixture-plain'
    Check (!(Test-Path -LiteralPath (Pin 'fixture-plain')) -and !$plain.recoveryPinned) 'No-key start writes no recovery pin'
    Stop-Manager
    Start-Manager $cli | Out-Null
    Stop-Fixture $cli 'fixture-plain' $plain.run.id
    $checks.Add('No-key run stopped through a restarted Manager')

    # 2. With a key, a server started by this build is not pinned either.
    $key = (Ih $cli @('recovery-key-rotate', 'none')).key
    Check ($key.generation -ge 1) 'Recovery key created'
    $keyed = Start-Fixture $cli 'fixture-keyed'
    Check (!(Test-Path -LiteralPath (Pin 'fixture-keyed')) -and !$keyed.recoveryPinned) 'Start with a key writes no recovery pin'
    Stop-Manager
    Start-Manager $cli | Out-Null
    Stop-Fixture $cli 'fixture-keyed' $keyed.run.id
    $checks.Add('Keyed workspace run stopped through a restarted Manager')
    $result.keyedRun = $keyed.run.id

    if ($legacy) {
        # 3. A run pinned by an earlier build stays controllable after this
        #    build's Manager replaces the one the pin names.
        Stop-Manager
        $pinned = Start-Fixture $legacy 'fixture-pinned'
        Check ((Test-Path -LiteralPath (Pin 'fixture-pinned')) -and $pinned.recoveryPinned) 'Legacy build pinned its run'
        $legacyManager = Published-Manager
        Stop-Manager
        Start-Manager $cli | Out-Null
        $refused = Ih-Error $cli @('stop', 'fixture-pinned', 'stop-before-recover', $pinned.run.id)
        Check ("$refused" -match 'HostRecoveryRequired') 'Pinned run refuses a different installation without proof'
        $recovered = Ih $cli @('host-recover', 'fixture-pinned', $pinned.run.id)
        Check ($recovered.state -eq 'Recovered' -and $recovered.run.id -eq $pinned.run.id) 'RecoverChallenge and RecoverProof moved the pin to the current Manager'
        Stop-Manager
        Start-Manager $cli | Out-Null
        $same = Ih $cli @('status', 'fixture-pinned')
        Check ($same.run.id -eq $pinned.run.id -and $same.state -eq 'Listening') 'Pinned run survived both Manager restarts'
        Stop-Fixture $cli 'fixture-pinned' $pinned.run.id
        $checks.Add('Rebind let a restarted Manager of the same installation stop the pinned run')
        $result.pinnedRun = $pinned.run.id
        $result.legacyManager = $legacyManager
    }
} finally {
    foreach ($name in $names) {
        $status = Ih $cli @('status', $name)
        if ($status.state -in 'Starting','Listening' -and $status.run.id) {
            $null = Ih-Error $cli @('force-stop', $name, ('cleanup-' + [Guid]::NewGuid().ToString('N')), $status.run.id)
        }
    }
    Stop-Manager
}
foreach ($name in $names) {
    Check ((Ih $cli @('status', $name)).state -eq 'Stopped') "Stopped $name and released its port"
}
$result.passed = $true
$result.checks = $checks
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Evidence 'recovery-pin-result.json')
$checks
