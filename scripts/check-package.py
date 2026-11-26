#!/usr/bin/env python3
"""Package Huggler, inspect the archive, then compile an unrelated Cargo binary against it."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib
import tempfile

root = Path(__file__).resolve().parents[1]
target = root / "target"
subprocess.run(["cargo", "package", "--locked", "--allow-dirty", "--target-dir", target],
               cwd=root, check=True)
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root))
version = metadata["packages"][0]["version"]
name = f"huggler-{version}"
with tempfile.TemporaryDirectory(prefix="huggler independent ") as tmp:
    consumer = Path(tmp)
    artifact = target / "package" / f"{name}.crate"
    with tarfile.open(artifact) as archive:
        members = set(archive.getnames())
        # Every tracked fixture and migration must survive Cargo's include filters.
        tracked = subprocess.check_output(
            ["git", "ls-files", "-z", "--", "tests/fixtures", "migrations"], cwd=root
        ).decode().split("\0")
        reference = "knowledge/reference/codex-resume"
        required = {p for p in tracked if p} | {
            "Cargo.lock", "LICENSE", "src/providers/codex.rs", "tests/support/resume.rs",
            *(f"{reference}/{f}" for f in ("LICENSE", "cursors.json", "expected.txt", "codex_transcript.ndjson")),
        }
        missing = {p for p in required if f"{name}/{p}" not in members}
        assert not missing, sorted(missing)
        assert not any(p.startswith((name + "/desktop/", name + "/adapters/")) or p.endswith("/huggler.mjs") for p in members)
        assert not any("__pycache__" in Path(p).parts or p.endswith(".pyc") for p in members)
        archive.extractall(consumer, filter="data")
    package = consumer / name
    for path in required - {"Cargo.lock"}:
        assert (package / path).read_bytes() == (root / path).read_bytes(), path
    manifest = artifact.with_suffix(".contents.txt")
    manifest.write_text("".join(f"{p}\n" for p in sorted(members)))
    print(f"{artifact.name}: {len(members)} files, {artifact.stat().st_size} bytes; contents in {manifest}", flush=True)
    src = consumer / "src"
    src.mkdir()
    shutil.copy2(package / "tests/support/resume.rs", src / "resume.rs")
    (src / "main.rs").write_text('''mod resume;
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let source = std::env::args_os().nth(1).unwrap();
    resume::scenario(std::path::Path::new(&source)).await;
    println!("Packaged native resume, receipt retry and isolated sibling passed");
}
''')
    (consumer / "Cargo.toml").write_text(f'''[package]
name = "independent-huggler-consumer"
version = "{version}"
edition = "2024"

[dependencies]
huggler = {{ path = "{name}" }}
tempfile = "3"
serde_json = "1"
tokio = {{ version = "1", features = ["rt", "macros", "time"] }}
''')
    env = dict(os.environ, CARGO_TARGET_DIR=str(target / "consumer"))
    # Seed the independent resolver with the exact dependency versions shipped by Cargo.
    shutil.copy2(package / "Cargo.lock", consumer / "Cargo.lock")
    subprocess.run(["cargo", "run", "--", str(package)], cwd=consumer, env=env, check=True)
    def registry_packages(lock):
        return {(p["name"], p["version"], p["source"], p["checksum"])
                for p in tomllib.loads(lock.read_text())["package"] if "source" in p}

    extra = registry_packages(consumer / "Cargo.lock") - registry_packages(package / "Cargo.lock")
    assert not extra, f"Consumer resolved dependencies outside the packaged lock: {extra}"
