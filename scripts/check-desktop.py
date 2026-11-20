#!/usr/bin/env python3
"""Install the Tauri app and drive three live Codex UI turns across actual process exits."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("installer", ROOT / "scripts/install-desktop.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


def check():
    provider = os.environ.get("HUGGLER_CODEX") or shutil.which("codex")
    if not provider:
        raise SystemExit("Install Codex or set HUGGLER_CODEX to its executable.")
    codex = Path(provider).resolve(strict=True)
    print(subprocess.check_output([codex, "--version"], text=True).strip(), flush=True)
    tag = uuid.uuid4().hex[:12]
    identifier = "dev.huggler.qualify." + tag
    app = Path.home() / "Applications" / f"Huggler Qualification {tag}.app"
    state = Path.home() / "Library/Application Support" / identifier
    with tempfile.TemporaryDirectory(prefix="huggler desktop ") as tmp:
        case = Path(tmp)
        workspace = case / "checkout with spaces"
        subprocess.run(["git", "init", "-q", workspace], check=True)
        ui = case / "ui"
        shutil.copytree(ROOT / "desktop/ui", ui)
        (ui / "smoke.js").write_text("const SMOKE_WORKSPACE = " + json.dumps(str(workspace)) + ";\n"
                                     + (ROOT / "scripts/desktop-smoke.js").read_text())
        html = ui / "index.html"
        html.write_text(html.read_text().replace("</head>", '<script defer src="smoke.js"></script></head>'))
        executable = installer.install(app, identifier=identifier, ui=ui)
        print(f"Installed {app}\nState {state}", flush=True)
        bin_dir = case / "bin"
        bin_dir.mkdir()
        (bin_dir / "codex").symlink_to(codex)
        env = dict(os.environ, PATH=f"{bin_dir}:/usr/bin:/bin:/usr/sbin:/sbin")
        env.pop("HUGGLER_CODEX", None)
        assert not shutil.which("node", path=env["PATH"])
        bundle_files = [p.name for p in app.rglob("*") if p.is_file()]
        assert not any("node" in name.lower() for name in bundle_files)
        assert "node" not in subprocess.check_output(["otool", "-L", executable], text=True).lower()
        database = state / "huggler.sqlite3"
        for step in range(1, 4):
            # Start the installed executable with a GUI session, outside its source and checkout.
            with subprocess.Popen([executable], cwd="/", env=env) as process:
                try:
                    deadline = time.monotonic() + 120
                    while process.poll() is None:
                        if database.exists():
                            with sqlite3.connect(database) as db:
                                ready = db.execute("PRAGMA user_version").fetchone()[0] > 0
                                fail = ready and db.execute("SELECT error FROM run WHERE status='failed'").fetchone()
                                if fail:
                                    raise RuntimeError(f"Codex turn failed: {fail[0]}")
                        if time.monotonic() >= deadline:
                            raise TimeoutError(f"UI did not observe expected answer on launch {step}")
                        time.sleep(0.25)
                    assert process.returncode == 0
                finally:
                    if process.poll() is None:
                        process.terminate()
            with sqlite3.connect(database) as db:
                rows = db.execute("SELECT thread_id,ordinal,status,native_turn FROM run ORDER BY rowid").fetchall()
                bindings = db.execute("SELECT id,native_thread FROM thread ORDER BY rowid").fetchall()
                assert len(rows) == step and all(r[2] == "completed" for r in rows)
                assert db.execute("SELECT count(*) FROM turn_effect").fetchone() == (0,)
                assert all(row[1] for row in bindings) and all(row[3] for row in rows)
                if step == 1:
                    original = bindings[0]
                else:
                    assert original == bindings[0] and rows[0][0] == rows[1][0]
                    assert rows[1][1] == 2 and rows[0][3] != rows[1][3]
                if step == 3:
                    assert bindings[1][1] and bindings[1][1] != original[1]
                    assert rows[2][0] == bindings[1][0] and rows[2][1] == 1
            print(f"Launch {step} passed, {len(rows)} completed runs", flush=True)
        # The qualification bundle contains a DOM driver and embeds this temporary
        # checkout path. Leave the database available for independent inspection.
        shutil.rmtree(app)


if __name__ == "__main__":
    check()
