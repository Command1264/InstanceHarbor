[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Export','Inspect','Restore')][string]$Action,
    [Parameter(Mandatory)][string]$Cli,
    [Parameter(Mandatory)][string]$Workspace,
    [Parameter(Mandatory)][string]$Backup,
    [uint64]$Generation,
    [string]$PublicKey
)

$ErrorActionPreference = 'Stop'
$cliPath = (Resolve-Path -LiteralPath $Cli).ProviderPath
if ([IO.Path]::GetFileName($cliPath) -ne 'ihctl.exe') { throw '請指定 InstanceHarbor 的 ihctl.exe。' }
if (![IO.Path]::IsPathFullyQualified($Workspace) -or ![IO.Path]::IsPathFullyQualified($Backup)) {
    throw '請使用工作區及備份檔的完整路徑。'
}
if ($Action -in 'Export','Restore' -and !$Generation) { throw '請提供要核對的金鑰世代。' }
if ($Action -eq 'Restore' -and $PublicKey -cnotmatch '^[a-f0-9]{128}$') {
    throw '請先執行 Inspect，取得這份備份的公開金鑰。'
}

$secret = Read-Host '輸入備份密碼' -AsSecureString
$pointer = [IntPtr]::Zero
$characters = $null
$bytes = $null
$child = $null
try {
    $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret)
    $characters = [char[]]::new($secret.Length)
    [Runtime.InteropServices.Marshal]::Copy($pointer, $characters, 0, $characters.Length)
    $bytes = [Text.UTF8Encoding]::new($false,$true).GetBytes($characters)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $cliPath
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $start.CreateNoWindow = $true
    foreach ($argument in @($Workspace, ('recovery-backup-' + $Action.ToLowerInvariant()), $Backup)) {
        $start.ArgumentList.Add($argument)
    }
    if ($Action -in 'Export','Restore') { $start.ArgumentList.Add($Generation.ToString([Globalization.CultureInfo]::InvariantCulture)) }
    if ($Action -eq 'Restore') { $start.ArgumentList.Add($PublicKey) }
    $child = [Diagnostics.Process]::Start($start)
    try { $child.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length) }
    finally { $child.StandardInput.Close() }
    $child.WaitForExit()
    if ($child.ExitCode -ne 0) { throw "復原金鑰操作失敗，請核對上方錯誤。" }
} finally {
    if ($child) { $child.Dispose() }
    if ($bytes) { [Array]::Clear($bytes,0,$bytes.Length) }
    if ($characters) { [Array]::Clear($characters,0,$characters.Length) }
    if ($pointer -ne [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer) }
    $secret.Dispose()
}
