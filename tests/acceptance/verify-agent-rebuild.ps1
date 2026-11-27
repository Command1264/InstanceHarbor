[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Evidence,
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$Java,
    [Parameter(Mandatory)][string]$OldPaper,
    [Parameter(Mandatory)][string]$NewPaper,
    [Parameter(Mandatory)][string]$Vanilla,
    [Parameter(Mandatory)][string]$LuckPerms
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Evidence = [IO.Path]::GetFullPath($Evidence)
if (Test-Path -LiteralPath $Evidence) { throw 'Choose a new isolated directory.' }
$root = Join-Path $Evidence 'workspace'
$cli = (Get-Item -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe')).FullName
$name = 'rebuild-paper'
$target = Join-Path $root "instances/Agent/Lobby/$name"
$oldCore = 'a0a58f532f08054ea92eb916d3edb6ed3c86429d77175317304f6e995cce9968'
$newCore = 'f649771a2190214306fb0cfba22c7b2cbac5df6308b1158309593603ed07d488'
$checks = [Collections.Generic.List[string]]::new()
$stopId = 0
$created = $false
function Ih([string[]]$argv, [string]$ExpectedError) {
    $reply = & $cli $root @argv 2>&1
    if ($ExpectedError) {
        if ($LASTEXITCODE -eq 0 -or "$reply" -notmatch $ExpectedError) { throw "Unexpected result $reply" }
        $checks.Add($ExpectedError)
        return
    }
    if ($LASTEXITCODE -ne 0) { throw "ihctl $($argv[0]) failed. $reply" }
    ConvertFrom-Json -InputObject ($reply -join [Environment]::NewLine) -NoEnumerate
}
function Check([bool]$Value, [string]$Label) {
    if (!$Value) { throw "Failed check $Label" }
    $checks.Add($Label)
}
function Wait-Ready([string]$ExpectedRun) {
    $until = [datetime]::UtcNow.AddSeconds(180)
    do {
        $status = Ih @('status',$name)
        if ($status.run.id -ne $ExpectedRun -or $status.state -notin 'Starting','Listening') { throw 'Paper run changed or exited' }
        if ($status.consoleReady -and $status.state -eq 'Listening') { return }
        Start-Sleep -Milliseconds 350
    } while ([datetime]::UtcNow -lt $until)
    throw 'Paper did not reach Done.'
}
function Stop-OwnPaper {
    $status = Ih @('status',$name)
    if ($status.state -eq 'Stopped') { return }
    if (!$status.run.id -or $status.state -notin 'Starting','Listening','StopTimeout') { throw 'Could not verify the isolated run.' }
    $script:stopId += 1
    $stopped = Ih @('stop',$name,"stop-$stopId",$status.run.id)
    if ($stopped.state -ne 'Stopped') { throw 'Isolated Paper did not stop normally.' }
}

Check ((Get-FileHash -LiteralPath $OldPaper).Hash -eq $oldCore -and
       (Get-FileHash -LiteralPath $NewPaper).Hash -eq $newCore) 'Pinned Paper inputs unchanged'
New-Item -ItemType Directory -Path $Evidence | Out-Null
$socket=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$socket.Start()
try { $port=$socket.LocalEndpoint.Port } finally { $socket.Stop() }
$payload = [ordered]@{
    audience='Agent';project='Lobby';instance=$name;java=$Java;port=$port
    coreJar=$OldPaper;mojangJar=$Vanilla;permissionJar=$LuckPerms
}
function Raw { $payload | ConvertTo-Json -Depth 4 -Compress }
try {
    Ih @('init') | Out-Null
    $policy = (Get-FileHash -LiteralPath (Join-Path $root 'scan-policy.json')).Hash
    Ih @('prepare',$name,'create-original',(Raw)) | Out-Null
    $created = $true
    $payload.coreJar = $NewPaper
    Ih @('prepare',$name,'no-replace',(Raw)) 'AlreadyManaged'
    $payload.replace = $true
    $payload.audience = 'Human'
    Ih @('prepare',$name,'wrong-audience',(Raw)) 'HumanInstancesCannotBeRebuilt'
    $payload.audience = 'Agent'
    $payload.instance = 'another-directory'
    Ih @('prepare',$name,'wrong-target',(Raw)) 'AgentRebuildIdentityChanged'
    $payload.instance = $name
    $payload.coreJar = Join-Path $root 'missing.jar'
    Ih @('prepare',$name,'invalid-source',(Raw)) 'InvalidPrepareArtifact|os error'
    $payload.coreJar = $NewPaper
    Check ((Get-FileHash -LiteralPath (Join-Path $target 'server.jar')).Hash -eq $oldCore) 'Refusals leave the original JAR intact'

    $original = Ih @('start',$name,'launch-original')
    Wait-Ready $original.run.id
    Ih @('prepare',$name,'while-running',(Raw)) 'InstanceNotConfirmedStopped|InstanceDirectoryInUse'
    Check ((Ih @('status',$name)).run.id -eq $original.run.id) 'Running original remains online after refusal'
    Stop-OwnPaper
    $world = Join-Path $target 'world/level.dat'
    $worldHash = (Get-FileHash -LiteralPath $world).Hash
    $oldConfig = (Get-FileHash -LiteralPath (Join-Path $target 'server.properties')).Hash
    $oldRegistry = Get-Content -LiteralPath (Join-Path $root "registry/$name.json") -Raw | ConvertFrom-Json
    $raw = Raw
    $success = Ih @('prepare',$name,'replace-original',$raw)
    $same = Ih @('prepare',$name,'replace-original',$raw)
    Check ($success.state -eq 'Stopped' -and $same.replayed -and $same.archive -eq $success.archive) 'Replacement publishes once and returns the same archive on retry'
    Ih @('prepare',$name,'replace-original',($raw+' ')) 'RequestConflict'
    $archive = Join-Path $root 'agent-rebuild-archives/replace-original'
    $old = Join-Path $archive 'instance'
    Check ((Get-FileHash -LiteralPath (Join-Path $old 'world/level.dat')).Hash -eq $worldHash -and
        (Get-FileHash -LiteralPath (Join-Path $old 'server.properties')).Hash -eq $oldConfig -and
        (Get-FileHash -LiteralPath (Join-Path $old 'server.jar')).Hash -eq $oldCore) 'Original world, configuration and JAR are preserved in the archive'
    $archivedRun = Get-Content -LiteralPath (Join-Path $archive 'run/identity.json') -Raw | ConvertFrom-Json
    $archivedRegistration = Get-Content -LiteralPath (Join-Path $archive 'registration.json') -Raw | ConvertFrom-Json
    Check ($archivedRun.id -eq $original.run.id -and $archivedRegistration.port -eq $oldRegistry.port) 'Archive preserves the former registration and run identity'
    Check (!(Test-Path -LiteralPath $world) -and (Get-FileHash -LiteralPath (Join-Path $target 'server.jar')).Hash -eq $newCore) 'Replacement is a fresh Paper generation'
    $ready = Ih @('status',$name)
    Check ($ready.state -eq 'Stopped' -and !$ready.run -and $ready.port -eq $port) 'Replacement keeps the port and stays stopped with no previous run'

    $new = Ih @('start',$name,'launch-replacement')
    Wait-Ready $new.run.id
    Check ($new.run.id -ne $original.run.id) 'Replacement reaches Done with a different run'
    Stop-OwnPaper
    Check (!(Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue)) 'Isolated port released'
    Check ((Get-FileHash -LiteralPath (Join-Path $root 'scan-policy.json')).Hash -eq $policy) 'Isolated scan policy preserved'
    $result = 'PASS'
} catch {
    $failure = $_.Exception.Message
    throw
} finally {
    if ($created) {
        try { Stop-OwnPaper } catch { $failure = (@($failure,$_.Exception.Message) | Where-Object { $_ }) -join ' / ' }
    }
    @{result=$result;error=$failure;checks=$checks;port=$port;workspace=$root;archive=$archive} |
        ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Evidence 'result.json')
}
if ($failure) { throw $failure }
Get-Content -LiteralPath (Join-Path $Evidence 'result.json') -Raw
