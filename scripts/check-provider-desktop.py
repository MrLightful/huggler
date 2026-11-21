#!/usr/bin/env python3
"""Exercise instance discovery in a macOS qualification build using isolated state."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

from desktop_check import ROOT, isolated_env, launch_thread, run_window, send_message


def check_models(executable, tmp, env):
    checkout = tmp / "model-checkout"
    state = tmp / "model-state"
    subprocess.run(["git", "init", "--quiet", checkout], check=True)
    env = dict(env, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"))
    default = "model/Provider default"
    run_window(executable, state, [
        {"click": default}, {"click": "provider/available"}, {"click": "provider/Codex"}, {"key": "escape"},
        {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"}, {"type": str(checkout)}, {"click": default},
        {"see": "model-option/fixture-fast"}, {"type": "deep"},
        {"absent": "model-option/fixture-fast"}, {"key": "enter"}, {"see": "model/fixture-deep"},
        {"click": "model/fixture-deep"}, {"click": "provider/available"}, {"see": default},
        {"see": "model-option/grok-build"}, {"absent": "model-option/fixture-deep"},
        {"key": "escape"}, {"click": default}, {"click": "provider/Codex"},
        {"click": "model-option/fixture-deep"}, {"click": "Conversation name"}, {"type": "Models"},
        {"click": "Open conversation"}, {"see": "status/Ready"}, {"see": "model/fixture-deep"},
        *send_message("First turn"), {"see": "Ready/Models/Completed"},
        {"click": "Message"}, {"type": "Second turn"}, {"click": "model/fixture-deep"},
        {"click": "model-option/fixture-fast"}, {"see": "model/fixture-fast"},
        {"click": "Send"}, {"see": "Codex/reply Second turn"}, {"see": "Ready/Models/Completed"},
    ], env)
    run_window(executable, state, [
        {"see": "model/fixture-fast"}, {"click": "model/fixture-fast"},
        {"click": "model-option/Provider default"}, {"see": default},
        *send_message("Default turn"), {"see": "Codex/reply Default turn"},
        {"see": "Ready/Models/Completed"},
    ], env)
    frames = [json.loads(line) for line in (checkout / "frames.ndjson").read_text().splitlines()]
    for methods in (("thread/start", "thread/resume"), ("turn/start",)):
        assert [f["params"].get("model") for f in frames if f.get("method") in methods] == [
            "fixture-deep", "fixture-fast", None]
    (checkout / "empty-model-list").touch()
    run_window(executable, state, [
        {"click": default}, {"see": "model-status/This provider listed no models."},
        {"see": "Retry models"}, {"click": "model-option/Provider default"},
        *send_message("Empty catalog"), {"see": "Codex/reply Empty catalog"},
        {"see": "Ready/Models/Completed"},
    ], env)


def main():
    executable = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="huggler discovery ") as directory:
        tmp = Path(directory)
        missing = tmp / "missing"
        config = tmp / "providers.json"
        config.write_text(json.dumps([
            {"id": "unavailable", "driver": "cursor", "executable": str(missing)},
            {"id": "available", "driver": "acpRegistry", "executable": sys.executable,
             "args": ["-B", str(ROOT / "tests/fixtures/providers/replay.py"), "registry"]},
        ]))
        status = "provider-status/"
        steps = [
            {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"}, {"type": str(tmp)},
            {"click": "model/Provider default"}, {"click": "provider/unavailable"}, {"click": "model-option/Provider default"}, {"absent": "model-option/Provider default"}, {"click": "Check provider"},
            {"see": status + "install the provider executable or configure its path: " + str(missing)},
            {"click": "model/Provider default"}, {"see": "Retry models"},
            {"click": "Retry models"}, {"see": "model-option/Provider default"}, {"key": "escape"},
            {"click": "model/Provider default"}, {"click": "provider/available"}, {"click": "model-option/Provider default"}, {"absent": "model-option/Provider default"}, {"click": "Check provider"},
            {"see": status + "ACP v1 · 1 · login unverified · 2 models"},
        ]
        env = dict(isolated_env(tmp), HUGGLER_PROVIDERS=str(config), HUGGLER_CODEX=str(missing))
        check_models(executable, tmp, env)
        state = tmp / "state"
        run_window(executable, state, steps, env)
        checkout = tmp / "original"
        moved = tmp / "moved"
        subprocess.run(["git", "init", "--quiet", checkout], check=True)
        # Launching from a Git hook must still register the selected checkout.
        subprocess.run(["git", "init", "--quiet", tmp], check=True)
        run_window(executable, state, [
            *launch_thread(checkout, "Original"), *send_message("Try Codex"),
            {"see": "status/Failed"},
        ], dict(env, GIT_DIR=str(tmp / ".git"), GIT_WORK_TREE=str(tmp)))
        with sqlite3.connect(state / "huggler.sqlite3") as db:
            assert db.execute("SELECT root FROM project").fetchall() == [(str(checkout.resolve()),)]
        checkout.rename(moved)
        run_window(executable, state, [
            {"see": "Checkout unavailable"}, {"see": "You/Try Codex"},
            {"click": "Choose another checkout"}, *launch_thread(moved, "Recovered"),
            {"absent": "Checkout unavailable"}, {"click": "thread/Original"},
            {"see": "Checkout unavailable"}, {"see": "You/Try Codex"},
        ], env)
        # Removing the inner .git leaves a directory inside a valid parent checkout.
        # The sidebar must report the missing inner repository.
        (moved / ".git").rename(tmp / "disconnected-git")
        run_window(executable, state, [
            {"click": "thread/Recovered"},
            {"contains": "not a git repository"},
        ], dict(env, LC_ALL="C"))
    print("Native window provider discovery passed")


if __name__ == "__main__":
    main()
