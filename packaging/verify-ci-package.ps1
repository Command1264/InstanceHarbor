#Requires -Version 7.0
# Builds and checks the release ZIP and Setup of one architecture (#66) without publishing
# anything. CI runs it on Windows x64 and Windows ARM64; it works the same on a developer
# computer that can build that architecture natively.
#
# The packages carry placeholder notices, so they are never distributions: they only prove that
# the architecture's executables and ConPTY files reach the ZIP and the Setup. With -Install the
# Setup is also installed silently for the current user into a new folder and uninstalled again;
# use it only on a disposable computer such as a CI runner or Windows Sandbox.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Architecture,
    # The pinned ConPTY NuGet package (packaging/console-runtime.json `source`).
    [Parameter(Mandatory)][string]$ConsoleRuntimePackage,
    # makensis.exe of NSIS 3.13.
    [Parameter(Mandatory)][string]$Compiler,
    # A new folder for the packages, the PDBs and the extracted Setup.
    [Parameter(Mandatory)][string]$Output,
    [switch]$Install
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$repository = Split-Path -Parent $PSScriptRoot
$machines = @{ x64 = 0x8664; arm64 = 0xAA64 }
$expected = $machines[$Architecture]

function Get-Machine([byte[]]$Bytes, [string]$Name) {
    $pe = [BitConverter]::ToInt32($Bytes, 0x3C)
    if ([BitConverter]::ToUInt32($Bytes, $pe) -ne 0x4550) { throw "Not a PE file: $Name" }
    [BitConverter]::ToUInt16($Bytes, $pe + 4)
}

function Assert-Machine([byte[]]$Bytes, [string]$Name) {
    $machine = Get-Machine $Bytes $Name
    if ($machine -ne $expected) { throw ("{0} has PE machine 0x{1:X4}; expected 0x{2:X4} ({3})." -f $Name, $machine, $expected, $Architecture) }
    "{0,-20} PE machine 0x{1:X4} ({2})" -f $Name, $machine, $Architecture
}

$outputPath = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $outputPath) { throw "Output must be a new folder: $outputPath" }
New-Item -ItemType Directory -Path $outputPath | Out-Null

# 1. Release build of this architecture, with its pinned ConPTY files beside the executables.
$version = ([regex]::Match((Get-Content -Raw -LiteralPath (Join-Path $repository 'src-tauri/Cargo.toml')), '(?m)^version = "([^"]+)"')).Groups[1].Value
if ($version -cnotmatch '^\d+\.\d+\.\d+$') { throw "Unexpected application version: $version" }
& (Join-Path $PSScriptRoot 'build-product.ps1') -Release -Architecture $Architecture -ConsoleRuntimeSource $ConsoleRuntimePackage -SymbolDirectory (Join-Path $outputPath 'symbols') | Out-Host
$triple = @{ x64 = 'x86_64-pc-windows-msvc'; arm64 = 'aarch64-pc-windows-msvc' }[$Architecture]
$binaries = Join-Path $repository "src-tauri/target/$triple/release"

# 2. Placeholder notices: the scripts require them for a stable version.
$notices = Join-Path $outputPath 'placeholder-notices'
New-Item -ItemType Directory -Path $notices | Out-Null
Copy-Item -LiteralPath (Join-Path $repository 'LICENSE') -Destination (Join-Path $notices 'LICENSE')
Set-Content -LiteralPath (Join-Path $notices 'THIRD-PARTY-NOTICES.txt') -Value 'CI placeholder: this package is a build check and is never distributed.'
Add-Type -AssemblyName System.IO.Compression
$empty = [IO.File]::Open((Join-Path $notices 'MPL-SOURCES.zip'), 'CreateNew')
try { ([IO.Compression.ZipArchive]::new($empty, [IO.Compression.ZipArchiveMode]::Create)).Dispose() } finally { $empty.Dispose() }

# 3. ZIP and Setup with the release file names.
$zip = & (Join-Path $PSScriptRoot 'pack-windows.ps1') -BinaryDirectory $binaries -NoticeDirectory $notices -Version $version -Architecture $Architecture `
    -OutputZip (Join-Path $outputPath "InstanceHarbor-$version-windows-$Architecture.zip")
$setup = & (Join-Path $PSScriptRoot 'pack-windows-setup.ps1') -Package $zip.packagePath -ExpectedPackageSha256 $zip.sha256 -Compiler $Compiler `
    -Output (Join-Path $outputPath "InstanceHarbor-$version-windows-$Architecture-setup.exe")
if ($zip.architecture -cne $Architecture -or $setup.architecture -cne $Architecture) { throw 'The packaging scripts reported another architecture.' }

# 4. The ZIP: its manifest and every executable and ConPTY file.
$names = 'InstanceHarbor.exe', 'ihctl.exe', 'conpty.dll', 'OpenConsole.exe'
$archive = [IO.Compression.ZipFile]::OpenRead($zip.packagePath)
$zipHashes = @{}
try {
    $reader = [IO.StreamReader]::new($archive.GetEntry('manifest.json').Open())
    try { $manifest = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    if ($manifest.architecture -cne $Architecture) { throw "The ZIP manifest names $($manifest.architecture)." }
    "ZIP $(Split-Path -Leaf $zip.packagePath): manifest architecture $($manifest.architecture)"
    foreach ($name in $names) {
        $stream = $archive.GetEntry($name).Open()
        try { $buffer = [IO.MemoryStream]::new(); $stream.CopyTo($buffer) } finally { $stream.Dispose() }
        $bytes = $buffer.ToArray()
        Assert-Machine $bytes "ZIP/$name"
        $zipHashes[$name] = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes))
    }
} finally { $archive.Dispose() }

# 5. The Setup: 7-Zip reads NSIS installers. The files must be the ones in the ZIP.
$sevenZip = Get-Command 7z -ErrorAction SilentlyContinue
if ($sevenZip) {
    $extracted = Join-Path $outputPath 'setup-files'
    & $sevenZip.Source x "-o$extracted" -y $setup.setup | Out-Null
    foreach ($name in $names) {
        $path = Join-Path $extracted $name
        Assert-Machine ([IO.File]::ReadAllBytes($path)) "Setup/$name"
        if ((Get-FileHash -LiteralPath $path).Hash -ne $zipHashes[$name]) { throw "Setup/$name differs from the ZIP." }
    }
} else {
    Write-Warning '7-Zip was not found; the Setup payload is checked only through the verified ZIP it was built from.'
}

# 6. Optional: install silently for the current user, check, and uninstall.
if ($Install) {
    $folder = Join-Path $outputPath 'installed'
    $run = Start-Process -FilePath $setup.setup -ArgumentList '/S', '/CurrentUser', "/D=$folder" -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "Setup exited with $($run.ExitCode)." }
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\InstanceHarbor-Desktop'
    $registered = Get-ItemProperty -LiteralPath $key
    if ($registered.Architecture -cne $Architecture -or $registered.DisplayVersion -cne $version) {
        throw "Setup registered architecture '$($registered.Architecture)' and version '$($registered.DisplayVersion)'."
    }
    foreach ($name in $names) { Assert-Machine ([IO.File]::ReadAllBytes((Join-Path $folder $name))) "installed/$name" }
    $cli = (& (Join-Path $folder 'ihctl.exe') --version | Out-String).Trim()
    "Installed ihctl: $cli"
    $run = Start-Process -FilePath (Join-Path $folder 'Uninstall.exe') -ArgumentList '/S', "_?=$folder" -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "The uninstaller exited with $($run.ExitCode)." }
    if (Test-Path -LiteralPath $key) { throw 'The uninstaller left the registration.' }
    'Setup installed and uninstalled silently for the current user.'
}

[pscustomobject]@{
    architecture = $Architecture
    version = $version
    zip = $zip.packagePath
    zipSha256 = $zip.sha256
    setup = $setup.setup
    setupSha256 = $setup.setupSha256
}
