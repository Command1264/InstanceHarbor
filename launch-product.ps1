[CmdletBinding()]
param([string]$Workspace)
$ErrorActionPreference='Stop'
$folder=Join-Path $PSScriptRoot '.evidence/vue-product-20261109'
$build=Get-Content -LiteralPath (Join-Path $folder 'build-manifest.json') -Raw|ConvertFrom-Json
$binary=Join-Path $PSScriptRoot 'src-tauri/target/debug/InstanceHarbor.exe'
$cli=Join-Path $PSScriptRoot 'src-tauri/target/debug/ihctl.exe'
if((Get-FileHash -LiteralPath $binary).Hash -ne $build.sha256 -or (Get-FileHash -LiteralPath $cli).Hash -ne $build.cliSha256){throw '候選與已核對的建置 manifest 不一致。請重新建置並驗證。'}
if(!$Workspace){
    $candidate=Get-Content -LiteralPath (Join-Path $folder 'candidate.json') -Raw|ConvertFrom-Json
    $Workspace=$candidate.workspace
}
$Workspace=(Resolve-Path -LiteralPath $Workspace).Path
$boundary=[IO.Path]::GetFullPath($folder)+[IO.Path]::DirectorySeparatorChar
if(!$Workspace.StartsWith($boundary,[StringComparison]::OrdinalIgnoreCase)){throw '此驗收入口僅允許本次隔離 Agent 工作區。'}
$owner=Get-Content -LiteralPath (Join-Path $Workspace 'task-manifest.json') -Raw|ConvertFrom-Json
if($owner.owner -ne 'vue-product-20261109'){throw '隔離工作區身分不符。'}
& (Join-Path $PSScriptRoot 'launch-acceptance.ps1') -Workspace $Workspace -BinaryDirectory (Split-Path $binary)
