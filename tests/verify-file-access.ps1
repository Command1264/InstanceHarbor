#Requires -Version 7.0
param([Parameter(Mandatory)][string]$Cli,[Parameter(Mandatory)][string]$Run)
$ErrorActionPreference='Stop'
if(Test-Path -LiteralPath $Run){throw 'Specify a fresh isolated directory.'}
$root=[IO.Directory]::CreateDirectory([IO.Path]::GetFullPath($Run)).FullName
$workspace=Join-Path $root 'workspace'
if(Test-Path -LiteralPath $workspace){throw 'Workspace must be new'}
$appdata=[IO.Directory]::CreateDirectory((Join-Path $root 'appdata')).FullName
function Ih([string[]]$arguments){
    $start=[Diagnostics.ProcessStartInfo]::new($cli)
    $start.UseShellExecute=$false
    $start.CreateNoWindow=$true
    $start.RedirectStandardOutput=$true
    $start.RedirectStandardError=$true
    $start.StandardOutputEncoding=[Text.Encoding]::UTF8
    $start.StandardErrorEncoding=[Text.Encoding]::UTF8
    $start.Environment['LOCALAPPDATA']=$appdata
    $start.ArgumentList.Add($workspace)
    foreach($arg in $arguments){$start.ArgumentList.Add($arg)}
    $proc=[Diagnostics.Process]::Start($start)
    try{
        $stdout=$proc.StandardOutput.ReadToEndAsync()
        $stderr=$proc.StandardError.ReadToEndAsync()
        if(!$proc.WaitForExit(12000)){$proc.Kill();$proc.WaitForExit();throw "CLI probe timeout: $($arguments[0])"}
        $out=$stdout.GetAwaiter().GetResult()
        $errorText=$stderr.GetAwaiter().GetResult().Trim()
        $value=if($out.Trim()){ConvertFrom-Json $out -NoEnumerate}
        [ordered]@{command=($arguments | Select-Object -First 1);code=$proc.ExitCode;error=$errorText;data=$value}
    }finally{$proc.Dispose()}
}
function Require($condition,[string]$message){if(!$condition){throw $message}}
function Fingerprint {
    $files=@(Get-ChildItem -LiteralPath $workspace -Recurse -File | Sort-Object FullName)
    @($files | ForEach-Object { "$([IO.Path]::GetRelativePath($workspace,$_.FullName)) $((Get-FileHash -LiteralPath $_.FullName).Hash)" })
}
Require ((Ih @('init')).code -eq 0) 'init failed'
$ports=@()
$owners=@()
try{
    for($i=0;$i -lt 2;$i++){
        $owner=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
        $owner.Start()
        $owners+=$owner
        $ports+=$owner.LocalEndpoint.Port
    }
}finally{foreach($owner in $owners){$owner.Stop()}}
$names=@('probe-healthy','probe-broken')
for($i=0;$i -lt $names.Length;$i++){
    $name=$names[$i]
    $dir=[IO.Directory]::CreateDirectory((Join-Path $workspace "instances/$name")).FullName
    @{name=$name;java='';port=$ports[$i];directory=$dir.Replace('\','/')} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $workspace "registry/$name.json") -Encoding utf8
}
$beforeList=Fingerprint
$lockPath=Join-Path $workspace 'manager.lock'
Require (!(Test-Path -LiteralPath $lockPath)) 'manager.lock unexpectedly exists'
$firstList=Ih @('list')
Require ($firstList.code -eq 0 -and $firstList.data.Count -eq 2) "synthetic registry unreadable: $($firstList.error)"
Require (!(Test-Path -LiteralPath $lockPath)) 'list created manager.lock'
Require (!(Compare-Object $beforeList (Fingerprint))) 'list wrote workspace data'
[IO.File]::WriteAllBytes($lockPath,[byte[]]@())
try {
    [IO.File]::SetAttributes($lockPath,[IO.FileAttributes]::ReadOnly)
    Require ((Ih @('list')).data.Count -eq 2) 'Read-only lock blocked list'
} finally {[IO.File]::SetAttributes($lockPath,[IO.FileAttributes]::Normal)}
function Hold($path,[IO.FileShare]$share=[IO.FileShare]::None){
    [IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,$share)
}
$handle=Hold $lockPath Read
try {Require ((Ih @('list')).data.Count -eq 2) 'Held lock blocked list'} finally {$handle.Dispose()}
$preferences=Join-Path $workspace 'preferences.json'
Require ((Ih @('save-preferences','{"theme":"dark","pins":[],"selectionOpacity":31}')).code -eq 0) 'Cannot seed preferences'
$record=Join-Path $workspace 'registry/probe-broken.json'
$baseline=[IO.File]::ReadAllBytes($record)
$handle=Hold $record
try{
    $partial=Ih @('list','--details')
    Require ($partial.data.rows.Count -eq 1 -and $partial.data.rows[0].name -ceq 'probe-healthy') 'Healthy row was lost'
    Require ($partial.data.issues[0].file -ceq 'probe-broken.json' -and $partial.data.issues[0].osError -eq 32) 'Sharing violation missing'
    Require ((Ih @('status','probe-healthy')).data.state -ceq 'Stopped') 'Healthy status failed'
} finally {$handle.Dispose()}
try{
    [IO.File]::WriteAllText($record,'{ bad')
    $handle=Hold $preferences
    try{
        $combined=Ih @('list','--details')
        Require ($combined.data.rows.Count -eq 1 -and $combined.data.issues[0].reason -ceq 'InvalidRecord') 'Combined faults lost row or parse diagnostic'
        $read=Ih @('preferences')
        $write=Ih @('save-preferences','{"theme":"light","pins":[]}')
        Require ($read.code -ne 0 -and $read.error -match 'preferences\.json.*os error 32') 'Missing preference read diagnosis'
        Require ($write.code -ne 0 -and $write.error -match 'preferences\.json.*os error 32') 'Save did not stop at unreadable settings'
    } finally {$handle.Dispose()}
    Require ((Ih @('preferences')).data.theme -ceq 'dark') 'Original preferences were lost'
} finally {[IO.File]::WriteAllBytes($record,$baseline)}
Require ((Ih @('list','--details')).data.issues.Count -eq 0) 'Registry did not recover'
$revision=(Ih @('preferences-snapshot')).data.revision
Require ($revision.Length -eq 64) 'Missing preferences revision'
Require ((Ih @('save-preferences','{"theme":"dark","pins":[],"selectionOpacity":37}')).code -eq 0) 'Cannot replace preferences'
$stale=Ih @('save-preferences','{"theme":"light","pins":[]}', $revision)
Require ($stale.code -ne 0 -and $stale.error -ceq 'PreferencesSettingsConflict') 'Stale preference draft was not rejected'
Require ((Ih @('preferences')).data.selectionOpacity -eq 37) 'Stale draft overwrote outside change'

$held=Hold $preferences ReadWrite
$beforeFile=(Get-FileHash -LiteralPath $preferences).Hash
try {
    $failedReplace=Ih @('save-preferences','{"theme":"light","pins":[]}')
    Require ($failedReplace.code -ne 0 -and $failedReplace.error -match 'FileAccess\[replace\].*preferences\.json') 'Atomic replace did not report the held destination'
    Require (!(Get-ChildItem -LiteralPath $workspace -Filter 'preferences.*.tmp')) 'Failed replace left a temporary file'
} finally {$held.Dispose()}
Require ((Get-FileHash -LiteralPath $preferences).Hash -ceq $beforeFile) 'Failed replace changed the original'
$writer=[IO.File]::Open($lockPath,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::ReadWrite)
try {
    $writer.Lock(0,1)
    Require ((Ih @('list')).data.Count -eq 2) 'Exclusive writer range lock blocked list'
    $conflictingWriter=Ih @('save-preferences','{"theme":"light","pins":[]}')
    Require ($conflictingWriter.code -ne 0 -and $conflictingWriter.error -ceq 'ManagerBusy') 'Writer coordination was lost'
} finally {$writer.Dispose()}
$globalJava=Join-Path $workspace 'global-java.json'
Require ((Ih @('global-java-settings')).data.mode -ceq 'automatic') 'Missing Java settings should default'
Require ((Ih @('preferences-snapshot')).data.preferences.theme -ceq 'dark') 'Cannot read preferences snapshot'
[IO.File]::WriteAllText($globalJava,'{"mode":"automatic","manualPath":""}')
$policy=Join-Path $workspace 'scan-policy.json'
$settings=@(
    @{file=$globalJava;read='global-java-settings';write='save-global-java-settings';payload=@{mode='automatic';manualPath='D:/synthetic/java.exe'}},
    @{file=$policy;read='scan-policy';write='save-scan-policy';payload=@{allowPaths=@();excludePaths=@()}}
)
foreach($setting in $settings){
    $readable=Ih @($setting.read)
    Require ($readable.code -eq 0) "Cannot load $($setting.read)"
    $setting.payload.revision=$readable.data.revision
    $original=[IO.File]::ReadAllBytes($setting.file)
    $fileHash=(Get-FileHash -LiteralPath $setting.file).Hash
    $handle=Hold $setting.file
    try {
        $blockedRead=Ih @($setting.read)
        $blockedWrite=Ih @($setting.write,($setting.payload|ConvertTo-Json -Compress))
        foreach($response in $blockedRead,$blockedWrite){
            Require ($response.code -ne 0 -and $response.error.Contains([IO.Path]::GetFileName($setting.file)) -and $response.error -match 'os error 32') "No sharing diagnosis for $($setting.read)"
        }
    } finally {$handle.Dispose()}
    Require ((Get-FileHash -LiteralPath $setting.file).Hash -ceq $fileHash) "Blocked operation changed $($setting.read)"
    try {
        [IO.File]::WriteAllText($setting.file,'{ invalid')
        Require ((Ih @($setting.read)).code -ne 0) "Malformed settings fell back to defaults: $($setting.read)"
        Require ((Ih @($setting.write,($setting.payload|ConvertTo-Json -Compress))).code -ne 0) "Malformed settings were overwritten: $($setting.read)"
        Require ([IO.File]::ReadAllText($setting.file) -ceq '{ invalid') 'Malformed file was modified'
    } finally {[IO.File]::WriteAllBytes($setting.file,$original)}
    Require ((Ih @($setting.read)).data.revision -ceq $setting.payload.revision) "Settings did not recover: $($setting.read)"
}
$originalPreferences=[IO.File]::ReadAllBytes($preferences)
try {
    [IO.File]::WriteAllText($preferences,'{ invalid')
    Require ((Ih @('preferences')).error -match 'PreferencesInvalidJson.*preferences.json') 'Invalid preferences were not diagnosed'
    Require ((Ih @('save-preferences','{"theme":"light","pins":[]}')).code -ne 0) 'Invalid preferences were overwritten'
    Require ([IO.File]::ReadAllText($preferences) -ceq '{ invalid') 'Invalid preferences changed'
} finally {[IO.File]::WriteAllBytes($preferences,$originalPreferences)}
$policyBackup="$policy.probe"
try {
    [IO.File]::Move($policy,$policyBackup)
    $missingPolicy=Ih @('scan-policy')
    Require ($missingPolicy.code -ne 0 -and $missingPolicy.error -match 'scan-policy\.json.*os error 2') 'Missing policy was not distinguished'
    Require (!(Test-Path -LiteralPath $policy)) 'Read recreated missing scan policy'
} finally {if(Test-Path -LiteralPath $policyBackup){[IO.File]::Move($policyBackup,$policy)}}
# A held registration must block a start when the port namespace is unconfirmed.
try {
    [IO.File]::WriteAllText($record,'{ invalid')
    $blockedStart=Ih @('start','probe-healthy','registry-fault-probe')
    Require ($blockedStart.code -ne 0 -and $blockedStart.error -match 'RegistryConflictUnconfirmed.*probe-broken') 'Unconfirmed namespace did not block a start'
    Require (!(Test-Path -LiteralPath (Join-Path $workspace 'runs/probe-healthy'))) 'Blocked start created a run'
} finally {[IO.File]::WriteAllBytes($record,$baseline)}
[ordered]@{result='PASS';workspace=$workspace;cliSha256=(Get-FileHash -LiteralPath $Cli).Hash;heldRecord=$partial.data.issues;combined=$combined.data.issues;stale=$stale.error;writerGuard=$conflictingWriter.error;failedReplace=$failedReplace.error;blockedStart=$blockedStart.error;settingsChecked=@('preferences.json','global-java.json','scan-policy.json')} |
    ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Run 'cli-results.json')
