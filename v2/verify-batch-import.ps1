#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Evidence,
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][string]$Java,
    [Parameter(Mandatory)][string]$Paper,
    [Parameter(Mandatory)][string]$Mojang,
    [Parameter(Mandatory)][string]$Velocity,
    [Parameter(Mandatory)][string]$PaperDependencies,
    [switch]$ManagementFolders,
    [switch]$SkipNativeUi,
    [switch]$PrepareOnly,
    [switch]$ResumePrepared,
    [switch]$ResumeRegistered
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if ($SkipNativeUi -and !$ManagementFolders) { throw 'SkipNativeUi only applies to management-folder verification.' }
if ($ResumeRegistered -and (!$ManagementFolders -or $ResumePrepared -or $PrepareOnly)) {
    throw 'ResumeRegistered requires a previously registered management-folder fixture.'
}
$resume = $ResumePrepared -or $ResumeRegistered
$Evidence = [IO.Path]::GetFullPath($Evidence)
if ((Test-Path -LiteralPath $Evidence) -ne [bool]$resume) {
    throw 'Use a new isolated evidence directory or explicitly resume the prepared fixture.'
}
$Java = (Get-Item -LiteralPath $Java).FullName
$cli = (Get-Item -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe')).FullName
$gui = (Get-Item -LiteralPath (Join-Path $BinaryDirectory 'InstanceHarbor.exe')).FullName
$assets = [ordered]@{
    paper = @{ path = $Paper; sha256 = 'A0A58F532F08054EA92EB916D3EDB6ED3C86429D77175317304F6E995CCE9968' }
    mojang = @{ path = $Mojang; sha256 = 'D052F14D7A173734FBA553711E5B570162E2F2A313267EE31A21B975A679BE64' }
    velocity = @{ path = $Velocity; sha256 = 'DB6BE7E3FFE9114E9A5D0645FBEA398BF4591276603718FFEEDF4D2D8EAC1AAA' }
}
foreach ($asset in $assets.Values) {
    $item = Get-Item -LiteralPath $asset.path
    if ($item.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint) -or
        (Get-FileHash -LiteralPath $item.FullName).Hash -cne $asset.sha256) {
        throw "Fixture identity mismatch at $($item.FullName)."
    }
    $asset.path = $item.FullName
}
$workspace = Join-Path $Evidence 'workspace'
$source = Join-Path $workspace 'instances'
$scanDirectory = 'instances/BatchFixture'
$scanPath = Join-Path $workspace $scanDirectory
$paperDir = Join-Path $source 'BatchFixture/測服/batch-paper'
$velocityDir = Join-Path $source 'BatchFixture/轉送/batch-velocity'
$excludedDir = Join-Path $source 'BatchFixture/忽略/batch-ignored'
$ignoredPortName = 'batch-ignored'
$laterPortName = 'batch-later'
$policy = Join-Path $workspace 'scan-policy.json'
$checks = [Collections.Generic.List[string]]::new()
$runIds = [ordered]@{}
$ports = [ordered]@{}
$fixtureNames = @('batch-paper', 'batch-velocity')
$portNames = if ($ManagementFolders) { $fixtureNames + $ignoredPortName + $laterPortName } else { $fixtureNames }
$fixtureOwner = if ($ManagementFolders) { 'windows-v2-isolated-management-folders' } else { 'windows-v2-isolated-batch-import' }
$originalLocalAppData = $env:LOCALAPPDATA
$originalJavaHome = $env:JAVA_HOME
$reservations = [Collections.Generic.List[Net.Sockets.TcpListener]]::new()

function Ih {
    $reply = & $cli $workspace @args
    if ($LASTEXITCODE) { throw "ihctl $($args[0]) exited with code $LASTEXITCODE." }
    ConvertFrom-Json -InputObject ($reply -join [Environment]::NewLine) -NoEnumerate
}

function Invoke-Json([string]$Verb, $Payload) {
    Ih $Verb ($Payload | ConvertTo-Json -Depth 8 -Compress)
}

function Assert-RuleRejected([string]$Excluded, [string]$Reason) {
    $current = Ih scan-policy
    $before = (Get-FileHash -LiteralPath $policy).Hash
    $request = @{
        revision = $current.revision
        allowPaths = @($current.policy.allowPaths)
        excludePaths = @(@($current.policy.excludePaths) + $Excluded)
    } | ConvertTo-Json -Depth 8 -Compress
    $errorText = & $cli $workspace save-scan-policy $request 2>&1 | Out-String
    Check ($LASTEXITCODE -ne 0 -and $errorText.Trim() -match [regex]::Escape($Reason)) "Rejected invalid exclude rule with $Reason"
    Check ((Get-FileHash -LiteralPath $policy).Hash -ceq $before) 'A rejected rule left the saved policy unchanged'
}

function Same-Path([string]$Left, [string]$Right) {
    $Left.Replace('\', '/').TrimEnd('/') -ieq $Right.Replace('\', '/').TrimEnd('/')
}

function Check([bool]$Condition, [string]$Description) {
    if (!$Condition) { throw "Failed: $Description" }
    if (!$checks.Contains($Description)) { $checks.Add($Description) }
}

function Source-Snapshot([switch]$ExcludeLater) {
    @(Get-ChildItem -LiteralPath $source -Recurse -Force | Sort-Object FullName | ForEach-Object {
        $hash = if (!$_.PSIsContainer) { (Get-FileHash -LiteralPath $_.FullName).Hash }
        $_.Refresh()
        $relative = [IO.Path]::GetRelativePath($source, $_.FullName)
        if ($ExcludeLater -and ($relative -eq "BatchFixture\$laterPortName" -or $relative.StartsWith("BatchFixture\$laterPortName\"))) { return }
        [ordered]@{
            path = $relative
            directory = $_.PSIsContainer
            sha256 = $hash
            writeUtc = if (!$_.PSIsContainer) { $_.LastWriteTimeUtc.Ticks }
        }
    }) | ConvertTo-Json -Depth 4 -Compress -AsArray
}

function Wait-Ready([string]$Name, [string]$Run, [int]$Port) {
    $deadline = [DateTime]::UtcNow.AddSeconds(150)
    do {
        $status = Ih status $Name
        if ($status.run.id -cne $Run -or $status.state -notin 'Listening', 'Starting') {
            throw "$Name did not stay on expected run $Run while starting."
        }
        if ($status.state -eq 'Listening' -and $status.consoleReady) {
            $listener = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
            if ($listener.Count -eq 1 -and $listener[0].LocalAddress -eq '127.0.0.1' -and
                $listener[0].OwningProcess -eq $status.run.java.pid) {
                return $status
            }
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "$Name did not reach readiness on its own loopback listener."
}

function Assert-ProcessExited($Identity) {
    $process = Get-Process -Id $Identity.pid -ErrorAction SilentlyContinue
    if ($process -and $process.StartTime.ToFileTimeUtc() -eq $Identity.created) {
        throw "Process $($Identity.pid) has not exited."
    }
}

function Stop-Fixture([string]$Name, [string]$Run, [switch]$Batch) {
    $current = Ih status $Name
    if ($current.run.id -cne $Run) { throw "$Name run changed. No stop command was sent." }
    $stopped = if ($current.state -eq 'Stopped') {
        $current
    } elseif ($Batch) {
        $targets = @(@{ name = $Name; guard = @{ previousRun = $Run; stopToken = $current.stopToken } })
        $payload = ConvertTo-Json -InputObject $targets -Depth 6 -Compress
        $workId = "acceptance-batch-stop-$Name"
        $reply = Ih batch-stop $workId $payload
        Check ($reply.id -ceq $workId) "$Name was accepted by batch stop after leaving the management scope"
        $deadline = [DateTime]::UtcNow.AddSeconds(55)
        do {
            $batches = Ih batch-list
            $progress = $batches | Where-Object id -CEQ $workId
            if ($progress.items[0].phase -ceq 'Completed') { break }
            Start-Sleep -Milliseconds 250
        } while ([DateTime]::UtcNow -lt $deadline)
        Check ($progress.items[0].phase -ceq 'Completed') "$Name completed batch stop on the original run"
        Ih status $Name
    } else {
        Ih stop $Name "acceptance-stop-$Name" $Run
    }
    Check ($stopped.state -eq 'Stopped') "$Name stopped normally"
    Assert-ProcessExited $current.run.host
    Assert-ProcessExited $current.run.java
    Check (!(Get-NetTCPConnection -LocalPort $ports[$Name] -State Listen -ErrorAction SilentlyContinue)) "$Name released its port"
    return $stopped
}

function Stop-FixtureManager {
    $descriptor = Join-Path $workspace 'resident-manager.json'
    if (!(Test-Path -LiteralPath $descriptor -PathType Leaf)) { return }
    $identity = Get-Content -LiteralPath $descriptor -Raw | ConvertFrom-Json
    $process = Get-Process -Id $identity.pid -ErrorAction SilentlyContinue
    if (!$process) { return }
    if ($process.Path -ne $cli -or $process.StartTime.ToFileTimeUtc() -ne $identity.created) {
        throw 'The isolated Manager identity changed. No exit command was sent.'
    }
    $response = Ih manager-exit
    Check ($response.exiting -and $process.WaitForExit(8000)) 'Isolated Manager exited'
}

if (!$resume) {
    try {
        foreach ($name in $portNames) {
            $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
            $listener.Start()
            $reservations.Add($listener)
            $ports[$name] = $listener.LocalEndpoint.Port
        }
    } finally {
        foreach ($listener in $reservations) { $listener.Stop() }
    }
}

$paperPort = $ports['batch-paper']
$proxyPort = $ports['batch-velocity']
$result = [ordered]@{
    result = 'FAIL'; workspace = $workspace; fixture = $null
    native = $null; management = $null; ports = $ports; runIds = $runIds; lifecycle = [ordered]@{}
    checks = $checks; error = $null; cleanupError = $null
}
$activeName = $null
$activeRun = $null
$resumeValidated = !$resume
$manifest = Join-Path $Evidence 'fixture.json'
$sourceManifest = Join-Path $Evidence 'source-before.json'
$null = [IO.Directory]::CreateDirectory($Evidence)
try {
    $env:LOCALAPPDATA = Join-Path $Evidence 'appdata'
    if ($ManagementFolders) { $env:JAVA_HOME = [IO.Directory]::GetParent([IO.Path]::GetDirectoryName($Java)).FullName }
    if ($resume) {
        $fixture = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json -AsHashtable
        if ($fixture.owner -cne $fixtureOwner -or $fixture.workspace -cne $workspace -or
            $fixture.source -cne $source -or $fixture.appData -cne $env:LOCALAPPDATA -or
            $fixture.cli -cne $cli -or $fixture.gui -cne $gui -or $fixture.java -cne $Java -or
            $fixture.scanDirectory -cne $scanDirectory -or $fixture.dependencySource -cne $PaperDependencies) {
            throw 'The prepared fixture identity does not match this invocation.'
        }
        foreach ($asset in $assets.Keys) {
            if ($fixture.assets[$asset].sha256 -cne $assets[$asset].sha256 -or
                $fixture.assets[$asset].path -cne $assets[$asset].path) { throw "The fixture asset changed: $asset." }
        }
        $baseline = (Get-Content -LiteralPath $sourceManifest -Raw).TrimEnd()
        $policyHash = $fixture.policySha256
        foreach ($name in $portNames) { $ports[$name] = [int]$fixture.ports[$name] }
        if ($ports.Values -contains 0 -or @($ports.Values | Sort-Object -Unique).Count -ne $portNames.Count -or
            (Get-NetTCPConnection -LocalPort ([int[]]$ports.Values) -State Listen -ErrorAction SilentlyContinue) -or
            (Get-ChildItem -LiteralPath (Join-Path $workspace 'runs') -Force) -or
            (Source-Snapshot -ExcludeLater:$ResumeRegistered) -cne $baseline) {
            throw 'The existing fixture is not the unchanged and unstarted prepared workspace.'
        }
        if ($ResumeRegistered) {
            $three = Ih list
            $saved = Ih scan-policy
            $prior = Get-Content -LiteralPath (Join-Path $Evidence 'result.json') -Raw | ConvertFrom-Json -AsHashtable
            if (@($three).Count -ne 3 -or @($three | Where-Object { $_.name -cnotin ($fixtureNames + $laterPortName) -or
                    $_.port -ne $ports[$_.name] -or $_.state -cne 'Stopped' -or $_.run -or $_.java }).Count -or
                !$saved.policy.managedFolders -or @($saved.policy.allowPaths).Count -ne 1 -or @($saved.policy.excludePaths).Count -ne 1 -or
                !(Same-Path $saved.policy.allowPaths[0] $scanPath) -or !(Same-Path $saved.policy.excludePaths[0] $excludedDir) -or
                (!$SkipNativeUi -and $prior.native.result -ne 'PASS') -or $prior.cleanupError) {
                throw 'Registered fixture does not match the saved isolated acceptance state.'
            }
            $laterDirectory = Join-Path $scanPath $laterPortName
            if ((Get-FileHash -LiteralPath (Join-Path $laterDirectory 'server.jar')).Hash -cne $assets.velocity.sha256 -or
                (Get-Content -LiteralPath (Join-Path $laterDirectory 'velocity.toml')) -cnotcontains "bind = ""127.0.0.1:$($ports[$laterPortName])""") {
                throw 'Automatically discovered fixture identity changed.'
            }
            foreach ($passed in $prior.checks) { $checks.Add($passed) }
            $result.native = $prior.native
            $result.management = $prior.management
            $result.management.later = $three | Where-Object name -CEQ $laterPortName
            $policyHash = (Get-FileHash -LiteralPath $policy).Hash
            Check $true 'A later server under the whitelist registered automatically and did not start'
            Check (@($three | Where-Object name -CEQ $ignoredPortName).Count -eq 0) 'Background discovery preserved the blacklist'
        } elseif ((Ih list).Count -or (Get-FileHash -LiteralPath $policy).Hash -cne $policyHash) {
            throw 'The prepared fixture policy or registry changed.'
        }
        $savedAttempt = Join-Path $Evidence ('prior-attempt-' + [Guid]::NewGuid().ToString('N'))
        $null = [IO.Directory]::CreateDirectory($savedAttempt)
        foreach ($file in @('fixture.json', 'native-result.json', 'result.json', 'native-failure.png')) {
            $previous = Join-Path $Evidence $file
            if (Test-Path -LiteralPath $previous) { Copy-Item -LiteralPath $previous -Destination $savedAttempt }
        }
        $fixture.cliSha256 = (Get-FileHash -LiteralPath $cli).Hash
        $fixture.guiSha256 = (Get-FileHash -LiteralPath $gui).Hash
        $resumeValidated = $true
        Check $true 'Reused the original unstarted fixture with verified source hashes and policy'
    } else {
        $null = [IO.Directory]::CreateDirectory($env:LOCALAPPDATA)
        $null = Ih init
        $policyHash = (Get-FileHash -LiteralPath $policy).Hash
        $null = [IO.Directory]::CreateDirectory((Join-Path $paperDir 'cache'))
        $null = [IO.Directory]::CreateDirectory((Join-Path $paperDir 'config'))
        $null = [IO.Directory]::CreateDirectory((Join-Path $paperDir 'plugins'))
        $null = [IO.Directory]::CreateDirectory((Join-Path $paperDir 'world'))
        $null = [IO.Directory]::CreateDirectory((Join-Path $velocityDir 'plugins/bStats'))

        Copy-Item -LiteralPath $assets.paper.path -Destination (Join-Path $paperDir 'server.jar')
        Copy-Item -LiteralPath $assets.mojang.path -Destination (Join-Path $paperDir 'cache/mojang_26.3.jar')
        Copy-Item -LiteralPath $assets.velocity.path -Destination (Join-Path $velocityDir 'server.jar')
        foreach ($relative in @('libraries', 'versions')) {
            $dependency = Get-Item -LiteralPath (Join-Path $PaperDependencies $relative)
            if ($dependency.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint) -or
                (Get-ChildItem -LiteralPath $dependency.FullName -Recurse -Force -Attributes ReparsePoint)) {
                throw "Fixture dependencies contain a link at $relative."
            }
            Copy-Item -LiteralPath $dependency.FullName -Destination (Join-Path $paperDir $relative) -Recurse
        }

        'eula=true' | Set-Content -LiteralPath (Join-Path $paperDir 'eula.txt')
        'Isolated world marker' | Set-Content -LiteralPath (Join-Path $paperDir 'world/import-fixture.txt')
        'Isolated plugin marker' | Set-Content -LiteralPath (Join-Path $paperDir 'plugins/import-fixture.txt')
        @(
            'server-ip=127.0.0.1'
            "server-port=$paperPort"
            'online-mode=false'
            'enable-rcon=false'
            'level-type=minecraft:flat'
            'difficulty=peaceful'
            'view-distance=3'
            'simulation-distance=3'
            'motd=InstanceHarbor batch import Paper'
        ) | Set-Content -LiteralPath (Join-Path $paperDir 'server.properties')
        $secret = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
        @('_version: 31', 'proxies:', '  velocity:', '    enabled: true', '    online-mode: true', "    secret: '$secret'") |
            Set-Content -LiteralPath (Join-Path $paperDir 'config/paper-global.yml')
        $secret | Set-Content -LiteralPath (Join-Path $velocityDir 'forwarding.secret') -NoNewline
        @('enabled=false', "serverUuid=$([Guid]::NewGuid())", 'logFailedRequests=false') |
            Set-Content -LiteralPath (Join-Path $velocityDir 'plugins/bStats/config.txt')
        @(
            'config-version = "2.9"'
            "bind = ""127.0.0.1:$proxyPort"""
            'motd = "InstanceHarbor batch import Velocity"'
            'online-mode = true'
            'player-info-forwarding-mode = "modern"'
            'forwarding-secret-file = "forwarding.secret"'
            ''
            '[servers]'
            "paper = ""127.0.0.1:$paperPort"""
            'try = ["paper"]'
            ''
            '[forced-hosts]'
        ) | Set-Content -LiteralPath (Join-Path $velocityDir 'velocity.toml')

        if ($ManagementFolders) {
            $null = [IO.Directory]::CreateDirectory($excludedDir)
            Copy-Item -LiteralPath $assets.velocity.path -Destination (Join-Path $excludedDir 'server.jar')
            @(
                'config-version = "2.9"'
                "bind = ""127.0.0.1:$($ports[$ignoredPortName])"""
            ) | Set-Content -LiteralPath (Join-Path $excludedDir 'velocity.toml')
            $null = [IO.Directory]::CreateDirectory((Join-Path $workspace 'instances/outside-management'))
        }

        $baseline = Source-Snapshot
        Set-Content -LiteralPath $sourceManifest -Value $baseline
        $fixture = [ordered]@{
            owner = $fixtureOwner; workspace = $workspace
            scanDirectory = $scanDirectory; source = $source
            cli = $cli; cliSha256 = (Get-FileHash -LiteralPath $cli).Hash
            gui = $gui; guiSha256 = (Get-FileHash -LiteralPath $gui).Hash
            java = $Java; appData = $env:LOCALAPPDATA
            assets = $assets; dependencySource = $PaperDependencies
            ports = $ports; policySha256 = $policyHash
            expected = @(
                @{ name = 'batch-paper'; directory = $paperDir; kind = 'Paper'; port = $paperPort }
                @{ name = 'batch-velocity'; directory = $velocityDir; kind = 'Velocity'; port = $proxyPort }
            )
        }
        if ($ManagementFolders) {
            $fixture.scanPath = $scanPath
            $fixture.excluded = $excludedDir
        }
    }
    $fixture | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifest
    $result.fixture = $manifest
    if ($PrepareOnly) {
        $result.result = 'PREPARED'
        return [PSCustomObject]$fixture
    }

    if ($ResumeRegistered) {
        $null = Ih manager-connect
    } elseif ($ManagementFolders) {
        $null = Ih manager-connect
        $current = Ih scan-policy
        $json = @{ directory = $scanPath; revision = $current.revision }
        $preview = Invoke-Json management-folder-preview $json
        $ready = @($preview.rows | Where-Object state -CEQ 'Ready')
        Check ($preview.complete -and $ready.Count -eq 3 -and $ready.name -ccontains $ignoredPortName) 'Management folder preview found all three real cores'
        Check ((Ih list).Count -eq 0 -and (Get-FileHash -LiteralPath $policy).Hash -ceq $policyHash -and
            (Source-Snapshot) -ceq $baseline) 'Preview left registry, policy and server files unchanged'
        if ($SkipNativeUi) {
            $json.previewRevision = $preview.previewRevision
            $json.excludedDirectories = @($ready | Where-Object name -CEQ $ignoredPortName | ForEach-Object directory)
            $registration = (Invoke-Json management-folder-save $json).registration
        } else {
            Stop-FixtureManager
            & node (Join-Path $PSScriptRoot 'verify-batch-import-window.mjs') $Evidence
            if ($LASTEXITCODE) { throw 'Native management folder validation failed. See native-result.json.' }
            $result.native = Get-Content -LiteralPath (Join-Path $Evidence 'native-result.json') -Raw | ConvertFrom-Json
            Check ($result.native.result -eq 'PASS') 'Native GUI picker, cancellation, preview, saving and per-row navigation passed'
            $registration = $result.native.registration
        }
        $pendingPolicy = Ih scan-policy
        Check ($pendingPolicy.policy.managedFolders -and @($pendingPolicy.policy.allowPaths).Count -eq 1) 'One whitelist folder was saved'
        $savedExclude = $pendingPolicy.policy.excludePaths[0]
        if ($savedExclude -and ![IO.Path]::IsPathRooted($savedExclude)) { $savedExclude = Join-Path $workspace $savedExclude }
        Check (@($pendingPolicy.policy.excludePaths).Count -eq 1 -and
            (Same-Path $savedExclude $excludedDir)) 'The unchecked server was saved to the blacklist'
        Check (@($registration.rows | Where-Object state -CEQ 'Imported').Count -eq 2) 'Only the two selected servers were registered'
        $result.management = [ordered]@{ preview = $preview; policy = $pendingPolicy }
        Assert-RuleRejected $scanPath 'ScanExclusionOutsideWhitelist'
        Assert-RuleRejected (Join-Path $workspace 'instances/outside-management') 'ScanExclusionOutsideWhitelist'
        Assert-RuleRejected (Join-Path $scanPath 'missing-directory') 'ScanDirectoryUnavailable'
        Stop-FixtureManager
        $null = Ih manager-connect
        Check ((Ih list).Count -eq 2) 'Reopening the isolated Manager retained both registrations without duplicating them'
        $policyHash = (Get-FileHash -LiteralPath $policy).Hash
    } else {
        & node (Join-Path $PSScriptRoot 'verify-batch-import-window.mjs') $Evidence
        if ($LASTEXITCODE) { throw 'Native batch import validation failed. See native-result.json.' }
        $result.native = Get-Content -LiteralPath (Join-Path $Evidence 'native-result.json') -Raw | ConvertFrom-Json
        Check ($result.native.result -eq 'PASS') 'Native GUI preview, cancellation, import and recovery passed'
    }
    $preStart = Source-Snapshot -ExcludeLater:$ResumeRegistered
    Set-Content -LiteralPath (Join-Path $Evidence 'source-before-java.json') -Value $preStart
    Check ($preStart -ceq $baseline) 'Confirmation and reopening preserved source paths, file SHA-256 and write times before Java ran'
    Check ((Get-FileHash -LiteralPath $policy).Hash -ceq $policyHash) 'Scan policy is unchanged before Java runs'

    $registered = Ih list
    Check ($registered.Count -eq ($fixtureNames.Count + [int]$ResumeRegistered.IsPresent)) 'The expected fixture instances are registered'
    foreach ($name in $fixtureNames) {
        $row = @($registered | Where-Object name -CEQ $name)
        Check ($row.Count -eq 1 -and $row[0].port -eq $ports[$name] -and $row[0].state -eq 'Stopped' -and !$row[0].run) "$name is stopped after registration and has never started"
    }
    if ($ManagementFolders -and !$ResumeRegistered) {
        $laterDir = Join-Path $scanPath $laterPortName
        $null = [IO.Directory]::CreateDirectory($laterDir)
        Copy-Item -LiteralPath $assets.velocity.path -Destination (Join-Path $laterDir 'server.jar')
        @(
            'config-version = "2.9"'
            "bind = ""127.0.0.1:$($ports[$laterPortName])"""
        ) | Set-Content -LiteralPath (Join-Path $laterDir 'velocity.toml')
        $deadline = [DateTime]::UtcNow.AddSeconds(60)
        do {
            $listed = Ih list
            $later = @($listed | Where-Object name -CEQ $laterPortName)
            if ($later.Count) { break }
            Start-Sleep -Milliseconds 500
        } while ([DateTime]::UtcNow -lt $deadline)
        Check ($later.Count -eq 1 -and $later[0].state -ceq 'Stopped' -and !$later[0].run) 'A later server under the whitelist registered automatically and did not start'
        Check (@($listed | Where-Object name -CEQ $ignoredPortName).Count -eq 0) 'Background discovery preserved the blacklist'
        $result.management.later = $later[0]
    }
    foreach ($name in $fixtureNames) {
        $activeName = $name
        $launch = Ih start $name "acceptance-start-$name"
        $activeRun = $launch.run.id
        if (!$activeRun) { throw "$name start did not return a run ID." }
        $runIds[$name] = $activeRun
        $ready = Wait-Ready $name $activeRun $ports[$name]
        Check $true "$name reached readiness under its own recorded Java identity"
        if ($ManagementFolders) {
            $process = Get-Process -Id $ready.run.java.pid
            Check ((Same-Path $process.Path $Java) -and $process.StartTime.ToFileTimeUtc() -eq $ready.run.java.created) "$name selected the verified Java installation on startup"
            $current = Ih scan-policy
            $target = if ($name -eq 'batch-paper') { $paperDir } else { $velocityDir }
            $null = Invoke-Json save-scan-policy @{
                revision = $current.revision
                allowPaths = @($current.policy.allowPaths)
                excludePaths = @(@($current.policy.excludePaths) + $target)
            }
            $listed = Ih list
            $visible = @($listed | Where-Object name -CEQ $name)
            Check ($visible.Count -eq 1 -and $visible[0].run.id -ceq $activeRun -and $visible[0].outsideManagement) "$name stayed visible and on the same run after it was excluded"
            $policyHash = (Get-FileHash -LiteralPath $policy).Hash
        }
        $stopped = Stop-Fixture $name $activeRun -Batch:($ManagementFolders -and $name -eq 'batch-velocity')
        if ($ManagementFolders) {
            $listed = Ih list
            Check (@($listed | Where-Object name -CEQ $name).Count -eq 0) "$name left the management list only after graceful stop"
        }
        $result.lifecycle[$name] = @{ ready = $ready; stopped = $stopped }
        $activeName = $null
        $activeRun = $null
    }
    Check ((Get-FileHash -LiteralPath $policy).Hash -ceq $policyHash) 'Scan policy is unchanged after both controlled runs'
    $result.result = 'PASS'
} catch {
    $result.error = $_.Exception.Message
    throw
} finally {
    try {
        if ($resumeValidated) {
            if ($activeName -and !$activeRun) {
                throw "$activeName may have started but its run could not be confirmed. Inspect this isolated workspace."
            }
            if ($activeRun) { $null = Stop-Fixture $activeName $activeRun }
            Stop-FixtureManager
        }
    } catch {
        $result.result = 'FAIL'
        $result.cleanupError = $_.Exception.Message
    } finally {
        $env:LOCALAPPDATA = $originalLocalAppData
        $env:JAVA_HOME = $originalJavaHome
        if ($resumeValidated) { $result | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $Evidence 'result.json') }
    }
    if ($result.cleanupError) { throw $result.cleanupError }
}
[PSCustomObject]$result
