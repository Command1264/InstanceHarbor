[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Evidence,
    [Parameter(Mandatory)][string]$Runtime,
    [ValidateSet('Fixture','Paper')][string]$Mode = 'Fixture',
    [string]$PaperCache,
    [string]$AgentLibraries,
    [string]$Java,
    [string]$LuckPerms
)
$ErrorActionPreference = 'Stop'
$Evidence = [IO.Path]::GetFullPath($Evidence)
$parent = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '.evidence'))
if ((Split-Path -Parent $Evidence) -ne $parent -or (Split-Path -Leaf $Evidence) -notmatch '^native-console-[a-z0-9-]+$') {
    throw 'Choose a new native-console evidence directory within this checkout.'
}
if (Get-CimInstance Win32_Process -Filter "Name='InstanceHarbor.exe' OR Name='instanceharbor-v2-acceptance.exe'" |
    Where-Object CommandLine -NotMatch '\s+native-console$') {
    throw 'An existing 2.0 window is open.'
}
$receipt = Get-Content -LiteralPath (Join-Path $PSScriptRoot '../experiments/conpty-a-prototype/package-receipt.json') -Raw | ConvertFrom-Json
foreach ($name in 'conpty.dll','OpenConsole.exe') {
    if ((Get-FileHash -LiteralPath (Join-Path $Runtime $name)).Hash -ne $receipt.runtimeFiles.$name.sha256) {
        throw "Private runtime does not match the pinned identity: $name"
    }
}
if ($Mode -eq 'Paper') {
    if (!$PaperCache -or !$AgentLibraries -or !$Java -or !$LuckPerms) { throw 'Pass verified local Paper inputs.' }
    & (Join-Path $PSScriptRoot 'verify-resident-paper.ps1') -PrepareOnly -Evidence $Evidence -PaperCache $PaperCache -AgentLibraries $AgentLibraries -Java $Java -LuckPerms $LuckPerms
} else {
    & (Join-Path $PSScriptRoot 'verify-resident-manager.ps1') -PrepareOnly -Evidence $Evidence
    foreach ($name in 'fixture-a','fixture-b') {
        $instance = Join-Path $Evidence "workspace/instances/$name"
        New-Item -ItemType File -Path (Join-Path $instance 'fixture-ready'),(Join-Path $instance 'fixture-native-console') | Out-Null
    }
}
foreach ($name in 'conpty.dll','OpenConsole.exe') {
    Copy-Item -LiteralPath (Join-Path $Runtime $name) -Destination (Join-Path $Evidence 'native')
}
node (Join-Path $PSScriptRoot 'verify-native-console-window.mjs') $Evidence $Mode.ToLowerInvariant()
if ($LASTEXITCODE -ne 0) { throw 'Native console verification failed.' }
