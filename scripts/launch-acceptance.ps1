[CmdletBinding()]
param([string]$Workspace, [string]$BinaryDirectory = (Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri/target/debug'))
$ErrorActionPreference='Stop'
if(!$Workspace){
    $verified=Get-ChildItem -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) '.evidence') -Directory -Filter 'acceptance-*' |
        Where-Object {Test-Path -LiteralPath (Join-Path $_.FullName 'acceptance.json')} |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if(!$verified){throw '找不到已通過驗證的隔離工作區。'}
    $Workspace=$verified.FullName
}
$Workspace=(Resolve-Path -LiteralPath $Workspace).Path
$previous=$env:IH_WORKSPACE
$previousWebView=$env:WEBVIEW2_USER_DATA_FOLDER
try{
    $env:IH_WORKSPACE=$Workspace
    $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $Workspace 'manual-webview'
    $process=Start-Process -FilePath (Join-Path $BinaryDirectory 'InstanceHarbor.exe') -PassThru
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    do {
        $process.Refresh()
        if($process.HasExited){throw "GUI 程序提前結束，代碼 $($process.ExitCode)。"}
        if($process.MainWindowHandle -ne [IntPtr]::Zero){break}
        Start-Sleep -Milliseconds 100
    }until([DateTime]::UtcNow -gt $deadline)
    if($process.MainWindowHandle -eq [IntPtr]::Zero){throw '在時限內未觀察到 GUI 視窗。'}
    "驗收 GUI PID $($process.Id)。工作區 $Workspace"
}finally{
    $env:IH_WORKSPACE=$previous
    $env:WEBVIEW2_USER_DATA_FOLDER=$previousWebView
}
