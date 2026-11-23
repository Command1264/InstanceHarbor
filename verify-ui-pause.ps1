[CmdletBinding()]
param([Parameter(Mandatory)][string]$Workspace)
$ErrorActionPreference='Stop'
$root=(Resolve-Path -LiteralPath $Workspace).Path
$boundary=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '.evidence/vue-product-20261109'))+[IO.Path]::DirectorySeparatorChar
if(!$root.StartsWith($boundary,[StringComparison]::OrdinalIgnoreCase)){throw 'Not a task-owned workspace.'}
$owner=Get-Content -LiteralPath (Join-Path $root 'task-manifest.json') -Raw|ConvertFrom-Json
if($owner.owner -ne 'vue-product-20261109'){throw 'Workspace owner mismatch.'}
$run=Get-Content -LiteralPath (Join-Path $root 'runs/paper-agent/identity.json') -Raw|ConvertFrom-Json
$child=Get-Process -Id $run.java.pid
if($child.StartTime.ToFileTimeUtc() -ne $run.java.created){throw 'Process identity mismatch.'}
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class IhProductFault {
    [DllImport("ntdll.dll")] public static extern int NtSuspendProcess(IntPtr handle);
    [DllImport("ntdll.dll")] public static extern int NtResumeProcess(IntPtr handle);
}
'@
$signal=Join-Path $root ('resume-'+[Guid]::NewGuid().ToString('N'))
$paused=$false
try{
    if([IhProductFault]::NtSuspendProcess($child.Handle) -ne 0){throw 'Could not pause the isolated Java.'}
    $paused=$true
    @{pid=$run.java.pid;created=$run.java.created;run=$run.id;resumeSignal=$signal;deadlineSeconds=90}|ConvertTo-Json|
        Set-Content -LiteralPath (Join-Path $root 'ui-pause.json')
    'Only the verified isolated Java is paused; automatic resume watchdog is active.'
    $deadline=[DateTime]::UtcNow.AddSeconds(90)
    while(!$child.HasExited -and !(Test-Path -LiteralPath $signal) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 200;$child.Refresh()}
}finally{
    $child.Refresh()
    if($paused -and !$child.HasExited){
        if([IhProductFault]::NtResumeProcess($child.Handle) -ne 0){throw 'Could not resume the isolated Java; preserve this process identity.'}
    }
    $child.Dispose()
}
