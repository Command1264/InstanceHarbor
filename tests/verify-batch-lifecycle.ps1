# Dot-sourced by verify-resident-manager.ps1 after creating its private workspace.
function Snapshot([string[]]$targets) {
    $rows = @($targets | ForEach-Object {
        $s = Ih @('status',$_)
        @{name=$_;guard=@{previousRun=$s.run.id;stopToken=$s.stopToken}}
    })
    ConvertTo-Json -InputObject $rows -Depth 6 -Compress
}
function Hold([string]$name) {
    New-Item -ItemType File -Path (Join-Path $root "instances/$name/fixture-hold-stop") -Force | Out-Null
}
function Release([string]$name) {
    $path = Join-Path $root "instances/$name/fixture-hold-stop"
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
}
function Manager-Ready {
    Wait-For { (Test-Path -LiteralPath $descriptor) -and
        ((Get-Content -LiteralPath $descriptor -Raw | ConvertFrom-Json).pid -eq $manager.Id) } | Out-Null
    Start-Sleep -Milliseconds 300
}
$descriptor = Join-Path $root 'resident-manager.json'
$previousWait = $env:IH_TEST_STOP_WAIT_MS
$batchId = 'controlled-restart'
try {
    foreach ($name in $names[0..2]) {
        Gate $name
        Ih @('start',$name,"initial-$name") | Out-Null
    }
    Wait-For {
        @($names[0..2] | Where-Object { (Ih @('status',$_)).consoleReady }).Count -eq 3
    } | Out-Null
    $original = @{}
    foreach ($name in $names[0..2]) { $original[$name] = (Ih @('status',$name)).run.id }
    Hold 'fixture-a'
    Hold 'fixture-c'
    # Keep the replacement runs at the readiness gate to measure shared slots.
    foreach ($name in $names[0..2]) {
        Remove-Item -LiteralPath (Join-Path $root "instances/$name/fixture-ready")
    }
    $env:IH_TEST_STOP_WAIT_MS = '10000'
    $manager = Start-Manager
    Manager-Ready
    $payload = Snapshot $names[0..2]
    $work = Ih @('batch-restart',$batchId,$payload)
    Check ($work.id -eq $batchId) 'Restart acknowledged with a durable job ID'
    Wait-For {
        @($names[0..2] | Where-Object {
            Test-Path -LiteralPath (Join-Path $root "instances/$_/fixture-stop-seen")
        }).Count -eq 3
    } | Out-Null
    Wait-For { (Batch $batchId).items[1].phase -eq 'Starting' } | Out-Null
    $newB = (Ih @('status','fixture-b')).run.id
    Check ($newB -ne $original['fixture-b'] -and (Ih @('status','fixture-a')).run.id -eq $original['fixture-a'] -and
        (Ih @('status','fixture-c')).run.id -eq $original['fixture-c']) 'Three concurrent stops, fast target restarts while two still wait'
    $oldManager = $manager.Id
    $manager.Kill()
    if (!$manager.WaitForExit(5000)) { throw 'Own Manager did not exit' }
    # Recover both a possibly dispatched stop and a start whose receipt was written.
    $journal = Join-Path $root "batches/$batchId.json"
    $saved = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    $deadline = $saved.items[0].stopIntent.deadline
    $saved.items[0].phase = 'StopDispatching'
    $saved.items[1].phase = 'Dispatching'
    $saved.items[1].run = $null
    $saved | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath $journal
    $manager = Start-Manager
    Manager-Ready
    $again = Ih @('batch-restart',$batchId,$payload)
    Check ($again.id -eq $work.id -and $manager.Id -ne $oldManager) 'Restart retry after replacing Manager returns the same work ID'
    Wait-For { (Batch $batchId).items[0].phase -eq 'Stopping' } | Out-Null
    $restored = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    Check ($restored.items[0].stopIntent.deadline -eq $deadline -and (Batch $batchId).items[1].runId -eq $newB) 'Recovered Host stop proof and start receipt with original deadline and run'
    $conflict = & $cli $root batch-stop $batchId $payload 2>&1
    Check ($LASTEXITCODE -ne 0 -and "$conflict" -match 'RequestConflict') 'Same ID with different action rejected'
    $parallel = Ih @('batch-start','parallel-start',(Snapshot $names[3..4]))
    Wait-For { (Batch $parallel.id).items[0].phase -eq 'Starting' } | Out-Null
    Release 'fixture-c'
    Wait-For { (Batch $batchId).items[2].phase -eq 'Queued' } | Out-Null
    Check ((Batch $parallel.id).items[1].phase -eq 'Queued' -and (Ih @('status','fixture-e')).state -eq 'Stopped') 'Batch restart and batch start share two readiness slots'
    Wait-For { (Batch $batchId).items[0].reason -eq 'StopTimeout' } | Out-Null
    $queued = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    Check ($queued.items[2].stopVerifiedAt -lt $queued.items[2].stopIntent.deadline) 'Verified stop time saved with the queued start'
    $manager.Kill()
    if (!$manager.WaitForExit(5000)) { throw 'Own Manager did not exit' }
    $manager = Start-Manager
    Manager-Ready
    Check ((Batch $batchId).items[2].phase -eq 'Queued') 'A verified queued restart survives Manager recovery past the original stop deadline'
    Gate 'fixture-b'
    Wait-For { (Batch $batchId).items[2].phase -eq 'Starting' } | Out-Null
    Check ((Batch $parallel.id).items[1].phase -eq 'Queued') 'Verified readiness releases exactly one slot'
    Gate 'fixture-c'
    Gate 'fixture-d'
    Gate 'fixture-e'
    Wait-For { (Batch $parallel.id).state -eq 'Completed' } | Out-Null
    Wait-For { (Batch $batchId).items[0].reason -eq 'StopTimeout' } | Out-Null
    Release 'fixture-a'
    Wait-For { (Ih @('status','fixture-a')).state -eq 'Stopped' } | Out-Null
    $final = Batch $batchId
    $saved = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    Check ($final.items[0].reason -eq 'StopTimeout' -and (Ih @('status','fixture-a')).run.id -eq $original['fixture-a'] -and
        !(Test-Path -LiteralPath (Join-Path $root "jobs/$($saved.items[0].request).json"))) 'Late exit never acquires an automatic start receipt'
    Check ($final.items[1].runId -eq $newB -and $final.items[2].phase -eq 'Completed') 'Timeout does not stop unrelated targets or redispatch a replacement'
    Hold 'fixture-b'
    Hold 'fixture-c'
    Hold 'fixture-d'
    Hold 'fixture-e'
    Ih @('batch-restart','cancel-restart',(Snapshot $names[1..4])) | Out-Null
    Wait-For { @((Batch 'cancel-restart').items | Where-Object phase -eq Stopping).Count -eq 4 } | Out-Null
    Ih @('batch-cancel','cancel-restart') | Out-Null
    $force = Ih @('force-stop','fixture-d','fixture-force',$((Ih @('status','fixture-d')).run.id))
    Check $force.forced 'Force is explicit and addressed to the verified fixture run'
    $replacementStop = Ih @('batch-stop','superseding-stop',(Snapshot @('fixture-e')))
    Release 'fixture-b'
    Release 'fixture-c'
    Release 'fixture-e'
    Wait-For { (Batch 'cancel-restart').state -ne 'Pending' } | Out-Null
    Wait-For { (Batch $replacementStop.id).state -eq 'Completed' } | Out-Null
    $cancelled = Batch 'cancel-restart'
    Check ($cancelled.items[0].phase -eq 'Cancelled' -and $cancelled.items[1].phase -eq 'Cancelled' -and
        $cancelled.items[2].phase -eq 'StopUnconfirmed' -and $cancelled.items[3].phase -eq 'Cancelled') 'Cancelled and forced targets do not enter the start queue'
    $stop = Ih @('batch-stop','already-stopped',(Snapshot $names))
    Check ($stop.state -eq 'Completed' -and @($stop.items | Where-Object phase -eq Skipped).Count -eq 5) 'Pure stop skips stopped fixtures and has no start phase'
    Check (@((Ih @('jobs')) | Where-Object action -eq 'start-if-current').Count -eq 4) 'Exactly one receipt per eligible replacement and parallel start'
    foreach ($name in $names[0..2]) {
        Gate $name
        Hold $name
        Ih @('start',$name,"fault-start-$name") | Out-Null
    }
    Wait-For { @($names[0..2] | Where-Object { (Ih @('status',$_)).consoleReady }).Count -eq 3 } | Out-Null
    $foreign = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $ports['fixture-e'])
    $foreign.Start()
    try {
        $faultTargets = @($names[0..2]) + @('fixture-e')
        $faults = Ih @('batch-restart','fault-restart',(Snapshot $faultTargets))
        Check ($faults.items[3].reason -eq 'PortOccupied') 'Foreign listener is neither stopped nor eligible for restart'
        Wait-For { @((Batch 'fault-restart').items | Where-Object phase -eq Stopping).Count -eq 3 } | Out-Null
        $manager.Kill()
        if (!$manager.WaitForExit(5000)) { throw 'Own Manager did not exit' }
        $reg = Join-Path $root 'registry/fixture-a.json'
        Set-Content -LiteralPath $reg -Value ((Get-Content -LiteralPath $reg -Raw) + ' ')
        $faultPath = Join-Path $root 'batches/fault-restart.json'
        $faultSaved = Get-Content -LiteralPath $faultPath -Raw | ConvertFrom-Json
        $faultSaved.items[1].stopRun.java.created++
        $faultSaved | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath $faultPath
        Release 'fixture-c'
        Wait-For { (Ih @('status','fixture-c')).state -eq 'Stopped' } | Out-Null
        foreach ($file in 'batch-stop-accepted.json','batch-stop-completion.json') {
            Move-Item -LiteralPath (Join-Path $root "runs/fixture-c/$file") -Destination (Join-Path $Evidence "withheld-$file")
        }
        $manager = Start-Manager
        Manager-Ready
        Wait-For { (Batch 'fault-restart').state -ne 'Pending' } | Out-Null
        $faultResults = Batch 'fault-restart'
        Check ($faultResults.items[0].reason -eq 'RegistrationChanged' -and
            $faultResults.items[1].reason -eq 'StopIntentChanged' -and
            $faultResults.items[2].reason -eq 'StopResultUnconfirmed') 'Changed registration, stale process identity and missing stop proof independently block restart'
        Check (@((Ih @('jobs')) | Where-Object action -eq 'start-if-current').Count -eq 4) 'Faulted targets have no additional start receipt'
    } finally { $foreign.Stop() }
    Ih @('start','fixture-e','before-dispatch-start') | Out-Null
    Wait-For { (Ih @('status','fixture-e')).consoleReady } | Out-Null
    $preDispatchRun = Ih @('status','fixture-e')
    $manager.Kill()
    if (!$manager.WaitForExit(5000)) { throw 'Own Manager did not exit' }
    # Seed precisely the journal boundary before Manager commits StopDispatching.
    # No stop was sent to this run before reloading the journal.
    $workId = 'before-stop-dispatch'
    $hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData(
        [Text.Encoding]::UTF8.GetBytes("$workId/fixture-e"))).ToLowerInvariant()
    $earlierJournal = Get-Content -LiteralPath (Join-Path $root 'batches/already-stopped.json') -Raw | ConvertFrom-Json
    $registration = ($earlierJournal.items | Where-Object {$_.target.name -eq 'fixture-e'}).registration
    $item = @{
        target=@{name='fixture-e';guard=@{previousRun=$preDispatchRun.run.id;stopToken=$preDispatchRun.stopToken}}
        registration=$registration
        phase='StopQueued';request=$hash;run=$null;deadline=0;reason=$null
        stopRun=$preDispatchRun.run
        stopIntent=@{request=$hash;deadline=[DateTimeOffset]::UtcNow.AddSeconds(12).ToUnixTimeMilliseconds()}
    }
    @{id=$workId;createdAt=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();action='Stop';items=@($item)} |
        ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $root "batches/$workId.json")
    $manager = Start-Manager
    Manager-Ready
    Wait-For { (Batch $workId).state -eq 'Completed' } | Out-Null
    $issued = Get-Content -LiteralPath (Join-Path $root 'runs/fixture-e/batch-stop-completion.json') -Raw | ConvertFrom-Json
    Check ($issued.intent.request -eq $hash -and $issued.run.id -eq $preDispatchRun.run.id) 'An undispatched stop resumes under its exact saved intent'
    Check ((Get-FileHash -LiteralPath $policy).Hash -eq $policyHash) 'Fixture policy preserved'
    @{restart=$final; cancelled=$cancelled; pureStop=$stop; faultResults=$faultResults; oldManager=$oldManager; newManager=$manager.Id} |
        ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $Evidence 'batch-lifecycle-jobs.json')
} finally {
    $env:IH_TEST_STOP_WAIT_MS = $previousWait
    if ($manager -and !$manager.HasExited) {
        foreach ($row in (Ih @('batch-list')) | Where-Object state -eq Pending) {
            Ih @('batch-cancel',$row.id) | Out-Null
        }
    }
    foreach ($name in $names) {
        Release $name
        Gate $name
    }
    foreach ($name in $names) {
        $status = Ih @('status',$name)
        if ($status.state -in 'Starting','Listening' -and $status.run.id) {
            try {
                Ih @('stop',$name,('cleanup-' + [Guid]::NewGuid().ToString('N')),$status.run.id) | Out-Null
            } catch {
                $current = Ih @('status',$name)
                if ($current.state -ne 'Stopped' -or $current.run.id -ne $status.run.id) { throw }
            }
        }
    }
    if ($manager -and !$manager.HasExited) {
        Wait-For { @((Ih @('batch-list')) | Where-Object state -eq Pending).Count -eq 0 } | Out-Null
        Ih @('manager-exit') | Out-Null
        if (!$manager.WaitForExit(6000)) { throw 'Manager did not exit' }
    }
}
foreach ($name in $names) {
    Check ((Ih @('status',$name)).state -eq 'Stopped') "Stopped $name and released its port"
}
@{passed=$true;checks=$checks;ports=$ports} |
    ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Evidence 'batch-lifecycle-result.json')
$checks
