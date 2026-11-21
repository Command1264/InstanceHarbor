[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Evidence,
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$Java,
    [Parameter(Mandatory)][string]$Paper205,
    [Parameter(Mandatory)][string]$Paper222,
    [Parameter(Mandatory)][string]$Mojang,
    [Parameter(Mandatory)][string]$Contact,
    [switch]$PrepareOnly
)

$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
$Evidence=[IO.Path]::GetFullPath($Evidence)
if(Test-Path -LiteralPath $Evidence){throw 'Specify a new isolated evidence directory.'}
$inputs=@($Paper205,$Paper222,$Mojang)
$hashes=@(
    'a0a58f532f08054ea92eb916d3edb6ed3c86429d77175317304f6e995cce9968',
    'f649771a2190214306fb0cfba22c7b2cbac5df6308b1158309593603ed07d488',
    'd052f14d7a173734fba553711e5b570162e2f2a313267ee31a21b975a679be64'
)
for($i=0;$i -lt $inputs.Length;$i++){
    if ((Get-FileHash -LiteralPath $inputs[$i]).Hash -ne $hashes[$i]) { throw 'Paper source identity changed.' }
}
$cli=(Get-Item -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe')).FullName
$root=Join-Path $Evidence 'workspace'
$store=Join-Path $root 'staging/core-prefetch'
$names=@('paper-prefetch-a','paper-prefetch-b')
$checks=[Collections.Generic.List[string]]::new()
$processEnv = 'INSTANCEHARBOR_PAPERMC_CONTACT_URL'
$oldContact=[Environment]::GetEnvironmentVariable($processEnv,'Process')
$armed=$false
$candidate=$null
$ports=@()

function Ih([string[]]$argv, [string]$ExpectedFailure){
    $reply=& $cli $root @argv 2>&1
    if($ExpectedFailure){
        if($LASTEXITCODE -eq 0 -or "$reply" -notmatch $ExpectedFailure){ throw "Expected $ExpectedFailure, got $reply" }
        return
    }
    if($LASTEXITCODE -ne 0){throw "ihctl $($argv[0]) failed: $reply"}
    ConvertFrom-Json -InputObject ($reply -join [Environment]::NewLine) -NoEnumerate
}
function Check([bool]$condition,[string]$label){
    if(!$condition){throw "Check failed: $label"}
    $checks.Add($label)
}
function Wait-Proof([scriptblock]$sample,[int]$seconds){
    $until=[DateTime]::UtcNow.AddSeconds($seconds)
    do {
        $proof=& $sample
        if($proof){return $proof}
        Start-Sleep -Milliseconds 350
    }while([DateTime]::UtcNow -lt $until)
    throw 'Timed out waiting for isolated proof.'
}
function Set-Prefetch([bool]$enabled){
    $prefs=Ih @('preferences')
    $prefs.corePrefetch=$enabled
    Ih @('save-preferences',($prefs|ConvertTo-Json -Compress -Depth 10)) | Out-Null
    Check ((Ih @('preferences')).corePrefetch -eq $enabled) "Saved prefetch setting $enabled"
}
function ManagedProcess {
    $descriptor=Get-Content -LiteralPath (Join-Path $root 'resident-manager.json') -Raw | ConvertFrom-Json
    $process=Get-Process -Id $descriptor.pid
    if($process.Path -ne $cli){throw 'Fixture Manager uses an unexpected binary.'}
    $process
}
function Stop-Manager {
    $process=ManagedProcess
    Ih @('manager-exit') | Out-Null
    if(!$process.WaitForExit(6000)){throw 'Fixture Manager did not exit.'}
}
function Stop-Fixture([string]$name,[string]$request) {
    $current=Ih @('status',$name)
    if($current.state -eq 'Stopped'){return}
    if(!$current.run.id -or $current.state -notin 'Starting','Listening','StopTimeout'){throw "Unexpected state $($current.state) for $name"}
    $stopped=Ih @('stop',$name,$request,$current.run.id)
    if($stopped.state -ne 'Stopped'){throw "Normal stop could not be verified for $name"}
}

try{
    [Environment]::SetEnvironmentVariable($processEnv,$Contact,'Process')
    New-Item -ItemType Directory -Path $Evidence | Out-Null
    Ih @('init') | Out-Null
    $leases=@()
    try{
        foreach($name in $names){
            $listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
            $listener.Start()
            $leases+=$listener
            $ports+=$listener.LocalEndpoint.Port
        }
    } finally {foreach($listener in $leases){$listener.Stop()}}
    $policy=(Get-FileHash -LiteralPath (Join-Path $root 'scan-policy.json')).Hash
    for($i=0;$i -lt $names.Length;$i++){
        Ih @('create',$names[$i],"new-$i",$Java,"$($ports[$i])",$inputs[$i],$Mojang) | Out-Null
    }
    @{workspace=$root;ports=$ports;instanceNames=$names;sourceSha256=$hashes;binary=$cli} |
        ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $Evidence 'fixture.json')
    if($PrepareOnly){return}
    Ih @('manager-connect') | Out-Null
    $armed=$true
    $source=Ih @('core-check',$names[0],$names[1])
    $candidate=$source.rows[0].candidate
    Check ($candidate.sha256 -and $candidate.sha256 -eq $source.rows[1].candidate.sha256) 'Two different Paper builds resolve to one official candidate'
    $file=Join-Path $store "$($candidate.sha256).jar"
    Start-Sleep -Seconds 4
    Check (!(Ih @('preferences')).corePrefetch -and !(Test-Path -LiteralPath $store)) 'Default off does not create a prefetch store'

    $running=Ih @('start',$names[1],'start-before-prefetch')
    Wait-Proof {
        $row=Ih @('status',$names[1])
        if($row.state -eq 'Listening' -and $row.consoleReady -and $row.run.id -eq $running.run.id){$row}
    } 170 | Out-Null
    Set-Prefetch $true
    Wait-Proof {
        $a=Ih @('core-state',$names[0])
        $b=Ih @('core-state',$names[1])
        if($a.prepared.sha256 -eq $candidate.sha256 -and $b.prepared.sha256 -eq $candidate.sha256){$true}
    } 120 | Out-Null
    $published=Get-Item -LiteralPath $file
    Check (@(Get-ChildItem -LiteralPath $store -Filter '*.jar').Count -eq 1 -and (Get-FileHash -LiteralPath $file).Hash -eq $candidate.sha256) 'One hash-verified JAR serves both instances'
    $afterPrep=Ih @('status',$names[1])
    Check ($afterPrep.run.id -eq $running.run.id -and $afterPrep.state -eq 'Listening' -and (Ih @('core-state',$names[1])).currentSha256 -eq $hashes[1]) 'Prefetch preserves the running server and its original core'

    $oldManager=(ManagedProcess).Id
    Stop-Manager
    [Environment]::SetEnvironmentVariable($processEnv,$null,'Process')
    Ih @('manager-connect') | Out-Null
    Check ((ManagedProcess).Id -ne $oldManager -and (Get-Item -LiteralPath $file).LastWriteTimeUtc -eq $published.LastWriteTimeUtc) 'A new Manager keeps the same candidate file without a configured network source'
    $first=Ih @('core-install-latest',$names[0],'install-a',$hashes[0],$candidate.sha256)
    Check ($first.outcome -eq 'applied' -and (Ih @('core-state',$names[0])).backups[0].id -eq 'install-a') 'First isolated Paper installed offline with its own backup'
    Check ((Get-Item -LiteralPath $file).LastWriteTimeUtc -eq $published.LastWriteTimeUtc) 'The pending second instance retains the same bytes'
    Ih @('core-install-latest',$names[1],'install-b',$hashes[1],$candidate.sha256) 'InstanceNotConfirmedStopped|RuntimeDirectoryBusy|InstanceDirectoryInUse|ManagerBusy'
    Check ((Ih @('core-state',$names[1])).currentSha256 -eq $hashes[1] -and (Test-Path -LiteralPath $file)) 'Refused installation keeps original and shared candidate for retry'
    Stop-Fixture $names[1] 'stop-for-retry'
    $second=Ih @('core-install-latest',$names[1],'install-b',$hashes[1],$candidate.sha256)
    Check ($second.outcome -eq 'applied' -and (Ih @('core-state',$names[1])).backups[0].id -eq 'install-b') 'Identical retry installs the second Paper with its own backup'
    Wait-Proof { if(!(Test-Path -LiteralPath $file)){$true} } 25 | Out-Null
    Check $true 'The final completed installation releases the unused candidate'

    Set-Prefetch $false
    Ih @('core-restore',$names[0],'restore-a',$candidate.sha256,'install-a') | Out-Null
    [Environment]::SetEnvironmentVariable($processEnv,$Contact,'Process')
    $direct=Ih @('core-install-latest',$names[0],'direct-a',$hashes[0],$candidate.sha256)
    Check ($direct.outcome -eq 'applied' -and @(Get-ChildItem -LiteralPath $store -Filter '*.jar').Count -eq 0) 'With prefetch off, confirmed update uses the original download-and-install path'
    Check ((Get-FileHash -LiteralPath (Join-Path $root 'scan-policy.json')).Hash -eq $policy) 'Isolated scan policy unchanged'
    for($i=0;$i -lt $inputs.Length;$i++){
        Check ((Get-FileHash -LiteralPath $inputs[$i]).Hash -eq $hashes[$i]) "Read-only source $i unchanged"
    }
    $result='PASS'
} catch {
    $failure=$_.Exception.Message
    throw
} finally {
    [Environment]::SetEnvironmentVariable($processEnv,$oldContact,'Process')
    if($armed){
        foreach($name in $names){
            try {Stop-Fixture $name "cleanup-$name"} catch {$failure=@($failure,$_.Exception.Message) -join ' / '}
        }
        try {Stop-Manager} catch {$failure=@($failure,$_.Exception.Message) -join ' / '}
        $listening=@(Get-NetTCPConnection -LocalPort $ports -State Listen -ErrorAction SilentlyContinue)
        if($listening.Count){$failure=@($failure,'An isolated port remains occupied') -join ' / '}
    }
    if(!$PrepareOnly -and (Test-Path -LiteralPath $Evidence)){
        @{result=if($failure){'FAIL'}else{$result};error=$failure;candidate=$candidate;checks=$checks;ports=$ports} |
            ConvertTo-Json -Depth 7 | Set-Content -LiteralPath (Join-Path $Evidence 'result.json')
    }
}
if($failure){throw $failure}
Get-Content -LiteralPath (Join-Path $Evidence 'result.json') -Raw
