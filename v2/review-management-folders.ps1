#Requires -Version 7.0
[CmdletBinding()]
param([ValidateSet('Open', 'Evidence')][string]$Action = 'Open')

$ErrorActionPreference = 'Stop'
$descriptor = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'candidate.json') -Raw | ConvertFrom-Json
$bin = Join-Path $PSScriptRoot 'bin'
$cli = Join-Path $bin 'ihctl.exe'
$gui = Join-Path $bin 'InstanceHarbor.exe'
if ((Get-FileHash -LiteralPath $cli).Hash -cne $descriptor.cliSha256 -or
    (Get-FileHash -LiteralPath $gui).Hash -cne $descriptor.guiSha256) {
    throw '候選執行檔與封裝紀錄不符。'
}
if ($Action -eq 'Evidence') {
    Get-Content -LiteralPath (Join-Path $PSScriptRoot $descriptor.agentEvidence) -Raw | ConvertFrom-Json
    return
}

$session = Join-Path $PSScriptRoot 'review-session'
$workspace = Join-Path $session 'workspace'
if (!(Test-Path -LiteralPath $session)) {
    $assets = $descriptor.assets
    $setup = @{
        Evidence = $session
        BinaryDirectory = $bin
        Java = $descriptor.java
        Paper = $assets.paper.path
        Mojang = $assets.mojang.path
        Velocity = $assets.velocity.path
        PaperDependencies = $descriptor.dependencySource
        ManagementFolders = $true
        PrepareOnly = $true
    }
    & (Join-Path $PSScriptRoot 'verify-batch-import.ps1') @setup | Out-Null
}

$fixture = Get-Content -LiteralPath (Join-Path $session 'fixture.json') -Raw | ConvertFrom-Json
if ($fixture.workspace -cne $workspace -or $fixture.cliSha256 -cne $descriptor.cliSha256 -or
    $fixture.guiSha256 -cne $descriptor.guiSha256 -or
    $fixture.owner -cne 'windows-v2-isolated-management-folders') {
    throw '隔離工作區與封裝紀錄不符。'
}

$previous = @{
    Workspace = $env:IH_V2_WORKSPACE
    WebView = $env:WEBVIEW2_USER_DATA_FOLDER
    AppData = $env:LOCALAPPDATA
    JavaHome = $env:JAVA_HOME
}
try {
    $env:IH_V2_WORKSPACE = $workspace
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $session 'review-webview'
    $env:LOCALAPPDATA = $fixture.appData
    $env:JAVA_HOME = [IO.Directory]::GetParent([IO.Path]::GetDirectoryName($fixture.java)).FullName
    $existing = Get-Process -Name InstanceHarbor,instanceharbor-v2 -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -ieq $gui -and $_.MainWindowTitle -ceq 'InstanceHarbor' } |
        Select-Object -First 1
    $start = [Diagnostics.ProcessStartInfo]::new($gui)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $process = [Diagnostics.Process]::Start($start)
    if (!$existing) {
        $deadline = [DateTime]::UtcNow.AddSeconds(12)
        do {
            $process.Refresh()
            if ($process.HasExited) { throw '候選沒有開啟視窗。如果另一個 InstanceHarbor 視窗已開啟，請稍後再開驗收版。' }
            if ($process.MainWindowTitle -ceq 'InstanceHarbor') { break }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $deadline)
        if ($process.MainWindowTitle -cne 'InstanceHarbor') { throw '尚未觀察到驗收視窗。' }
    }
    [pscustomobject]@{
        版本 = $descriptor.commit
        GUI = if ($existing) { $existing.Id } else { $process.Id }
        隔離工作區 = $workspace
        選取此資料夾 = $fixture.scanPath
        可取消勾選 = 'batch-ignored'
    }
} finally {
    $env:IH_V2_WORKSPACE = $previous.Workspace
    $env:WEBVIEW2_USER_DATA_FOLDER = $previous.WebView
    $env:LOCALAPPDATA = $previous.AppData
    $env:JAVA_HOME = $previous.JavaHome
}
