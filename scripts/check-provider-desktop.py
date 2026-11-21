#!/usr/bin/env python3
"""Exercise instance discovery in a macOS qualification build using isolated state."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from desktop_check import ROOT, isolated_env, launch_thread, run_window, send_message


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
            {"click": "Git checkout"}, {"type": str(tmp)},
            {"click": "unavailable"}, {"click": "Check provider"},
            {"see": status + "install the provider executable or configure its path: " + str(missing)},
            {"click": "available"}, {"click": "Check provider"},
            {"see": status + "ACP v1 · 1 · login unverified · 2 models"},
        ]
        env = dict(isolated_env(tmp), HUGGLER_PROVIDERS=str(config), HUGGLER_CODEX=str(missing))
        state = tmp / "state"
        run_window(executable, state, steps, env)
        checkout = tmp / "original"
        moved = tmp / "moved"
        subprocess.run(["git", "init", "--quiet", checkout], check=True)
        run_window(executable, state, [
            *launch_thread(checkout, "Original"), *send_message("Try Codex"),
            {"see": "status/Failed"},
        ], env)
        checkout.rename(moved)
        run_window(executable, state, [
            {"see": "Checkout unavailable"}, {"see": "You/Try Codex"},
            {"click": "Choose another checkout"}, *launch_thread(moved, "Recovered"),
            {"absent": "Checkout unavailable"}, {"click": "thread/Original"},
            {"see": "Checkout unavailable"}, {"see": "You/Try Codex"},
        ], env)
    print("Native window provider discovery passed")


if __name__ == "__main__":
    main()
