#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Evidence,
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$Paper,
    [Parameter(Mandatory)][string]$Mojang,
    [Parameter(Mandatory)][string]$Spigot,
    [Parameter(Mandatory)][string]$LuckPerms,
    [Parameter(Mandatory)][string]$LegacyLuckPerms,
    [string]$NativeVerifier
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Evidence = [IO.Path]::GetFullPath($Evidence)
if (Test-Path -LiteralPath $Evidence) { throw 'Choose a new evidence directory.' }
$cli = (Get-Item -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe')).FullName
$inputs = @(
    @{ id='paper-26.3'; file='server.jar'; source=$Paper; sha='A0A58F532F08054EA92EB916D3EDB6ED3C86429D77175317304F6E995CCE9968' },
    @{ id='paper-26.3'; file='mojang.jar'; source=$Mojang; sha='D052F14D7A173734FBA553711E5B570162E2F2A313267EE31A21B975A679BE64' },
    @{ id='paper-26.3'; file='luckperms.jar'; source=$LuckPerms; sha='DC637CE18F48D3B75A7FFD1784B85BE16A627359090DFE5ADF15DAB6D145DC7D' },
    @{ id='spigot-1.8.8'; file='server.jar'; source=$Spigot; sha='6ED6046687CE9255228FE7BD31E097DA49BEFF2A0C577E7BE56D14F09753239C' },
    @{ id='spigot-1.8.8'; file='luckperms.jar'; source=$LegacyLuckPerms; sha='99FA6656FB3CD968ACB800A1D1D9494E34C80DEEF68145521F1F61364117C1E5' }
)
foreach ($inputFile in $inputs) {
    if ((Get-FileHash -LiteralPath $inputFile.source).Hash -cne $inputFile.sha) { throw 'Source hash mismatch.' }
}
New-Item -ItemType Directory -Path $Evidence | Out-Null
$root = Join-Path $Evidence 'workspace'
$report = [ordered]@{ result='FAIL'; cases=[Collections.Generic.List[string]]::new(); inputs=$inputs; workspace=$root; runs=@() }
$active = $null

function Ih([string[]]$Command, [string]$ExpectedError) {
    $reply = & $cli $root @Command 2>&1
    if ($ExpectedError) {
        if ($LASTEXITCODE -eq 0 -or "$reply" -notmatch $ExpectedError) { throw "Expected $ExpectedError. $reply" }
        $report.cases.Add($ExpectedError)
        return
    }
    if ($LASTEXITCODE -ne 0) { throw "ihctl $($Command[0]) failed. $reply" }
    ConvertFrom-Json -InputObject ($reply -join [Environment]::NewLine) -NoEnumerate
}
function Check([bool]$Condition, [string]$Case) {
    if (!$Condition) { throw $Case }
    $report.cases.Add($Case)
}
function Request([string]$Name, [string]$Profile, [int]$Port = 0) {
    @{audience='Agent';project='Lobby';instance=$Name;profile=$Profile;port=$Port} | ConvertTo-Json -Compress
}
function Rewind-Receipt([string]$Name, [string]$Id, [bool]$ToStaging) {
    $receiptPath=Join-Path $root "jobs/$Id.json"
    $registration=Join-Path $root "registry/$Name.json"
    $target=Join-Path $root "instances/Agent/Lobby/$Name"
    $receipt=Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    $receipt.complete=$false
    $receipt.output=@{
        properties=(Get-FileHash -LiteralPath (Join-Path $target 'server.properties')).Hash.ToLowerInvariant()
        registration=(Get-Content -LiteralPath $registration -Raw | ConvertFrom-Json)
        manifest=(Get-Content -LiteralPath (Join-Path $target 'prepare.json') -Raw | ConvertFrom-Json)
    }
    $receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $receiptPath
    ConvertTo-Json -InputObject $Id | Set-Content -LiteralPath (Join-Path $root "staging/agent-profile-$Name.json")
    if ($ToStaging) { [IO.Directory]::Move($target,(Join-Path $root "staging/$Id")) }
    [IO.File]::Delete($registration)
}

try {
    Ih @('init') | Out-Null
    $policy = Get-FileHash -LiteralPath (Join-Path $root 'scan-policy.json')
    foreach ($inputFile in $inputs) {
        $folder = Join-Path $root "profile-assets/$($inputFile.id)"
        $null = [IO.Directory]::CreateDirectory($folder)
        Copy-Item -LiteralPath $inputFile.source -Destination (Join-Path $folder $inputFile.file)
    }
    Ih @('prepare','unknown','unknown-profile',(Request 'unknown' 'unknown-profile')) 'UnknownAgentProfile'
    $core = Join-Path $root 'profile-assets/paper-26.3/server.jar'
    [IO.File]::Move($core, "$core.valid")
    try {
        Set-Content -LiteralPath $core -Value 'invalid fixture'
        Ih @('prepare','invalid','invalid-profile',(Request 'invalid' 'paper-26.3')) 'AgentProfileCoreMismatch'
    } finally {
        [IO.File]::Move("$core.valid",$core,$true)
    }
    $occupier=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
    $occupier.Start()
    try {
        Ih @('prepare','occupied','occupied-profile',(Request 'occupied' 'paper-26.3' $occupier.LocalEndpoint.Port)) 'PortOccupied'
    } finally { $occupier.Stop() }

    $prepared = @()
    foreach ($case in @(@{name='paper-agent';profile='paper-26.3';staged=$true},@{name='spigot-agent';profile='spigot-1.8.8';staged=$false})) {
        $name = $case.name
        $id = "prepare-$name"
        $payload = Request $name $case.profile
        $command = @('prepare',$name,$id,$payload)
        $created = Ih $command
        Check ($created.state -eq 'Stopped' -and $created.port -gt 0 -and $created.profile -eq $case.profile) "Created $name stopped with an automatic port"
        Rewind-Receipt $name $id $case.staged
        Ih @('prepare',$name,"other-$id",$payload) 'AgentProfilePending'
        Ih @('prepare',$name,$id,($payload+' ')) 'RequestConflict'
        $recovered = Ih $command
        Check ($recovered.replayed -and $recovered.workId -eq $id -and $created.port -eq $recovered.port -and $created.core.sha256 -eq $recovered.core.sha256) "Recovered $name with the original receipt and port"
        $again = Ih $command
        Check ($again.replayed -and $again.port -eq $created.port) "Completed replay for $name"
        $prepared += @{name=$name;port=$created.port}
    }
    Check ($prepared[0].port -ne $prepared[1].port) 'Automatically chosen ports are distinct'

    $unmanaged = Join-Path $root 'instances/Agent/Fixtures/unregistered'
    $null = [IO.Directory]::CreateDirectory($unmanaged)
    [IO.File]::WriteAllBytes((Join-Path $unmanaged 'data.bin'), [byte[]]::new(123))
    $inventory=Ih @('agent-list')
    Check ($inventory.items.Count -eq 3 -and ($inventory.items | Where-Object id -eq 'Fixtures/unregistered').logicalBytes -eq 123) 'Manager lists unmanaged Agent directories and logical sizes'
    $allFiles = Get-ChildItem -LiteralPath (Join-Path $root 'instances/Agent/Lobby/paper-agent') -File -Recurse -Force
    $expected = ($allFiles | Measure-Object -Property Length -Sum).Sum
    Check (($inventory.items | Where-Object id -eq 'Lobby/paper-agent').logicalBytes -eq $expected) 'Inventory size matches file lengths'
    $page = Ih @('agent-list','Fixtures/unregistered')
    Check ($page.items.Count -eq 2 -and !$page.nextCursor) 'Inventory resumes after an exact folder cursor'
    $report.inventory=$inventory

    foreach ($fixture in $prepared) {
        $name=$fixture.name
        $started = Ih @('start',$name,"start-$name")
        $active = @{name=$name;run=$started.run.id}
        $until=[DateTime]::UtcNow.AddSeconds(150)
        do {
            $status=Ih @('status',$name)
            if ($status.run.id -ne $active.run -or $status.state -notin 'Starting','Listening') { throw "$name exited during startup." }
            if ($status.consoleReady -and $status.state -eq 'Listening') { break }
            Start-Sleep -Milliseconds 350
        } while ([DateTime]::UtcNow -lt $until)
        Check ($status.consoleReady -and $status.state -eq 'Listening') "$name reached Done"
        $listener=@(Get-NetTCPConnection -LocalPort $fixture.port -State Listen)
        Check ($listener.Count -eq 1 -and $listener[0].LocalAddress -eq '127.0.0.1' -and $listener[0].OwningProcess -eq $status.run.java.pid) "$name listens only on loopback"
        if ($NativeVerifier) {
            $gui=Join-Path $BinaryDirectory 'InstanceHarbor.exe'
            $first=(& node $NativeVerifier $root $gui $name $active.run $Evidence | ConvertFrom-Json)
            if ($LASTEXITCODE -ne 0) {throw 'First GUI check failed.'}
        }
        $managerPath = Join-Path $root 'resident-manager.json'
        $beforeManager = Get-Content -LiteralPath $managerPath -Raw | ConvertFrom-Json -AsHashtable
        Ih @('manager-exit') | Out-Null
        $managerDeadline = [DateTime]::UtcNow.AddSeconds(15)
        while (Get-Process -Id $beforeManager.pid -ErrorAction SilentlyContinue) {
            if ([DateTime]::UtcNow -gt $managerDeadline) {throw 'Manager did not exit.'}
            Start-Sleep -Milliseconds 100
        }
        Ih @('manager-connect') | Out-Null
        $afterManager = Get-Content -LiteralPath $managerPath -Raw | ConvertFrom-Json -AsHashtable
        Check ($beforeManager.pid -ne $afterManager.pid -or $beforeManager.created -ne $afterManager.created) 'Manager process identity changed'
        $reopened=Ih @('status',$name)
        Check ($reopened.run.id -eq $active.run -and $reopened.ownerPid -eq $status.ownerPid) "$name retains the same Run ID and PID on Manager reopen"
        if ($NativeVerifier) {
            $second=(& node $NativeVerifier $root $gui $name $active.run $Evidence | ConvertFrom-Json)
            if ($LASTEXITCODE -ne 0) {throw 'Second GUI check failed.'}
            Check ($first.pid -ne $second.pid -and $first.run -eq $second.run) "$name retains the same Run ID across native GUI reopen"
        }
        $stopped=Ih @('stop',$name,"stop-$name",$active.run)
        Check ($stopped.state -eq 'Stopped') "$name stopped normally"
        $active=$null
        Check (!(Get-NetTCPConnection -LocalPort $fixture.port -State Listen -ErrorAction SilentlyContinue)) "$name port released"
        $report.runs += @{name=$name;run=$reopened.run.id;port=$fixture.port}
    }
    Check ((Get-FileHash -LiteralPath $policy.Path).Hash -eq $policy.Hash) 'Isolated scan policy unchanged'
    $report.result='PASS'
} catch {
    $report.error=$_.Exception.Message
    throw
} finally {
    if ($active) {
        try {
            $cleanup=Ih @('stop',$active.name,"cleanup-$($active.name)",$active.run)
            if ($cleanup.state -ne 'Stopped') { throw 'Test server could not be confirmed stopped.' }
        } catch { $report.cleanupError=$_.Exception.Message }
    }
    try { Ih @('manager-exit') | Out-Null } catch {
        if ($_.Exception.Message -notmatch 'ManagerUnavailable') { $report.cleanupError=$_.Exception.Message }
    }
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $Evidence 'result.json')
}
if ($report.cleanupError) { throw $report.cleanupError }
Get-Content -LiteralPath (Join-Path $Evidence 'result.json') -Raw
