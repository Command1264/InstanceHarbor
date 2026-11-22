# Copy the pinned tier-1 console runtime (conpty.dll, OpenConsole.exe) next to built binaries.
# The source is a local directory or a local copy of the Microsoft ConPTY NuGet package named in
# console-runtime.json. This script never downloads. Each file must match the pinned SHA-256, be
# x64 and carry a valid Microsoft Authenticode signature before it is copied.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][string]$Destination
)
$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'Windows PowerShell 7 is required.' }

function Get-PlainItem([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    $ancestor = $item
    while ($null -ne $ancestor) {
        if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Redirected path: $Path" }
        $ancestor = if ($ancestor -is [IO.DirectoryInfo]) { $ancestor.Parent } else { $ancestor.Directory }
    }
    $item
}

function Get-Sha256([byte[]]$Bytes) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
}

function Assert-Runtime([string]$Name, [byte[]]$Bytes, [string]$Expected) {
    if ((Get-Sha256 $Bytes) -cne $Expected) { throw "Console runtime SHA-256 mismatch: $Name" }
    $peOffset = [BitConverter]::ToInt32($Bytes, 0x3C)
    if ([BitConverter]::ToUInt16($Bytes, $peOffset + 4) -ne 0x8664) { throw "Console runtime is not x64: $Name" }
}

function Assert-Signature([string]$Path, [string]$Signer) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch "O=$([regex]::Escape($Signer))(,|$)") {
        throw "Console runtime signature is not a valid $Signer signature: $Path ($($signature.Status))"
    }
}

$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'console-runtime.json') -Raw | ConvertFrom-Json
$names = @($manifest.files.PSObject.Properties.Name)
$sourceItem = Get-PlainItem $Source
$destinationItem = Get-PlainItem $Destination
if (!$destinationItem.PSIsContainer) { throw 'Destination must be an existing directory.' }

$payloads = [ordered]@{}
if ($sourceItem.PSIsContainer) {
    foreach ($name in $names) {
        $payloads[$name] = [IO.File]::ReadAllBytes((Get-PlainItem (Join-Path $sourceItem.FullName $name)).FullName)
    }
} elseif ($sourceItem.Extension -ceq '.nupkg') {
    Add-Type -AssemblyName System.IO.Compression
    $archive = [IO.Compression.ZipFile]::OpenRead($sourceItem.FullName)
    try {
        foreach ($name in $names) {
            # The package carries one build per architecture; the pinned hash selects the x64 one.
            $expected = $manifest.files.$name.sha256
            foreach ($entry in @($archive.Entries | Where-Object { $_.Name -ceq $name -and $_.Length -le 64MB })) {
                $stream = $entry.Open()
                try {
                    $buffer = [IO.MemoryStream]::new()
                    $stream.CopyTo($buffer)
                    $bytes = $buffer.ToArray()
                } finally { $stream.Dispose() }
                if ((Get-Sha256 $bytes) -ceq $expected) { $payloads[$name] = $bytes; break }
            }
            if (!$payloads.Contains($name)) { throw "The package has no file matching the pinned $name." }
        }
    } finally { $archive.Dispose() }
} else {
    throw 'Source must be a directory or a .nupkg file.'
}

foreach ($name in $names) {
    $expected = $manifest.files.$name.sha256
    Assert-Runtime $name $payloads[$name] $expected
    $target = Join-Path $destinationItem.FullName $name
    if (Test-Path -LiteralPath $target) {
        $existing = Get-PlainItem $target
        if ((Get-Sha256 ([IO.File]::ReadAllBytes($existing.FullName))) -ceq $expected) {
            Assert-Signature $existing.FullName $manifest.signer
            continue
        }
        throw "A different $name already exists in the destination. Remove it after checking it: $target"
    }
    $stage = Join-Path $destinationItem.FullName ".$name.$PID.tmp"
    try {
        [IO.File]::WriteAllBytes($stage, $payloads[$name])
        Assert-Runtime $name ([IO.File]::ReadAllBytes($stage)) $expected
        Assert-Signature $stage $manifest.signer
        Move-Item -LiteralPath $stage -Destination $target
    } finally {
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage }
    }
}
[pscustomobject]@{ version = $manifest.version; destination = $destinationItem.FullName; files = $names }
