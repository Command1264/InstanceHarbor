# Keep the PDBs of a release build on this machine, in symbol-store layout, so a stack of a
# running or crashed InstanceHarbor can be read later (docs/development.md, "Manager stacks").
# PDBs name local build paths: they are never committed, uploaded or packaged. The destination
# must lie outside the repository and every worktree of it.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$Commit,
    # Defaults to artifacts/symbols in the project folder that holds repo/ and worktrees/.
    [string]$Destination,
    [string[]]$Names = @('InstanceHarbor', 'ihctl')
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

function Get-FullPath([string]$Path) {
    [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)).TrimEnd('\', '/')
}

function Test-Inside([string]$Path, [string]$Folder) {
    $Path.Equals($Folder, [StringComparison]::OrdinalIgnoreCase) -or
        $Path.StartsWith($Folder + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}

# The CodeView record of a PE32+ file: the key a debugger uses to find its PDB.
function Get-PdbKey([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    $optional = $pe + 24
    if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550 -or [BitConverter]::ToUInt16($bytes, $optional) -ne 0x20B) {
        throw "Not a PE32+ file: $Path"
    }
    $imageKey = '{0:X8}{1:x}' -f [BitConverter]::ToUInt32($bytes, $pe + 8), [BitConverter]::ToUInt32($bytes, $optional + 56)
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
    $directory = [BitConverter]::ToUInt32($bytes, $optional + 160)
    $directorySize = [BitConverter]::ToUInt32($bytes, $optional + 164)
    $section = $sections | Where-Object { $directory -ge $_.address -and $directory -lt $_.address + $_.size } | Select-Object -First 1
    if (!$section) { throw "No debug directory: $Path" }
    $start = $section.raw + $directory - $section.address
    for ($entry = $start; $entry -lt $start + $directorySize; $entry += 28) {
        if ([BitConverter]::ToUInt32($bytes, $entry + 12) -ne 2) { continue }
        $data = [BitConverter]::ToUInt32($bytes, $entry + 24)
        if ([BitConverter]::ToUInt32($bytes, $data) -ne 0x53445352) { continue }
        $guid = [Guid]::new([byte[]]$bytes[($data + 4)..($data + 19)])
        $age = [BitConverter]::ToUInt32($bytes, $data + 20)
        $end = [Array]::IndexOf($bytes, [byte]0, $data + 24)
        $pdbPath = [Text.Encoding]::UTF8.GetString($bytes, $data + 24, $end - $data - 24)
        return [pscustomobject]@{
            image = $imageKey
            pdb = $guid.ToString('N').ToUpperInvariant() + ('{0:X}' -f $age)
            pdbName = Split-Path -Leaf $pdbPath
            pdbPath = $pdbPath
        }
    }
    throw "No CodeView (RSDS) record: $Path"
}

# Copies a file to its symbol-store slot. An existing slot must hold the same bytes.
function Save-Slot([string]$Source, [string]$Slot) {
    $target = Join-Path $Slot (Split-Path -Leaf $Source)
    $hash = (Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash
    $added = !(Test-Path -LiteralPath $target)
    if (!$added) {
        if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $hash) {
            throw "A different file already uses this symbol slot: $target"
        }
    } else {
        New-Item -ItemType Directory -Path $Slot -Force | Out-Null
        Copy-Item -LiteralPath $Source -Destination $target
        if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $hash) { throw "Copy changed the file: $target" }
    }
    [pscustomobject]@{ path = $target; sha256 = $hash; added = $added }
}

$repository = Get-FullPath (& git -C $PSScriptRoot rev-parse --show-toplevel | Out-String).Trim()
$common = Get-FullPath (& git -C $PSScriptRoot rev-parse --path-format=absolute --git-common-dir | Out-String).Trim()
# The main checkout: the folder whose .git holds the shared repository data.
$mainCheckout = Split-Path -Parent $common
$worktrees = @(& git -C $PSScriptRoot worktree list --porcelain | Where-Object { $_ -like 'worktree *' } |
    ForEach-Object { Get-FullPath $_.Substring(9) })
if (!$Destination) {
    $Destination = Join-Path (Split-Path -Parent $mainCheckout) 'artifacts/symbols'
}
$Destination = Get-FullPath $Destination
# `-File` passes "a,b" as one string.
$Names = @($Names | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
foreach ($folder in @($repository, $mainCheckout) + $worktrees) {
    if (Test-Inside $Destination $folder) {
        throw "PDBs must stay outside the repository and its worktrees: $Destination"
    }
}

$version = (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'package.json') -Raw | ConvertFrom-Json).version
$saved = foreach ($name in $Names) {
    $exe = Join-Path $BinaryDirectory "$name.exe"
    $key = Get-PdbKey $exe
    $pdb = Join-Path $BinaryDirectory $key.pdbName
    if (!(Test-Path -LiteralPath $pdb)) { throw "Missing PDB for ${name}: $pdb" }
    if ([IO.Path]::IsPathRooted($key.pdbPath)) {
        throw "$name.exe records the absolute PDB path $($key.pdbPath); link with /PDBALTPATH:%_PDB%."
    }
    $image = Save-Slot $exe (Join-Path $Destination "$name.exe/$($key.image)")
    $symbols = Save-Slot $pdb (Join-Path $Destination "$($key.pdbName)/$($key.pdb)")
    # A build saved before keeps its first index line.
    if (!$image.added -and !$symbols.added) { continue }
    [pscustomobject]@{
        version = $version; commit = $Commit; binary = "$name.exe"
        imageSha256 = $image.sha256; pdbSha256 = $symbols.sha256; pdbKey = $key.pdb
        savedAt = (Get-Date).ToUniversalTime().ToString('o')
    }
}
$index = Join-Path $Destination 'index.jsonl'
foreach ($entry in $saved) { Add-Content -LiteralPath $index -Value ($entry | ConvertTo-Json -Compress) -Encoding utf8 }
$Destination
