# Exercise the portable installer with a real native pair and a disposable, empty workspace.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [string]$ConsoleRuntimeDirectory,
    [string]$VCRuntimeDirectory,
    [string]$LegacyPackage,
    [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$LegacyPackageSha256,
    [Parameter(Mandatory)][string]$FixtureRoot
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if (!$IsWindows) { throw 'Windows PowerShell 7 is required.' }
if ([bool]$LegacyPackage -ne [bool]$LegacyPackageSha256) { throw 'A legacy package requires its trusted SHA-256.' }
$fixture = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($FixtureRoot)
if (Test-Path -LiteralPath $fixture) { throw 'FixtureRoot must be a new directory.' }
$parent = Get-Item -LiteralPath ([IO.Path]::GetDirectoryName($fixture)) -Force
for ($ancestor = $parent; $null -ne $ancestor; $ancestor = $ancestor.Parent) {
    if ($ancestor -isnot [IO.DirectoryInfo] -or ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Use an existing plain parent directory.' }
}
$null = [IO.Directory]::CreateDirectory($fixture)
$app = Join-Path $fixture 'application'
$workspace = Join-Path $fixture 'workspace'
$logs = Join-Path $fixture 'logs'
$null = [IO.Directory]::CreateDirectory($logs)
$log = Join-Path $logs 'session.log'
[IO.File]::WriteAllText($log, 'retained fixture log')
$pack = Join-Path $PSScriptRoot 'pack-windows.ps1'
$install = Join-Path $PSScriptRoot 'install-windows.ps1'
$first = if ($LegacyPackage) {
    @{ packagePath = (Resolve-Path -LiteralPath $LegacyPackage).Path; sha256 = $LegacyPackageSha256.ToLowerInvariant() }
} else {
    & $pack -BinaryDirectory $BinaryDirectory -Version 'fixture-1' -OutputZip (Join-Path $fixture 'fixture-1.zip')
}
$firstGui = if ($LegacyPackage) { 'instanceharbor-v2.exe' } else { 'InstanceHarbor.exe' }
$console = @{}
if ($ConsoleRuntimeDirectory) { $console.ConsoleRuntimeDirectory = $ConsoleRuntimeDirectory }
if ($VCRuntimeDirectory) { $console.VCRuntimeDirectory = $VCRuntimeDirectory }
$next = & $pack -BinaryDirectory $BinaryDirectory @console -Version 'fixture-2' -OutputZip (Join-Path $fixture 'fixture-2.zip')
$invalid = Join-Path $fixture 'invalid-entry.zip'
$file = [IO.File]::Open($invalid, 'CreateNew', 'Write', 'None')
try {
    $zip = [IO.Compression.ZipArchive]::new($file, [IO.Compression.ZipArchiveMode]::Create, $true)
    try {
        foreach ($name in @('manifest.json', 'ihctl.exe', '../outside.exe')) { $null = $zip.CreateEntry($name) }
    } finally { $zip.Dispose() }
} finally { $file.Dispose() }

function Assert([bool]$Condition, [string]$Message) { if (!$Condition) { throw $Message } }

function Assert-Error([scriptblock]$Operation, [string]$Expected) {
    try { $null = & $Operation } catch {
        if ($_.Exception.Message -like "*$Expected*") { return }
        throw
    }
    throw "Expected error: $Expected"
}

Assert-Error { & $install -Action Stage -InstallRoot $app -Package $first.packagePath } 'separately trusted'
Assert-Error { & $install -Action Install -InstallRoot $app -Package $first.packagePath -ExpectedPackageSha256 ('0' * 64) } 'does not match'
Assert-Error { & $install -Action Stage -InstallRoot $app -Package $invalid -ExpectedPackageSha256 (Get-FileHash -LiteralPath $invalid).Hash } 'Unexpected or duplicate ZIP entry'
$null = & $install -Action Install -InstallRoot $app -Package $first.packagePath -ExpectedPackageSha256 $first.sha256
$initial = & $install -Action Paths -InstallRoot $app
Assert ($initial.release -ceq $first.sha256) 'Initial release was not activated.'
Assert ((Split-Path -Leaf $initial.gui) -ceq $firstGui) 'Unexpected initial GUI filename.'
$null = & $initial.cli $workspace init
Assert ((& $initial.cli $workspace list | ConvertFrom-Json -NoEnumerate).Count -eq 0) 'Fixture workspace is not empty.'
$workspaceFile = Join-Path $workspace 'workspace.json'
$workspaceHash = (Get-FileHash -LiteralPath $workspaceFile).Hash

$knownInterruptedStage = '.stage-' + ('a' * 32)
$null = [IO.Directory]::CreateDirectory((Join-Path $app "releases/$knownInterruptedStage"))
$null = & $install -Action Stage -InstallRoot $app -Package $next.packagePath -ExpectedPackageSha256 $next.sha256
$staged = & $install -Action Status -InstallRoot $app
Assert ($staged.active -ceq $first.sha256 -and $staged.interruptedStages -ccontains $knownInterruptedStage) 'Staging changed the selected release or lost the interrupted stage.'

$null = & $install -Action Activate -InstallRoot $app -Release $next.sha256
$after = & $install -Action Status -InstallRoot $app
Assert ($after.active -ceq $next.sha256 -and $after.previous -ceq $first.sha256) 'Activation did not preserve the previous release.'
$paths = & $install -Action Paths -InstallRoot $app
Assert ($paths.release -ceq $next.sha256 -and (Split-Path -Leaf $paths.gui) -ceq 'InstanceHarbor.exe') 'The active executable path did not change.'
if ($ConsoleRuntimeDirectory) {
    $paths = & $install -Action Paths -InstallRoot $app
    Assert $paths.nativeConsoleRuntime 'The upgrade omitted the native console.'
    foreach ($name in @('conpty.dll', 'OpenConsole.exe')) {
        $from = (Get-FileHash -LiteralPath (Join-Path $ConsoleRuntimeDirectory $name)).Hash
        $to = (Get-FileHash -LiteralPath (Join-Path ([IO.Path]::GetDirectoryName($paths.cli)) $name)).Hash
        Assert ($from -ceq $to) "The native console runtime changed: $name"
    }
    $incomplete = Join-Path $fixture 'incomplete-console'
    $null = [IO.Directory]::CreateDirectory($incomplete)
    [IO.File]::Copy((Join-Path $ConsoleRuntimeDirectory 'conpty.dll'), (Join-Path $incomplete 'conpty.dll'))
    Assert-Error { & $pack -BinaryDirectory $BinaryDirectory -ConsoleRuntimeDirectory $incomplete -Version 'fixture-partial' -OutputZip (Join-Path $fixture 'partial.zip') } 'requires both'
    [IO.File]::WriteAllText((Join-Path $incomplete 'OpenConsole.exe'), 'changed runtime')
    Assert-Error { & $pack -BinaryDirectory $BinaryDirectory -ConsoleRuntimeDirectory $incomplete -Version 'fixture-mismatch' -OutputZip (Join-Path $fixture 'mismatch.zip') } 'Native console runtime SHA-256 mismatch'
}

if ($VCRuntimeDirectory) {
    $paths = & $install -Action Paths -InstallRoot $app
    Assert $paths.visualCppRuntime 'The upgrade omitted app-local Microsoft C++ runtime.'
    $incomplete = Join-Path $fixture 'incomplete-vc'
    $null = [IO.Directory]::CreateDirectory($incomplete)
    foreach ($name in @('vcruntime140.dll','vcruntime140_1.dll')) {
        $source = Join-Path $VCRuntimeDirectory $name
        $target = Join-Path ([IO.Path]::GetDirectoryName($paths.cli)) $name
        Assert ((Get-FileHash -LiteralPath $source).Hash -ceq (Get-FileHash -LiteralPath $target).Hash) "Microsoft C++ runtime file changed: $name"
    }
    [IO.File]::Copy((Join-Path $VCRuntimeDirectory 'vcruntime140.dll'),(Join-Path $incomplete 'vcruntime140.dll'))
    Assert-Error { & $pack -BinaryDirectory $BinaryDirectory -VCRuntimeDirectory $incomplete -Version 'fixture-vc-partial' -OutputZip (Join-Path $fixture 'partial-vc.zip') } 'requires both'
    [IO.File]::WriteAllText((Join-Path $incomplete 'vcruntime140_1.dll'),'changed runtime')
    Assert-Error { & $pack -BinaryDirectory $BinaryDirectory -VCRuntimeDirectory $incomplete -Version 'fixture-vc-mismatch' -OutputZip (Join-Path $fixture 'mismatch-vc.zip') } 'Microsoft C++ runtime SHA-256 mismatch'
}

$null = & $install -Action Rollback -InstallRoot $app
$rolledBack = & $install -Action Status -InstallRoot $app
Assert ($rolledBack.active -ceq $first.sha256 -and $rolledBack.previous -ceq $next.sha256) 'Rollback did not restore the previous release.'
Assert ((& $install -Action Paths -InstallRoot $app).gui -ceq $initial.gui) 'Rollback did not restore the previous executable path.'

$result = & $install -Action Uninstall -InstallRoot $app
$removed = & $install -Action Status -InstallRoot $app
Assert ($result.removed -eq 2 -and !$removed.active -and $removed.releases.Count -eq 0) 'Uninstall did not clear installed releases.'
Assert ((Get-FileHash -LiteralPath $workspaceFile).Hash -ceq $workspaceHash) 'Workspace bytes changed.'
Assert ([IO.File]::ReadAllText($log) -ceq 'retained fixture log') 'Retained log changed.'
Assert ($removed.interruptedStages -ccontains $knownInterruptedStage) 'Uninstall deleted unverified interrupted content.'
[pscustomobject]@{ fixture = $fixture; initial = $first.sha256; upgrade = $next.sha256; removed = $result.removed; workspaceSha256 = $workspaceHash }
