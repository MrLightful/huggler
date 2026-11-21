#!/usr/bin/env python3
"""Install the GPUI app and drive three live Codex UI turns across actual process exits."""
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile

from desktop_check import install_check, launch_thread, run_window, send_message


def check():
    provider = os.environ.get("HUGGLER_CODEX") or shutil.which("codex")
    if not provider:
        raise SystemExit("Install Codex or set HUGGLER_CODEX to its executable.")
    codex = Path(provider).resolve(strict=True)
    print(subprocess.check_output([codex, "--version"], text=True).strip(), flush=True)
    with tempfile.TemporaryDirectory(prefix="huggler desktop ") as tmp:
        case = Path(tmp)
        workspace = case / "checkout with spaces"
        subprocess.run(["git", "init", "-q", workspace], check=True)
        app = case / "Huggler Qualification.app"
        state = case / "state"
        executable = install_check(app)
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
        first = "provider thread resume fixture first turn complete"
        second = "provider thread resume fixture second turn complete"
        resumed = first + "\n" + second
        prompt = "Respond with exactly: " + first
        workflows = [
            [*launch_thread(workspace, "Resume check"), *send_message(prompt),
             {"see": "Codex/" + first}, {"see": "status/Completed"}],
            [{"see": "Codex/" + first}, *send_message(
                "Using the conversation history available in this resumed thread, first repeat the exact final "
                "answer you gave in the previous turn. Then on a new line write exactly: " + second
             ), {"see": "Codex/" + resumed}, {"see": "status/Completed"}],
            [{"see": "Codex/" + resumed}, {"click": "New conversation"},
             *launch_thread(workspace, "Independent thread"), {"absent": "Codex/" + first},
             *send_message(prompt), {"see": "Codex/" + first}, {"see": "status/Completed"},
             {"click": "thread/Resume check"}, {"see": "Codex/" + resumed},
             {"click": "thread/Independent thread"}, {"absent": "Codex/" + resumed}],
        ]
        for step, controls in enumerate(workflows, start=1):
            run_window(executable, state, controls, env)
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
        run_window(executable, state, [
            {"see": "Codex/" + first}, {"absent": "Codex/" + resumed},
            {"click": "thread/Resume check"}, {"see": "Codex/" + resumed},
        ], env)
        print("Launch 4 restored the selected sibling", flush=True)


if __name__ == "__main__":
    check()
