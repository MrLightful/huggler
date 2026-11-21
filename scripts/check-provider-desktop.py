#!/usr/bin/env python3
"""Exercise instance discovery in a macOS qualification build using isolated state."""
import json
from pathlib import Path
import sys
import tempfile

from desktop_check import ROOT, isolated_env, run_window


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
        run_window(executable, tmp / "state", steps, dict(isolated_env(tmp), HUGGLER_PROVIDERS=str(config)))
    print("Native window provider discovery passed")


if __name__ == "__main__":
    main()
