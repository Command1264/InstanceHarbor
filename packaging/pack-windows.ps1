# Package a previously built Windows GUI/CLI pair. Trust the resulting digest via a separate channel.
# Uses only local files and the .NET ZIP API available to PowerShell 7.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [string]$ConsoleRuntimeDirectory,
    [string]$VCRuntimeDirectory,
    [string]$NoticeDirectory,
    [Parameter(Mandatory)][string]$OutputZip,
    [Parameter(Mandatory)][ValidatePattern('^[0-9A-Za-z][0-9A-Za-z.-]{0,63}$')][string]$Version,
    # The architecture of the executables (#66). Release files are named
    # InstanceHarbor-<version>-windows-<architecture>.zip; the manifest records it too.
    [ValidateSet('x64', 'arm64')][string]$Architecture = 'x64'
)

$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'Windows PowerShell 7 is required.' }
# 1.0.0-rc2 and stable versions are distributions: they ship the license notices and the
# startup guide, and carry no app-local Visual C++ DLLs. Earlier candidates may omit them.
$distribution = $Version -ceq '1.0.0-rc2' -or $Version -cmatch '^\d+\.\d+\.\d+$'

function Get-PlainItem([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    $ancestor = $item
    while ($null -ne $ancestor) {
        if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Redirected path: $Path" }
        $ancestor = if ($ancestor -is [IO.DirectoryInfo]) { $ancestor.Parent } else { $ancestor.Directory }
    }
    $item
}

$binaryRoot = Get-PlainItem $BinaryDirectory
if (!$binaryRoot.PSIsContainer) { throw 'BinaryDirectory must be a directory.' }
$destination = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputZip)
$parentPath = [IO.Path]::GetDirectoryName($destination)
$parent = Get-PlainItem $parentPath
if (!$parent.PSIsContainer -or [IO.Path]::GetExtension($destination) -cne '.zip' -or (Test-Path -LiteralPath $destination)) {
    throw 'OutputZip must name a new .zip file in an existing directory.'
}
# The GUI's update picks the ZIP by name, so a release file name must state the architecture.
$releaseName = "InstanceHarbor-$Version-windows-$Architecture.zip"
$outputName = [IO.Path]::GetFileName($destination)
if ($outputName -like 'InstanceHarbor-*' -and $outputName -cne $releaseName) {
    throw "A release ZIP of this package must be named $releaseName."
}

$names = @('InstanceHarbor.exe', 'ihctl.exe')
# Neither PDBs nor executables that name the build machine's folders may ship (#118).
& (Join-Path $PSScriptRoot 'verify-no-local-paths.ps1') -Binary @($names | ForEach-Object { Join-Path $binaryRoot.FullName $_ }) | Out-Host
$gui = Get-PlainItem (Join-Path $binaryRoot.FullName 'InstanceHarbor.exe')
if ($gui.VersionInfo.ProductName -eq 'InstanceHarbor Dev') {
    throw 'Development binaries cannot be packaged as the stable product.'
}

# The PE machine type of each architecture; the executables and the console runtime must match.
$machines = @{ x64 = 0x8664; arm64 = 0xAA64 }
function Get-Machine([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550) { throw "Not a PE file: $Path" }
    [BitConverter]::ToUInt16($bytes, $pe + 4)
}
foreach ($name in $names) {
    $machine = Get-Machine (Join-Path $binaryRoot.FullName $name)
    if ($machine -ne $machines[$Architecture]) { throw ("$name is not a Windows $Architecture executable (PE machine 0x{0:X4})." -f $machine) }
}

# Names of the DLLs in a PE32+ file's import directory.
function Get-ImportedDll([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    $optional = $pe + 24
    if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550 -or [BitConverter]::ToUInt16($bytes, $optional) -ne 0x20B) {
        throw "Not a PE32+ file: $Path"
    }
    $sections = @()
    $sectionTable = $optional + [BitConverter]::ToUInt16($bytes, $pe + 20)
    for ($i = 0; $i -lt [BitConverter]::ToUInt16($bytes, $pe + 6); $i++) {
        $at = $sectionTable + 40 * $i
        $sections += [pscustomobject]@{
            address = [BitConverter]::ToUInt32($bytes, $at + 12)
            size = [Math]::Max([BitConverter]::ToUInt32($bytes, $at + 8), [BitConverter]::ToUInt32($bytes, $at + 16))
            raw = [BitConverter]::ToUInt32($bytes, $at + 20)
        }
    }
    $offset = {
        param([uint32]$Rva)
        $section = $sections | Where-Object { $Rva -ge $_.address -and $Rva -lt $_.address + $_.size } | Select-Object -First 1
        if (!$section) { throw "Import table outside every section: $Path" }
        [int]($Rva - $section.address + $section.raw)
    }
    $importRva = [BitConverter]::ToUInt32($bytes, $optional + 120)
    if ($importRva -eq 0) { return @() }
    for ($descriptor = & $offset $importRva; ; $descriptor += 20) {
        $nameRva = [BitConverter]::ToUInt32($bytes, $descriptor + 12)
        if ($nameRva -eq 0) { break }
        $start = & $offset $nameRva
        $end = [Array]::IndexOf($bytes, [byte]0, $start)
        [Text.Encoding]::ASCII.GetString($bytes, $start, $end - $start)
    }
}

# Stable versions link the Visual C++ runtime statically (.cargo/config.toml), so a
# missing or removed Visual C++ Redistributable cannot stop them from starting.
if ($Version -cmatch '^\d+\.\d+\.\d+$') {
    foreach ($name in $names) {
        $crtImports = @(Get-ImportedDll (Join-Path $binaryRoot.FullName $name) | Where-Object { $_ -match '^(vcruntime|msvcp)\d' })
        if ($crtImports.Count) {
            throw "$name imports $($crtImports -join ', '). Build it with the static C runtime from .cargo/config.toml."
        }
    }
}
# Match console-runtime.json and native_console.rs. Release candidates can be packaged without them.
$consoleHashesByArchitecture = @{
    x64 = [ordered]@{
        'conpty.dll' = 'feeef341d891643c62d30b6b07800bc70f0bc148f44cb8c3bee84aa557ae805a'
        'OpenConsole.exe' = '3d66b23d0a71bb8eed2b77edc8b9df9bf54ce6c8fb74c863a30e760997f80586'
    }
    arm64 = [ordered]@{
        'conpty.dll' = '151a02355227710c20c9f8f1eaf68ac4668aa09cdc921adc7ec661804d8eb1bf'
        'OpenConsole.exe' = '4313825a3ba634d113cc6ed3bc8ce735e3c19d360cecd985794787479469f91f'
    }
}
$consoleHashes = $consoleHashesByArchitecture[$Architecture]
$consoleRoot = $binaryRoot
if ($ConsoleRuntimeDirectory) {
    $consoleRoot = Get-PlainItem $ConsoleRuntimeDirectory
    if (!$consoleRoot.PSIsContainer) { throw 'ConsoleRuntimeDirectory must be a directory.' }
}
$consoleNames = @($consoleHashes.Keys | Where-Object { Test-Path -LiteralPath (Join-Path $consoleRoot.FullName $_) })
if ($consoleNames.Count -notin @(0, 2) -or ($ConsoleRuntimeDirectory -and $consoleNames.Count -ne 2)) {
    throw 'The native console requires both conpty.dll and OpenConsole.exe.'
}
# From 1.0.0 the server console is a default feature; only release candidates may omit tier 1.
if ($consoleNames.Count -ne 2 -and $Version -notlike '1.0.0-rc*') {
    throw 'A 1.0 package requires the pinned conpty.dll and OpenConsole.exe. Provide them with provision-console-runtime.ps1 or ConsoleRuntimeDirectory.'
}
$crtHashes = [ordered]@{
    'vcruntime140.dll' = 'd5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066'
    'vcruntime140_1.dll' = '1f2d41c4aa5db0bc33ebf7b66d72943a817d7ce6cbe880502a9403823633093f'
}
$crtNames = @()
if ($distribution -and $VCRuntimeDirectory) {
    throw 'Distributions carry no app-local Visual C++ DLLs. Omit VCRuntimeDirectory.'
}
if ($VCRuntimeDirectory) {
    $crtRoot = Get-PlainItem $VCRuntimeDirectory
    if (!$crtRoot.PSIsContainer) { throw 'VCRuntimeDirectory must be a directory.' }
    $crtNames = @($crtHashes.Keys)
    if (@($crtNames | Where-Object { !(Test-Path -LiteralPath (Join-Path $crtRoot.FullName $_)) }).Count) {
        throw 'The app-local Microsoft C++ runtime requires both VCRUNTIME DLLs.'
    }
}
$noticeNames = @()
if ($NoticeDirectory) {
    $noticeRoot = Get-PlainItem $NoticeDirectory
    if (!$noticeRoot.PSIsContainer) { throw 'NoticeDirectory must be a directory.' }
    $noticeNames = @('LICENSE', 'THIRD-PARTY-NOTICES.txt', 'MPL-SOURCES.zip')
} elseif ($distribution) {
    throw 'A distribution requires NoticeDirectory.'
}
$guideNames = @()
if ($distribution) { $guideNames = @('WINDOWS-GETTING-STARTED.txt') }
$manifest = [ordered]@{ schema = 'instanceharbor-windows-package/v1'; version = $Version; architecture = $Architecture; files = @() }
$stage = Join-Path $parent.FullName ('.instanceharbor-package-' + [guid]::NewGuid().ToString('N') + '.tmp')
$encoding = [Text.UTF8Encoding]::new($false)
$zipStream = [IO.File]::Open($stage, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
try {
    try {
        $zip = [IO.Compression.ZipArchive]::new($zipStream, [IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            foreach ($name in ($names + $consoleNames + $crtNames + $noticeNames + $guideNames)) {
                if ($name -like '*.pdb') { throw "PDBs stay on the build machine: $name" }
                $sourceRoot = if ($name -cin $consoleNames) { $consoleRoot }
                    elseif ($name -cin $crtNames) { $crtRoot }
                    elseif ($name -cin $noticeNames) { $noticeRoot }
                    elseif ($name -cin $guideNames) { Get-Item -LiteralPath $PSScriptRoot }
                    else { $binaryRoot }
                $item = Get-PlainItem (Join-Path $sourceRoot.FullName $name)
                if ($item.PSIsContainer -or $item.Length -le 0 -or $item.Length -gt 256MB) { throw "Invalid package input: $name" }
                $input = [IO.File]::OpenRead($item.FullName)
                try {
                    $hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($input)).ToLowerInvariant()
                    if ($name -cin $consoleNames -and $hash -cne $consoleHashes[$name]) { throw "Native console runtime SHA-256 mismatch for $($Architecture): $name" }
                    if ($name -cin $crtNames -and $hash -cne $crtHashes[$name]) { throw "Microsoft C++ runtime SHA-256 mismatch: $name" }
                    $input.Position = 0
                    $manifest.files += @{ name = $name; size = $input.Length; sha256 = $hash }
                    $output = $zip.CreateEntry($name).Open()
                    try { $input.CopyTo($output) } finally { $output.Dispose() }
                } finally { $input.Dispose() }
            }
            $output = $zip.CreateEntry('manifest.json').Open()
            try { $output.Write($encoding.GetBytes(($manifest | ConvertTo-Json -Depth 4))) } finally { $output.Dispose() }
        } finally { $zip.Dispose() }
        $zipStream.Flush($true)
        $zipStream.Position = 0
        $digest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($zipStream)).ToLowerInvariant()
    } finally { $zipStream.Dispose() }
    [IO.File]::Move($stage, $destination)
    [pscustomobject]@{ version = $Version; architecture = $Architecture; packagePath = $destination; sha256 = $digest }
} finally {
    if ([IO.File]::Exists($stage)) { [IO.File]::Delete($stage) }
}
