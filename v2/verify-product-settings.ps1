[CmdletBinding()]
param([Parameter(Mandatory)][string]$Workspace, [string]$BinaryDirectory=(Join-Path $PSScriptRoot 'src-tauri/target/debug'), [switch]$Recheck)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
$root=(Resolve-Path -LiteralPath $Workspace).Path
$boundary=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '.evidence/vue-product-20261109'))+[IO.Path]::DirectorySeparatorChar
if(!$root.StartsWith($boundary,[StringComparison]::OrdinalIgnoreCase)){throw 'Not a task-owned workspace.'}
$manifest=Get-Content -LiteralPath (Join-Path $root 'task-manifest.json') -Raw|ConvertFrom-Json
if($manifest.owner -ne 'vue-product-20261109'){throw 'Workspace owner mismatch.'}
$cli=Join-Path $BinaryDirectory 'ihctl.exe'
$checks=[Collections.Generic.List[object]]::new()
function Check($name,$condition){if(!$condition){throw "Check failed: $name"};$checks.Add(@{name=$name;passed=$true})}
function Ih([string[]]$arguments){$reply=& $cli $root @arguments 2>&1;if($LASTEXITCODE -ne 0){throw "$reply"};$reply|ConvertFrom-Json}
if((Ih @('status','paper-agent')).state -ne 'Stopped'){throw 'Expected the isolated Paper to remain stopped.'}
$beforeEmergency=Ih @('status','paper-agent')
$stoppedRun=$beforeEmergency.run.id
if($stoppedRun){
    Ih @('force-stop','paper-agent',('settings-cancel-'+[Guid]::NewGuid().ToString('N')),$stoppedRun)|Out-Null
    $cancelled=Get-Content -LiteralPath (Join-Path $root 'runs/paper-agent/restart-cancelled.json') -Raw|ConvertFrom-Json
    Check 'Emergency after normal exit records a run-bound restart cancellation' ($cancelled.run -eq $stoppedRun -and (Ih @('status','paper-agent')).state -eq 'Stopped')
    $guard=@{previousRun=$stoppedRun;stopToken=$beforeEmergency.stopToken}|ConvertTo-Json -Compress
    $request='settings-stale-start-'+[Guid]::NewGuid().ToString('N')
    $reply=& $cli $root start-if-current paper-agent $request $guard 2>&1
    Check 'Already queued start is rejected after a newer emergency intent' ($LASTEXITCODE -ne 0 -and "$reply" -match 'StartIntentChanged' -and (Ih @('status','paper-agent')).state -eq 'Stopped')
    Check 'Rejected stale start cannot create a misleading work receipt' (!(Test-Path -LiteralPath (Join-Path $root "jobs/$request.json")))
}
$settings=Ih @('instance-settings','paper-agent')
$reply=& $cli $root save-instance-settings paper-agent (@{java=$settings.java;port=$settings.port;revision=('0'*64)}|ConvertTo-Json -Compress) 2>&1
Check 'Stale Java and port settings are rejected' ($LASTEXITCODE -ne 0 -and "$reply" -match 'SettingsConflict')
$initial=Ih @('get-initialization')
$changed=Ih @('save-initialization',(@{revision=$initial.revision;values=@{viewDistance=7}}|ConvertTo-Json -Compress))
$reply=& $cli $root save-initialization (@{revision=$initial.revision;values=@{maxPlayers=13}}|ConvertTo-Json -Compress) 2>&1
Check 'Stale initialization cannot overwrite another editor' ($LASTEXITCODE -ne 0 -and "$reply" -match 'SettingsConflict')
Ih @('save-initialization',(@{revision=$changed.revision;values=$initial.values}|ConvertTo-Json -Compress))|Out-Null
$props=Join-Path $root 'instances/paper-agent/server.properties'
$original=[IO.File]::ReadAllText($props)
try{
    [IO.File]::WriteAllText($props,$original+"`nrcon.password=isolated-regression-secret`n")
    $response=Ih @('world-settings','paper-agent')
    $serialized=$response|ConvertTo-Json -Depth 8
    Check 'Opaque revision excludes noneditable secrets' ($response.revision -match '^[0-9a-f]{64}$' -and $serialized -notmatch 'isolated-regression-secret|rcon.password')
    $changed=$response.revision
    [IO.File]::WriteAllText($props,$original+"`nrcon.password=changed-isolated-secret`n")
    $reply=& $cli $root save-world-settings paper-agent (@{revision=$changed;values=@{motd='Conflict probe'}}|ConvertTo-Json -Compress) 2>&1
    Check 'Hidden field changes invalidate revision' ($LASTEXITCODE -ne 0 -and "$reply" -match 'SettingsConflict')
}finally{[IO.File]::WriteAllText($props,$original)}
$other=Join-Path $root 'instances/second-agent'
if(Test-Path -LiteralPath $other){
    if(!$Recheck -or !(Test-Path -LiteralPath (Join-Path $root 'settings-acceptance.json'))){throw 'Second fixture already exists; refusing to overwrite.'}
    $prior=Get-Content -LiteralPath (Join-Path $root 'settings-acceptance.json') -Raw|ConvertFrom-Json
    if($prior.status -ne 'PASS' -or $prior.workspace -ne $root -or
       (Get-ChildItem -LiteralPath $other -Directory).Count -ne 0 -or
       (Get-Item -LiteralPath $other).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint) -or
       (Get-FileHash -LiteralPath (Join-Path $other 'server.jar')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $root 'instances/paper-agent/server.jar')).Hash){throw 'Previous disposable fixture ownership/content mismatch.'}
}else{
    New-Item -ItemType Directory -Path $other|Out-Null
    Copy-Item -LiteralPath (Join-Path $root 'instances/paper-agent/server.jar') -Destination (Join-Path $other 'server.jar')
}
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$listener.Start();$otherPort=$listener.LocalEndpoint.Port;$listener.Stop()
[IO.File]::WriteAllText((Join-Path $other 'server.properties'),"server-ip=127.0.0.1`nserver-port=$otherPort`n")
Ih @('import','second-agent',('settings-second-'+[Guid]::NewGuid().ToString('N')),$manifest.java)|Out-Null
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$listener.Start();$conflictPort=$listener.LocalEndpoint.Port;$listener.Stop()
$pending=@()
foreach($name in 'paper-agent','second-agent'){
    $settings=Ih @('instance-settings',$name)
    $info=[Diagnostics.ProcessStartInfo]::new($cli)
    $info.UseShellExecute=$false;$info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
    foreach($argument in @($root,'save-instance-settings',$name,(@{java=$manifest.java;port=$conflictPort;revision=$settings.revision}|ConvertTo-Json -Compress))){$info.ArgumentList.Add($argument)}
    $pending+=[Diagnostics.Process]::Start($info)
}
$results=@()
foreach($runtimeProcess in $pending){
    if(!$runtimeProcess.WaitForExit(20000)){throw 'Isolated configuration process timed out.'}
    $results+=@{exit=$runtimeProcess.ExitCode;output=$runtimeProcess.StandardOutput.ReadToEnd();error=$runtimeProcess.StandardError.ReadToEnd()}
    $runtimeProcess.Dispose()
}
Check 'Concurrent port assignment has only one winner' (@($results|Where-Object exit -eq 0).Count -eq 1)
Check 'Concurrent conflict is explicit' (@($results|Where-Object {$_.error -match 'ManagerBusy|PortAlreadyRegistered'}).Count -eq 1)
$rows=@(Ih @('list'))
Check 'No duplicate registered ports' (@($rows|Group-Object port|Where-Object Count -gt 1).Count -eq 0)
$settings=Ih @('instance-settings','paper-agent')
Ih @('save-instance-settings','paper-agent',(@{java=$manifest.java;port=$manifest.port;revision=$settings.revision}|ConvertTo-Json -Compress))|Out-Null
Ih @('remove-management','second-agent','delete')|Out-Null
Check 'Main fixture preserved stopped' ((Ih @('status','paper-agent')).state -eq 'Stopped')
$result=@{status='PASS';workspace=$root;checks=$checks}
$result|ConvertTo-Json -Depth 7|Set-Content -LiteralPath (Join-Path $root 'settings-acceptance.json')
$result|ConvertTo-Json -Depth 7
