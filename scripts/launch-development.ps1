#Requires -Version 7.0
[CmdletBinding()]
param(
    [ValidateNotNullOrEmpty()][string]$Workspace,
    [string]$BinaryDirectory = (Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri/target/x86_64-pc-windows-msvc/dev-channel'),
    [switch]$Initialize,
    [switch]$Build
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$automatic = !$PSBoundParameters.ContainsKey('Workspace')
if ($automatic) {
    # Hash the checkout root, not scripts/, so the automatic workspace stays the same.
    $checkout = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path.ToUpperInvariant()
    $id = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($checkout)))
    $Workspace = Join-Path $env:LOCALAPPDATA ('InstanceHarbor Dev/workspaces/checkout-' + $id.Substring(0, 24))
}
$cli = Join-Path $BinaryDirectory 'ihctl.exe'
$gui = Join-Path $BinaryDirectory 'InstanceHarborDev.exe'
if ($Build -or !(Test-Path -LiteralPath $gui -PathType Leaf) -or !(Test-Path -LiteralPath $cli -PathType Leaf)) {
    if ($PSBoundParameters.ContainsKey('BinaryDirectory')) {
        throw '自訂資料夾必須包含已建置的開發版。'
    }
    & (Join-Path (Split-Path -Parent $PSScriptRoot) 'packaging/build-product.ps1') -Development | Out-Host
}
if ((& $cli --version) -notlike 'InstanceHarbor Dev CLI *') {
    throw '指定資料夾未包含開發版 CLI。'
}
if ($Initialize -or ($automatic -and !(Test-Path -LiteralPath $Workspace))) {
    & $cli $Workspace init | Out-Null
}
$start = [Diagnostics.ProcessStartInfo]::new((Resolve-Path -LiteralPath $gui).Path)
$start.UseShellExecute = $false
$start.Environment['IH_WORKSPACE'] = $Workspace
[Diagnostics.Process]::Start($start)
