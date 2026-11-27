# Fails when an executable names a path of the machine that built it: a PDB path with a
# drive letter, the checkout (repository, main checkout or any worktree), the user profile,
# or the Rust toolchain and Cargo folders. Published binaries must not carry them (#118).
# Reads the files only; ASCII and UTF-16LE strings are both checked.
[CmdletBinding()]
param([Parameter(Mandatory)][string[]]$Binary)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
# `-File` passes "a,b" as one string.
$Binary = @($Binary | ForEach-Object { $_ -split ',' } | Where-Object { $_ })

function Get-FullPath([string]$Path) {
    [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)).TrimEnd('\', '/')
}

$folders = [Collections.Generic.List[string]]::new()
$repository = (& git -C $PSScriptRoot rev-parse --show-toplevel | Out-String).Trim()
$common = (& git -C $PSScriptRoot rev-parse --path-format=absolute --git-common-dir | Out-String).Trim()
$folders.Add((Get-FullPath $repository))
$folders.Add((Split-Path -Parent (Get-FullPath $common)))
& git -C $PSScriptRoot worktree list --porcelain | Where-Object { $_ -like 'worktree *' } |
    ForEach-Object { $folders.Add((Get-FullPath $_.Substring(9))) }
foreach ($variable in 'USERPROFILE', 'CARGO_HOME', 'RUSTUP_HOME') {
    $value = [Environment]::GetEnvironmentVariable($variable)
    if ($value) { $folders.Add((Get-FullPath $value)) }
}
$sysroot = (& rustc --print sysroot 2>$null | Out-String).Trim()
if ($sysroot) { $folders.Add((Get-FullPath $sysroot)) }

# Each folder in both separator styles, matched without regard to case.
$patterns = @($folders | Where-Object { $_ } | Sort-Object -Unique | ForEach-Object {
    # The folder and the rest of the path after it, so the report shows what refers to it.
    [regex]::Escape($_).Replace('\\', '[\\/]+') + '(?![A-Za-z0-9_.-])[^\x00-\x1f"*<>|]{0,120}'
}) + @('[A-Za-z]:[\\/][^\x00-\x1f"*<>|]{0,260}?\.pdb')
$regex = [regex]::new(($patterns -join '|'), 'IgnoreCase')

$found = foreach ($path in $Binary) {
    $bytes = [IO.File]::ReadAllBytes((Get-FullPath $path))
    foreach ($text in [Text.Encoding]::Latin1.GetString($bytes), [Text.Encoding]::Unicode.GetString($bytes),
        [Text.Encoding]::Unicode.GetString($bytes, 1, $bytes.Length - 1)) {
        foreach ($match in $regex.Matches($text)) {
            [pscustomobject]@{ binary = Split-Path -Leaf $path; text = $match.Value }
        }
    }
}
if ($found) {
    $list = $found | Sort-Object binary, text -Unique | ForEach-Object { "  $($_.binary): $($_.text)" }
    throw "Local build paths found in executables:`n$($list -join "`n")"
}
"No local build paths in: $(($Binary | ForEach-Object { Split-Path -Leaf $_ }) -join ', ')"
