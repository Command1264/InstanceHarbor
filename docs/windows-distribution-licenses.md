# Windows distribution licenses

The original InstanceHarbor source is under MIT. In both the Windows x64 ZIP and the Setup installation folder, `LICENSE` contains the project license and `THIRD-PARTY-NOTICES.txt` preserves the applicable upstream notices. The ZIP manifest covers the three license and source files and the Windows startup guide. Setup installs the same files beside the executables. Use a separately authenticated ZIP SHA-256 when staging a package that contains these files. The executables’ Authenticode signatures, if present, do not cover separate documents.

## MPL source

Open `MPL-SOURCES.zip` without a network connection. `SOURCE-INDEX.txt` lists each crate, its Cargo.lock SHA-256 and its original download URL. Each enclosed `.crate` is an unmodified gzip-compressed tar archive containing the source. The five RC2 archives are cssparser 0.36.0, cssparser-macros 0.6.1, dtoa-short 0.3.5, option-ext 0.2.0 and selectors 0.36.1. The archive includes the MPL 2.0 license text. [Mozilla MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/)

## Microsoft runtime

- `conpty.dll` and `OpenConsole.exe` are the original x64 files from Microsoft's signed ConPTY NuGet release `1.25.260930003`. Include the Windows Terminal MIT license and upstream notice. [Windows Terminal v1.25.2733.0](https://github.com/microsoft/terminal/releases/tag/v1.25.2733.0)
- The ZIP and Setup use the system Microsoft Visual C++ v14 x64 Runtime. Setup checks for version 14.44.35211.0 or newer. Users can install the latest supported version from Microsoft. [Microsoft x64 Runtime](https://aka.ms/vc14/vc_redist.x64.exe)
- WebView2 is obtained separately from Windows or Microsoft. The release packages contain no WebView2 runtime, Java runtime, Paper or Minecraft server JAR.

## Console runtime source

`v2/console-runtime.json` is the single record of the console runtime. It pins:

| Item | Value |
| --- | --- |
| Package | `Microsoft.Windows.Console.ConPTY` `1.25.260930003`, signed by Microsoft Corporation |
| Download | [nuget.org flat container](https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/1.25.260930003/microsoft.windows.console.conpty.1.25.260930003.nupkg) |
| Package size | 1,763,391 bytes |
| Package SHA-256 | `02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee` |
| `conpty.dll` | entry `runtimes/win-x64/native/conpty.dll`, SHA-256 `feeef341…ae805a` |
| `OpenConsole.exe` | entry `build/native/runtimes/x64/OpenConsole.exe`, SHA-256 `3d66b23d…f80586` |

`v2/provision-console-runtime.ps1` never downloads. Given the package, it checks the package size and SHA-256, reads the two x64 entries, and checks each file's SHA-256, x64 header and Microsoft Authenticode signature before it copies them. It refuses to replace a different file at the destination. `native_console.rs`, `pack-windows.ps1` and `install-windows.ps1` pin the same file hashes; `v2/src/console-runtime.test.mjs` and a Rust test keep them equal.

## Build a distribution

A distribution is `1.0.0-rc2` or a stable version such as `1.0.0`. It must contain the console runtime, the license notices and the startup guide, and must not contain an app-local Visual C++ runtime. `pack-windows.ps1` and `pack-windows-setup.ps1` refuse a stable version without the console runtime. Run these steps from the repository root in PowerShell 7. The publisher downloads the two upstream files; no script downloads them.

1. Download the ConPTY package and check its SHA-256. Optionally verify its NuGet signature with the .NET SDK.

   ```powershell
   $package = "$env:TEMP\microsoft.windows.console.conpty.1.25.260930003.nupkg"
   Invoke-WebRequest 'https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/1.25.260930003/microsoft.windows.console.conpty.1.25.260930003.nupkg' -OutFile $package
   (Get-FileHash -LiteralPath $package).Hash -eq '02B07B349AF66D801159BDF9E440D4A1CE78BB951F37FC8609731665AFDAE7EE'
   dotnet nuget verify --all $package
   ```

2. Build the release binaries. The build copies the verified runtime beside them and fails without it.

   ```powershell
   v2/build-product.ps1 -Release -ConsoleRuntimeSource $package
   $binaries = 'v2/src-tauri/target/x86_64-pc-windows-msvc/release'
   ```

3. Prepare the notices. Provide the official NSIS 3.08 `COPYING`, the Windows Terminal 1.25.2733.0 x64 ZIP from the release above, and the Rust toolchain's `share/doc/rust` directory. The generator reads the Windows x64 Cargo dependency closure (including build dependencies for coverage), the npm runtime tree, and the original MPL crate archives in Cargo's registry cache.

   ```powershell
   python v2/prepare-windows-notices.py --output $notices --nsis-copying $nsisCopying --terminal-zip $terminalZip --rust-docs $rustDocs
   ```

4. Package the ZIP. The binary folder already holds the verified runtime.

   ```powershell
   $packed = v2/pack-windows.ps1 -BinaryDirectory $binaries -ConsoleRuntimeDirectory $binaries -NoticeDirectory $notices -OutputZip $zip -Version 1.0.0
   ```

5. Build Setup from the ZIP and the SHA-256 that step 4 returned, with the NSIS 3.08 compiler.

   ```powershell
   v2/pack-windows-setup.ps1 -Package $zip -ExpectedPackageSha256 $packed.sha256 -Compiler $makensis -Output $setup
   ```

6. Before distribution, check that the ZIP manifest lists `conpty.dll` and `OpenConsole.exe` with the pinned hashes, then install the Setup on a test machine, start a Paper instance and confirm that the instance details show **Console: InstanceHarbor console** (tier 1).

The version passed to `-Version` is the package version. Release the matching product version (Cargo, Tauri and npm metadata) separately.
