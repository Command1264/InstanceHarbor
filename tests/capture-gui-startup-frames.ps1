#Requires -Version 7.0
<#
.SYNOPSIS
Captures the screen under the development GUI's window every few milliseconds while it opens and
counts frames whose colors do not match the theme.

.DESCRIPTION
Uses only the given development workspace under %LOCALAPPDATA%/InstanceHarbor Dev/workspaces.
Each run sets the workspace's theme, opens the GUI (directly, or the way the tray icon's left click
does), copies the screen area of the main window's client area from the moment the window becomes
visible, and then asks the workspace to exit with `app-exit` (GUI and Manager). Nothing else is
stopped.

Other windows (including other workspaces' GUIs, which open at the same centered place) would
cover the test window or show through its fade-in animation. So while the test window is still
hidden, the script puts a plain backdrop in the expected theme's background color at the same
place, and makes the test window and the backdrop topmost. Windows' open animation stays on,
because that is when an old frame of the page can show. Both are removed when the run ends.

A frame counts as mismatched when more than -MismatchFraction of its sampled pixels are light
(luma >= 128) under a dark theme, or dark (luma < 128) under a light theme. Mismatched frames are
saved as PNG files; -SaveFirst also saves the first frames of each run.

-Launch Tray posts the notification-area icon's left-click message to this workspace's own
Manager window (`tray_icon_app`), so the Manager opens the GUI exactly as a real click does.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Workspace,
    [ValidateSet('dark', 'light', 'system')][string]$Theme = 'dark',
    [ValidateSet('Direct', 'Tray')][string]$Launch = 'Direct',
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../src-tauri/target/x86_64-pc-windows-msvc/dev-channel'),
    [ValidateRange(1, 50)][int]$Runs = 3,
    [ValidateRange(200, 10000)][int]$DurationMs = 2000,
    [ValidateRange(1, 200)][int]$IntervalMs = 16,
    [ValidateRange(0.05, 1.0)][double]$MismatchFraction = 0.3,
    [ValidateRange(0, 200)][int]$SaveFirst = 0,
    [string]$Output = (Join-Path ([IO.Path]::GetTempPath()) 'ih-startup-frames'),
    [string]$BrowserArguments
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

Add-Type -ReferencedAssemblies System.Drawing.Common, System.Drawing.Primitives, System.Collections, System.Runtime, System.Runtime.InteropServices, System.Threading, System.Threading.Thread, System.Private.Windows.GdiPlus, System.Private.Windows.Core -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class IhFrames {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam, lParam; public uint time; public POINT pt; public uint priv; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WNDCLASSEXW { public uint cbSize, style; public IntPtr lpfnWndProc; public int cbClsExtra, cbWndExtra; public IntPtr hInstance, hIcon, hCursor, hbrBackground; public string lpszMenuName, lpszClassName; public IntPtr hIconSm; }
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
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandleW(string name);
    [DllImport("kernel32.dll", CharSet = CharSet.Ansi)] static extern IntPtr GetProcAddress(IntPtr module, string name);
    [DllImport("gdi32.dll")] static extern IntPtr CreateSolidBrush(uint bgr);

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

    // The GUI's main window; Tao's helper windows use other classes.
    public static IntPtr MainWindow(int pid, bool visibleOnly) { return FindWindow(pid, name => name.StartsWith("Tauri Window"), visibleOnly); }
    public static IntPtr TrayWindow(int pid) { return FindWindow(pid, name => name == "tray_icon_app", false); }
    // WM_USER_TRAYICON (tray-icon 0.24) with WM_LBUTTONUP: the Manager opens the GUI.
    public static bool ClickTray(IntPtr tray) { return PostMessageW(tray, 6002, IntPtr.Zero, (IntPtr)0x0202); }

    // A plain topmost window in one color, drawn by DefWindowProc with the class brush.
    public sealed class Backdrop : IDisposable {
        uint thread; IntPtr window; readonly ManualResetEventSlim ready = new ManualResetEventSlim();
        public Backdrop(RECT r, uint rgb) {
            uint bgr = ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF);
            var t = new Thread(() => {
                thread = GetCurrentThreadId();
                var cls = "IhCaptureBackdrop" + bgr.ToString("x6");
                var wc = new WNDCLASSEXW { cbSize = (uint)Marshal.SizeOf<WNDCLASSEXW>(), hInstance = GetModuleHandleW(null),
                    lpfnWndProc = GetProcAddress(GetModuleHandleW("user32.dll"), "DefWindowProcW"), hbrBackground = CreateSolidBrush(bgr), lpszClassName = cls };
                RegisterClassExW(ref wc);
                // WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, WS_POPUP | WS_VISIBLE
                window = CreateWindowExW(0x8 | 0x80 | 0x08000000, cls, "", 0x80000000 | 0x10000000, r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top, IntPtr.Zero, IntPtr.Zero, wc.hInstance, IntPtr.Zero);
                ready.Set();
                MSG msg;
                while (GetMessageW(out msg, IntPtr.Zero, 0, 0) > 0) DispatchMessageW(ref msg);
                if (window != IntPtr.Zero) DestroyWindow(window);
            });
            t.IsBackground = true; t.Start(); ready.Wait(5000);
        }
        public void Dispose() { PostThreadMessageW(thread, 0x0012 /* WM_QUIT */, IntPtr.Zero, IntPtr.Zero); }
    }

    public class Frame { public double Ms; public double Light; public double Dark; public Bitmap Image; }

    // Captures the client area until durationMs has passed since the window became visible.
    public static List<Frame> Capture(int pid, uint backdropRgb, int durationMs, int intervalMs, int waitMs, out double visibleAtMs, Stopwatch clock) {
        var frames = new List<Frame>();
        visibleAtMs = -1;
        IntPtr window = IntPtr.Zero;
        Backdrop backdrop = null;
        try {
            while (clock.ElapsedMilliseconds < waitMs) {
                if (backdrop == null) {
                    IntPtr hidden = MainWindow(pid, false);
                    RECT r;
                    if (hidden != IntPtr.Zero && GetWindowRect(hidden, out r) && r.Right > r.Left) {
                        backdrop = new Backdrop(r, backdropRgb);
                        // HWND_TOPMOST without SWP_SHOWWINDOW: the window stays hidden until the GUI shows it,
                        // and then sits above the backdrop and every other window.
                        SetWindowPos(hidden, (IntPtr)(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
                    }
                }
                window = MainWindow(pid, true);
                if (window != IntPtr.Zero) break;
                Thread.Sleep(1);
            }
            if (window == IntPtr.Zero) return frames;
            visibleAtMs = clock.Elapsed.TotalMilliseconds;
            double next = visibleAtMs;
            while (clock.Elapsed.TotalMilliseconds - visibleAtMs < durationMs) {
                while (clock.Elapsed.TotalMilliseconds < next) Thread.Sleep(0);
                next += intervalMs;
                RECT r; if (!GetClientRect(window, out r)) break;
                var origin = new POINT(); ClientToScreen(window, ref origin);
                int w = r.Right - r.Left, h = r.Bottom - r.Top;
                if (w <= 0 || h <= 0) continue;
                var frame = new Frame { Ms = clock.Elapsed.TotalMilliseconds - visibleAtMs };
                var bitmap = new Bitmap(w, h, PixelFormat.Format32bppArgb);
                using (var g = Graphics.FromImage(bitmap)) g.CopyFromScreen(origin.X, origin.Y, 0, 0, new Size(w, h));
                Measure(bitmap, frame);
                frame.Image = bitmap;
                frames.Add(frame);
            }
            return frames;
        } finally {
            if (backdrop != null) backdrop.Dispose();
        }
    }

    static void Measure(Bitmap bitmap, Frame frame) {
        var data = bitmap.LockBits(new Rectangle(0, 0, bitmap.Width, bitmap.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        try {
            int stride = data.Stride; var row = new byte[stride];
            long light = 0, total = 0;
            for (int y = 0; y < bitmap.Height; y += 6) {
                Marshal.Copy(data.Scan0 + y * stride, row, 0, stride);
                for (int x = 0; x < bitmap.Width; x += 6) {
                    int i = x * 4; double luma = 0.2126 * row[i + 2] + 0.7152 * row[i + 1] + 0.0722 * row[i];
                    if (luma >= 128) light++;
                    total++;
                }
            }
            frame.Light = total == 0 ? 0 : (double)light / total;
            frame.Dark = 1 - frame.Light;
        } finally { bitmap.UnlockBits(data); }
    }
}
'@

# Windows reports these processes' paths with the \\?\ prefix.
function Get-PlainPath([string]$path) { if ($path) { $path.Replace('\\?\', '') } }

function Get-WindowsDark {
    $value = Get-ItemPropertyValue -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' -Name AppsUseLightTheme -ErrorAction SilentlyContinue
    $value -eq 0
}

function Set-WorkspaceTheme([string]$theme) {
    $preferences = (& $cli $Workspace preferences-snapshot | ConvertFrom-Json -AsHashtable).preferences
    $preferences.theme = $theme
    & $cli $Workspace save-preferences ($preferences | ConvertTo-Json -Compress -Depth 10) | Out-Null
}

function Get-Manager {
    Get-CimInstance Win32_Process -Filter "Name = 'ihctl.exe'" |
        Where-Object { (Get-PlainPath $_.ExecutablePath) -eq $cli -and (Get-PlainPath $_.CommandLine) -like "*$Workspace*manager-serve*" } | Select-Object -First 1
}

function Wait-NoGui {
    # Only this workspace's GUI (opened by this script or by its Manager); other GUIs are left alone.
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ($true) {
        $manager = Get-Manager
        $running = Get-CimInstance Win32_Process -Filter "Name = 'InstanceHarborDev.exe'" |
            Where-Object { (Get-PlainPath $_.ExecutablePath) -eq $gui -and ($_.ParentProcessId -eq $PID -or ($manager -and $_.ParentProcessId -eq $manager.ProcessId)) }
        if (!$running) { return }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'A GUI for this workspace is still running.' }
        Start-Sleep -Milliseconds 200
    }
}

$expectedDark = switch ($Theme) { 'dark' { $true } 'light' { $false } default { Get-WindowsDark } }
# The page's --bg for the expected theme (src/app.css).
$backdrop = if ($expectedDark) { 0x121c2b } else { 0xf4f6fa }
Set-WorkspaceTheme $Theme
$summary = [Collections.Generic.List[object]]::new()
try {
    for ($run = 1; $run -le $Runs; $run++) {
        Wait-NoGui
        $clock = [Diagnostics.Stopwatch]::StartNew()
        if ($Launch -eq 'Tray') {
            & $cli $Workspace manager-connect | Out-Null
            $until = [DateTime]::UtcNow.AddSeconds(10)
            $manager = $null
            while (!$manager -and [DateTime]::UtcNow -lt $until) { $manager = Get-Manager; if (!$manager) { Start-Sleep -Milliseconds 100 } }
            if (!$manager) { throw 'This workspace has no Manager.' }
            $tray = [IntPtr]::Zero
            while ($tray -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $until) { $tray = [IhFrames]::TrayWindow($manager.ProcessId); Start-Sleep -Milliseconds 50 }
            if ($tray -eq [IntPtr]::Zero) { throw 'The Manager has no tray window yet.' }
            if ($BrowserArguments) { Write-Warning 'BrowserArguments do not reach a GUI opened by the Manager.' }
            $clock.Restart()
            if (![IhFrames]::ClickTray($tray)) { throw 'Could not post the tray click.' }
            $process = $null
            while (!$process -and $clock.ElapsedMilliseconds -lt 10000) {
                $child = Get-CimInstance Win32_Process -Filter "ParentProcessId = $($manager.ProcessId) AND Name = 'InstanceHarborDev.exe'"
                if ($child) { $process = Get-Process -Id @($child)[0].ProcessId } else { Start-Sleep -Milliseconds 5 }
            }
            if (!$process) { throw 'The tray click did not open the GUI.' }
        } else {
            $start = [Diagnostics.ProcessStartInfo]::new($gui)
            $start.UseShellExecute = $false
            $start.Environment['IH_WORKSPACE'] = $Workspace
            if ($BrowserArguments) { $start.Environment['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = $BrowserArguments }
            $clock.Restart()
            $process = [Diagnostics.Process]::Start($start)
        }
        $visibleAt = 0.0
        $frames = [IhFrames]::Capture($process.Id, $backdrop, $DurationMs, $IntervalMs, 30000, [ref]$visibleAt, $clock)
        try {
            if ($frames.Count -eq 0) { throw "Run ${run}: the GUI window never became visible." }
            $mismatched = @($frames | Where-Object { if ($expectedDark) { $_.Light -gt $MismatchFraction } else { $_.Dark -gt $MismatchFraction } })
            $prefix = '{0}-{1}-run{2}' -f $Theme, $Launch.ToLowerInvariant(), $run
            for ($index = 0; $index -lt $frames.Count; $index++) {
                $frame = $frames[$index]
                if ($index -lt $SaveFirst -or ($mismatched -contains $frame -and $index -lt 40)) {
                    $mark = if ($mismatched -contains $frame) { '-MISMATCH' } else { '' }
                    $frame.Image.Save((Join-Path $Output ('{0}-frame{1:000}-{2:0000}ms-light{3:0.00}{4}.png' -f $prefix, $index, $frame.Ms, $frame.Light, $mark)), [Drawing.Imaging.ImageFormat]::Png)
                }
            }
            $frames[$frames.Count - 1].Image.Save((Join-Path $Output "$prefix-last.png"), [Drawing.Imaging.ImageFormat]::Png)
            $summary.Add([pscustomobject]@{
                run = $run; theme = $Theme; expected = $(if ($expectedDark) { 'dark' } else { 'light' }); launch = $Launch
                visibleAtMs = [math]::Round($visibleAt); frames = $frames.Count; mismatched = $mismatched.Count
                firstMismatchMs = $(if ($mismatched.Count) { [math]::Round($mismatched[0].Ms) } else { $null })
                lastMismatchMs = $(if ($mismatched.Count) { [math]::Round($mismatched[-1].Ms) } else { $null })
                maxLight = [math]::Round(($frames | Measure-Object Light -Maximum).Maximum, 3)
                maxDark = [math]::Round(($frames | Measure-Object Dark -Maximum).Maximum, 3)
            })
        } finally {
            foreach ($frame in $frames) { $frame.Image.Dispose() }
            & $cli $Workspace app-exit | Out-Null
            if (!$process.WaitForExit(20000)) { throw "GUI $($process.Id) did not exit after app-exit." }
        }
    }
} finally {
    if (Get-Manager) { & $cli $Workspace app-exit 2>$null | Out-Null }
}
$summary | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
$summary | ConvertTo-Json -Depth 4
if (@($summary | Where-Object { $_.mismatched -gt 0 }).Count) { exit 1 }
