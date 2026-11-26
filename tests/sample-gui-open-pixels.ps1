#Requires -Version 7.0
<#
.SYNOPSIS
Samples the development GUI's client area on every desktop composition (vsync) while the window
opens, and reports samples drawn in the other theme (for example a white page under a dark theme).

.DESCRIPTION
Uses only the given development workspace under %LOCALAPPDATA%/InstanceHarbor Dev/workspaces.
Nothing outside this workspace is stopped.

Two ways to open the window:

- Cold:   the workspace's GUI and Manager are not running (`app-exit` first), and the GUI is
          started directly, as a Start menu shortcut does.
- Reopen: the Manager keeps running, the previous window was closed with the title bar's X
          (SC_CLOSE), and the GUI is opened with the notification-area icon's left click.
          The first run of a series opens once beforehand, so every measured run is a reopen.

While the test window is still hidden, the script puts a magenta backdrop at its place and makes
the window topmost, so other windows cannot show through. Magenta in a sample means the window
showed nothing there yet; Windows' open animation fades the window in over the backdrop, so the
first samples of every run are partly magenta. Magenta is reported but does not fail a run.

The sampler copies the client area from the screen in a tight loop; each copy waits for the
desktop to compose, so it runs about once per composition (vsync). The report gives the mean and
largest interval between samples and the coverage: samples divided by the compositions in the
sampled time (the composition interval comes from DwmGetCompositionTimingInfo). Each copy is
checked at a grid of 24 x 16 points.

Each point that is not magenta is light (luma >= 128) or dark. A sample is "mismatched" when the
share of points in the other theme's brightness (light points under a dark theme, dark points under
a light theme) exceeds that of the final, settled frame by at least -Fraction. The first
mismatched sample of each run is saved as a BMP file. The script exits 1 when any run has a
mismatched sample. "system" expects the Windows app color mode.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Workspace,
    [ValidateSet('dark', 'light', 'system')][string]$Theme = 'dark',
    [ValidateSet('Cold', 'Reopen')][string]$Launch = 'Reopen',
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../src-tauri/target/x86_64-pc-windows-msvc/dev-channel'),
    [ValidateRange(1, 100)][int]$Runs = 10,
    [ValidateRange(200, 10000)][int]$DurationMs = 1500,
    [ValidateRange(0.01, 1.0)][double]$Fraction = 0.25,
    [string]$Output = (Join-Path ([IO.Path]::GetTempPath()) 'ih-open-pixels')
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$devRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'InstanceHarbor Dev'))
$Workspace = [IO.Path]::GetFullPath($Workspace)
if ([IO.Path]::GetDirectoryName($Workspace) -ne (Join-Path $devRoot 'workspaces')) {
    throw "Workspace must be a direct child of $(Join-Path $devRoot 'workspaces')."
}
$gui = (Resolve-Path -LiteralPath (Join-Path $BinaryDirectory 'InstanceHarborDev.exe')).Path
$cli = (Resolve-Path -LiteralPath (Join-Path $BinaryDirectory 'ihctl.exe')).Path
if (!(Test-Path -LiteralPath (Join-Path $Workspace 'workspace.json'))) { & $cli $Workspace init | Out-Null }
New-Item -ItemType Directory -Force -Path $Output | Out-Null

Add-Type -ReferencedAssemblies System.Collections, System.Runtime, System.Runtime.InteropServices, System.Threading, System.Threading.Thread -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class IhPixels {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam, lParam; public uint time; public POINT pt; public uint priv; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WNDCLASSEXW { public uint cbSize, style; public IntPtr lpfnWndProc; public int cbClsExtra, cbWndExtra; public IntPtr hInstance, hIcon, hCursor, hbrBackground; public string lpszMenuName, lpszClassName; public IntPtr hIconSm; }
    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    struct TIMING { public uint cbSize; public ulong rateRefresh, qpcRefreshPeriod; public uint rateCompose_n, rateCompose_d; public ulong qpcVBlank, cRefresh; public uint cDXRefresh; public ulong qpcCompose, cFrame; public uint cDXPresent; public ulong cRefreshFrame, cFrameSubmitted; public uint cDXPresentSubmitted; public ulong cFrameConfirmed; public uint cDXPresentConfirmed; public ulong cRefreshConfirmed; public uint cDXRefreshConfirmed; public ulong cFramesLate; public uint cFramesOutstanding; public ulong cFrameDisplayed, qpcFrameDisplayed, cRefreshFrameDisplayed, cFrameComplete, qpcFrameComplete, cFramePending, qpcFramePending, cFramesDisplayed, cFramesComplete, cFramesPending, cFramesAvailable, cFramesDropped, cFramesMissed, cRefreshNextDisplayed, cRefreshNextPresented, cRefreshesDisplayed, cRefreshesPresented, cRefreshStarted, cPixelsReceived, cPixelsDrawn, cBuffersEmpty; }
    [StructLayout(LayoutKind.Sequential)]
    struct BITMAPINFOHEADER { public uint biSize; public int biWidth, biHeight; public ushort biPlanes, biBitCount; public uint biCompression, biSizeImage; public int biXPelsPerMeter, biYPelsPerMeter; public uint biClrUsed, biClrImportant; }
    delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr hwnd, ref POINT point);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int size);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern ushort RegisterClassExW(ref WNDCLASSEXW wc);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint exStyle, string cls, string title, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
    [DllImport("user32.dll")] static extern int GetMessageW(out MSG msg, IntPtr hwnd, uint min, uint max);
    [DllImport("user32.dll")] static extern IntPtr DispatchMessageW(ref MSG msg);
    [DllImport("user32.dll")] static extern bool PostThreadMessageW(uint thread, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandleW(string name);
    [DllImport("kernel32.dll", CharSet = CharSet.Ansi)] static extern IntPtr GetProcAddress(IntPtr module, string name);
    [DllImport("gdi32.dll")] static extern IntPtr CreateSolidBrush(uint bgr);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateDIBSection(IntPtr dc, ref BITMAPINFOHEADER info, uint usage, out IntPtr bits, IntPtr section, uint offset);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc, IntPtr obj);
    [DllImport("gdi32.dll")] static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, uint rop);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);
    [DllImport("dwmapi.dll")] static extern int DwmGetCompositionTimingInfo(IntPtr hwnd, ref TIMING timing);

    static IntPtr FindWindow(int pid, Func<string, bool> matches, bool visibleOnly) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint owner; GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid) return true;
            var name = new StringBuilder(256); GetClassNameW(hwnd, name, 256);
            if (!matches(name.ToString()) || (visibleOnly && !IsWindowVisible(hwnd))) return true;
            found = hwnd; return false;
        }, IntPtr.Zero);
        return found;
    }

    public static IntPtr MainWindow(int pid, bool visibleOnly) { return FindWindow(pid, name => name.StartsWith("Tauri Window"), visibleOnly); }
    public static IntPtr TrayWindow(int pid) { return FindWindow(pid, name => name == "tray_icon_app", false); }
    // WM_USER_TRAYICON (tray-icon 0.24) with WM_LBUTTONUP: the Manager opens the GUI.
    public static bool ClickTray(IntPtr tray) { return PostMessageW(tray, 6002, IntPtr.Zero, (IntPtr)0x0202); }
    // WM_SYSCOMMAND SC_CLOSE: what the title bar's close button sends.
    public static bool CloseButton(IntPtr window) { return PostMessageW(window, 0x0112, (IntPtr)0xF060, IntPtr.Zero); }

    public static double RefreshMs() {
        var t = new TIMING(); t.cbSize = (uint)Marshal.SizeOf<TIMING>();
        if (DwmGetCompositionTimingInfo(IntPtr.Zero, ref t) != 0 || t.qpcRefreshPeriod == 0) return 0;
        return t.qpcRefreshPeriod * 1000.0 / Stopwatch.Frequency;
    }

    sealed class Backdrop : IDisposable {
        uint thread; readonly ManualResetEventSlim ready = new ManualResetEventSlim();
        public Backdrop(RECT r, uint bgr) {
            var t = new Thread(() => {
                thread = GetCurrentThreadId();
                var cls = "IhPixelsBackdrop";
                var wc = new WNDCLASSEXW { cbSize = (uint)Marshal.SizeOf<WNDCLASSEXW>(), hInstance = GetModuleHandleW(null),
                    lpfnWndProc = GetProcAddress(GetModuleHandleW("user32.dll"), "DefWindowProcW"), hbrBackground = CreateSolidBrush(bgr), lpszClassName = cls };
                RegisterClassExW(ref wc);
                // WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, WS_POPUP | WS_VISIBLE
                IntPtr window = CreateWindowExW(0x8 | 0x80 | 0x08000000, cls, "", 0x80000000 | 0x10000000, r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top, IntPtr.Zero, IntPtr.Zero, wc.hInstance, IntPtr.Zero);
                ready.Set();
                MSG msg;
                while (GetMessageW(out msg, IntPtr.Zero, 0, 0) > 0) DispatchMessageW(ref msg);
                if (window != IntPtr.Zero) DestroyWindow(window);
            });
            t.IsBackground = true; t.Start(); ready.Wait(5000);
        }
        public void Dispose() { PostThreadMessageW(thread, 0x0012 /* WM_QUIT */, IntPtr.Zero, IntPtr.Zero); }
    }

    public class Sample { public double Ms; public double Light, Magenta, Dark; public int CenterRgb; }
    public class Result { public double VisibleAtMs = -1; public List<Sample> Samples = new List<Sample>(); public double MeanIntervalMs, MaxIntervalMs; }

    const int Columns = 24, Rows = 16;

    // Writes the 32-bit top-down pixels as a BMP file.
    static void SaveBmp(string path, IntPtr bits, int w, int h) {
        int size = w * h * 4;
        var data = new byte[54 + size];
        data[0] = (byte)'B'; data[1] = (byte)'M';
        BitConverter.GetBytes(54 + size).CopyTo(data, 2); BitConverter.GetBytes(54).CopyTo(data, 10);
        BitConverter.GetBytes(40).CopyTo(data, 14); BitConverter.GetBytes(w).CopyTo(data, 18); BitConverter.GetBytes(-h).CopyTo(data, 22);
        BitConverter.GetBytes((short)1).CopyTo(data, 26); BitConverter.GetBytes((short)32).CopyTo(data, 28);
        Marshal.Copy(bits, data, 54, size);
        System.IO.File.WriteAllBytes(path, data);
    }

    // Samples from the moment the window becomes visible until durationMs later, once per composition.
    public static Result Run(int pid, int durationMs, int waitMs, Stopwatch clock, bool expectDark, string firstMismatchBmp) {
        var result = new Result();
        Backdrop backdrop = null;
        IntPtr window = IntPtr.Zero;
        try {
            while (clock.ElapsedMilliseconds < waitMs) {
                if (backdrop == null) {
                    IntPtr hidden = MainWindow(pid, false);
                    RECT r;
                    if (hidden != IntPtr.Zero && GetWindowRect(hidden, out r) && r.Right > r.Left) {
                        backdrop = new Backdrop(r, 0xFF00FF);
                        // HWND_TOPMOST without SWP_SHOWWINDOW: stays hidden until the GUI shows it.
                        SetWindowPos(hidden, (IntPtr)(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
                    }
                }
                window = MainWindow(pid, true);
                if (window != IntPtr.Zero) break;
                Thread.Sleep(0);
            }
            if (window == IntPtr.Zero) return result;
            result.VisibleAtMs = clock.Elapsed.TotalMilliseconds;
            RECT c; GetClientRect(window, out c);
            var origin = new POINT(); ClientToScreen(window, ref origin);
            int w = c.Right - c.Left, h = c.Bottom - c.Top;
            IntPtr screen = GetDC(IntPtr.Zero), memory = CreateCompatibleDC(screen);
            var info = new BITMAPINFOHEADER { biSize = (uint)Marshal.SizeOf<BITMAPINFOHEADER>(), biWidth = w, biHeight = -h, biPlanes = 1, biBitCount = 32 };
            IntPtr bits; IntPtr dib = CreateDIBSection(memory, ref info, 0, out bits, IntPtr.Zero, 0);
            IntPtr old = SelectObject(memory, dib);
            var row = new int[w];
            double last = result.VisibleAtMs, sum = 0, max = 0;
            try {
                while (clock.Elapsed.TotalMilliseconds - result.VisibleAtMs < durationMs) {
                    double now = clock.Elapsed.TotalMilliseconds;
                    // A copy from the screen waits for the desktop to compose, so it needs no extra wait.
                    BitBlt(memory, 0, 0, w, h, screen, origin.X, origin.Y, 0x00CC0020 /* SRCCOPY */);
                    var s = new Sample { Ms = now - result.VisibleAtMs };
                    int light = 0, magenta = 0, dark = 0, total = 0;
                    for (int gy = 0; gy < Rows; gy++) {
                        Marshal.Copy(bits + (int)((gy + 0.5) * h / Rows) * w * 4, row, 0, w);
                        for (int gx = 0; gx < Columns; gx++) {
                            int p = row[(int)((gx + 0.5) * w / Columns)];
                            int r = (p >> 16) & 0xFF, g = (p >> 8) & 0xFF, b = p & 0xFF;
                            if (r >= 200 && b >= 200 && g <= 80) magenta++;
                            else if (0.2126 * r + 0.7152 * g + 0.0722 * b < 128) dark++;
                            else light++;
                            total++;
                        }
                    }
                    Marshal.Copy(bits + (h / 2) * w * 4, row, 0, w);
                    s.CenterRgb = row[w / 2] & 0xFFFFFF;
                    s.Light = (double)light / total; s.Magenta = (double)magenta / total; s.Dark = (double)dark / total;
                    result.Samples.Add(s);
                    if (firstMismatchBmp != null && (expectDark ? s.Light : s.Dark) >= 0.5) { SaveBmp(firstMismatchBmp, bits, w, h); firstMismatchBmp = null; }
                    sum += now - last; if (now - last > max) max = now - last; last = now;
                }
            } finally {
                SelectObject(memory, old); DeleteObject(dib); DeleteDC(memory); ReleaseDC(IntPtr.Zero, screen);
            }
            if (result.Samples.Count > 0) { result.MeanIntervalMs = sum / result.Samples.Count; result.MaxIntervalMs = max; }
            return result;
        } finally {
            if (backdrop != null) backdrop.Dispose();
        }
    }
}
'@

function Get-PlainPath([string]$path) { if ($path) { $path.Replace('\\?\', '') } }

function Set-WorkspaceTheme([string]$theme) {
    $preferences = (& $cli $Workspace preferences-snapshot | ConvertFrom-Json -AsHashtable).preferences
    $preferences.theme = $theme
    & $cli $Workspace save-preferences ($preferences | ConvertTo-Json -Compress -Depth 10) | Out-Null
}

function Get-Manager {
    Get-CimInstance Win32_Process -Filter "Name = 'ihctl.exe'" |
        Where-Object { (Get-PlainPath $_.ExecutablePath) -eq $cli -and (Get-PlainPath $_.CommandLine) -like "*$Workspace*manager-serve*" } | Select-Object -First 1
}

# This workspace's GUI processes: started by this script or by this workspace's Manager.
function Get-OwnGui {
    $manager = Get-Manager
    Get-CimInstance Win32_Process -Filter "Name = 'InstanceHarborDev.exe'" |
        Where-Object { (Get-PlainPath $_.ExecutablePath) -eq $gui -and ($_.ParentProcessId -eq $PID -or ($manager -and $_.ParentProcessId -eq $manager.ProcessId)) }
}

function Wait-NoGui {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (Get-OwnGui) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'A GUI for this workspace is still running.' }
        Start-Sleep -Milliseconds 100
    }
}

function Wait-NoManager {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (Get-Manager) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'This workspace''s Manager is still running.' }
        Start-Sleep -Milliseconds 100
    }
}

function Open-ByTray([Diagnostics.Stopwatch]$clock) {
    $until = [DateTime]::UtcNow.AddSeconds(15)
    $manager = $null
    while (!$manager -and [DateTime]::UtcNow -lt $until) { $manager = Get-Manager; if (!$manager) { Start-Sleep -Milliseconds 100 } }
    if (!$manager) { throw 'This workspace has no Manager.' }
    $tray = [IntPtr]::Zero
    while ($tray -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $until) { $tray = [IhPixels]::TrayWindow($manager.ProcessId); if ($tray -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 50 } }
    if ($tray -eq [IntPtr]::Zero) { throw 'The Manager has no tray window.' }
    $clock.Restart()
    if (![IhPixels]::ClickTray($tray)) { throw 'Could not post the tray click.' }
    while ($clock.ElapsedMilliseconds -lt 15000) {
        $child = Get-CimInstance Win32_Process -Filter "ParentProcessId = $($manager.ProcessId) AND Name = 'InstanceHarborDev.exe'"
        if ($child) { return Get-Process -Id @($child)[0].ProcessId }
        Start-Sleep -Milliseconds 2
    }
    throw 'The tray click did not open the GUI.'
}

function Close-ByButton([Diagnostics.Process]$process) {
    $window = [IhPixels]::MainWindow($process.Id, $true)
    if ($window -eq [IntPtr]::Zero -or ![IhPixels]::CloseButton($window)) { throw "Could not close GUI $($process.Id)." }
    if (!$process.WaitForExit(20000)) { throw "GUI $($process.Id) did not exit after its window was closed." }
}

function Wait-Visible([Diagnostics.Process]$process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([IhPixels]::MainWindow($process.Id, $true) -eq [IntPtr]::Zero) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "GUI $($process.Id) never showed its window." }
        Start-Sleep -Milliseconds 50
    }
    Start-Sleep -Milliseconds 1500
}

$expectedDark = switch ($Theme) {
    'dark' { $true }
    'light' { $false }
    default { (Get-ItemPropertyValue -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' -Name AppsUseLightTheme -ErrorAction SilentlyContinue) -eq 0 }
}
Set-WorkspaceTheme $Theme
$refreshMs = [IhPixels]::RefreshMs()
$summary = [Collections.Generic.List[object]]::new()
try {
    if ($Launch -eq 'Reopen') {
        Wait-NoGui
        & $cli $Workspace manager-connect | Out-Null
        $warm = Open-ByTray ([Diagnostics.Stopwatch]::StartNew())
        Wait-Visible $warm
        Close-ByButton $warm
    }
    for ($run = 1; $run -le $Runs; $run++) {
        Wait-NoGui
        $clock = [Diagnostics.Stopwatch]::StartNew()
        if ($Launch -eq 'Cold') {
            if (Get-Manager) { & $cli $Workspace app-exit | Out-Null }
            Wait-NoManager
            Start-Sleep -Milliseconds 500
            $start = [Diagnostics.ProcessStartInfo]::new($gui)
            $start.UseShellExecute = $false
            $start.Environment['IH_WORKSPACE'] = $Workspace
            $clock.Restart()
            $process = [Diagnostics.Process]::Start($start)
        } else {
            $process = Open-ByTray $clock
        }
        $image = Join-Path $Output ('{0}-{1}-run{2}-first-mismatch.bmp' -f $Theme, $Launch.ToLowerInvariant(), $run)
        $result = [IhPixels]::Run($process.Id, $DurationMs, 30000, $clock, $expectedDark, $image)
        try {
            if ($result.Samples.Count -eq 0) { throw "Run ${run}: the GUI window never became visible." }
            $settled = $result.Samples[$result.Samples.Count - 1]
            $other = if ($expectedDark) { 'Light' } else { 'Dark' }
            $mismatched = @($result.Samples | Where-Object { $_.$other - $settled.$other -ge $Fraction })
            $undrawn = @($result.Samples | Where-Object { $_.Magenta - $settled.Magenta -ge $Fraction })
            $compositions = if ($refreshMs -gt 0) { $result.Samples[$result.Samples.Count - 1].Ms / $refreshMs } else { 0 }
            $summary.Add([pscustomobject]@{
                run = $run; theme = $Theme; launch = $Launch
                visibleAtMs = [math]::Round($result.VisibleAtMs); samples = $result.Samples.Count
                coverage = $(if ($compositions -gt 0) { [math]::Round([math]::Min(1.0, $result.Samples.Count / $compositions), 2) } else { $null })
                meanMs = [math]::Round($result.MeanIntervalMs, 1); maxMs = [math]::Round($result.MaxIntervalMs, 1)
                mismatched = $mismatched.Count
                mismatchFirstMs = $(if ($mismatched.Count) { [math]::Round($mismatched[0].Ms) } else { $null })
                mismatchLastMs = $(if ($mismatched.Count) { [math]::Round($mismatched[-1].Ms) } else { $null })
                maxOther = [math]::Round(($result.Samples | Measure-Object $other -Maximum).Maximum, 2)
                settledOther = [math]::Round($settled.$other, 2)
                undrawnLastMs = $(if ($undrawn.Count) { [math]::Round($undrawn[-1].Ms) } else { $null })
            })
            $trace = $result.Samples | Select-Object -First 60 | ForEach-Object { '{0,7:0.0} l{1:0.00} m{2:0.00} d{3:0.00} c{4:x6}' -f $_.Ms, $_.Light, $_.Magenta, $_.Dark, $_.CenterRgb }
            Set-Content -LiteralPath (Join-Path $Output ('{0}-{1}-run{2}.txt' -f $Theme, $Launch.ToLowerInvariant(), $run)) -Value $trace
        } finally {
            if ($Launch -eq 'Reopen') { Close-ByButton $process }
            else {
                & $cli $Workspace app-exit | Out-Null
                if (!$process.WaitForExit(20000)) { throw "GUI $($process.Id) did not exit after app-exit." }
            }
        }
    }
} finally {
    # The Manager may already be ending; a failed request then changes nothing.
    if (Get-Manager) { try { & $cli $Workspace app-exit 2>$null | Out-Null } catch { Write-Verbose "app-exit: $_" } }
}
Write-Host ('Composition interval: {0:0.00} ms' -f $refreshMs)
$summary | Format-Table -AutoSize | Out-String -Width 220 | Write-Host
$summary | ConvertTo-Json -Depth 4
if (@($summary | Where-Object { $_.mismatched -gt 0 }).Count) { exit 1 }
