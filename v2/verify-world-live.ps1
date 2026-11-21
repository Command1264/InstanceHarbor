[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$Core,
    [Parameter(Mandatory)][string]$Mojang,
    [Parameter(Mandatory)][string]$Java,
    [ValidateSet('Paper263', 'Folia2612')][string]$Profile = 'Paper263'
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$coreHash, $mojangHash = switch ($Profile) {
    'Paper263' {
        'A0A58F532F08054EA92EB916D3EDB6ED3C86429D77175317304F6E995CCE9968'
        'D052F14D7A173734FBA553711E5B570162E2F2A313267EE31A21B975A679BE64'
    }
    'Folia2612' {
        '607AFD1C3320008E1FFD2EAEE6780ACE4419D5F8C527B75E79F259BE79EBF57B'
        'CD47E7C38328F64768FD17AF8FCD8B22496B40B63D4FFEE81E71AE059FEDCB42'
    }
}
if ((Get-FileHash -LiteralPath $Core).Hash -ne $coreHash -or (Get-FileHash -LiteralPath $Mojang).Hash -ne $mojangHash) {
    throw "Input hashes must match the pinned ${Profile} profile and its companion JAR."
}
$cli = Join-Path $BinaryDirectory 'ihctl.exe'
$binaryHash = (Get-FileHash -LiteralPath $cli).Hash
$evidence = Join-Path $PSScriptRoot '.evidence'
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$task = Join-Path $evidence ('windows-world-live-' + [Guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $task) { throw 'Evidence directory already exists.' }
New-Item -ItemType Directory -Path $task | Out-Null
$root = Join-Path $task 'workspace'
$name = 'world-live'
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()
$manifest = [ordered]@{
    task = 'windows-world-live'; profile = $Profile; workspace = $root; instance = $name; port = $port
    cli = $cli; cliSha256 = $binaryHash; core = $Core; coreSha256 = $coreHash
    mojang = $Mojang; mojangSha256 = $mojangHash; java = $Java
    lifecycle = 'Batch 1 isolated detached runtime. Normal stop each owned run; verify port released.'
}
$manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $task 'manifest.json')
Write-Host "Evidence: $task"

function Invoke-Ih { & $cli $root @args | ConvertFrom-Json }

function Assert-Error([string]$Expected, [string[]]$Arguments) {
    $PSNativeCommandUseErrorActionPreference = $false
    $text = & $cli $root @Arguments 2>&1
    if ($LASTEXITCODE -eq 0 -or "$text" -notmatch $Expected) {
        throw "Expected $Expected, received $text"
    }
}

function Wait-Ready([string]$Run) {
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    do {
        $status = Invoke-Ih status $name
        if ($status.run.id -ne $Run) { throw 'Unexpected run during readiness check.' }
        if ($status.state -eq 'Listening' -and $status.consoleReady) { return $status }
        if ($status.state -notin 'Listening', 'Starting') { throw "Unexpected state: $($status.state)" }
        Start-Sleep -Milliseconds 300
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'The isolated server did not reach Done.'
}

function Assert-State($Result, [string]$Expected, [string]$Observed) {
    if ($Result.state -ne $Expected -or $Result.observed -cne $Observed) {
        throw ($Result | ConvertTo-Json -Depth 5 -Compress)
    }
}

function Invoke-Change([string]$WorkId, [string]$Run, [string]$Key, [string]$Value, [string]$Confirmation) {
    $plan = Invoke-Ih world-live-preview $name $Run $Key $Value
    $request = $plan | ConvertTo-Json -Depth 5 -Compress
    $result = Invoke-Ih world-live-apply $name $WorkId $request $Confirmation
    Assert-State $result 'AwaitingNormalStop' $Value
    return @{ plan = $plan; request = $request; result = $result }
}

Invoke-Ih init | Out-Null
Invoke-Ih create $name create-world-live $Java "$port" $Core $Mojang | Out-Null
$active = $null
try {
    $active = Invoke-Ih start $name start-world-live
    $run = $active.run.id
    Wait-Ready $run | Out-Null
    $advance = Invoke-Ih world-live-preview $name $run 'gamerules.advance_time' 'false'
    if ($advance.before -cne 'true') { throw 'Unexpected clock default.' }
    Assert-Error 'WorldRunChanged' @('world-live-preview', $name, 'stale-run', 'gamerules.spawn_mobs', 'false')
    Assert-Error 'WorldFieldUnavailable' @('world-live-preview', $name, $run, 'gamerules.not_a_rule', 'false')
    $rule = Invoke-Change 'live-spawn' $run 'gamerules.spawn_mobs' 'false' 'rules'
    Assert-State (Invoke-Ih world-live-apply $name 'live-spawn' $rule.request rules) 'AwaitingNormalStop' 'false'
    Assert-State (Invoke-Ih world-live-result $name 'live-spawn') 'AwaitingNormalStop' 'false'
    if ((Invoke-Ih world-live-preview $name $run 'gamerules.advance_time' 'false').before -cne $advance.before) {
        throw 'Applying spawn rules changed another field.'
    }
    $stale = $rule.plan | ConvertTo-Json -Depth 5 -Compress
    Assert-Error 'WorldPreviewChanged' @('world-live-apply', $name, 'stale-rule', $stale, 'rules')
    $expire = Invoke-Ih world-live-preview $name $run 'gamerules.spawn_mobs' 'true'
    $expire.createdAt = 0
    Assert-Error 'WorldPreviewExpired' @('world-live-apply', $name, 'expired-rule', ($expire | ConvertTo-Json -Compress), 'rules')
    $flow = Invoke-Change 'clock-flow' $run 'gamerules.advance_time' 'false' 'rules'
    $plannedTime = Invoke-Ih world-live-preview $name $run 'initialTime' '12000'
    $timeRequest = $plannedTime | ConvertTo-Json -Depth 5 -Compress
    Assert-Error 'WorldConfirmationRequired' @('world-live-apply', $name, 'wrong-confirmation', $timeRequest, 'rules')
    $time = Invoke-Ih world-live-apply $name live-time $timeRequest time
    Assert-State $time 'AwaitingNormalStop' '12000'
    Invoke-Ih stop $name stop-first $run | Out-Null
    $active = $null
    $active = Invoke-Ih start $name start-world-verify
    $secondRun = $active.run.id
    Wait-Ready $secondRun | Out-Null
    if ($secondRun -eq $run) { throw 'Expected new run after persistence check restart.' }
    $savedRule = Invoke-Ih world-live-preview $name $secondRun 'gamerules.spawn_mobs' 'true'
    $savedFlow = Invoke-Ih world-live-preview $name $secondRun 'gamerules.advance_time' 'true'
    $savedTime = Invoke-Ih world-live-preview $name $secondRun 'initialTime' '13000'
    if ($savedRule.before -cne 'false' -or $savedFlow.before -cne 'false' -or $savedTime.before -cne '12000') {
        throw 'Readback after normal stop and restart did not match.'
    }
    [ordered]@{
        status = 'Passed'; firstRun = $run; secondRun = $secondRun
        staleRuleRejected = $true; expiredPlanRejected = $true; staleRunRejected = $true
        wrongConfirmationRejected = $true; unrelatedSettingUnchanged = $true; replayHonored = $true
        readback = @{ spawnMobs = $savedRule.before; advanceTime = $savedFlow.before; time = $savedTime.before }
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $task 'result.json')
}
finally {
    if ($active) {
        $status = Invoke-Ih status $name
        if ($status.run.id -eq $active.run.id -and $status.state -in 'Starting', 'Listening') {
            Invoke-Ih stop $name ('world-live-final-' + $active.run.id) $active.run.id | Out-Null
        }
    }
    $after = Invoke-Ih status $name
    if ($after.state -ne 'Stopped') { throw "Owned test instance state is $($after.state)." }
    $listener.Start()
    $listener.Stop()
}
