#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Package,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ExpectedPackageSha256,
    [Parameter(Mandatory)][string]$Compiler,
    [Parameter(Mandatory)][string]$Output
)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$true
$packagePath=(Resolve-Path -LiteralPath $Package).Path
$compilerPath=(Resolve-Path -LiteralPath $Compiler).Path
$outputPath=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
$parent=Split-Path -Parent $outputPath
if(!(Test-Path -LiteralPath $parent -PathType Container) -or (Test-Path -LiteralPath $outputPath) -or
   [IO.Path]::GetExtension($outputPath) -ine '.exe'){throw 'Specify a new .exe file in an existing directory.'}
$stage=Join-Path $parent ('setup-input-'+[guid]::NewGuid().ToString('N').Substring(0,12))
# Read and verify the product package through the existing portable installer.
$prepared=& (Join-Path $PSScriptRoot 'install-windows.ps1') -Action Stage -InstallRoot $stage -Package $packagePath -ExpectedPackageSha256 $ExpectedPackageSha256
# Setup serves 1.0.0-rc2 and stable versions, always with the pinned ConPTY runtime.
$stable=$prepared.version -cmatch '^(\d+)\.(\d+)\.(\d+)$'
if(!($stable -or $prepared.version -ceq '1.0.0-rc2') -or !$prepared.nativeConsoleRuntime -or $prepared.visualCppRuntime){throw 'Expected 1.0.0-rc2 or a stable version with ConPTY and without app-local Visual C++ DLLs.'}
$fileVersion=if($stable){'{0}.{1}.{2}.0' -f $Matches[1],$Matches[2],$Matches[3]}else{'1.0.0.0'}
$payload=Join-Path (Join-Path $stage 'releases') $prepared.release
foreach ($name in @('LICENSE', 'THIRD-PARTY-NOTICES.txt', 'MPL-SOURCES.zip', 'WINDOWS-GETTING-STARTED.txt')) {
    if (!(Test-Path -LiteralPath (Join-Path $payload $name) -PathType Leaf)) { throw "Missing distribution notice: $name" }
}
if ((Get-Item -LiteralPath (Join-Path $payload 'InstanceHarbor.exe')).VersionInfo.ProductName -eq 'InstanceHarbor Dev') {
    throw 'Development binaries cannot be installed as the stable product.'
}
Push-Location $PSScriptRoot
try {
    & $compilerPath /NOCONFIG /V3 "/DVERSION=$($prepared.version)" "/DFILE_VERSION=$fileVersion" "/DOUTPUT=$outputPath" "/DPAYLOAD=$payload" windows-setup.nsi | Out-Host
    if(!(Test-Path -LiteralPath $outputPath -PathType Leaf)){throw 'NSIS did not create the candidate.'}
} finally {Pop-Location}
[pscustomobject]@{
    version=$prepared.version
    packageSha256=$prepared.release
    setup=$outputPath
    setupSha256=(Get-FileHash -LiteralPath $outputPath).Hash.ToLowerInvariant()
    compilerSha256=(Get-FileHash -LiteralPath $compilerPath).Hash.ToLowerInvariant()
    verifiedInput=$stage
}
