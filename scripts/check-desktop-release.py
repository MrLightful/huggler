#!/usr/bin/env python3
"""Verify separate archives for the production app and Pi extension, with an optional app launch."""
import argparse
from pathlib import Path
import plistlib
import sqlite3
import subprocess
import tempfile
import time
import zipfile

from desktop_check import ROOT, installer, isolated_env


def launch_and_quit(app, case):
    env = isolated_env(case)
    # Use the ordinary application entry point with a fresh home and no overrides.
    database = Path(env["HOME"]) / ".huggler/userdata/huggler.sqlite3"
    process = subprocess.Popen([app / "Contents/MacOS/huggler-desktop"], env=env, cwd="/")
    try:
        until = time.monotonic() + 30
        while not database.exists():
            if process.poll() is not None or time.monotonic() >= until:
                raise RuntimeError(f"App failed to open its default database at {database}")
            time.sleep(0.1)
        # Address this process by PID so other installed copies are unaffected.
        subprocess.run(["osascript", "-l", "JavaScript", "-e", """
            ObjC.import('AppKit')
            function run([pid]) {
                for (let i = 0; i < 150; i++) {
                    const app = $.NSRunningApplication.runningApplicationWithProcessIdentifier(Number(pid))
                    if (!app.isNil() && app.finishedLaunching) {
                        if (!app.terminate) throw Error('AppKit rejected Quit for the installed app')
                        return
                    }
                    delay(0.1)
                }
                throw Error('AppKit did not report the installed app ready')
            }
        """, str(process.pid)], check=True)
        returncode = process.wait(timeout=20)
        if returncode:
            raise RuntimeError(f"Installed app exited with status {returncode}")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
    with sqlite3.connect(database) as db:
        schema = db.execute("PRAGMA user_version").fetchone()[0]
        expected = max(int(path.stem) for path in (ROOT / "migrations").glob("[0-9]*.sql"))
        assert schema == expected, (schema, expected)
        print("Production app opened default state and quit, schema", schema)


def check(output, *, package_only=False):
    output.mkdir(parents=True, exist_ok=True)
    with (tempfile.TemporaryDirectory(prefix="huggler release ") as tmp,
          tempfile.TemporaryDirectory(dir=output) as staging):
        case = Path(tmp)
        bundle = case / "build/Huggler.app"
        installer.install(bundle)
        info = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
        version = info["CFBundleShortVersionString"]
        arch = subprocess.check_output(["lipo", "-archs", bundle / "Contents/MacOS/huggler-desktop"], text=True).strip()
        archive = Path(staging) / f"huggler-{version}-macos-{arch}.zip"
        subprocess.run(["ditto", "-c", "-k", "--norsrc", "--keepParent", bundle, archive], check=True)
        apps = case / "Installed Applications"
        subprocess.run(["ditto", "-x", "-k", archive, apps], check=True)
        installed = apps / bundle.name
        subprocess.run(["codesign", "--verify", "--strict", installed], check=True)
        contents = installed / "Contents"
        files = {p.relative_to(contents).as_posix(): p.stat().st_size for p in contents.rglob("*") if p.is_file()}
        assert files.keys() == {
            "MacOS/huggler-desktop", "Info.plist", "_CodeSignature/CodeResources",
            "Resources/LICENSE.txt", "Resources/Third-party notices.txt", "Resources/Rust-library.html",
        }, sorted(files)
        executable = contents / "MacOS/huggler-desktop"
        links = subprocess.check_output(["otool", "-L", executable], text=True).splitlines()[1:]
        assert links and all(line.lstrip().startswith(("/usr/lib/", "/System/Library/")) for line in links), links
        print(f"{arch}, minimum macOS {info['LSMinimumSystemVersion']}, {len(links)} system libraries")
        print(f"{archive.name}: {archive.stat().st_size} zipped, {sum(files.values())} installed bytes")
        for path, size in sorted(files.items()):
            print(f"  {size:>10}  {path}")
        bridge = Path(staging) / f"huggler-{version}-pi.zip"
        with zipfile.ZipFile(bridge, "w", compression=zipfile.ZIP_DEFLATED) as z:
            for path in [ROOT / "adapters/pi/huggler.mjs", ROOT / "LICENSE"]:
                z.write(path, path.name)
        with zipfile.ZipFile(bridge) as z:
            assert z.testzip() is None
            assert z.read("huggler.mjs") == (ROOT / "adapters/pi/huggler.mjs").read_bytes()
            assert z.read("LICENSE") == (ROOT / "LICENSE").read_bytes()
            print(f"{bridge.name}: {bridge.stat().st_size} zipped, {sum(p.file_size for p in z.infolist())} installed bytes")
        if not package_only:
            launch_and_quit(installed, case)
        for artifact in (archive, bridge):
            artifact.replace(output / artifact.name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-only", action="store_true", help="verify archives without launching the app")
    parser.add_argument("output", type=Path, help="directory for the release ZIPs")
    options = parser.parse_args()
    check(options.output.resolve(), package_only=options.package_only)
