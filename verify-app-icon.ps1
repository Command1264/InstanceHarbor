#Requires -Version 7.0
<#
.SYNOPSIS
Reads the application icon (resource 32512) embedded in a Windows executable.

.DESCRIPTION
With -Expected, fails unless every embedded image equals the matching image of that
ICO file, byte for byte and in the same order. With -Export, writes the embedded
icon as an ICO file. build-product.ps1 uses -Expected so that a product build can
only ship the blue icon and a development build only the red one.

.EXAMPLE
./verify-app-icon.ps1 -Binary src-tauri/target/x86_64-pc-windows-msvc/release/InstanceHarbor.exe -Expected src-tauri/icons/icon.ico
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Binary,
    [string]$Expected,
    [string]$Export
)
$ErrorActionPreference = 'Stop'

if (-not ('InstanceHarbor.IconResource' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
namespace InstanceHarbor {
public static class IconResource {
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
    [DllImport("kernel32", SetLastError = true)]
    static extern bool FreeLibrary(IntPtr module);
    [DllImport("kernel32", SetLastError = true)]
    static extern IntPtr FindResource(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32", SetLastError = true)]
    static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32", SetLastError = true)]
    static extern IntPtr LockResource(IntPtr data);
    [DllImport("kernel32", SetLastError = true)]
    static extern uint SizeofResource(IntPtr module, IntPtr resource);
    const uint AsDataFile = 0x2, AsImageResource = 0x20;
    const int RtIcon = 3, RtGroupIcon = 14;

    static byte[] Read(IntPtr module, int id, int type) {
        IntPtr found = FindResource(module, (IntPtr)id, (IntPtr)type);
        if (found == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "Resource " + id + " of type " + type + " not found");
        uint size = SizeofResource(module, found);
        IntPtr pointer = LockResource(LoadResource(module, found));
        if (pointer == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "Resource " + id + " could not be loaded");
        byte[] data = new byte[size];
        Marshal.Copy(pointer, data, 0, (int)size);
        return data;
    }

    // Returns the 12 directory bytes (width .. image size) and the data of each image.
    public static List<Tuple<byte[], byte[]>> Load(string path, int group) {
        IntPtr module = LoadLibraryEx(path, IntPtr.Zero, AsDataFile | AsImageResource);
        if (module == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot open " + path);
        try {
            byte[] directory = Read(module, group, RtGroupIcon);
            int count = BitConverter.ToUInt16(directory, 4);
            var images = new List<Tuple<byte[], byte[]>>();
            for (int index = 0; index < count; index++) {
                int offset = 6 + 14 * index;
                byte[] entry = new byte[12];
                Array.Copy(directory, offset, entry, 0, 12);
                int id = BitConverter.ToUInt16(directory, offset + 12);
                images.Add(Tuple.Create(entry, Read(module, id, RtIcon)));
            }
            return images;
        } finally { FreeLibrary(module); }
    }
}
}
'@
}

function Read-IcoFile([string]$Path) {
    $data = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path).Path)
    if ($data.Length -lt 6 -or [BitConverter]::ToUInt16($data, 2) -ne 1) { throw "不是 ICO 檔：$Path" }
    $count = [BitConverter]::ToUInt16($data, 4)
    foreach ($index in 0..($count - 1)) {
        $offset = 6 + 16 * $index
        $size = [BitConverter]::ToUInt32($data, $offset + 8)
        $start = [BitConverter]::ToUInt32($data, $offset + 12)
        [pscustomobject]@{ Entry = $data[$offset..($offset + 11)]; Image = $data[$start..($start + $size - 1)] }
    }
}

$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
$embedded = @([InstanceHarbor.IconResource]::Load($binaryPath, 32512) | ForEach-Object {
    [pscustomobject]@{ Entry = $_.Item1; Image = $_.Item2 }
})

if ($Export) {
    $stream = [IO.MemoryStream]::new()
    $writer = [IO.BinaryWriter]::new($stream)
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$embedded.Count)
    $offset = 6 + 16 * $embedded.Count
    foreach ($item in $embedded) {
        $writer.Write([byte[]]$item.Entry); $writer.Write([uint32]$offset); $offset += $item.Image.Length
    }
    foreach ($item in $embedded) { $writer.Write([byte[]]$item.Image) }
    $writer.Flush()
    [IO.File]::WriteAllBytes([IO.Path]::GetFullPath($Export, (Get-Location).Path), $stream.ToArray())
}

if ($Expected) {
    $reference = @(Read-IcoFile $Expected)
    if ($reference.Count -ne $embedded.Count) {
        throw "$binaryPath 內嵌 $($embedded.Count) 個圖示尺寸，$Expected 有 $($reference.Count) 個。"
    }
    for ($index = 0; $index -lt $reference.Count; $index++) {
        $same = [Linq.Enumerable]::SequenceEqual([byte[]]$reference[$index].Image, [byte[]]$embedded[$index].Image) -and
            [Linq.Enumerable]::SequenceEqual([byte[]]$reference[$index].Entry, [byte[]]$embedded[$index].Entry)
        if (!$same) { throw "$binaryPath 內嵌的第 $($index + 1) 個圖示與 $Expected 不同，執行檔用錯圖示。" }
    }
}

[pscustomobject]@{
    Binary = $binaryPath
    Sizes = ($embedded | ForEach-Object { $w = $_.Entry[0]; if ($w -eq 0) { 256 } else { $w } }) -join ','
    Sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([byte[]]($embedded | ForEach-Object { $_.Image })))
}
