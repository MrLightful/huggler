#!/usr/bin/env python3
"""Check or update the Huggler Rust crate version and its Cargo lockfile entries."""
import argparse
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]
LOCKFILES = (
    "Cargo.lock",
    "desktop/Cargo.lock",
    "bindings/python/Cargo.lock",
    "bindings/node/native/Cargo.lock",
)
VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", nargs="?", help="new MAJOR.MINOR.PATCH version (omit to check)")
    args = parser.parse_args()

    manifest = ROOT / "Cargo.toml"
    contents = manifest.read_text()
    package = tomllib.loads(contents)["package"]
    current = package["version"]
    if package["name"] != "huggler" or not VERSION.fullmatch(current):
        parser.error(f"expected a stable huggler crate version in {manifest.name}")

    version = current if args.version is None else args.version
    if not VERSION.fullmatch(version):
        parser.error("version must be MAJOR.MINOR.PATCH without leading zeros")
    if tuple(map(int, version.split("."))) < tuple(map(int, current.split("."))):
        parser.error(f"version cannot be lower than {current}")

    package_header = contents.index("[package]\n")
    version_line = contents.index(f'version = "{current}"\n', package_header)
    version_end = version_line + len(f'version = "{current}"')
    updates = {manifest: contents[:version_line] + f'version = "{version}"' + contents[version_end:]}
    for filename in LOCKFILES:
        path = ROOT / filename
        contents = path.read_text()
        local_entries = [
            entry for entry in tomllib.loads(contents)["package"]
            if entry["name"] == "huggler" and "source" not in entry
        ]
        old_entry = f'[[package]]\nname = "huggler"\nversion = "{current}"\n'
        if (
            len(local_entries) != 1
            or local_entries[0]["version"] != current
            or contents.count(old_entry) != 1
        ):
            parser.error(f"{filename} must contain one local huggler {current} entry")
        new_entry = f'[[package]]\nname = "huggler"\nversion = "{version}"\n'
        updates[path] = contents.replace(old_entry, new_entry, 1)

    if version != current:
        for path, contents in updates.items():
            path.write_text(contents)
        print(f"huggler {current} → {version}")
    else:
        print(f"huggler {current} matches all Cargo lockfiles")


if __name__ == "__main__":
    main()
