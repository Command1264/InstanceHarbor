<#
.SYNOPSIS
Manage a portable InstanceHarbor installation from PowerShell 7 on Windows.
.DESCRIPTION
Install stages a verified ZIP and selects its immutable release. Stage alone is safe while
the installed product is active. Activate selects a staged digest, and Rollback selects the
previous release. Selection changes and Uninstall require the installed processes to exit.
Status lists interrupted stages. Retry Stage with the same package and trusted digest.
Paths returns the verified GUI and CLI paths. Launch supplies a separate existing workspace.
The caller must choose an installation directory whose contents they trust and control.
Use a package digest from a separate trusted channel for unsigned candidates. A pinned
publisher certificate validates both executables without vouching for the ZIP metadata.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Stage', 'Activate', 'Install', 'Rollback', 'Status', 'Paths', 'Launch', 'Uninstall')][string]$Action,
    [Parameter(Mandatory)][string]$InstallRoot,
    [string]$Package,
    [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedPackageSha256,
    [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$PublisherCertificateSha256,
    [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$Release,
    [string]$Workspace
)

$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'Windows PowerShell 7 is required.' }

$packageSchema = 'instanceharbor-windows-package/v1'
$installSchema = 'instanceharbor-windows-install/v1'
$guiNames = @('InstanceHarbor.exe', 'instanceharbor-v2.exe')
$fileNames = $guiNames + 'ihctl.exe'
# Match native_console.rs. Microsoft console files do not use the product's publisher certificate.
$consoleHashes = [ordered]@{
    'conpty.dll' = 'feeef341d891643c62d30b6b07800bc70f0bc148f44cb8c3bee84aa557ae805a'
    'OpenConsole.exe' = '3d66b23d0a71bb8eed2b77edc8b9df9bf54ce6c8fb74c863a30e760997f80586'
}
$crtHashes = [ordered]@{
    'vcruntime140.dll' = 'd5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066'
    'vcruntime140_1.dll' = '1f2d41c4aa5db0bc33ebf7b66d72943a817d7ce6cbe880502a9403823633093f'
}
$noticeNames = @('LICENSE', 'THIRD-PARTY-NOTICES.txt', 'MPL-SOURCES.zip')
$startupGuide = 'WINDOWS-GETTING-STARTED.txt'
$allNames = $fileNames + @($consoleHashes.Keys) + @($crtHashes.Keys) + $noticeNames + $startupGuide
$versionPattern = '^[0-9A-Za-z][0-9A-Za-z.-]{0,63}$'
$hashPattern = '^[a-f0-9]{64}$'
$encoding = [Text.UTF8Encoding]::new($false, $true)

function Get-PlainItem([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    $ancestor = $item
    while ($null -ne $ancestor) {
        if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Redirected path: $Path" }
        $ancestor = if ($ancestor -is [IO.DirectoryInfo]) { $ancestor.Parent } else { $ancestor.Directory }
    }
    $item
}

function Get-Hash([IO.Stream]$Stream) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Stream)).ToLowerInvariant()
}

function Read-Json([string]$Path) {
    $item = Get-PlainItem $Path
    if ($item.PSIsContainer -or $item.Length -gt 32KB) { throw "Invalid installation metadata: $Path" }
    [IO.File]::ReadAllText($item.FullName, $encoding) | ConvertFrom-Json -AsHashtable
}

function Write-JsonAtomic([string]$Path, $Data) {
    if (Test-Path -LiteralPath $Path) { $null = Get-PlainItem $Path }
    $temp = $Path + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
    $file = [IO.File]::Open($temp, 'CreateNew', 'Write', 'None')
    try {
        try {
            $file.Write($encoding.GetBytes(($Data | ConvertTo-Json -Depth 8)))
            $file.Flush($true)
        } finally { $file.Dispose() }
        if ([IO.File]::Exists($Path)) { [IO.File]::Replace($temp, $Path, [NullString]::Value) }
        else { [IO.File]::Move($temp, $Path) }
    } finally {
        if ([IO.File]::Exists($temp)) { [IO.File]::Delete($temp) }
    }
}

function Test-Within([string]$Parent, [string]$Path) {
    if ($Path.StartsWith('\\?\', [StringComparison]::Ordinal)) { $Path = $Path.Substring(4) }
    $Path.Equals($Parent, [StringComparison]::OrdinalIgnoreCase) -or
        $Path.StartsWith($Parent.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)
}

function Assert-Manifest($Manifest) {
    if ($Manifest.schema -cne $packageSchema -or $Manifest.version -cnotmatch $versionPattern) { throw 'Invalid package manifest.' }
    $names = @($Manifest.files | ForEach-Object name)
    if (@($names | Where-Object { $_ -cnotin $allNames }).Count) { throw 'Invalid package manifest entries.' }
    $gui = @($names | Where-Object { $_ -cin $guiNames })
    if ($gui.Count -ne 1) { throw 'Expected exactly one stable GUI executable.' }
    $expected = @($gui[0], 'ihctl.exe')
    foreach ($optional in @($consoleHashes,$crtHashes)) {
        if (@($names | Where-Object { $_ -cin $optional.Keys }).Count) { $expected += @($optional.Keys) }
    }
    # Earlier ZIPs and rollback receipts remain valid without the files.
    if (@($names | Where-Object { $_ -cin $noticeNames }).Count) {
        if (@($noticeNames | Where-Object { $_ -cnotin $names }).Count) { throw 'Incomplete package notices.' }
        $expected += $noticeNames
    }
    if ($startupGuide -cin $names) {
        if ($noticeNames[0] -cnotin $names -or @($names | Where-Object { $_ -cin $crtHashes.Keys }).Count) {
            throw 'The Windows startup guide requires distribution notices and a system runtime package.'
        }
        $expected += $startupGuide
    }
    if ($names.Count -ne $expected.Count) { throw 'Incomplete package runtime pair.' }
    foreach ($name in $expected) {
        $records = @($Manifest.files | Where-Object { $_.name -ceq $name })
        if ($records.Count -ne 1 -or $records[0].sha256 -cnotmatch $hashPattern -or
            $records[0].size -isnot [long] -and $records[0].size -isnot [int] -or
            $records[0].size -le 0 -or $records[0].size -gt 256MB) { throw "Invalid package file record: $name" }
        if ($consoleHashes.Contains($name) -and $records[0].sha256 -cne $consoleHashes[$name]) { throw "Native console runtime SHA-256 mismatch: $name" }
        if ($crtHashes.Contains($name) -and $records[0].sha256 -cne $crtHashes[$name]) { throw "Microsoft C++ runtime SHA-256 mismatch: $name" }
    }
}

function Assert-Publisher([string]$Path, [string]$ExpectedPublisher) {
    $auth = Get-AuthenticodeSignature -LiteralPath $Path
    if ($auth.Status -ne 'Valid' -or $null -eq $auth.SignerCertificate) { throw 'Authenticode signature is missing or untrusted.' }
    $certificateHash = $auth.SignerCertificate.GetCertHashString([Security.Cryptography.HashAlgorithmName]::SHA256)
    if ($certificateHash -ine $ExpectedPublisher) { throw 'Authenticode signer does not match the separately supplied certificate SHA-256.' }
}

function Get-Release([string]$Id) {
    if ($Id -cnotmatch $hashPattern) { throw 'Invalid release identifier.' }
    $path = Join-Path $releases $Id
    $item = Get-PlainItem $path
    if (!$item.PSIsContainer) { throw 'Invalid release directory.' }
    $receipt = Read-Json (Join-Path $path 'release.json')
    if ($receipt.packageSha256 -cne $Id -or $receipt.schema -cne $installSchema) { throw 'Invalid release receipt.' }
    Assert-Manifest $receipt.manifest
    if ($receipt.trust -cnotin @('PackageDigest', 'Publisher', 'PackageDigestAndPublisher') -or
        ($receipt.trust -ne 'PackageDigest' -and $receipt.publisherSha256 -cnotmatch $hashPattern)) { throw 'Invalid release trust record.' }
    $names = @(Get-ChildItem -LiteralPath $path -Force | ForEach-Object Name)
    $releaseNames = @($receipt.manifest.files | ForEach-Object name)
    if ($names.Count -ne ($releaseNames.Count + 1) -or 'release.json' -cnotin $names -or @($releaseNames | Where-Object { $_ -cnotin $names }).Count) {
        throw 'Release contains unexpected files.'
    }
    foreach ($record in $receipt.manifest.files) {
        $binary = Get-PlainItem (Join-Path $path $record.name)
        if ($binary.PSIsContainer -or $binary.Length -ne $record.size -or
            (Get-FileHash -LiteralPath $binary.FullName -Algorithm SHA256).Hash -ine $record.sha256) { throw "Release file changed: $($record.name)" }
        if ($receipt.trust -ne 'PackageDigest' -and $record.name -cin $fileNames) { Assert-Publisher $binary.FullName $receipt.publisherSha256 }
    }
    [pscustomobject]@{ id = $Id; path = $path; version = $receipt.manifest.version; trust = $receipt.trust; names = $releaseNames; guiName = ($releaseNames | Where-Object { $_ -cin $guiNames }); nativeConsoleRuntime = ('conpty.dll' -cin $releaseNames); visualCppRuntime = ('vcruntime140.dll' -cin $releaseNames) }
}

function Install-Stage {
    if (!$Package -or (!$ExpectedPackageSha256 -and !$PublisherCertificateSha256)) {
        throw 'Supply Package and a separately trusted ExpectedPackageSha256 or PublisherCertificateSha256.'
    }
    $item = Get-PlainItem $Package
    if ($item.PSIsContainer -or $item.Length -le 0 -or $item.Length -gt 520MB) { throw 'Invalid portable package size.' }
    $input = [IO.File]::OpenRead($item.FullName)
    try {
        $digest = Get-Hash $input
        if ($ExpectedPackageSha256 -and $digest -ine $ExpectedPackageSha256) { throw 'Package SHA-256 does not match.' }
        $input.Position = 0
        $published = Join-Path $releases $digest
        $stage = Join-Path $releases ('.stage-' + [guid]::NewGuid().ToString('N'))
        if (Test-Path -LiteralPath $stage) { throw 'Staging name already exists.' }
        $null = [IO.Directory]::CreateDirectory($stage)
        try {
            $zip = [IO.Compression.ZipArchive]::new($input, [IO.Compression.ZipArchiveMode]::Read, $true)
            try {
                $allowed = $allNames + 'manifest.json'
                $entries = @($zip.Entries)
                $entriesByName = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
                if ($entries.Count -lt 3 -or $entries.Count -gt $allNames.Count) { throw 'Unexpected ZIP entry count.' }
                foreach ($entry in $entries) {
                    if ($entry.FullName -cnotin $allowed -or !$entriesByName.TryAdd($entry.FullName, $entry)) { throw 'Unexpected or duplicate ZIP entry.' }
                }
                $manifestEntry = $entriesByName['manifest.json']
                if (!$manifestEntry) { throw 'Package manifest is missing.' }
                if ($manifestEntry.Length -gt 32KB) { throw 'Package manifest is too large.' }
                $reader = [IO.StreamReader]::new($manifestEntry.Open(), $encoding)
                try {
                    $characters = [char[]]::new(32769)
                    $length = $reader.ReadBlock($characters, 0, $characters.Length)
                    if ($length -ge $characters.Length) { throw 'Package manifest is too large.' }
                    $manifest = [string]::new($characters, 0, $length) | ConvertFrom-Json -AsHashtable
                } finally { $reader.Dispose() }
                Assert-Manifest $manifest
                if (!$ExpectedPackageSha256 -and @($manifest.files | Where-Object { $_.name -cin $noticeNames }).Count) {
                    throw 'Supply ExpectedPackageSha256 to verify the unsigned distribution notices.'
                }
                if ($entries.Count -ne ($manifest.files.Count + 1)) { throw 'ZIP entries do not match the file manifest.' }
                foreach ($record in $manifest.files) {
                    $entry = $entriesByName[$record.name]
                    if (!$entry -or $entry.Length -ne $record.size) { throw "Invalid ZIP entry: $($record.name)" }
                    $path = Join-Path $stage $record.name
                    $destination = [IO.File]::Open($path, 'CreateNew', 'Write', 'None')
                    try {
                        $source = $entry.Open()
                        try {
                            $buffer = [byte[]]::new(65536)
                            $total = 0L
                            while (($count = $source.Read($buffer, 0, $buffer.Length)) -gt 0) {
                                $total += $count
                                if ($total -gt $record.size) { throw 'ZIP entry exceeds its manifest size.' }
                                $destination.Write($buffer, 0, $count)
                            }
                            if ($total -ne $record.size) { throw 'Truncated ZIP entry.' }
                            $destination.Flush($true)
                        } finally { $source.Dispose() }
                    } finally { $destination.Dispose() }
                    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $record.sha256) { throw "Package file SHA-256 mismatch: $($record.name)" }
                    if ($PublisherCertificateSha256 -and $record.name -cin $fileNames) { Assert-Publisher $path $PublisherCertificateSha256 }
                }
            } finally { $zip.Dispose() }
            $trust = if ($ExpectedPackageSha256 -and $PublisherCertificateSha256) { 'PackageDigestAndPublisher' }
                elseif ($ExpectedPackageSha256) { 'PackageDigest' } else { 'Publisher' }
            Write-JsonAtomic (Join-Path $stage 'release.json') @{
                schema = $installSchema; packageSha256 = $digest; manifest = $manifest; trust = $trust
                publisherSha256 = $PublisherCertificateSha256.ToLowerInvariant()
            }
            if (Test-Path -LiteralPath $published) {
                $existing = Get-Release $digest
                foreach ($record in $manifest.files) {
                    if ((Get-FileHash -LiteralPath (Join-Path $existing.path $record.name)).Hash -ine $record.sha256) { throw 'Existing release differs from this package.' }
                }
            } else { [IO.Directory]::Move($stage, $published) }
            Get-Release $digest
        } finally {
            if ([IO.Directory]::Exists($stage)) {
                $null = Get-PlainItem $stage
                foreach ($name in ($allNames + 'release.json')) {
                    $path = Join-Path $stage $name
                    if ([IO.File]::Exists($path)) { [IO.File]::Delete((Get-PlainItem $path).FullName) }
                }
                [IO.Directory]::Delete($stage, $false)
            }
        }
    } finally { $input.Dispose() }
}

function Get-Pointer {
    if (!(Test-Path -LiteralPath $pointerPath)) { return @{ active = $null; previous = $null } }
    $pointer = Read-Json $pointerPath
    if ($pointer.schema -cne $installSchema -or
        ($null -ne $pointer.active -and $pointer.active -cnotmatch $hashPattern) -or
        ($null -ne $pointer.previous -and $pointer.previous -cnotmatch $hashPattern)) { throw 'Invalid active release pointer.' }
    $pointer
}

function Assert-Idle {
    $processes = Get-CimInstance Win32_Process -Filter "Name = 'ihctl.exe' OR Name = 'InstanceHarbor.exe' OR Name = 'instanceharbor-v2.exe'"
    foreach ($process in $processes) {
        if (!$process.ExecutablePath) { throw "Cannot verify executable path for PID $($process.ProcessId)." }
        if (Test-Within $releases $process.ExecutablePath) {
            throw "Installed GUI, Manager or Host is active (PID $($process.ProcessId)); retry once it has exited."
        }
    }
}

function Set-Active($Next, $Previous) {
    Write-JsonAtomic $pointerPath @{ schema = $installSchema; active = $Next; previous = $Previous }
}

$root = [IO.Path]::TrimEndingDirectorySeparator($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($InstallRoot))
if (!$root -or $root -ceq [IO.Path]::GetPathRoot($root) -or $root.StartsWith('\\')) { throw 'Use a dedicated local installation directory.' }
if ($Workspace) {
    $workRoot = (Get-PlainItem $Workspace).FullName
    if ((Test-Within $root $workRoot) -or (Test-Within $workRoot $root)) { throw 'Workspace and installation paths must be separate.' }
}
$newInstall = $Action -in @('Stage', 'Install')
if (!(Test-Path -LiteralPath $root)) {
    if (!$newInstall) { throw 'Installation is missing.' }
    $null = Get-PlainItem ([IO.Path]::GetDirectoryName($root))
    $null = [IO.Directory]::CreateDirectory($root)
}
if (!(Get-PlainItem $root).PSIsContainer) { throw 'InstallRoot must be a directory.' }
$ownerPath = Join-Path $root 'installation.json'
$lockPath = Join-Path $root 'installation.lock'
if (!(Test-Path -LiteralPath $ownerPath)) {
    if (!$newInstall -or @(Get-ChildItem -LiteralPath $root -Force | Where-Object Name -CNE 'installation.lock').Count) {
        throw 'Use an empty dedicated installation directory.'
    }
}
if (Test-Path -LiteralPath $lockPath) { $null = Get-PlainItem $lockPath }
$lock = [IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None')
try {
    if (!(Test-Path -LiteralPath $ownerPath)) { Write-JsonAtomic $ownerPath @{ schema = $installSchema } }
    if ((Read-Json $ownerPath).schema -cne $installSchema) { throw 'InstallRoot has an unexpected owner marker.' }
    $pointerPath = Join-Path $root 'active.json'
    $releases = Join-Path $root 'releases'
    if (!(Test-Path -LiteralPath $releases)) {
        if (!$newInstall) { throw 'Release directory is missing.' }
        $null = [IO.Directory]::CreateDirectory($releases)
    }
    if (!(Get-PlainItem $releases).PSIsContainer) { throw 'Release location must be a directory.' }
    $pointer = Get-Pointer
    switch ($Action) {
        { $_ -in @('Stage', 'Install', 'Activate') } {
            if ($Action -eq 'Activate' -and !$Release) { throw 'Activate requires a staged release SHA-256.' }
            $next = if ($Action -eq 'Activate') { Get-Release $Release.ToLowerInvariant() } else { Install-Stage }
            if ($Action -ne 'Stage' -and $pointer.active -cne $next.id) {
                if ($pointer.active) { $null = Get-Release $pointer.active }
                Assert-Idle
                Set-Active $next.id $pointer.active
            }
            [pscustomobject]@{ release = $next.id; version = $next.version; trust = $next.trust; nativeConsoleRuntime = $next.nativeConsoleRuntime; visualCppRuntime = $next.visualCppRuntime; active = ($Action -ne 'Stage' -or $pointer.active -ceq $next.id) }
        }
        'Rollback' {
            if (!$pointer.previous) { throw 'No previous release is recorded.' }
            $next = Get-Release $pointer.previous
            Assert-Idle
            Set-Active $next.id $pointer.active
            $next
        }
        'Status' {
            [pscustomobject]@{
                active = $pointer.active; previous = $pointer.previous
                releases = @(Get-ChildItem -LiteralPath $releases -Directory -Force | Where-Object Name -CMatch $hashPattern | ForEach-Object Name)
                interruptedStages = @(Get-ChildItem -LiteralPath $releases -Directory -Force | Where-Object Name -CMatch '^\.stage-[a-f0-9]{32}$' | ForEach-Object Name)
                interruptedRemovals = @(Get-ChildItem -LiteralPath $releases -Directory -Force | Where-Object Name -CMatch '^\.removed-[a-f0-9]{64}$' | ForEach-Object Name)
            }
        }
        { $_ -in @('Paths', 'Launch') } {
            if (!$pointer.active) { throw 'No release is active.' }
            $current = Get-Release $pointer.active
            $gui = Join-Path $current.path $current.guiName
            $cli = Join-Path $current.path 'ihctl.exe'
            if ($Action -eq 'Launch') {
                if (!$Workspace -or !(Get-PlainItem $workRoot).PSIsContainer) { throw 'Launch requires a separate existing Workspace directory.' }
                $null = Get-PlainItem (Join-Path $workRoot 'workspace.json')
                $process = [Diagnostics.ProcessStartInfo]::new($gui)
                $process.UseShellExecute = $false
                $process.WorkingDirectory = $current.path
                $process.Environment['IH_V2_WORKSPACE'] = $workRoot
                $started = [Diagnostics.Process]::Start($process)
                try { [pscustomobject]@{ pid = $started.Id; release = $current.id } } finally { $started.Dispose() }
            } else { [pscustomobject]@{ release = $current.id; gui = $gui; cli = $cli; nativeConsoleRuntime = $current.nativeConsoleRuntime; visualCppRuntime = $current.visualCppRuntime } }
        }
        'Uninstall' {
            Assert-Idle
            $installed = @(Get-ChildItem -LiteralPath $releases -Directory -Force | Where-Object Name -CMatch $hashPattern | ForEach-Object { Get-Release $_.Name })
            Set-Active $null $null
            foreach ($item in $installed) {
                $removedPath = Join-Path $releases ('.removed-' + $item.id)
                [IO.Directory]::Move($item.path, $removedPath)
                foreach ($name in ($item.names + 'release.json')) {
                    [IO.File]::Delete((Get-PlainItem (Join-Path $removedPath $name)).FullName)
                }
                [IO.Directory]::Delete($removedPath, $false)
            }
            [pscustomobject]@{ removed = $installed.Count; installRoot = $root }
        }
    }
} finally { $lock.Dispose() }
