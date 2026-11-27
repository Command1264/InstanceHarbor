# Acceptance for the Setup's two install scopes (#38), run in Windows Sandbox only.
#
# On the host (PowerShell 7) it builds two Setups from one verified ZIP: the package version
# with packaging/pack-windows-setup.ps1, and the same payload as the next patch version
# (for update, repair and downgrade). It then writes a .wsb into -Evidence, starts Windows
# Sandbox and waits for the result. Nothing is installed on the host.
#
# Inside the Sandbox (Windows PowerShell 5.1, -InSandbox) phase "admin" runs with the
# Sandbox's elevated account (User Account Control is off). It then turns UAC on and restarts
# the Sandbox; phase "uac" runs the elevation cases from a medium-integrity scheduled task.
# Evidence: run.log (PASS/FAIL lines), screenshots and the registry exports in -Evidence\out.
[CmdletBinding()]
param(
    [string]$Compiler,
    [string]$Package,
    [string]$PackageSha256,
    # The controlled test server (tests/controlled-server.rs) built as an .exe.
    [string]$FakeServer,
    # The maintainer's Sandbox fix folder (Install-CjkFonts.ps1): Traditional Chinese fonts
    # and a working WebView2 in the Sandbox. It is mapped read-only to C:\CjkSetup.
    [string]$SandboxFix,
    # A new folder for the candidates, the .wsb and the results.
    [string]$Evidence,
    [int]$TimeoutMinutes = 45,
    # Skips the cases of the elevated account and runs only the UAC phase.
    [switch]$OnlyUac,
    [switch]$InSandbox,
    [ValidateSet('admin', 'uac', 'medium')][string]$Phase = 'admin'
)
$ErrorActionPreference = 'Stop'

if (!$InSandbox) {
    if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'Run the host part with PowerShell 7.' }
    foreach ($name in 'Compiler', 'Package', 'PackageSha256', 'FakeServer', 'Evidence') {
        if (!(Get-Variable -Name $name -ValueOnly)) { throw "-$name is required on the host." }
    }
    $evidencePath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Evidence)
    if (Test-Path -LiteralPath $evidencePath) { throw "-Evidence must be a new folder: $evidencePath" }
    $cjk = $SandboxFix
    if (!$cjk -or !(Test-Path -LiteralPath (Join-Path $cjk 'Install-CjkFonts.ps1'))) { throw '-SandboxFix must be the folder with Install-CjkFonts.ps1 (the Sandbox font and WebView2 fix).' }
    $inputs = Join-Path $evidencePath 'in'
    $results = Join-Path $evidencePath 'out'
    New-Item -ItemType Directory -Path $inputs, $results | Out-Null
    $repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    $packed = & (Join-Path $repo 'packaging\pack-windows-setup.ps1') -Package $Package -ExpectedPackageSha256 $PackageSha256 -Compiler $Compiler -Output (Join-Path $inputs 'setup-a.exe')
    if ($packed.version -notmatch '^(\d+)\.(\d+)\.(\d+)$') { throw "Expected a stable package version, got $($packed.version)." }
    $newer = '{0}.{1}.{2}' -f $Matches[1], $Matches[2], ([int]$Matches[3] + 1)
    $payload = Join-Path (Join-Path $packed.verifiedInput 'releases') $packed.packageSha256
    Push-Location (Join-Path $repo 'packaging')
    try {
        & $Compiler /NOCONFIG /V2 "/DVERSION=$newer" "/DFILE_VERSION=$newer.0" "/DOUTPUT=$(Join-Path $inputs 'setup-b.exe')" "/DPAYLOAD=$payload" windows-setup.nsi | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
    Copy-Item -LiteralPath $FakeServer -Destination (Join-Path $inputs 'java.exe')
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $inputs 'run.ps1')
    Set-Content -LiteralPath (Join-Path $inputs 'versions.txt') -Value @($packed.version, $newer)
    if ($OnlyUac) { Set-Content -LiteralPath (Join-Path $inputs 'only-uac.txt') -Value 'only-uac' }
    $wsb = Join-Path $evidencePath 'setup-scope.wsb'
    $logon = 'cmd.exe /d /c powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File C:\CjkSetup\Install-CjkFonts.ps1 -LogPath C:\Results\cjk-fonts.log &amp;&amp; powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File C:\Inputs\run.ps1 -InSandbox'
    Set-Content -LiteralPath $wsb -Encoding UTF8 -Value @"
<Configuration>
  <Networking>Disable</Networking>
  <MemoryInMB>6144</MemoryInMB>
  <MappedFolders>
    <MappedFolder><HostFolder>$cjk</HostFolder><SandboxFolder>C:\CjkSetup</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$inputs</HostFolder><SandboxFolder>C:\Inputs</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$results</HostFolder><SandboxFolder>C:\Results</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
  </MappedFolders>
  <LogonCommand><Command>$logon</Command></LogonCommand>
</Configuration>
"@
    if (Get-Process -Name WindowsSandboxRemoteSession -ErrorAction SilentlyContinue) { throw 'Another Windows Sandbox is running; close it first.' }
    $launched = Get-Date
    Start-Process -FilePath $wsb
    $deadline = (Get-Date).AddMinutes($TimeoutMinutes)
    $done = Join-Path $results 'DONE'
    while (!(Test-Path -LiteralPath $done) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 10 }
    # The Sandbox shuts itself down after DONE, but its window can stay open. Close only the
    # Sandbox window that this run started.
    $until = (Get-Date).AddSeconds(90)
    while ((Get-Date) -lt $until -and (Get-Process -Name WindowsSandboxRemoteSession -ErrorAction SilentlyContinue)) { Start-Sleep -Seconds 5 }
    Get-Process -Name WindowsSandboxRemoteSession -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -ge $launched } | Stop-Process -Force
    $log = Join-Path $results 'run.log'
    $lines = if (Test-Path -LiteralPath $log) { @(Get-Content -LiteralPath $log) } else { @() }
    $lines | Where-Object { $_ -match ' (PASS|FAIL|ERROR) ' } | Out-Host
    $failed = @($lines | Where-Object { $_ -match ' (FAIL|ERROR) ' }).Count
    $passed = @($lines | Where-Object { $_ -match ' PASS ' }).Count
    if (!(Test-Path -LiteralPath $done)) { throw "The Sandbox did not finish within $TimeoutMinutes minutes. See $log." }
    if ($failed) { throw "$failed check(s) failed, $passed passed. See $log." }
    "Setup install scope acceptance: $passed checks passed. Evidence: $results"
    return
}

# ---------------------------------------------------------------- inside Windows Sandbox
if ($env:USERNAME -ne 'WDAGUtilityAccount') { throw 'Sandbox only.' }
Add-Content -LiteralPath 'C:\Results\started.txt' -Value ("{0:HH:mm:ss} phase={1} args={2}" -f (Get-Date), $Phase, ($args -join ' '))
$in = 'C:\Inputs'
$out = 'C:\Results'
$log = Join-Path $out 'run.log'
$versions = @(Get-Content -LiteralPath (Join-Path $in 'versions.txt'))
$va = $versions[0]
$vb = $versions[1]
$public = 'C:\Users\Public\ih-scope'
$setupA = Join-Path $public 'setup-a.exe'
$setupB = Join-Path $public 'setup-b.exe'
$key = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\InstanceHarbor-Desktop'
$userRoot = Join-Path $env:LOCALAPPDATA 'Programs\InstanceHarbor'
$machineRoot = Join-Path $env:ProgramFiles 'InstanceHarbor'
$ws = Join-Path $env:LOCALAPPDATA 'InstanceHarbor\workspaces\default'
$userShortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'InstanceHarbor.lnk'
$machineShortcut = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'InstanceHarbor.lnk'
$installed = @('InstanceHarbor.exe', 'ihctl.exe', 'conpty.dll', 'OpenConsole.exe', 'WINDOWS-GETTING-STARTED.txt', 'LICENSE', 'THIRD-PARTY-NOTICES.txt', 'MPL-SOURCES.zip', 'instanceharbor.install', 'Uninstall.exe')
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

function Log([string]$m) {
    $line = "{0:HH:mm:ss.fff} [{1}] {2}" -f (Get-Date), $Phase, $m
    # The host may be reading the mapped log at the same moment.
    for ($try = 1; ; $try++) {
        try { Add-Content -LiteralPath $log -Value $line -Encoding UTF8 -ErrorAction Stop; return }
        catch { if ($try -ge 20) { throw }; Start-Sleep -Milliseconds 250 }
    }
}
function Check([bool]$ok, [string]$label) { if ($ok) { Log "PASS $label" } else { Log "FAIL $label" } }
function Shot([string]$name) {
    $b = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [Drawing.Point]::Empty, $b.Size)
    $bmp.Save((Join-Path $out "$name.png"), [Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Log "screenshot $name.png"
}
function Reg([string]$hive) {
    $path = if ($hive -eq 'HKLM') { "HKLM:\$key" } else { "HKCU:\$key" }
    Get-ItemProperty -LiteralPath $path -ErrorAction SilentlyContinue
}
function Setup([string]$exe, [string]$arguments) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = if ($arguments) { Start-Process -FilePath $exe -ArgumentList $arguments -Wait -PassThru } else { Start-Process -FilePath $exe -Wait -PassThru }
    Log ("setup {0} {1} -> exit {2} in {3:N1}s" -f (Split-Path -Leaf $exe), $arguments, $p.ExitCode, $sw.Elapsed.TotalSeconds)
    $p.ExitCode
}
# Like the GUI's automatic update (#65): CreateProcess, not ShellExecute.
function Setup-CreateProcess([string]$exe, [string]$arguments) {
    $info = New-Object Diagnostics.ProcessStartInfo $exe, $arguments
    $info.UseShellExecute = $false
    $p = [Diagnostics.Process]::Start($info)
    $p.WaitForExit()
    Log ("setup (CreateProcess) {0} {1} -> exit {2}" -f (Split-Path -Leaf $exe), $arguments, $p.ExitCode)
    $p.ExitCode
}
# Uninstall.exe copies itself to %TEMP% and returns at once; wait for that copy.
function Uninstall([string]$root, [string]$arguments = '/S') {
    $p = Start-Process -FilePath (Join-Path $root 'Uninstall.exe') -ArgumentList $arguments -Wait -PassThru
    $until = (Get-Date).AddSeconds(90)
    do { Start-Sleep -Milliseconds 500 } while ((Get-Process | Where-Object { $_.Path -like '*\~nsu*.tmp\*' }) -and (Get-Date) -lt $until)
    Log "uninstall $root $arguments -> launcher exit $($p.ExitCode)"
}
function Files([string]$root) { @($installed | Where-Object { Test-Path -LiteralPath (Join-Path $root $_) }) }
function Procs([string]$root) { @(Get-Process | Where-Object { $_.Path -and $_.Path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) }) }
function Hash([string]$root) { (Get-FileHash -LiteralPath (Join-Path $root 'InstanceHarbor.exe')).Hash }
function Ih([string]$root, [string]$workspace, [string[]]$argv) {
    $ErrorActionPreference = 'Continue'
    $r = & (Join-Path $root 'ihctl.exe') $workspace @argv 2>&1
    $text = ($r | Out-String).Trim() -replace '\s+', ' '
    if ($text.Length -gt 200) { $text = $text.Substring(0, 200) }
    Log "ihctl $($argv -join ' ') -> $LASTEXITCODE $text"
    $r
}
function Gui([string]$root) {
    $p = Start-Process -FilePath (Join-Path $root 'InstanceHarbor.exe') -ArgumentList '--stable-default' -PassThru
    Start-Sleep -Seconds 15
    Log "GUI started #$($p.Id); processes: $((Procs $root | ForEach-Object { "$($_.Name)#$($_.Id)" }) -join ' ')"
    $p
}
function Prepare-Fake([string]$root, [string]$workspace, [int]$port) {
    if (!(Test-Path -LiteralPath $workspace)) { Ih $root $workspace @('init') | Out-Null }
    New-Item -ItemType Directory -Path (Join-Path $workspace 'instances\fake') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $workspace 'instances\fake\server.properties') -Value @("server-port=$port", 'server-ip=127.0.0.1')
    New-Item -ItemType File -Path (Join-Path $workspace 'instances\fake\server.jar') -Force | Out-Null
    New-Item -ItemType File -Path (Join-Path $workspace 'instances\fake\fixture-ready') -Force | Out-Null
    @{name = 'fake'; java = (Join-Path $public 'java.exe'); port = $port; removed = $false } | ConvertTo-Json |
        Set-Content -LiteralPath (Join-Path $workspace 'registry\fake.json')
}
function State([string]$root, [string]$workspace) { ((Ih $root $workspace @('status', 'fake')) | Out-String | ConvertFrom-Json).state }
function Start-Fake([string]$root, [string]$workspace) {
    Ih $root $workspace @('start', 'fake', ('start-' + [guid]::NewGuid().ToString('N'))) | Out-Null
    $until = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $until -and (State $root $workspace) -eq 'Stopped') { Start-Sleep -Milliseconds 500 }
}
function Stop-Fake([string]$root, [string]$workspace) {
    $run = ((Ih $root $workspace @('status', 'fake')) | Out-String | ConvertFrom-Json).run.id
    if ($run) { Ih $root $workspace @('stop', 'fake', ('stop-' + [guid]::NewGuid().ToString('N')), $run) | Out-Null }
    $until = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $until -and (State $root $workspace) -ne 'Stopped') { Start-Sleep -Milliseconds 500 }
}
function Owner-Acl([string]$path) { (Get-Acl -LiteralPath $path) }
function Integrity { (whoami /groups | Select-String 'Mandatory Label\\(\w+) Mandatory Level').Matches[0].Groups[1].Value }

Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class SetupUi {
    delegate bool EnumProc(IntPtr window, IntPtr context);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr context);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr context);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr window, StringBuilder text, int length);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr window, StringBuilder text, int length);
    [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    // BM_CLICK is posted: a sent click would wait for any message box that the click opens.
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr w, string l);
    static List<IntPtr> Tops(int pid) {
        var list = new List<IntPtr>();
        EnumWindows((w, c) => { uint owner; GetWindowThreadProcessId(w, out owner); if (owner == pid && IsWindowVisible(w)) list.Add(w); return true; }, IntPtr.Zero);
        return list;
    }
    // Windows that take input: a message box disables the Setup window while it is open.
    static List<IntPtr> Active(int pid) { return Tops(pid).FindAll(w => IsWindowEnabled(w)); }
    // The text of an open message box (its static control 0xFFFF), or "".
    public static string MessageText(int pid) {
        foreach (var top in Tops(pid)) foreach (var c in Children(top))
            if (GetDlgCtrlID(c) == 0xFFFF && Class(c) == "Static") return Text(c).Replace((char)13, (char)32).Replace((char)10, (char)32);
        return "";
    }
    static List<IntPtr> Children(IntPtr top) {
        var list = new List<IntPtr>();
        EnumChildWindows(top, (w, c) => { list.Add(w); return true; }, IntPtr.Zero);
        return list;
    }
    static string Text(IntPtr w) { var b = new StringBuilder(4096); GetWindowText(w, b, b.Capacity); return b.ToString(); }
    static string Class(IntPtr w) { var b = new StringBuilder(256); GetClassName(w, b, b.Capacity); return b.ToString(); }
    // Clicks the enabled control with this ID, in any visible window of the process.
    public static bool ClickId(int pid, int id) {
        foreach (var top in Active(pid)) foreach (var c in Children(top))
            if (GetDlgCtrlID(c) == id && IsWindowVisible(c) && IsWindowEnabled(c) && Class(c) == "Button") { PostMessage(c, 0xF5, IntPtr.Zero, IntPtr.Zero); return true; }
        return false;
    }
    // Clicks the visible button (or radio button) whose text starts with this text.
    public static bool ClickText(int pid, string text) {
        foreach (var top in Active(pid)) foreach (var c in Children(top))
            if (Class(c) == "Button" && IsWindowVisible(c) && Array.Exists(text.Split('|'), t => Text(c).StartsWith(t))) { PostMessage(c, 0xF5, IntPtr.Zero, IntPtr.Zero); return true; }
        return false;
    }
    public static bool SetEdit(int pid, int id, string text) {
        foreach (var top in Tops(pid)) foreach (var c in Children(top))
            if (GetDlgCtrlID(c) == id && Class(c) == "Edit") { SendMessage(c, 0x000C, IntPtr.Zero, text); return true; }
        return false;
    }
    // The texts of every visible window of the process, for the log.
    public static string Texts(int pid) {
        var b = new StringBuilder();
        foreach (var top in Tops(pid)) { b.Append("[" + Text(top) + "]"); foreach (var c in Children(top)) { var t = Text(c); if (t.Length > 0 && IsWindowVisible(c)) b.Append(" | " + t); } }
        return b.ToString().Replace("\r", " ").Replace("\n", " ");
    }
    // WM_GETTEXT: GetWindowText cannot read an edit control of another process.
    public static string EditText(int pid, int id) {
        foreach (var top in Tops(pid)) foreach (var c in Children(top))
            if (GetDlgCtrlID(c) == id && Class(c) == "Edit") { var b = new StringBuilder(1024); SendMessage(c, 0x000D, (IntPtr)b.Capacity, b); return b.ToString(); }
        return "";
    }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr w, StringBuilder l);
    public static bool Enabled(int pid, int id) {
        foreach (var top in Tops(pid)) foreach (var c in Children(top))
            if (GetDlgCtrlID(c) == id && Class(c) == "Button") return IsWindowEnabled(c);
        return false;
    }
}
"@
function Ui($p) { $t = [SetupUi]::Texts($p.Id); Log "ui #$($p.Id): $t"; $t }
function Click($p, [int]$id, [string]$label) { Log "click $label ($id): $([SetupUi]::ClickId($p.Id, $id))"; Start-Sleep -Seconds 2 }
function ClickText($p, [string]$text) { Log "click '$text': $([SetupUi]::ClickText($p.Id, $text))"; Start-Sleep -Seconds 2 }
function Finish($p, [int]$seconds = 20) { if (!$p.WaitForExit($seconds * 1000)) { Log "setup #$($p.Id) did not exit"; Stop-Process -Id $p.Id -Force; return -1 }; $p.ExitCode }

# The standard user who runs InstanceHarbor from the installation for all users.
$stdName = 'ihscope'
$stdSecret = Join-Path $out 'std.clixml'
function Std-Credential { Import-Clixml -LiteralPath $stdSecret }
function As-Std([string]$script, [int]$seconds = 60) {
    $id = [guid]::NewGuid().ToString('N')
    $file = Join-Path $public "std-$id.ps1"
    $outer = Join-Path $public "std-$id-run.ps1"
    $result = Join-Path $public "std-$id.exit"
    Set-Content -LiteralPath $file -Value $script -Encoding UTF8
    # A process started with -Credential reports no exit code; the wrapper writes it.
    Set-Content -LiteralPath $outer -Encoding UTF8 -Value "& powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File '$file'; Set-Content -LiteralPath '$result' -Value `$LASTEXITCODE"
    $p = Start-Process -FilePath powershell.exe -ArgumentList "-NoLogo -NoProfile -ExecutionPolicy Bypass -File `"$outer`"" -Credential (Std-Credential) -WorkingDirectory $public -LoadUserProfile -WindowStyle Hidden -PassThru
    if (!$p.WaitForExit($seconds * 1000)) { Log "std script did not exit"; return -1 }
    if (!(Test-Path -LiteralPath $result)) { Log 'std script wrote no exit code'; return -2 }
    [int](Get-Content -LiteralPath $result | Select-Object -First 1)
}

# Runs a command elevated. In the UAC phase administrators are elevated without a prompt
# until the last case asks for consent again.
function Elevated([string]$command) {
    $file = Join-Path $public ("elevated-" + [guid]::NewGuid().ToString('N') + '.ps1')
    Set-Content -LiteralPath $file -Value $command -Encoding UTF8
    $p = Start-Process -FilePath powershell.exe -ArgumentList "-NoLogo -NoProfile -ExecutionPolicy Bypass -File `"$file`"" -Verb RunAs -WindowStyle Hidden -Wait -PassThru
    Log "elevated command -> $($p.ExitCode)"
}

function Phase-Admin {
    Log "integrity $(Integrity); EnableLUA $((Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System').EnableLUA); versions $va $vb"
    New-Item -ItemType Directory -Path $public -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $in 'setup-a.exe'), (Join-Path $in 'setup-b.exe'), (Join-Path $in 'java.exe') -Destination $public
    & icacls.exe $public /grant '*S-1-5-32-545:(OI)(CI)M' /Q | Out-Null
    if (Test-Path -LiteralPath (Join-Path $in 'only-uac.txt')) { Log 'only the UAC phase (-OnlyUac)'; Enable-Uac; return }

    # --- Only for the current user (default) -------------------------------------------
    Check ((Setup $setupA '/S') -eq 0) 'Silent install without arguments ends with 0'
    $r = Reg HKCU
    Check ($r.InstallLocation -eq $userRoot -and $r.DisplayVersion -eq $va -and !(Reg HKLM)) 'Default scope is the current user: HKCU, %LOCALAPPDATA%\Programs\InstanceHarbor, no HKLM key'
    Check ((Files $userRoot).Count -eq $installed.Count -and (Test-Path -LiteralPath $userShortcut) -and !(Test-Path -LiteralPath $machineShortcut)) 'Per-user files and the per-user Start menu shortcut'
    Check ((Setup $setupA '/S') -eq 0 -and (Reg HKCU).DisplayVersion -eq $va) 'Per-user repair keeps the version'
    Check ((Setup $setupB '/S') -eq 0 -and (Reg HKCU).DisplayVersion -eq $vb -and (Reg HKCU).InstallLocation -eq $userRoot) 'Per-user update keeps the scope and folder'
    Check ((Setup $setupA '/S') -eq 4 -and (Reg HKCU).DisplayVersion -eq $vb) 'Per-user silent downgrade is refused by default (4)'
    $p = Start-Process -FilePath $setupA -PassThru
    Start-Sleep -Seconds 4
    $t = Ui $p
    Shot 'downgrade-prompt'
    Click $p 7 'No (downgrade)'
    Check ($t -match "newer InstanceHarbor $([regex]::Escape($vb)) is installed|已安裝較新的 InstanceHarbor $([regex]::Escape($vb))" -and (Finish $p) -eq 4 -and (Reg HKCU).DisplayVersion -eq $vb) 'Interactive downgrade asks first; No keeps the newer version (4)'
    Check ((Setup $setupB '/S /AllUsers') -eq 2 -and !(Reg HKLM) -and !(Test-Path -LiteralPath $machineRoot)) 'With a per-user installation, /AllUsers is refused (2)'
    Check ((Setup $setupB '/S /CurrentUser /D=C:\Elsewhere\InstanceHarbor') -eq 2 -and !(Test-Path -LiteralPath 'C:\Elsewhere')) 'An update cannot move the folder (2)'
    Check ((Setup $setupB '/S /CurrentUser') -eq 0 -and (Reg HKCU).DisplayVersion -eq $vb) 'An update with the registered scope given still works'
    # Interactive: the registered scope and folder are not asked again.
    $p = Start-Process -FilePath $setupB -PassThru
    Start-Sleep -Seconds 4
    $t = Ui $p
    Click $p 6 'Yes (repair)'
    Click $p 1 'Next (welcome)'
    Start-Sleep -Seconds 3
    $t = Ui $p
    Shot 'current-repair-no-scope-page'
    Check ($t -notmatch 'Choose who can use|選擇安裝範圍' -and $t -notmatch 'following folder|安裝到下列資料夾') 'Interactive repair skips the scope and folder pages'
    Click $p 1 'Finish'
    Check ((Finish $p) -eq 0) 'Interactive repair finished'
    # Running GUI, then a running server.
    $gui = Gui $userRoot
    Check ((Setup $setupB '/S') -eq 0 -and $gui.HasExited -and (Procs $userRoot).Count -eq 0) 'Per-user silent repair closes the GUI and Manager'
    Prepare-Fake $userRoot $ws 25599
    Start-Fake $userRoot $ws
    $before = Hash $userRoot
    Check ((Setup $setupB '/S') -eq 5 -and (Hash $userRoot) -eq $before) 'Per-user silent repair with a running server exits 5 and changes nothing'
    Stop-Fake $userRoot $ws
    New-Item -ItemType Directory -Path (Join-Path $ws 'keep-me') -Force | Out-Null
    Uninstall $userRoot
    Check (!(Reg HKCU) -and !(Test-Path -LiteralPath $userRoot) -and !(Test-Path -LiteralPath $userShortcut) -and (Test-Path -LiteralPath (Join-Path $ws 'keep-me'))) 'Per-user uninstall removes files, shortcut and key; the workspace stays'

    # --- Per-user folders ----------------------------------------------------------------
    Check ((Setup $setupA '/S /AllUsers /CurrentUser') -eq 2) '/AllUsers with /CurrentUser is refused (2)'
    Check ((Setup $setupA "/S /CurrentUser /D=$env:ProgramFiles\IH-scope") -eq 2 -and !(Test-Path -LiteralPath "$env:ProgramFiles\IH-scope")) 'Per-user into Program Files is refused (2), even when Setup has administrator rights'
    New-Item -ItemType Directory -Path 'C:\IH-used' -Force | Out-Null
    Set-Content -LiteralPath 'C:\IH-used\notes.txt' -Value 'not InstanceHarbor'
    Check ((Setup $setupA '/S /D=C:\IH-used') -eq 2 -and @(Get-ChildItem -LiteralPath 'C:\IH-used').Count -eq 1) 'A folder with other files is refused (2) and left as it was'
    # NSIS itself drops a /D= that is not a full path, so Setup uses the default folder.
    $code = Setup $setupA '/S /D=IH-relative'
    Check ($code -eq 0 -and (Reg HKCU).InstallLocation -eq $userRoot) 'A relative /D= is ignored by NSIS and the default folder is used'
    Uninstall $userRoot
    $custom = 'C:\IH-user\Apps\InstanceHarbor'
    Check ((Setup $setupA "/S /CurrentUser /D=$custom") -eq 0 -and (Reg HKCU).InstallLocation -eq $custom -and (Files $custom).Count -eq $installed.Count) 'Per-user install into a chosen folder (/D=)'
    Check ((Setup $setupB '/S') -eq 0 -and (Reg HKCU).InstallLocation -eq $custom -and (Reg HKCU).DisplayVersion -eq $vb) 'Update of a chosen folder keeps it'
    Set-Content -LiteralPath (Join-Path $custom 'user-notes.txt') -Value 'added after installation'
    Uninstall $custom
    Check (!(Reg HKCU) -and (Files $custom).Count -eq 0 -and (Test-Path -LiteralPath (Join-Path $custom 'user-notes.txt'))) 'Uninstall removes only what Setup installed (a later file stays)'

    # --- For all users ---------------------------------------------------------------------
    Check ((Setup $setupA '/S /AllUsers') -eq 0) 'Silent install for all users ends with 0'
    $r = Reg HKLM
    Check ($r.InstallLocation -eq $machineRoot -and $r.DisplayVersion -eq $va -and !(Reg HKCU)) 'For all users: HKLM, Program Files\InstanceHarbor, no HKCU key'
    Check ((Files $machineRoot).Count -eq $installed.Count -and (Test-Path -LiteralPath $machineShortcut) -and !(Test-Path -LiteralPath $userShortcut)) 'Files and the Start menu shortcut for all users'
    Check (!(Owner-Acl $machineRoot).AreAccessRulesProtected) 'Program Files keeps its inherited permissions'
    Check ((Setup $setupA '/S /CurrentUser') -eq 2 -and !(Reg HKCU) -and !(Test-Path -LiteralPath $userRoot)) 'With an installation for all users, /CurrentUser is refused (2)'
    Check ((Setup $setupA '/S') -eq 0 -and (Reg HKLM).DisplayVersion -eq $va -and !(Reg HKCU)) 'Repair without arguments keeps the scope for all users'
    Check ((Setup $setupA "/S /D=$machineRoot") -eq 0 -and (Reg HKLM).InstallLocation -eq $machineRoot) 'Repair with /D= naming the registered folder works'

    # Other users: a standard user runs a server from the installation for all users.
    $secret = -join ((1..24) | ForEach-Object { [char](Get-Random -Minimum 65 -Maximum 90) }) + 'a1!'
    $secure = ConvertTo-SecureString $secret -AsPlainText -Force
    New-LocalUser -Name $stdName -Password $secure -PasswordNeverExpires -AccountNeverExpires | Out-Null
    New-Object Management.Automation.PSCredential ".\$stdName", $secure | Export-Clixml -LiteralPath $stdSecret
    $stdOut = Join-Path $public 'std-out.txt'
    $code = As-Std @"
`$ErrorActionPreference = 'Continue'
`$ws = Join-Path `$env:LOCALAPPDATA 'InstanceHarbor\workspaces\default'
`$ih = '$machineRoot\ihctl.exe'
`$v = & `$ih --version 2>&1
New-Item -ItemType Directory -Path (Split-Path -Parent `$ws) -Force | Out-Null
`$i =& `$ih `$ws init 2>&1
New-Item -ItemType Directory -Path (Join-Path `$ws 'instances\fake') -Force | Out-Null
Set-Content -LiteralPath (Join-Path `$ws 'instances\fake\server.properties') -Value @('server-port=25598', 'server-ip=127.0.0.1')
New-Item -ItemType File -Path (Join-Path `$ws 'instances\fake\server.jar'), (Join-Path `$ws 'instances\fake\fixture-ready') -Force | Out-Null
@{name='fake'; java='$public\java.exe'; port=25598; removed=`$false} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path `$ws 'registry\fake.json')
`$s = & `$ih `$ws start fake ('start-' + [guid]::NewGuid().ToString('N')) 2>&1
`$w = try { Set-Content -LiteralPath '$machineRoot\std-write-test.txt' -Value x -ErrorAction Stop; 'written' } catch { 'denied' }
Set-Content -LiteralPath '$stdOut' -Value @("version=`$v", "workspace=`$ws", "init=`$((`$i | Out-String).Trim() -replace '\s+', ' ')","start=`$((`$s | Out-String).Trim() -replace '\s+', ' ')", "write=`$w")
"@ 90
    $std = Get-Content -LiteralPath $stdOut -ErrorAction SilentlyContinue
    Log "standard user: exit $code; $($std -join ' || ')"
    Check (($std -join ' ') -match 'workspace=C:\\Users\\ihscope\\AppData\\Local\\InstanceHarbor\\workspaces\\default' -and ($std -join ' ') -match 'write=denied') 'Another user runs the shared installation with a workspace in their own %LOCALAPPDATA% and cannot write the program folder'
    $stdHost = @(Procs $machineRoot | Where-Object { $_.Name -eq 'ihctl' })
    Log "processes from the shared installation: $((Procs $machineRoot | ForEach-Object { "$($_.Name)#$($_.Id)" }) -join ' ')"
    $before = Hash $machineRoot
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $code = Setup $setupB '/S'
    Check ($stdHost.Count -ge 1 -and $code -eq 2 -and (Hash $machineRoot) -eq $before -and (Reg HKLM).DisplayVersion -eq $va) "Silent update waits while another user's server holds the files, then exits 2 without changes ($([int]$sw.Elapsed.TotalSeconds) s)"
    $p = Start-Process -FilePath $setupB -PassThru
    Start-Sleep -Seconds 4
    Click $p 6 'Yes (update)'
    Click $p 1 'Next (welcome)'
    Start-Sleep -Seconds 6
    $t = Ui $p
    Shot 'all-users-wait-other-user'
    Check ($t -match 'other users have InstanceHarbor or its servers open|其他使用者開著 InstanceHarbor') 'The wait page explains that other users keep the files in use'
    Click $p 2 'Cancel'
    Check ((Finish $p) -eq 4 -and (Hash $machineRoot) -eq $before) 'Cancel on the wait page exits 4 and changes nothing'
    $code = As-Std @"
`$ws = Join-Path `$env:LOCALAPPDATA 'InstanceHarbor\workspaces\default'
`$ih = '$machineRoot\ihctl.exe'
`$run = ((& `$ih `$ws status fake) | Out-String | ConvertFrom-Json).run.id
& `$ih `$ws stop fake ('stop-' + [guid]::NewGuid().ToString('N')) `$run | Out-Null
Start-Sleep -Seconds 3
& `$ih `$ws manager-exit | Out-Null
"@ 60
    $until = (Get-Date).AddSeconds(30)
    while ((Procs $machineRoot).Count -and (Get-Date) -lt $until) { Start-Sleep -Milliseconds 500 }
    Log "after the other user stopped: $((Procs $machineRoot | ForEach-Object { "$($_.Name)#$($_.Id)" }) -join ' ')"
    Check ((Setup $setupB '/S') -eq 0 -and (Reg HKLM).DisplayVersion -eq $vb -and (Reg HKLM).InstallLocation -eq $machineRoot) 'Update for all users once the other user closed it'
    Check ((Setup $setupA '/S') -eq 4 -and (Reg HKLM).DisplayVersion -eq $vb) 'Silent downgrade for all users is refused by default (4)'
    # Own GUI and server with the installation for all users.
    Prepare-Fake $machineRoot $ws 25599
    Start-Fake $machineRoot $ws
    $gui = Gui $machineRoot
    $p = Start-Process -FilePath $setupB -PassThru
    Start-Sleep -Seconds 4
    Click $p 6 'Yes (repair)'
    Click $p 1 'Next (welcome)'
    Start-Sleep -Seconds 6
    $t = Ui $p
    Shot 'all-users-wait-servers'
    Check ($t -match '1 server\(s\) are not stopped|還有 1 台伺服器沒有停止' -and $t -match 'installed for all users|為這台電腦的所有使用者安裝的') 'All-users wait page counts the own servers and adds the note for other users'
    Stop-Fake $machineRoot $ws
    $until = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $until -and !$p.HasExited -and ![SetupUi]::Enabled($p.Id, 1)) { Start-Sleep -Milliseconds 500 }
    Start-Sleep -Seconds 8
    Click $p 1 'Finish'
    Check ((Finish $p 30) -eq 0 -and $gui.HasExited) 'Interactive repair for all users continues after the server stopped'
    Uninstall $machineRoot
    Check (!(Reg HKLM) -and !(Test-Path -LiteralPath $machineRoot) -and !(Test-Path -LiteralPath $machineShortcut) -and (Test-Path -LiteralPath (Join-Path $ws 'keep-me'))) 'Uninstall for all users removes files, shortcut and HKLM key; the workspace stays'

    # --- All-users folders ---------------------------------------------------------------
    Check ((Setup $setupA '/S /AllUsers /D=C:\Users\Public\IH-shared') -eq 2 -and !(Test-Path -LiteralPath 'C:\Users\Public\IH-shared')) 'For all users, a folder inside the Users folder is refused (2)'
    $customAll = 'C:\IH-all\InstanceHarbor'
    Check ((Setup $setupA "/S /AllUsers /D=$customAll") -eq 0 -and (Reg HKLM).InstallLocation -eq $customAll) 'Install for all users into a chosen folder (/D=)'
    $acl = Owner-Acl $customAll
    $sidRules = @($acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
    $rules = @($sidRules | ForEach-Object { "$($_.IdentityReference.Value)=$($_.FileSystemRights)" })
    Log "ACL of $customAll owner=$($acl.Owner) protected=$($acl.AreAccessRulesProtected) rules=$($rules -join ', ')"
    $usersWrite = @($sidRules | Where-Object { $_.IdentityReference.Value -in 'S-1-5-32-545', 'S-1-5-11', 'S-1-1-0' -and ($_.FileSystemRights -band 0x116) })
    Check ($acl.AreAccessRulesProtected -and $acl.Owner -match 'Administrators' -and $usersWrite.Count -eq 0) 'A chosen folder for all users is protected like Program Files (owner Administrators, users read only)'
    $code = As-Std "try { Set-Content -LiteralPath '$customAll\std.txt' -Value x -ErrorAction Stop; exit 1 } catch { exit 0 }" 30
    Check ($code -eq 0 -and !(Test-Path -LiteralPath "$customAll\std.txt")) 'A standard user cannot write the chosen folder for all users'
    Uninstall $customAll
    Check (!(Reg HKLM) -and !(Test-Path -LiteralPath $customAll)) 'Uninstall of the chosen folder for all users'

    # --- Interactive pages ---------------------------------------------------------------
    $p = Start-Process -FilePath $setupA -PassThru
    Start-Sleep -Seconds 4
    Click $p 1 'Next (welcome)'
    $t = Ui $p
    Shot 'scope-page'
    Check ($t -match 'Only for me|只為目前使用者安裝' -and $t -match 'For all users of this computer|為這台電腦的所有使用者安裝') 'The scope page offers both scopes'
    ClickText $p 'For all users|為這台電腦的所有使用者安裝'
    Click $p 1 'Next (scope)'
    $t = Ui $p
    Shot 'folder-page-all-users'
    $folder = [SetupUi]::EditText($p.Id, 1019); Log "folder field: $folder"
    Check ($folder -eq $machineRoot) 'The folder page proposes Program Files for all users'
    Log "set folder: $([SetupUi]::SetEdit($p.Id, 1019, 'C:\IH-used'))"
    Click $p 1 'Next (folder)'
    $t = Ui $p
    Shot 'folder-refused-not-empty'
    $m = [SetupUi]::MessageText($p.Id); Log "message: $m"
    Check ($m -match 'already contains other files|已有其他檔案') 'The folder page refuses a folder with other files'
    Click $p 2 'OK (a message box with one button uses ID 2)'
    Click $p 3 'Back'
    ClickText $p 'Only for me|只為目前使用者安裝'
    Click $p 1 'Next (scope)'
    $t = Ui $p
    $folder = [SetupUi]::EditText($p.Id, 1019); Log "folder field: $folder"
    Check ($folder -eq $userRoot) 'Choosing only for me proposes %LOCALAPPDATA%\Programs\InstanceHarbor'
    Log "set folder: $([SetupUi]::SetEdit($p.Id, 1019, "$env:ProgramFiles\IH-scope"))"
    Click $p 1 'Next (folder)'
    $t = Ui $p
    Shot 'folder-refused-protected'
    $m = [SetupUi]::MessageText($p.Id); Log "message: $m"
    Check ($m -match 'This folder needs administrator rights|這個資料夾需要系統管理員權限') 'Only for me refuses a folder that needs administrator rights'
    Click $p 2 'OK (a message box with one button uses ID 2)'
    Click $p 2 'Cancel'
    Check ((Finish $p) -eq 4 -and !(Reg HKCU) -and !(Reg HKLM)) 'Cancel on the folder page installs nothing'
    Enable-Uac
}

# Turns UAC on for the next phase and restarts the Sandbox.
function Enable-Uac {
    $policy = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
    Set-ItemProperty -LiteralPath $policy -Name EnableLUA -Value 1
    # Administrators are elevated without a prompt; standard users are always refused.
    Set-ItemProperty -LiteralPath $policy -Name ConsentPromptBehaviorAdmin -Value 0
    Set-ItemProperty -LiteralPath $policy -Name ConsentPromptBehaviorUser -Value 0
    Set-ItemProperty -LiteralPath $policy -Name PromptOnSecureDesktop -Value 0
    Set-Content -LiteralPath (Join-Path $out 'phase.txt') -Value 'uac'
    Log 'UAC turned on; restarting'
    & shutdown.exe /r /t 3
}

function Phase-Uac {
    # The LogonCommand may run elevated; the cases run as the signed-in user at medium
    # integrity, like a double-click in Explorer.
    $level = Integrity
    Log "logon command integrity $level; EnableLUA $((Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System').EnableLUA)"
    if ($level -eq 'Medium') {
        $script:Phase = 'medium'
        try { Phase-Medium } finally { $script:Phase = 'uac' }
        return
    }
    $marker = Join-Path $out 'medium-done.txt'
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoLogo -NoProfile -ExecutionPolicy Bypass -File C:\Inputs\run.ps1 -InSandbox -Phase medium"
    $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
    Register-ScheduledTask -TaskName 'ih-scope-medium' -Action $action -Principal $principal -Force | Out-Null
    Start-ScheduledTask -TaskName 'ih-scope-medium'
    $until = (Get-Date).AddMinutes(20)
    while (!(Test-Path -LiteralPath $marker) -and (Get-Date) -lt $until) { Start-Sleep -Seconds 5 }
    if (!(Test-Path -LiteralPath $marker)) { Log 'ERROR the medium-integrity phase did not finish' }
}

function Phase-Medium {
    $policy = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
    $level = Integrity
    Log "integrity $level"
    Check ($level -eq 'Medium') 'The UAC phase runs without administrator rights (medium integrity)'
    Check ((Setup $setupA '/S') -eq 0 -and (Reg HKCU).InstallLocation -eq $userRoot) 'Per-user install needs no elevation'
    Uninstall $userRoot
    Check (!(Reg HKCU) -and !(Test-Path -LiteralPath $userRoot)) 'Per-user uninstall needs no elevation'
    Check ((Setup $setupA '/S /AllUsers') -eq 0 -and (Reg HKLM).InstallLocation -eq $machineRoot -and !(Reg HKCU)) 'Install for all users from medium integrity elevates through UAC'
    # The GUI's automatic update starts Setup with CreateProcess and no arguments.
    Check ((Setup-CreateProcess $setupB '/S') -eq 0 -and (Reg HKLM).DisplayVersion -eq $vb) 'Update for all users started with CreateProcess elevates and keeps the scope'
    Check ((Setup $setupB '/S /CurrentUser') -eq 2 -and !(Reg HKCU)) 'The other scope is refused before elevation (2)'
    Uninstall $machineRoot
    Check (!(Reg HKLM) -and !(Test-Path -LiteralPath $machineRoot)) 'Uninstall for all users from medium integrity elevates through UAC'

    # Interactive: "for all users" restarts Setup elevated, which continues at the folder page.
    $p = Start-Process -FilePath $setupA -PassThru
    Start-Sleep -Seconds 4
    Click $p 1 'Next (welcome)'
    ClickText $p 'For all users|為這台電腦的所有使用者安裝'
    Start-Sleep -Seconds 1
    Shot 'scope-page-shield'
    Click $p 1 'Next (scope)'
    Start-Sleep -Seconds 6
    Shot 'elevated-folder-page'
    $copies = Join-Path $out 'elevated-setup.txt'
    Elevated "Get-CimInstance Win32_Process -Filter `"Name='setup-a.exe'`" | ForEach-Object { '{0} {1}' -f `$_.ProcessId, `$_.CommandLine } | Set-Content -LiteralPath '$copies'"
    $lines = @(Get-Content -LiteralPath $copies -ErrorAction SilentlyContinue)
    Log "setup copies: $($lines -join ' || ')"
    $child = @($lines | Where-Object { $_ -match '/AllUsers /Elevated /Continue' })
    Check ($child.Count -eq 1 -and ([SetupUi]::Texts($p.Id)) -eq '' -and !$p.HasExited) 'The first copy hides and waits while the elevated copy continues (/AllUsers /Elevated /Continue)'
    if ($child.Count -eq 1) { Elevated "Stop-Process -Id $(($child[0] -split ' ')[0]) -Force" }
    [void]$p.WaitForExit(15000)
    Log "first copy after the elevated copy ended: exited=$($p.HasExited) code=$(if ($p.HasExited) { $p.ExitCode })"
    Check ($p.HasExited -and !(Reg HKLM)) 'The first copy ends with the elevated copy'

    # The prompt itself, and a refused prompt. An elevated helper (started while
    # administrators are still elevated without a prompt) turns the prompt on, and ends each
    # prompt after a few seconds as if the user had not approved it.
    $helperLog = Join-Path $out 'consent-helper.txt'
    $helper = Join-Path $public 'consent-helper.ps1'
    Set-Content -LiteralPath $helper -Encoding UTF8 -Value @"
Set-ItemProperty -LiteralPath '$policy' -Name ConsentPromptBehaviorAdmin -Value 5
Add-Content -LiteralPath '$helperLog' -Value "prompt on `$(Get-Date -Format HH:mm:ss)"
`$until = (Get-Date).AddMinutes(4); `$ended = 0
while ((Get-Date) -lt `$until -and `$ended -lt 2) {
    `$c = @(Get-Process -Name consent -ErrorAction SilentlyContinue)
    if (`$c.Count) {
        Start-Sleep -Seconds 8
        `$c | Stop-Process -Force
        `$ended++
        Add-Content -LiteralPath '$helperLog' -Value "ended prompt `$ended at `$(Get-Date -Format HH:mm:ss)"
    }
    Start-Sleep -Milliseconds 500
}
"@
    Start-Process -FilePath powershell.exe -ArgumentList "-NoLogo -NoProfile -ExecutionPolicy Bypass -File `"$helper`"" -Verb RunAs -WindowStyle Hidden
    $until = (Get-Date).AddSeconds(30)
    while (!(Test-Path -LiteralPath $helperLog) -and (Get-Date) -lt $until) { Start-Sleep -Milliseconds 500 }
    $p = Start-Process -FilePath $setupA -PassThru
    Start-Sleep -Seconds 4
    Click $p 1 'Next (welcome)'
    ClickText $p 'For all users|為這台電腦的所有使用者安裝'
    Click $p 1 'Next (scope)'
    Start-Sleep -Seconds 3
    $consent = @(Get-Process -Name consent -ErrorAction SilentlyContinue)
    Shot 'uac-consent-prompt'
    Check ($consent.Count -ge 1 -and !$p.HasExited) "Choosing all users without rights shows the UAC prompt (consent.exe running: $($consent.Count))"
    $until = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $until -and ![SetupUi]::MessageText($p.Id)) { Start-Sleep -Milliseconds 500 }
    $m = [SetupUi]::MessageText($p.Id)
    Log "message after the refused prompt: $m"
    Shot 'uac-refused-message'
    Check ($m -match 'needs administrator approval, which was not given|需要系統管理員確認，但沒有取得確認|could not start with administrator rights|無法以系統管理員權限啟動') 'A refused prompt is explained and Setup stays open'
    Click $p 2 'OK (a message box with one button uses ID 2)'
    $t = Ui $p
    Click $p 2 'Cancel'
    Check ((Finish $p) -eq 4 -and !(Reg HKLM)) 'After a refused prompt, Cancel exits 4 and nothing is installed'
    $code = Setup $setupA '/S /AllUsers'
    Log "helper: $((Get-Content -LiteralPath $helperLog -ErrorAction SilentlyContinue) -join ' || ')"
    Check (($code -eq 4 -or $code -eq 2) -and !(Reg HKLM) -and !(Test-Path -LiteralPath $machineRoot)) "Silent install for all users with a refused prompt changes nothing (exit $code)"
}

$finished = $false
try {
    if ($Phase -ne 'medium' -and (Test-Path -LiteralPath (Join-Path $out 'phase.txt'))) { $Phase = 'uac' }
    switch ($Phase) {
        'admin' { Phase-Admin }
        'uac' { Phase-Uac; $finished = $true }
        'medium' { Phase-Medium }
    }
} catch {
    Log "ERROR $($_ | Out-String)"
    $finished = $Phase -ne 'medium'
} finally {
    if ($Phase -eq 'medium') { Set-Content -LiteralPath (Join-Path $out 'medium-done.txt') -Value 'done' }
    if ($finished) {
        Log 'DONE'
        Set-Content -LiteralPath (Join-Path $out 'DONE') -Value 'done'
        & shutdown.exe /s /t 20
    }
}
