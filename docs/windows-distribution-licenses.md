# Windows distribution licenses

The original InstanceHarbor source is under MIT. In both the Windows x64 ZIP and the Setup installation folder, `LICENSE` contains the project license and `THIRD-PARTY-NOTICES.txt` preserves the applicable upstream notices. The ZIP manifest covers the three license and source files. Setup installs the same files beside the executables. Use a separately authenticated ZIP SHA-256 when staging a package that contains these files. The executables’ Authenticode signatures, if present, do not cover separate documents.

## MPL source

Open `MPL-SOURCES.zip` without a network connection. `SOURCE-INDEX.txt` lists each crate, its Cargo.lock SHA-256 and its original download URL. Each enclosed `.crate` is an unmodified gzip-compressed tar archive containing the source. The five RC2 archives are cssparser 0.36.0, cssparser-macros 0.6.1, dtoa-short 0.3.5, option-ext 0.2.0 and selectors 0.36.1. The archive includes the MPL 2.0 license text. [Mozilla MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/)

## Microsoft runtime

- `conpty.dll` and `OpenConsole.exe` are the original x64 files from Microsoft's signed ConPTY NuGet release `1.25.260930003`. Include the Windows Terminal MIT license and upstream notice. [Windows Terminal v1.25.2733.0](https://github.com/microsoft/terminal/releases/tag/v1.25.2733.0)
- `vcruntime140.dll` and `vcruntime140_1.dll` are original, signed x64 Visual C++ 14.44.35211.0 files from `VC/Redist/MSVC/14.44.35112/x64/Microsoft.VC143.CRT`. Keep these files unmodified and beside the application. The distributor's rights depend on its applicable Microsoft license and Distributable Code terms. [Visual Studio 2022 REDIST list](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution)
- WebView2 is obtained separately from Windows or Microsoft. The release packages contain no WebView2 runtime, Java runtime, Paper or Minecraft server JAR.

## Reproduce RC2 notices

Use the release Rust toolchain and the installed dependencies from the two lockfiles. The generator reads the Windows x64 Cargo dependency closure (including build dependencies for coverage), the npm runtime tree, and the original MPL crate archives in Cargo's registry cache. Provide the official NSIS 3.08 `COPYING`, the Windows Terminal 1.25.2733.0 x64 ZIP from the Microsoft release above, and the Rust toolchain's `share/doc/rust` directory.

```powershell
python v2/prepare-windows-notices.py --output $notices --nsis-copying $nsisCopying --terminal-zip $terminalZip --rust-docs $rustDocs
v2/pack-windows.ps1 -BinaryDirectory $binaries -ConsoleRuntimeDirectory $consoleRuntime -VCRuntimeDirectory $vcRuntime -NoticeDirectory $notices -OutputZip $zip -Version 1.0.0-rc2
```

Use the resulting ZIP and its returned SHA-256 with `pack-windows-setup.ps1`. Before distribution, verify the signatures and x64 architecture of the four Microsoft files and compare their hashes to the original payloads. The third-party notice inventory identifies upstream terms separately from InstanceHarbor's MIT license.
