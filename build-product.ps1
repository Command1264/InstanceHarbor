[CmdletBinding()]
# ConsoleRuntimeSource: a directory or local .nupkg holding the pinned console runtime
# (console-runtime.json). Defaults to IH_CONSOLE_RUNTIME_SOURCE. Never downloaded here.
param([switch]$Acceptance,[switch]$Release,[switch]$Development,[string]$ConsoleRuntimeSource=$env:IH_CONSOLE_RUNTIME_SOURCE)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$true
Push-Location $PSScriptRoot
try {
    if (@($Release, $Acceptance, $Development).Where({ $_ }).Count -gt 1) {
        throw 'Release, Acceptance and Development are mutually exclusive.'
    }
    # Reuse the already installed, lockfile-bound dependencies. Never install here.
    if(!(Test-Path -LiteralPath 'node_modules/.bin/vite.cmd')){throw '缺少本機依賴。此建置入口不會自動安裝。'}
    # The Git commit written into both executables: seven hex digits, `-dirty` with
    # uncommitted changes, or `unknown` without Git. build.rs checks the format.
    $commit='unknown'
    try{
        $hash=(& git -C $PSScriptRoot rev-parse HEAD 2>$null|Out-String).Trim()
        $changes=(& git -C $PSScriptRoot status --porcelain 2>$null|Out-String).Trim()
        if($hash -cmatch '^[0-9a-f]{40,64}$'){$commit=$hash.Substring(0,7)+$(if($changes){'-dirty'}else{''})}
    }catch{}
    if($commit -eq 'unknown'){Write-Warning '無法從 Git 取得 commit，執行檔會標示為 unknown。'}
    elseif($Release -and $commit.EndsWith('-dirty')){Write-Warning "工作目錄有未提交的變更，執行檔會標示為 $commit。"}
    $previousCommit=$env:IH_BUILD_COMMIT
    $env:IH_BUILD_COMMIT=$commit
    $frontendDirectory=if($Development){'dist-dev'}else{'dist'}
    $npm=& (Join-Path $PSScriptRoot 'resolve-npm.ps1')
    $viteArgs=@('--outDir',$frontendDirectory)
    # The development mode makes Vite use the red sidebar brand image (vite.config.ts).
    if($Development){$viteArgs+=@('--mode','development-channel')}
    & $npm run build -- @viteArgs
    $cargoArgs=@('build','--offline','--locked','--manifest-path','src-tauri/Cargo.toml')
    if($Development){
        $cargoArgs+=@('--profile','dev-channel','--target','x86_64-pc-windows-msvc','--features','development-channel')
        $binaryDirectory=Join-Path $PSScriptRoot 'src-tauri/target/x86_64-pc-windows-msvc/dev-channel'
    }elseif($Release){
        $cargoArgs+=@('--release','--target','x86_64-pc-windows-msvc')
        $binaryDirectory=Join-Path $PSScriptRoot 'src-tauri/target/x86_64-pc-windows-msvc/release'
    }else{
        $binaryDirectory=Join-Path $PSScriptRoot 'src-tauri/target/debug'
    }
    $binaryTarget = if ($Development) { 'InstanceHarborDev' }
        elseif ($Acceptance) { 'instanceharbor-acceptance' }
        else { 'InstanceHarbor' }
    if ($Acceptance) { $cargoArgs += @('--features', 'acceptance-ui') }
    $cargoArgs += @('--bin', $binaryTarget, '--bin', 'ihctl')
    $binaryName = $binaryTarget + '.exe'
    & cargo @cargoArgs
    $index=Join-Path $frontendDirectory 'index.html'
    $html=Get-Content -LiteralPath $index -Raw
    $assets=@([regex]::Matches($html,'/assets/([^" ]+\.(?:js|css))')|ForEach-Object{$_.Groups[1].Value})
    $builtBinary=Join-Path $binaryDirectory $binaryName
    $builtCli=Join-Path $binaryDirectory 'ihctl.exe'
    $binaryText=[Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($builtBinary))
    if($assets.Count -lt 2){throw '找不到 Vite 資產身分。'}
    $cliVersion=(& $builtCli --version|Out-String).Trim()
    if(!$cliVersion.EndsWith(" ($commit)")){throw "ihctl 未包含本次建置的 commit（$commit）：$cliVersion"}
    foreach($asset in $assets){if(!$binaryText.Contains($asset)){throw "原生候選未包含最新前端資產：$asset"}}
    # Blue icons for the product and acceptance builds, red ones for development only.
    $iconFiles=@{product='src-tauri/icons/icon.ico';development='src-tauri/icons/icon-dev.ico'}
    $brandFiles=@{product='src/assets/instanceharbor-brand.png';development='src/assets/instanceharbor-brand-dev.png'}
    $channel=if($Development){'development'}else{'product'}
    $otherChannel=if($Development){'product'}else{'development'}
    foreach($executable in @($builtBinary,$builtCli)){
        & (Join-Path $PSScriptRoot 'verify-app-icon.ps1') -Binary $executable -Expected $iconFiles[$channel]|Out-Null
    }
    $brandHashes=@(Get-ChildItem -LiteralPath (Join-Path $frontendDirectory 'assets') -Filter '*.png'|ForEach-Object{(Get-FileHash -LiteralPath $_.FullName).Hash})
    if($brandHashes -notcontains (Get-FileHash -LiteralPath $brandFiles[$channel]).Hash){throw "前端未包含 $($brandFiles[$channel]) 的側欄品牌圖。"}
    if($brandHashes -contains (Get-FileHash -LiteralPath $brandFiles[$otherChannel]).Hash){throw "前端誤含 $($brandFiles[$otherChannel]) 的側欄品牌圖。"}
    $folder=Join-Path $PSScriptRoot $(if($Development){'.evidence/windows-x64-development'}elseif($Release){'.evidence/windows-x64-release'}else{'.evidence/vue-product-20261109'})
    $binary=$builtBinary
    $cli=$builtCli
    if($Acceptance){
        $folder=Join-Path $PSScriptRoot 'artifacts/pin-drag-acceptance-20261110/native'
        if(Test-Path -LiteralPath $folder){throw "驗收交付目錄已存在，請先保存並指定新的候選路徑：$folder"}
        New-Item -ItemType Directory -Path $folder|Out-Null
        $folder=(Resolve-Path -LiteralPath $folder).Path
        $binary=Join-Path $folder 'InstanceHarbor.exe'
        $cli=Join-Path $folder 'ihctl.exe'
        Copy-Item -LiteralPath $builtBinary -Destination $binary
        Copy-Item -LiteralPath $builtCli -Destination $cli
        if((Get-FileHash -LiteralPath $binary).Hash -ne (Get-FileHash -LiteralPath $builtBinary).Hash -or
           (Get-FileHash -LiteralPath $cli).Hash -ne (Get-FileHash -LiteralPath $builtCli).Hash){throw '驗收檔案複製後雜湊不符。'}
    }else{
        New-Item -ItemType Directory -Path $folder -Force|Out-Null
    }
    # Tier 1 of the server console: the pinned runtime beside the binaries. Without it a
    # run falls back to the Windows built-in console, which release builds must not rely on.
    $runtimeTarget=Split-Path -Parent $binary
    $runtimeNames=@('conpty.dll','OpenConsole.exe')
    $runtimeSource=if($ConsoleRuntimeSource){$ConsoleRuntimeSource}
        elseif(@($runtimeNames|Where-Object{Test-Path -LiteralPath (Join-Path $runtimeTarget $_)}).Count -eq 2){$runtimeTarget}
        elseif($Acceptance -and @($runtimeNames|Where-Object{Test-Path -LiteralPath (Join-Path $binaryDirectory $_)}).Count -eq 2){$binaryDirectory}
    $consoleRuntime=$null
    if($runtimeSource){
        $consoleRuntime=(& (Join-Path $PSScriptRoot 'provision-console-runtime.ps1') -Source $runtimeSource -Destination $runtimeTarget).version
    }elseif($Release){
        throw '發行建置需要已核對的後台元件。請以 -ConsoleRuntimeSource 指定 conpty.dll／OpenConsole.exe 的本機來源。'
    }else{
        Write-Warning '未提供後台元件（conpty.dll、OpenConsole.exe）。此建置的伺服器後台會改用 Windows 內建主控台。'
    }
    @{assets=$assets;frontendDirectory=(Resolve-Path -LiteralPath $frontendDirectory).Path;binary=$binary;sha256=(Get-FileHash -LiteralPath $binary).Hash;cliSha256=(Get-FileHash -LiteralPath $cli).Hash;indexSha256=(Get-FileHash -LiteralPath $index).Hash;acceptanceUi=[bool]$Acceptance;development=[bool]$Development;commit=$commit;consoleRuntime=$consoleRuntime}|
        ConvertTo-Json|Set-Content -LiteralPath (Join-Path $folder 'build-manifest.json')
    'Vue 與原生候選資產已核對。'
} finally {$env:IH_BUILD_COMMIT=$previousCommit;Pop-Location}
