#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$GuiPid,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$InitialFolder,
    [string]$SelectFolder
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$gui = Get-Process -Id $GuiPid
if ($gui.Path -ine $Executable) { throw 'The isolated GUI process identity changed.' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class IsolatedPickerWindow
{
    private delegate bool WindowCallback(IntPtr window, IntPtr state);
    [DllImport("user32.dll")] private static extern bool EnumWindows(WindowCallback callback, IntPtr state);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, WindowCallback callback, IntPtr state);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr window, StringBuilder name, int length);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageW")]
    private static extern IntPtr SendText(IntPtr window, uint message, IntPtr wParam, string text);
    public static IntPtr[] Enumerate(uint pid)
    {
        var windows = new List<IntPtr>();
        EnumWindows((window, state) => {
            GetWindowThreadProcessId(window, out uint owner);
            if (owner == pid && IsWindowVisible(window)) windows.Add(window);
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
    }
    public static void Command(IntPtr window, int id, IntPtr accessibleButton)
    {
        IntPtr button = accessibleButton != IntPtr.Zero ? accessibleButton : GetDlgItem(window, id);
        var name = new StringBuilder(128);
        bool isButton = button != IntPtr.Zero && GetClassName(button, name, name.Capacity) > 0 && name.ToString() == "Button";
        if (!(isButton ? PostMessage(button, 0xf5, IntPtr.Zero, IntPtr.Zero)
            : PostMessage(window, 0x111, (IntPtr)id, button)))
            throw new InvalidOperationException("Unable to post an isolated picker command.");
    }
    public static void SetText(IntPtr window, string text)
    {
        if (window == IntPtr.Zero || SendText(window, 0xc, IntPtr.Zero, text) == IntPtr.Zero)
            throw new InvalidOperationException("Unable to set the isolated picker text.");
    }
    public static IntPtr FileNameEdit(IntPtr parent, IntPtr accessibleField)
    {
        IntPtr field = accessibleField != IntPtr.Zero ? accessibleField : GetDlgItem(parent, 1152);
        var name = new StringBuilder(128);
        if (field == IntPtr.Zero) return IntPtr.Zero;
        if (GetClassName(field, name, name.Capacity) > 0 && name.ToString() == "Edit") return field;
        IntPtr result = IntPtr.Zero;
        EnumChildWindows(field, (window, state) => {
            if (GetClassName(window, name, name.Capacity) > 0 && name.ToString() == "Edit") {
                result = window;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return result;
    }
}
'@
$ae = [System.Windows.Automation.AutomationElement]
$ts = [System.Windows.Automation.TreeScope]
$all = [System.Windows.Automation.Condition]::TrueCondition
$until = [DateTime]::UtcNow.AddSeconds(10)
$dialog = $null
do {
    $windows = [IsolatedPickerWindow]::Enumerate($GuiPid) | ForEach-Object { $ae::FromHandle($_) }
    $dialog = $windows |
        Where-Object { $_.Current.ClassName -eq '#32770' } |
        Select-Object -First 1
    if (!$dialog) { Start-Sleep -Milliseconds 120 }
} until ($dialog -or [DateTime]::UtcNow -ge $until)
if (!$dialog) {
    $visible = @($windows | ForEach-Object { "$($_.Current.ClassName) $($_.Current.Name)" }) -join ', '
    throw "The native folder selection window did not open. Visible windows for owned PID: $visible"
}
$handle = [IntPtr]$dialog.Current.NativeWindowHandle
$nativeId = {
    param([string]$Id)
    $found = $dialog.FindAll($ts::Descendants, $all) | Where-Object { $_.Current.AutomationId -eq $Id } | Select-Object -First 1
    if ($found) { return [IntPtr]$found.Current.NativeWindowHandle }
    [IntPtr]::Zero
}
$cancelHandle = & $nativeId '2'
$submitted = $false
try {
    $until = [DateTime]::UtcNow.AddSeconds(6)
    do {
        $controls = @($dialog.FindAll($ts::Descendants, $all) | ForEach-Object {
            [ordered]@{ name = $_.Current.Name; id = $_.Current.AutomationId; type = $_.Current.ControlType.ProgrammaticName }
        })
        $address = $controls | Where-Object {
            $_.id -eq '1001' -and
                $_.name.Replace('/', '\').EndsWith($InitialFolder, [StringComparison]::OrdinalIgnoreCase)
        } | Select-Object -First 1
        if (!$address) { Start-Sleep -Milliseconds 100 }
    } until ($address -or [DateTime]::UtcNow -ge $until)
    if (!$address) { throw "Unexpected initial folder. Expected $InitialFolder. Accessible controls: $($controls | ConvertTo-Json -Compress)" }
    if ($SelectFolder) {
        if (!(Test-Path -LiteralPath $SelectFolder -PathType Container)) { throw 'Missing isolated selection fixture.' }
        $fieldHandle = & $nativeId '1152'
        $submitHandle = & $nativeId '1'
        [IsolatedPickerWindow]::SetText([IsolatedPickerWindow]::FileNameEdit($handle, $fieldHandle), $SelectFolder)
        [IsolatedPickerWindow]::Command($handle, 1, $submitHandle)
        $until = [DateTime]::UtcNow.AddSeconds(5)
        $submittedAgain = $false
        do {
            Start-Sleep -Milliseconds 150
            $submitted = ![IsolatedPickerWindow]::IsWindow($handle)
            if (!$submitted -and !$submittedAgain) {
                $current = $dialog.FindAll($ts::Descendants, $all) | Where-Object { $_.Current.AutomationId -eq '1001' } | Select-Object -First 1
                if ($current -and $current.Current.Name.EndsWith($SelectFolder, [StringComparison]::OrdinalIgnoreCase)) {
                    [IsolatedPickerWindow]::Command($handle, 1, (& $nativeId '1'))
                    $submittedAgain = $true
                }
            }
        } until ($submitted -or [DateTime]::UtcNow -ge $until)
        if (!$submitted) { throw "The isolated picker stayed open when selecting $SelectFolder." }
    }
    [pscustomobject]@{ opened = $true; address = $address.name; selected = $SelectFolder } | ConvertTo-Json -Compress
} finally {
    if (!$submitted) {
        try { if ([IsolatedPickerWindow]::IsWindow($handle)) { [IsolatedPickerWindow]::Command($handle, 2, $cancelHandle) } }
        catch [System.Windows.Automation.ElementNotAvailableException] {
            # The isolated dialog already closed.
        }
    }
}
