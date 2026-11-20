#!/usr/bin/env python3
"""Build and install the native GPUI host as an ad-hoc signed macOS .app."""
import argparse
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

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
    if qualification:
        args.extend(["--features", "qualification"])
    subprocess.run(args, check=True)
    contents = destination / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    shutil.copy2(target / "release/huggler-desktop", contents / "MacOS/huggler-desktop")
    info = {
        "CFBundleExecutable": "huggler-desktop", "CFBundleIdentifier": identifier,
        "CFBundleName": "Huggler", "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": metadata["packages"][0]["version"],
        "CFBundleVersion": "1",
        "NSHighResolutionCapable": True, "LSMinimumSystemVersion": "11.0",
    }
    (contents / "Info.plist").write_bytes(plistlib.dumps(info))
    subprocess.run(["codesign", "--sign", "-", str(destination)], check=True)
    subprocess.run(["codesign", "--verify", "--strict", str(destination)], check=True)
    return contents / "MacOS/huggler-desktop"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    install(parser.parse_args().destination.absolute())
