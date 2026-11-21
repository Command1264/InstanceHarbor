[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Evidence,
    [switch]$MissingReceipt,
    [ValidateSet('None','ReadinessTimeout','RunMismatch','RegistrationChanged')]
    [string]$Fault = 'None',
    [switch]$PrepareOnly,
    [switch]$BatchLifecycle,
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot 'src-tauri/target/debug')
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Evidence = [IO.Path]::GetFullPath($Evidence)
if (Test-Path -LiteralPath $Evidence) { throw 'Choose a new evidence directory.' }
New-Item -ItemType Directory -Path $Evidence | Out-Null
$native = Join-Path $Evidence 'native'
New-Item -ItemType Directory -Path $native | Out-Null
Copy-Item -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe') -Destination $native
Copy-Item -LiteralPath (Join-Path $BinaryDirectory 'instanceharbor-v2-acceptance.exe') -Destination (Join-Path $native 'InstanceHarbor.exe')
$cli = Join-Path $native 'ihctl.exe'
$root = Join-Path $Evidence 'workspace'
$java = Join-Path $Evidence 'java.exe'
$checks = [Collections.Generic.List[string]]::new()
$names = @('fixture-a', 'fixture-b', 'fixture-c')
if ($BatchLifecycle) { $names += @('fixture-d', 'fixture-e') }
$manager = $null
$managerCount = 0
function Check($condition, [string]$label) {
    if (!$condition) { throw "Check failed: $label" }
    $checks.Add($label)
}
function Ih([string[]]$argv) {
    $reply = & $cli $root @argv 2>&1
    if ($LASTEXITCODE -ne 0) { throw "$reply" }
    ConvertFrom-Json -InputObject ($reply -join "`n") -NoEnumerate
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
function Start-Manager {
    $script:managerCount++
    $attempt = $script:managerCount
    Start-Process -FilePath $cli -ArgumentList @(('"' + $root + '"'), 'manager-serve') -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $Evidence "manager-$attempt.out") `
        -RedirectStandardError (Join-Path $Evidence "manager-$attempt.err")
}
function Batch([string]$id) {
    $list = Ih @('batch-list')
    $list | Where-Object id -eq $id
}
function Gate([string]$name) {
    New-Item -ItemType File -Path (Join-Path $root "instances/$name/fixture-ready") -Force | Out-Null
}
& rustc --edition 2021 (Join-Path $PSScriptRoot 'tests/controlled-server.rs') -o $java
if ($LASTEXITCODE -ne 0) { throw 'Fixture compilation failed.' }
Ih @('init') | Out-Null
$policy = Join-Path $root 'scan-policy.json'
$policyHash = (Get-FileHash -LiteralPath $policy).Hash
$ports = @{}
foreach ($name in $names) {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try { $port = $listener.LocalEndpoint.Port } finally { $listener.Stop() }
    if ($ports.Values -contains $port) { throw 'Fixture ports must be distinct' }
    $ports[$name] = $port
    $instance = Join-Path $root "instances/$name"
    New-Item -ItemType Directory -Path $instance | Out-Null
    Set-Content -LiteralPath (Join-Path $instance 'server.properties') -Value @("server-port=$port", 'server-ip=127.0.0.1')
    New-Item -ItemType File -Path (Join-Path $instance 'server.jar') | Out-Null
    @{name=$name; java=$java; port=$port; removed=$false} | ConvertTo-Json |
        Set-Content -LiteralPath (Join-Path $root "registry/$name.json")
}
@{workspace=$root;owner='resident-manager-controlled';ports=$ports;fixtureSha256=(Get-FileHash -LiteralPath $java).Hash} |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Evidence 'fixture-manifest.json')
if ($PrepareOnly) { return }
if ($BatchLifecycle) {
    . (Join-Path $PSScriptRoot 'tests/verify-batch-lifecycle.ps1')
    return
}
$targets = @($names | ForEach-Object { @{name=$_;guard=@{previousRun=$null;stopToken=$null}} })
$payload = ConvertTo-Json -InputObject $targets -Depth 5 -Compress
try {
    $manager = Start-Manager
    $descriptor = Join-Path $root 'resident-manager.json'
    Wait-For { if (Test-Path -LiteralPath $descriptor) { (Get-Content -LiteralPath $descriptor -Raw | ConvertFrom-Json).pid -eq $manager.Id } } | Out-Null
    Start-Sleep -Milliseconds 250
    # Each ihctl invocation is a separate client that exits after receiving its reply.
    $first = Ih @('batch-start','controlled-batch',$payload)
    Check ($first.id -eq 'controlled-batch') 'Persisted work before acknowledging the client'
    $waiting = Wait-For {
        $work = Batch 'controlled-batch'
        if (@($work.items | Where-Object phase -eq Starting).Count -eq 2) { $work }
    }
    Check ($waiting.items[2].phase -eq 'Queued' -and !(Test-Path -LiteralPath (Join-Path $root 'runs/fixture-c'))) 'Third target waits for an actual slot'
    $duplicate = Start-Manager
    Check ($duplicate.WaitForExit(5000) -and $duplicate.ExitCode -ne 0) 'Only one Manager holds the workspace lease'
    $prior = (Ih @('status','fixture-a')).run
    $oldManager = $manager.Id
    $manager.Kill()
    if (!$manager.WaitForExit(5000)) { throw 'Own Manager did not exit' }
    # Reproduce the persisted boundary after Host dispatch and before the batch
    # journal records the accepted run. The running processes are untouched.
    $journal = Join-Path $root 'batches/controlled-batch.json'
    $saved = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    if ($Fault -eq 'ReadinessTimeout') {
        $saved.items[0].deadline = 1
    } elseif ($Fault -eq 'RunMismatch') {
        $saved.items[0].run.java.created++
    } else {
        $saved.items[0].phase = 'Dispatching'
        $saved.items[0].run = $null
    }
    $saved | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $journal
    if ($MissingReceipt) {
        $request = $saved.items[0].request
        if ($request -notmatch '^[a-f0-9]{64}$') { throw 'Unexpected fixture request ID' }
        Move-Item -LiteralPath (Join-Path $root "jobs/$request.json") -Destination (Join-Path $Evidence 'withheld-receipt.json')
    }
    if ($Fault -eq 'RegistrationChanged') {
        $registration = Join-Path $root 'registry/fixture-c.json'
        $record = Get-Content -LiteralPath $registration -Raw
        Set-Content -LiteralPath $registration -Value ($record + ' ')
    }
    $manager = Start-Manager
    Wait-For { (Get-Content -LiteralPath $descriptor -Raw | ConvertFrom-Json).pid -eq $manager.Id } | Out-Null
    Start-Sleep -Milliseconds 250
    $same = Ih @('batch-start','controlled-batch',$payload)
    Check ($same.id -eq $first.id) 'Retry after Manager restart returned original work'
    $wrong = & $cli $root batch-start controlled-batch (ConvertTo-Json -InputObject @($targets[0]) -Depth 5 -Compress) 2>&1
    Check ($LASTEXITCODE -ne 0 -and "$wrong" -match 'RequestConflict') 'Same ID with different targets rejected'
    if ($MissingReceipt -or $Fault -in 'ReadinessTimeout','RunMismatch') {
        Wait-For { (Batch 'controlled-batch').items[0].phase -eq 'Unconfirmed' } | Out-Null
        $reason = if ($Fault -eq 'ReadinessTimeout') { 'ReadinessUnconfirmed' } else { 'RunUnconfirmed' }
        Check ((Batch 'controlled-batch').items[0].reason -eq $reason) "Explicit reconciliation reason: $reason"
        Gate 'fixture-b'
        Wait-For { (Batch 'controlled-batch').items[1].phase -eq 'Completed' } | Out-Null
        Check ((Batch 'controlled-batch').items[2].phase -eq 'Queued' -and !(Test-Path -LiteralPath (Join-Path $root 'runs/fixture-c'))) 'Unconfirmed dispatch holds the next target'
        $completed = Ih @('batch-cancel','controlled-batch')
        Check ($completed.items[2].phase -eq 'Cancelled' -and $completed.items[0].phase -eq 'Unconfirmed') 'Cancellation only withdraws the undispatched target'
    } elseif ($Fault -eq 'RegistrationChanged') {
        Gate 'fixture-a'
        Gate 'fixture-b'
        $completed = Wait-For { $work = Batch 'controlled-batch'; if ($work.items[2].phase -eq 'Failed') { $work } }
        Check ($completed.items[2].reason -eq 'StartIntentChanged' -and !(Test-Path -LiteralPath (Join-Path $root 'runs/fixture-c'))) 'Changed registration rejected before Host dispatch'
    } else {
        Wait-For { (Batch 'controlled-batch').items[0].runId -eq $prior.id } | Out-Null
        Gate 'fixture-a'
        Wait-For { Test-Path -LiteralPath (Join-Path $root 'instances/fixture-c/fixture-started') } | Out-Null
        Check ((Batch 'controlled-batch').items[0].phase -eq 'Completed') 'After client exit, third start waited for first verified readiness'
        Gate 'fixture-b'
        Gate 'fixture-c'
        $completed = Wait-For { $work = Batch 'controlled-batch'; if ($work.state -eq 'Completed') { $work } }
        Check ((Ih @('jobs')).Count -eq 3) 'Exactly one start receipt per target'
    }
    $observed = (Ih @('status','fixture-a')).run
    Check ($observed.id -eq $prior.id -and $observed.java.pid -eq $prior.java.pid -and
        $observed.java.created -eq $prior.java.created -and $observed.host.pid -eq $prior.host.pid -and
        $observed.host.created -eq $prior.host.created -and $manager.Id -ne $oldManager) 'Original Host and child identity across distinct Manager processes'
    $completed | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Evidence 'completed-batch.json')
    Check ((Get-FileHash -LiteralPath $policy).Hash -eq $policyHash) 'Fixture policy preserved'
} finally {
    if ($manager -and !$manager.HasExited) {
        Ih @('batch-cancel','controlled-batch') | Out-Null
    }
    foreach ($name in $names) {
        Gate $name
        $status = Ih @('status',$name)
        if ($status.state -in 'Starting','Listening' -and $status.run.id) {
            Ih @('stop',$name,('cleanup-' + [Guid]::NewGuid().ToString('N')),$status.run.id) | Out-Null
        }
    }
    if ($manager -and !$manager.HasExited) {
        Wait-For { (Batch 'controlled-batch').state -ne 'Pending' } | Out-Null
        Ih @('manager-exit') | Out-Null
        if (!$manager.WaitForExit(6000)) { throw 'Manager did not exit' }
    }
}
foreach ($name in $names) {
    Check ((Ih @('status',$name)).state -eq 'Stopped') "Stopped $name and released its port"
}
@{passed=$true; checks=$checks; oldManager=$oldManager; newManager=$manager.Id; originalRun=$prior; finalRun=$observed} |
    ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Evidence 'controlled-result.json')
$checks
