#!/usr/bin/env python3
"""Build and install the embedded Tauri host as an ad-hoc signed macOS .app."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def install(destination, *, identifier="dev.huggler.stage2", ui=None):
    if sys.platform != "darwin":
        raise SystemExit("This installer targets macOS.")
    if destination.exists():
        raise SystemExit(f"Select an unused install location: {destination}")
    target = ROOT / "target" / "desktop"
    env = os.environ.copy()
    config = {"identifier": identifier}
    if ui:
        config["build"] = {"frontendDist": str(ui)}
    env["TAURI_CONFIG"] = json.dumps(config)
    subprocess.run(["cargo", "build", "--locked", "--release", "--manifest-path",
                    str(ROOT / "desktop/Cargo.toml"), "--target-dir", str(target)], env=env, check=True)
    contents = destination / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    shutil.copy2(target / "release/huggler-desktop", contents / "MacOS/huggler-desktop")
    info = {
        "CFBundleExecutable": "huggler-desktop", "CFBundleIdentifier": identifier,
        "CFBundleName": "Huggler", "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": json.loads((ROOT / "desktop/tauri.conf.json").read_text())["version"],
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
