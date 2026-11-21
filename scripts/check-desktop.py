#!/usr/bin/env python3
"""Install the GPUI app and drive three live Codex UI turns across actual process exits."""
import argparse
import os
from pathlib import Path
import plistlib
import shutil
import sqlite3
import subprocess
import tempfile

from desktop_check import installer, isolated_env, launch_thread, run_window, send_message


def check(previous_app):
    provider = os.environ.get("HUGGLER_CODEX") or shutil.which("codex")
    if not provider:
        raise SystemExit("Install Codex or set HUGGLER_CODEX to its executable.")
    codex = Path(provider).resolve(strict=True)
    print(subprocess.check_output([codex, "--version"], text=True).strip(), flush=True)
    with tempfile.TemporaryDirectory(prefix="huggler desktop ") as tmp:
        case = Path(tmp)
        workspace = case / "checkout with spaces"
        subprocess.run(["git", "init", "-q", workspace], check=True)
        app = case / "Applications" / "Huggler.app"
        current = case / "build" / "Huggler.app"
        installer.install(current, qualification=True)
        info_path = Path("Contents/Info.plist")
        new = plistlib.loads((current / info_path).read_bytes())
        if previous_app:
            previous = plistlib.loads((previous_app / info_path).read_bytes())
            assert previous["CFBundleIdentifier"] == new["CFBundleIdentifier"]
            assert int(previous["CFBundleVersion"]) < int(new["CFBundleVersion"])
            print(f"Upgrade {previous['CFBundleShortVersionString']} to {new['CFBundleShortVersionString']}", flush=True)
        shutil.copytree(previous_app or current, app)
        executable = app / "Contents/MacOS/huggler-desktop"
        subprocess.run(["codesign", "--verify", "--strict", app], check=True)
        env = isolated_env(case, codex_login=True)
        state = Path(env["HOME"]) / "Library/Application Support" / new["CFBundleIdentifier"]
        bin_dir = case / "bin"
        bin_dir.mkdir()
        (bin_dir / "codex").symlink_to(codex)
        env["PATH"] = f"{bin_dir}:{env['PATH']}"
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
            if step == 2 and previous_app:
                # Both bundles and the destination belong to this disposable install.
                app.rename(case / "previous.app")
                current.rename(app)
                subprocess.run(["codesign", "--verify", "--strict", app], check=True)
            run_window(executable, state, controls, env)
            with sqlite3.connect(database) as db:
                assert (state / "selected-thread").is_file()
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--previous-app", type=Path, help="signed older qualification bundle for the first launch")
    check(parser.parse_args().previous_app)
