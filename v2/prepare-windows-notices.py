"""Prepare the notices and corresponding source included with Windows RC2."""
import argparse
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tomllib
from zipfile import ZipFile, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-msvc"
TERMINAL_RELEASE = "https://github.com/microsoft/terminal/releases/tag/v1.25.2733.0"
WEBVIEW2_DOWNLOAD = "https://developer.microsoft.com/microsoft-edge/webview2/#download-section"
MOZILLA = "https://www.mozilla.org/en-US/MPL/2.0/"
NOTICE_NAME = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright|authors)", re.I)
FALLBACKS = {
    "alloc-stdlib": ("alloc-stdlib-LICENSE",),
    "defmt-parser": ("defmt-parser-LICENSE-MIT", "defmt-parser-LICENSE-APACHE"),
    "selectors": ("MPL-2.0.txt",),
    "webview2-com": ("webview2-com-LICENSE",),
    "webview2-com-macros": ("webview2-com-macros-LICENSE",),
    "webview2-com-sys": ("webview2-com-LICENSE",),
}
for component in ("unic-char-property", "unic-char-range", "unic-common", "unic-ucd-ident", "unic-ucd-version"):
    prefix = component if component == "unic-ucd-ident" else "unic-char-property"
    FALLBACKS[component] = (f"{prefix}-LICENSE-MIT", f"{prefix}-LICENSE-APACHE")


def cargo(*args):
    return subprocess.check_output(["cargo", *args], cwd=ROOT / "v2/src-tauri", encoding="utf-8")


def digest(data):
    return sha256(data).hexdigest()


def notice_files(directory):
    return [f for f in sorted(directory.iterdir()) if f.is_file() and NOTICE_NAME.match(f.name)]


def render(output, upstream, nsis_copying, terminal_zip, rust_docs):
    output.mkdir(exist_ok=True)
    inventory = []
    texts = {}

    def add(label, terms, origin, files):
        references = []
        for filename, data in files:
            key = digest(data)
            # Preserve upstream wording, including its copyright notices.
            texts.setdefault(key, data.decode("utf-8-sig", errors="strict"))
            references.append(f"{filename} [{key[:16]}]")
        if not references:
            raise ValueError(f"Missing license text for {label}")
        inventory.append(f"{label} — {terms}\n  {origin}\n  " + "; ".join(references))

    cargo_data = json.loads(cargo("metadata", "--offline", "--locked", "--filter-platform", TARGET, "--format-version", "1"))
    tree = cargo("tree", "--offline", "--locked", "--target", TARGET, "-e", "normal,build", "--prefix", "none", "--format", "{p}")
    packages = set(tuple(match.groups()) for match in re.finditer(r"^([\w-]+) v([^\s]+)", tree, re.M))
    locks = {(p["name"], p["version"]): p for p in tomllib.loads((ROOT / "v2/src-tauri/Cargo.lock").read_text(encoding="utf-8"))["package"]}
    mpl_sources = {}
    for package in sorted(cargo_data["packages"], key=lambda p: (p["name"], p["version"])):
        name, version = package["name"], package["version"]
        if not package["source"] or (name, version) not in packages:
            continue
        directory = Path(package["manifest_path"]).parent
        files = notice_files(directory)
        if package["license_file"]:
            path = directory / package["license_file"]
            if path not in files:
                files.append(path)
        if not files:
            files = [upstream / entry for entry in FALLBACKS[name]]
        label = f"{name} {version}"
        url = f"https://crates.io/crates/{name}/{version}"
        add(label, package["license"], url, [(f.name, f.read_bytes()) for f in files])
        if "MPL-2.0" in package["license"]:
            filename = f"{name}-{version}.crate"
            registry = directory.parents[2]
            payload = (registry / "cache" / directory.parent.name / filename).read_bytes()
            if digest(payload) != locks[(name, version)]["checksum"]:
                raise ValueError(f"Cargo.lock checksum mismatch for {filename}")
            with tarfile.open(fileobj=BytesIO(payload), mode="r:gz") as crate:
                if not any(member.isfile() and member.name.endswith(".rs") for member in crate):
                    raise ValueError(f"No Rust source in {filename}")
            source_url = f"https://static.crates.io/crates/{name}/{filename}"
            mpl_sources[filename] = (payload, source_url)
    if not mpl_sources:
        raise ValueError("No corresponding MPL sources found")

    front = ROOT / "v2"
    npm_lock = json.loads((front / "package-lock.json").read_text(encoding="utf-8"))["packages"]
    for path, package in sorted(npm_lock.items()):
        # Vite may contribute its modulepreload helper to the shipped JS.
        if not path or (package.get("dev") and path != "node_modules/vite"):
            continue
        directory = front / path
        if not directory.is_dir():
            if package.get("optional"):
                continue
            raise ValueError(f"Missing installed npm package {path}")
        installed = json.loads((directory / "package.json").read_text(encoding="utf-8"))
        if installed["version"] != package["version"]:
            raise ValueError(f"npm lock mismatch for {path}")
        add(f'{installed["name"]} {installed["version"]}', installed.get("license", "See upstream notices"),
            f'https://www.npmjs.com/package/{installed["name"]}/v/{installed["version"]}',
            [(f.name, f.read_bytes()) for f in notice_files(directory)])

    def add_file(label, terms, origin, path):
        add(label, terms, origin, [(path.name, path.read_bytes())])

    add_file("Lucide icons", "ISC, with Feather MIT attribution", "https://lucide.dev/license",
             front / "public/licenses/lucide.txt")
    vendor = ROOT / "experiments/conpty-a-prototype/ui/vendor"
    add_file("xterm.js", "MIT", "https://github.com/xtermjs/xterm.js", vendor / "xterm-LICENSE")
    add_file("xterm.js Unicode graphemes add-on", "MIT", "https://github.com/xtermjs/xterm.js",
             vendor / "addon-unicode-graphemes-LICENSE")
    rust_version = subprocess.check_output(["rustc", "--version"], encoding="utf-8").split()[1]
    rust_license_files = [rust_docs / "COPYRIGHT-library.html", *sorted((rust_docs / "licenses").glob("*.txt"))]
    if len(rust_license_files) < 2:
        raise ValueError("Rust toolchain license texts are missing")
    add(f"Rust standard library {rust_version}", "See upstream library copyright and exceptions",
        f"https://github.com/rust-lang/rust/tree/{rust_version}/library",
        [(f.name, f.read_bytes()) for f in rust_license_files])
    add_file("NSIS 3.08 (Setup only)", "See upstream COPYING", "https://nsis.sourceforge.io/License",
             nsis_copying)
    expected_terminal = "bf3ef2012f6c44d8340a4c58125acc9498d19b580f9890dc043cdf831852e796"
    if digest(terminal_zip.read_bytes()) != expected_terminal:
        raise ValueError("Unexpected official Windows Terminal ZIP SHA-256")
    with ZipFile(terminal_zip) as terminal:
        notice = next(name for name in terminal.namelist() if name.endswith("/NOTICE.html"))
        add("Microsoft.Windows.Console.ConPTY 1.25.260930003 (conpty.dll and OpenConsole.exe)", "MIT",
            TERMINAL_RELEASE, [("LICENSE", (upstream / "windows-terminal-LICENSE").read_bytes()),
                               ("NOTICE.html", terminal.read(notice))])

    source_index = "InstanceHarbor 1.0.0-rc2 — corresponding MPL 2.0 source\n" + MOZILLA + "\n"
    source_index += "The .crate archives are the original Source Code Form. Open them as gzip-compressed tar archives.\n"
    for filename, (data, url) in mpl_sources.items():
        source_index += f"\n{filename}\n  SHA-256: {digest(data)}\n  {url}\n"
    with ZipFile(output / "MPL-SOURCES.zip", "x", ZIP_DEFLATED) as archive:
        archive.writestr("SOURCE-INDEX.txt", source_index)
        archive.writestr("MPL-2.0.txt", (upstream / "MPL-2.0.txt").read_bytes())
        for filename, (data, _) in mpl_sources.items():
            archive.writestr(filename, data)

    header = f"""InstanceHarbor 1.0.0-rc2 — Third-Party Notices

InstanceHarbor's own source terms are in LICENSE. This file preserves third-party terms
for the Windows distribution. The Cargo dependency inventory also includes build-time
components conservatively.

Windows prerequisites obtained separately
  InstanceHarbor.exe and ihctl.exe link the Microsoft C runtime statically;
  no Visual C++ Redistributable is needed.
  Microsoft Edge WebView2 Runtime is supplied separately by Windows or Microsoft:
  {WEBVIEW2_DOWNLOAD}

{source_index}
The source archives are located in MPL-SOURCES.zip beside this file, also available offline.

Component index
===============

"""
    index = "\n\n".join(inventory)
    licenses = "\n\n".join(f"===== Upstream notice [{key[:16]}] =====\n\n{body}" for key, body in texts.items())
    (output / "THIRD-PARTY-NOTICES.txt").write_text(header + index + "\n\nFull notices\n============\n\n" + licenses + "\n", encoding="utf-8")
    (output / "LICENSE").write_bytes((ROOT / "LICENSE").read_bytes())
    print(f"{len(inventory)} component records; {len(mpl_sources)} exact MPL source archives")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--upstream-notices", type=Path, default=ROOT / "v2/windows-upstream-notices")
    parser.add_argument("--nsis-copying", type=Path, required=True)
    parser.add_argument("--terminal-zip", type=Path, required=True)
    parser.add_argument("--rust-docs", type=Path, required=True)
    args = parser.parse_args()
    render(args.output, args.upstream_notices, args.nsis_copying, args.terminal_zip, args.rust_docs)
