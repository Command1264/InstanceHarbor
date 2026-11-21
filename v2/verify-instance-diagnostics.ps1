#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Evidence,
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$Java,
    [Parameter(Mandatory)][string]$Velocity,
    [Parameter(Mandatory)][string]$Paper,
    [Parameter(Mandatory)][string]$Mojang,
    [Parameter(Mandatory)][string]$LuckPerms,
    [Parameter(Mandatory)][string]$PaperDependencies,
    [switch]$ClientAcceptance,
    [switch]$GuiReview
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Evidence = [IO.Path]::GetFullPath($Evidence)
if (Test-Path -LiteralPath $Evidence) { throw 'Choose a new evidence directory.' }
$Java = (Get-Item -LiteralPath $Java).FullName
$cli = (Get-Item -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe')).FullName
$gui = (Get-Item -LiteralPath (Join-Path $BinaryDirectory 'InstanceHarbor.exe')).FullName
$assets = [ordered]@{
    velocity = @{ path = $Velocity; sha256 = 'DB6BE7E3FFE9114E9A5D0645FBEA398BF4591276603718FFEEDF4D2D8EAC1AAA' }
    paper = @{ path = $Paper; sha256 = 'A0A58F532F08054EA92EB916D3EDB6ED3C86429D77175317304F6E995CCE9968' }
    mojang = @{ path = $Mojang; sha256 = 'D052F14D7A173734FBA553711E5B570162E2F2A313267EE31A21B975A679BE64' }
    luckPerms = @{ path = $LuckPerms; sha256 = 'DC637CE18F48D3B75A7FFD1784B85BE16A627359090DFE5ADF15DAB6D145DC7D' }
}
foreach ($asset in $assets.Values) {
    $asset.path = (Get-Item -LiteralPath $asset.path).FullName
    if ((Get-FileHash -LiteralPath $asset.path).Hash -cne $asset.sha256) {
        throw "Fixture digest mismatch for $($asset.path)."
    }
}

$workspace = Join-Path $Evidence 'workspace'
$backend = 'diagnostics-paper'
$proxy = 'diagnostics-velocity'
$route = 'paper'
$pluginJar = 'LuckPerms-Bukkit-5.5.85.jar'
$paperDir = Join-Path $workspace "instances/Agent/Diagnostics/$backend"
$velocityDir = Join-Path $workspace "instances/$proxy"
$velocityConfig = Join-Path $velocityDir 'velocity.toml'

function Ih {
    $reply = & $cli $workspace @args
    $exitCode = $LASTEXITCODE
    if ($exitCode) { throw "ihctl $($args[0]) failed with exit code $exitCode." }
    ConvertFrom-Json -InputObject ($reply -join [Environment]::NewLine) -NoEnumerate
}

function Check([bool]$Condition, [string]$Description) {
    if (!$Condition) { throw "Failed: $Description" }
    $checks.Add($Description)
}

function Shared-FileHash([string]$Path) {
    $stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite, Delete')
    try { [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)) }
    finally { $stream.Dispose() }
}

function Snapshot([switch]$Live) {
    $entries = @(Get-Item -LiteralPath $workspace) + @(Get-ChildItem -LiteralPath $workspace -Recurse -Force)
    @($entries |
        Sort-Object FullName |
        ForEach-Object {
            $relative = [IO.Path]::GetRelativePath($workspace, $_.FullName)
            $javaWorldLock = $Live -and $_.FullName -eq (Join-Path $paperDir 'world/session.lock')
            $hash = if ($javaWorldLock) { 'ByteLockedByPaper' } elseif (!$_.PSIsContainer) { Shared-FileHash $_.FullName }
            $_.Refresh()
            [ordered]@{
                path = $relative
                directory = $_.PSIsContainer
                sha256 = $hash
                lastWriteUtc = if (!$_.PSIsContainer) { $_.LastWriteTimeUtc.Ticks }
            }
        }) | ConvertTo-Json -Depth 4 -Compress -AsArray
}

function Stable-Snapshot([switch]$Live) {
    $previous = Snapshot -Live:$Live
    for ($attempt = 0; $attempt -lt 4; $attempt++) {
        Start-Sleep -Milliseconds 300
        $current = Snapshot -Live:$Live
        if ($current -ceq $previous) { return $current }
        $previous = $current
    }
    throw 'The isolated workspace did not settle for the read-only snapshot.'
}

function Assert-NoWrites([string]$Before, [string]$Phase, [switch]$Live) {
    $after = Snapshot -Live:$Live
    $digest = { param($Content) [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Content))) }
    $result.evidence["$Phase-workspace"] = @{
        beforeSha256 = & $digest $Before
        afterSha256 = & $digest $after
        liveContentExclusion = if ($Live) { 'Paper byte-locks world/session.lock. Its path and write time are still compared.' }
    }
    if ($after -cne $Before) {
        Set-Content -LiteralPath (Join-Path $Evidence "$Phase-before.json") -Value $Before
        Set-Content -LiteralPath (Join-Path $Evidence "$Phase-after.json") -Value $after
    }
    Check ($after -ceq $Before) "$Phase query preserved directory names, file SHA-256 and write times"
}

function Wait-Ready([string]$Name, [string]$Run, [int]$Port) {
    $deadline = [DateTime]::UtcNow.AddSeconds(150)
    do {
        $status = Ih status $Name
        if ($status.run.id -cne $Run -or $status.state -notin 'Listening', 'Starting') {
            throw "$Name left the expected $Run while starting."
        }
        if ($status.state -eq 'Listening' -and $status.consoleReady) {
            $listener = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
            if ($listener.LocalAddress -eq '127.0.0.1' -and $listener.OwningProcess -eq $status.run.java.pid) {
                return $status
            }
        }
        Start-Sleep -Milliseconds 375
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "$Name did not reach console readiness on its own loopback listener."
}

function Import-Fixture([string]$Name, [string]$Path, [int]$Port) {
    $preview = Ih import-preview $Name $Java $Path
    Check ($preview.port -eq $Port) "$Name import preview matched its private port"
    $null = Ih import $Name "import-$Name" $Java $Path "$Port" $preview.revision
}

function Start-Fixtures([string]$Phase) {
    foreach ($name in @($backend, $proxy)) {
        $started.Add($name)
        $launch = Ih start $name "$Phase-start-$name"
        if (!$launch.run.id) { throw "$name did not return a run ID." }
        $currentRuns[$name] = $launch.run.id
        $port = if ($name -ceq $proxy) { $proxyPort } else { $paperPort }
        $result.evidence["$Phase-$name"] = Wait-Ready $name $launch.run.id $port
        Check $true "$Phase $name reached console readiness on its own loopback listener"
    }
}

function Stop-Fixtures([string]$Phase) {
    for ($i = $started.Count - 1; $i -ge 0; $i--) {
        $name = $started[$i]
        try {
            $state = Ih status $name
            if ($state.state -ne 'Stopped') {
                $expected = $currentRuns[$name]
                if (!$expected -or $state.run.id -cne $expected) {
                    throw "$name has an unconfirmed run. Investigate without stopping it."
                }
                $state = Ih stop $name "$Phase-stop-$name" $expected
            }
            Check ($state.state -eq 'Stopped') "$Phase $name stopped normally"
            $port = if ($name -ceq $proxy) { $proxyPort } else { $paperPort }
            Check (!(Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue)) "$Phase $name released its port"
            $result.evidence["$Phase-stopped-$name"] = $state
        } catch {
            $cleanupErrors.Add($_.Exception.Message)
        }
    }
    if ($cleanupErrors.Count -eq 0) { $started.Clear() }
}

function Stop-FixtureManager {
    $descriptor = Join-Path $workspace 'resident-manager.json'
    if (!(Test-Path -LiteralPath $descriptor -PathType Leaf)) { return }
    $identity = Get-Content -LiteralPath $descriptor -Raw | ConvertFrom-Json
    $manager = Get-Process -Id $identity.pid -ErrorAction SilentlyContinue
    if (!$manager) { return }
    if ($manager.Path -ne $cli -or $manager.StartTime.ToFileTimeUtc() -ne $identity.created) {
        throw 'Isolated Manager identity changed. No exit command was sent.'
    }
    $finished = Ih manager-exit
    Check ($finished.exiting -and $manager.WaitForExit(8000)) 'the isolated Manager exited after both ports were released'
}

$ports = [Collections.Generic.List[int]]::new()
$reservations = [Collections.Generic.List[Net.Sockets.TcpListener]]::new()
try {
    foreach ($name in @($backend, $proxy)) {
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
        $listener.Start()
        $reservations.Add($listener)
        $ports.Add($listener.LocalEndpoint.Port)
    }
} finally {
    foreach ($listener in $reservations) { $listener.Stop() }
}
$paperPort, $proxyPort = $ports
$checks = [Collections.Generic.List[string]]::new()
$runIds = [ordered]@{}
$restartRunIds = [ordered]@{}
$currentRuns = $runIds
$cleanupErrors = [Collections.Generic.List[string]]::new()
$started = [Collections.Generic.List[string]]::new()
$playerAcceptance = 'NotRun'
$result = [ordered]@{
    result = 'FAIL'
    workspace = $workspace
    endpoint = "127.0.0.1:$proxyPort"
    ports = [ordered]@{ paper = $paperPort; velocity = $proxyPort }
    runIds = $runIds
    restartRunIds = $restartRunIds
    evidence = [ordered]@{}
    checks = $checks
    cleanupErrors = $cleanupErrors
    error = $null
}

$null = [IO.Directory]::CreateDirectory($Evidence)
$originalLocalAppData = $env:LOCALAPPDATA
try {
    $env:LOCALAPPDATA = Join-Path $Evidence 'appdata'
    $null = [IO.Directory]::CreateDirectory($env:LOCALAPPDATA)
    $null = Ih init
    $policy = Join-Path $workspace 'scan-policy.json'
    $policyHash = (Get-FileHash -LiteralPath $policy).Hash
    $null = [IO.Directory]::CreateDirectory((Join-Path $paperDir 'cache'))
    $null = [IO.Directory]::CreateDirectory((Join-Path $paperDir 'config'))
    $null = [IO.Directory]::CreateDirectory((Join-Path $paperDir 'plugins'))
    $null = [IO.Directory]::CreateDirectory((Join-Path $velocityDir 'plugins/bStats'))
    Copy-Item -LiteralPath $assets.paper.path -Destination (Join-Path $paperDir 'server.jar')
    Copy-Item -LiteralPath $assets.mojang.path -Destination (Join-Path $paperDir 'cache/mojang_26.3.jar')
    Copy-Item -LiteralPath $assets.velocity.path -Destination (Join-Path $velocityDir 'server.jar')

    # Seed only generated dependencies, never worlds, settings, accounts or logs.
    foreach ($relative in @('libraries', 'versions')) {
        $source = Get-Item -LiteralPath (Join-Path $PaperDependencies $relative)
        if ($source.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint) -or
            (Get-ChildItem -LiteralPath $source.FullName -Recurse -Force -Attributes ReparsePoint)) {
            throw "Fixture dependencies contain a link at $relative."
        }
        $dest = Join-Path $paperDir $relative
        $null = [IO.Directory]::CreateDirectory((Split-Path -Parent $dest))
        Copy-Item -LiteralPath $source.FullName -Destination $dest -Recurse
    }
    $secret = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
    @('enabled=false', "serverUuid=$([Guid]::NewGuid())", 'logFailedRequests=false') |
        Set-Content -LiteralPath (Join-Path $velocityDir 'plugins/bStats/config.txt')
    'eula=true' | Set-Content -LiteralPath (Join-Path $paperDir 'eula.txt')
    @(
        'server-ip=127.0.0.1'
        "server-port=$paperPort"
        'online-mode=false'
        'enable-rcon=false'
        'level-type=minecraft:flat'
        'difficulty=peaceful'
        'view-distance=3'
        'simulation-distance=3'
        'max-players=4'
        'motd=InstanceHarbor isolated diagnostics Paper'
    ) | Set-Content -LiteralPath (Join-Path $paperDir 'server.properties')
    @(
        '_version: 31'
        'proxies:'
        '  velocity:'
        '    enabled: true'
        '    online-mode: true'
        "    secret: '$secret'"
    ) | Set-Content -LiteralPath (Join-Path $paperDir 'config/paper-global.yml')
    @('settings:', '  bungeecord: false') |
        Set-Content -LiteralPath (Join-Path $paperDir 'spigot.yml')
    $secret | Set-Content -LiteralPath (Join-Path $velocityDir 'forwarding.secret') -NoNewline
    @(
        'config-version = "2.9"'
        "bind = ""127.0.0.1:$proxyPort"""
        'motd = "InstanceHarbor isolated diagnostics Velocity"'
        'online-mode = true'
        'force-key-authentication = true'
        'player-info-forwarding-mode = "modern"'
        'forwarding-secret-file = "forwarding.secret"'
        ''
        '[servers]'
        "$route = ""127.0.0.1:$paperPort"""
        "try = [""$route""]"
        ''
        '[forced-hosts]'
    ) | Set-Content -LiteralPath $velocityConfig
    Import-Fixture $backend $paperDir $paperPort
    Import-Fixture $proxy $velocityDir $proxyPort
    $installed = Ih admin-install $backend $assets.luckPerms.path
    Check ($installed.artifactSha256 -eq $assets.luckPerms.sha256 -and $installed.status -eq 'SeededJson') 'Paper test permissions were staged using the pinned plugin and Manager policy'
    $luckPermsLibraries = Get-Item -LiteralPath (Join-Path $PaperDependencies 'plugins/LuckPerms/libs')
    if ($luckPermsLibraries.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint) -or
        (Get-ChildItem -LiteralPath $luckPermsLibraries.FullName -Recurse -Force -Attributes ReparsePoint)) {
        throw 'LuckPerms fixture dependencies contain a link.'
    }
    Copy-Item -LiteralPath $luckPermsLibraries.FullName -Destination (Join-Path $paperDir 'plugins/LuckPerms/libs') -Recurse
    $opts = @{ proxy = $proxy; route = $route; plugin = $pluginJar } | ConvertTo-Json -Compress
    $manifest = [ordered]@{
        workspace = $workspace; cli = $cli; cliSha256 = (Get-FileHash -LiteralPath $cli).Hash
        gui = $gui; guiSha256 = (Get-FileHash -LiteralPath $gui).Hash; appData = $env:LOCALAPPDATA
        java = $Java; assets = $assets; dependencySource = $PaperDependencies
        backend = $backend; proxy = $proxy; route = $route; plugin = $pluginJar; ports = $result.ports
    }
    $manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Evidence 'fixture.json')

    $before = Stable-Snapshot
    $stopped = Ih instance-diagnostics $backend $opts
    $result.evidence.stopped = $stopped
    Check ($stopped.configuration.state -eq 'Match' -and $stopped.runtime.state -eq 'Stopped') 'matching configuration does not imply an active runtime'
    Check ($stopped.plugin.state -eq 'Present' -and $stopped.player.state -eq 'Unverified') 'a plugin JAR alone and no player activity are not live proof'
    Assert-NoWrites $before 'stopped'

    Start-Fixtures 'original'
    $before = Stable-Snapshot -Live
    $running = Ih instance-diagnostics $backend $opts
    $result.evidence.running = $running
    Check ($running.configuration.state -eq 'Match' -and $running.runtime.state -eq 'Ready') 'the same configuration and both active runs are independently proven'
    Check ($running.runtime.backendRun -eq $runIds[$backend] -and $running.runtime.proxyRun -eq $runIds[$proxy]) 'runtime evidence belongs to the two current run IDs'
    Check ($running.plugin.state -eq 'Enabled') 'current Paper run enabled LuckPerms'
    Check ($running.runtime.forwarding -eq 'Unconfirmed' -and $running.player.state -eq 'Unverified') 'runtime and plugin proof do not establish forwarding or player login'
    Assert-NoWrites $before 'live' -Live
    if ($GuiReview) {
        Write-Host "隔離 GUI 的工作區 $workspace"
        Write-Host "候選執行檔 $gui"
        if ((Read-Host '確認已關閉驗收 GUI 後按 Enter，取消請輸入 cancel') -ne '') { throw 'GUI review cancelled.' }
    }
    if ($ClientAcceptance) {
        $proxyLog = Join-Path $velocityDir 'logs/latest.log'
        $paperLog = Join-Path $paperDir 'logs/latest.log'
        $proxyBefore = @(Get-Content -LiteralPath $proxyLog).Count
        $paperBefore = @(Get-Content -LiteralPath $paperLog).Count
        Write-Host "請用已登入的 Minecraft 26.3 客戶端連到 $($result.endpoint)，進入 Paper 後保持連線。"
        $confirmation = Read-Host '確認已進入世界後按 Enter，略過請輸入 skip，取消請輸入 cancel'
        if ($confirmation -eq 'skip') {
            $playerAcceptance = 'Skipped'
        } elseif ($confirmation -eq '') {
            try {
                $proxyNew = @(Get-Content -LiteralPath $proxyLog | Select-Object -Skip $proxyBefore)
                $paperNew = @(Get-Content -LiteralPath $paperLog | Select-Object -Skip $paperBefore)
                $player = [regex]::Match(($proxyNew -join "\n"), '\[server connection\] (\w{1,16}) -> paper has connected')
                $paperLogin = '^\[\d{2}:\d{2}:\d{2}\] \[Server thread/INFO\]: ' +
                    [regex]::Escape($player.Groups[1].Value) +
                    '\[/[^\]]+\] logged in with entity id \d+ at \('
                Check ($player.Success -and ($paperNew -match $paperLogin).Count -gt 0) 'live player evidence matches the Velocity route and Paper login event'
                $accountAfter = Ih instance-diagnostics $backend $opts
                $result.evidence.afterPlayer = $accountAfter
                Check ($accountAfter.runtime.forwarding -eq 'Observed' -and $accountAfter.player.state -eq 'Unverified') 'matching current-run route evidence is observed while the player verification stays in acceptance evidence'
                $playerAcceptance = 'VerifiedViaVelocityAndPaperLogs'
            } catch {
                if ($GuiReview) {
                    Write-Host $_.Exception.Message
                    $null = Read-Host '登入證據尚待核對，核對完成後按 Enter 停止隔離測服'
                }
                throw
            }
            $null = Read-Host '請離開隔離測服後按 Enter，開始核對並正常停止'
        } else {
            throw 'Client acceptance cancelled.'
        }
    }

    # An edited file only affects configuration evidence, never proves that
    # the live proxy picked up the edit or changes which current run is shown.
    $configBytes = [IO.File]::ReadAllBytes($velocityConfig)
    try {
        $drift = [Text.Encoding]::UTF8.GetString($configBytes).Replace("$route = ""127.0.0.1:$paperPort""", "$route = ""127.0.0.1:$proxyPort""")
        [IO.File]::WriteAllText($velocityConfig, $drift)
        $changed = Ih instance-diagnostics $backend $opts
        $result.evidence.drift = $changed
        Check ($changed.configuration.state -eq 'Mismatch' -and $changed.player.state -eq 'Unverified') 'external route drift is not reported as a matching configuration or player proof'
        Check ($changed.runtime.state -eq 'Ready' -and $changed.runtime.backendRun -eq $runIds[$backend] -and
            $changed.runtime.proxyRun -eq $runIds[$proxy] -and $changed.runtime.forwarding -eq 'Unconfirmed') 'route edits preserve the identity of both active runs without inferring reload'
    } finally {
        [IO.File]::WriteAllBytes($velocityConfig, $configBytes)
    }
    Stop-Fixtures 'original'
    if ($cleanupErrors.Count) { throw 'The original runs did not both stop normally.' }
    $afterStop = Ih instance-diagnostics $backend $opts
    $result.evidence.afterOriginalStop = $afterStop
    Check ($afterStop.configuration.state -eq 'Match' -and $afterStop.runtime.state -eq 'Stopped' -and
        $afterStop.plugin.state -eq 'Present' -and $afterStop.runtime.forwarding -eq 'Unconfirmed') 'after normal stop the restored route matches and past evidence is not presented as current'

    $currentRuns = $restartRunIds
    Start-Fixtures 'replacement'
    $restarted = Ih instance-diagnostics $backend $opts
    $result.evidence.restarted = $restarted
    Check ($restarted.configuration.state -eq 'Match' -and $restarted.runtime.state -eq 'Ready' -and
        $restarted.runtime.backendRun -eq $restartRunIds[$backend] -and $restarted.runtime.proxyRun -eq $restartRunIds[$proxy] -and
        $restartRunIds[$backend] -ne $runIds[$backend] -and $restartRunIds[$proxy] -ne $runIds[$proxy]) 'replacement listeners are observed under distinct run IDs'
    Check ($restarted.plugin.state -eq 'Enabled' -and $restarted.runtime.forwarding -eq 'Unconfirmed' -and
        $restarted.player.state -eq 'Unverified') 'after restart plugin proof comes from the new run and previous connection evidence does not transfer'
    Check ((Get-FileHash -LiteralPath $policy).Hash -ceq $policyHash) 'isolated scan policy is unchanged'
    $result.result = 'PASS'
} catch {
    $result.error = $_.Exception.Message
    throw
} finally {
    $result.playerAcceptance = $playerAcceptance
    try {
        Stop-Fixtures 'final'
        if (!$cleanupErrors.Count) {
            try { Stop-FixtureManager } catch {
                $cleanupErrors.Add($_.Exception.Message)
            }
        }
        if ($cleanupErrors.Count) { $result.result = 'FAIL' }
        $result | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath (Join-Path $Evidence 'result.json')
    } finally {
        $env:LOCALAPPDATA = $originalLocalAppData
    }
    if ($cleanupErrors.Count) { throw ($cleanupErrors -join [Environment]::NewLine) }
}

[PSCustomObject]$result
