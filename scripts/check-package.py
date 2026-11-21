#!/usr/bin/env python3
"""Package Huggler, inspect the archive, then compile an unrelated Cargo binary against it."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
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
    with tarfile.open(target / "package" / f"{name}.crate") as archive:
        members = set(archive.getnames())
        fixtures = ("fx", "grok", "kiro", "registry", "muse", "pi", "open1", "open2")
        for required in ("Cargo.lock", "migrations/001.sql", "migrations/013.sql",
                         "docs/reference/codex-resume/LICENSE",
                         "tests/fixtures/providers/LICENSE", "docs/reference/codex-resume/codex_transcript.ndjson",
                         "src/providers/codex.rs", *(f"tests/fixtures/providers/{f}.ndjson" for f in fixtures),
                         "tests/fixtures/providers/antigravity-initialize.json",
                         "tests/fixtures/providers/antigravity-cli-startup.json",
                         "tests/fixtures/providers/cursor-cli-startup.json"):
            assert f"{name}/{required}" in members, required
        assert not any(p.startswith((name + "/desktop/", name + "/adapters/")) or p.endswith("/huggler.mjs") for p in members)
        assert not any("__pycache__" in Path(p).parts or p.endswith(".pyc") for p in members)
        archive.extractall(consumer, filter="data")
    package = consumer / name
    assert (package / "docs/reference/codex-resume/LICENSE").read_bytes() == (root / "docs/reference/codex-resume/LICENSE").read_bytes()
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
