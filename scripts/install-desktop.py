#!/usr/bin/env python3
"""Build and install the native GPUI host as an ad-hoc signed macOS .app."""
import argparse
import fcntl
import json
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile

from desktop_notices import write_notices

ROOT = Path(__file__).resolve().parents[1]


def install(destination, *, identifier="dev.huggler.stage2", qualification=False):
    if sys.platform != "darwin":
        raise SystemExit("This installer targets macOS.")
    if destination.exists():
        raise SystemExit(f"Select an unused install location: {destination}")
    target = ROOT / "target" / "desktop"
    manifest = ROOT / "desktop/Cargo.toml"
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--no-deps", "--format-version", "1", "--manifest-path", manifest
    ]))
    args = ["cargo", "build", "--locked", "--release", "--manifest-path", manifest, "--target-dir", target]
    features = ["--features", "qualification"] if qualification else []
    destination.parent.mkdir(parents=True, exist_ok=True)
    target.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as staging:
        bundle = Path(staging) / destination.name
        executable = bundle / "Contents/MacOS/huggler-desktop"
        executable.parent.mkdir(parents=True)
        # Cargo puts both build variants at the same path. Copy the one we requested
        # before a concurrent qualification or production install can replace it.
        with (target / "installer.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            messages = subprocess.check_output([*args, *features, "--message-format=json-render-diagnostics"])
            artifacts = (json.loads(line) for line in messages.splitlines())
            binary, = (msg["executable"] for msg in artifacts if msg["reason"] == "compiler-artifact"
                       and msg["target"]["name"] == "huggler-desktop" and msg["executable"])
            shutil.copy2(binary, executable)
        assemble(bundle, executable, metadata["packages"][0], identifier, features)
        bundle.rename(destination)
    return destination / "Contents/MacOS/huggler-desktop"


def assemble(bundle, executable, package, identifier, features):
    contents = bundle / "Contents"
    arch = subprocess.check_output(["lipo", "-archs", executable], text=True).strip()
    triple = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}[arch]
    build = subprocess.check_output(["vtool", "-show-build", executable], text=True)
    minimum = re.search(r"^\s*minos (\S+)$", build, re.MULTILINE)
    if not minimum:
        raise ValueError("Unable to read deployment target from the executable")
    write_notices(ROOT, contents / "Resources", triple, features)
    info = {
        "CFBundleExecutable": "huggler-desktop", "CFBundleIdentifier": identifier,
        "CFBundleName": "Huggler", "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": package["version"],
        "CFBundleVersion": package["metadata"]["macos"]["bundle-version"],
        "NSHighResolutionCapable": True, "LSMinimumSystemVersion": minimum[1],
    }
    (contents / "Info.plist").write_bytes(plistlib.dumps(info))
    subprocess.run(["codesign", "--sign", "-", str(bundle)], check=True)
    subprocess.run(["codesign", "--verify", "--strict", str(bundle)], check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--qualification", action="store_true", help="enable the scripted window driver")
    options = parser.parse_args()
    install(options.destination.absolute(), qualification=options.qualification)
