#!/usr/bin/env python3
"""Drive an offline ACP peer exchange in the installed macOS GPUI app."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

from desktop_check import ROOT, install_check, launch_thread, run_window, send_message


def check():
    with tempfile.TemporaryDirectory(prefix="huggler desktop peers ") as tmp:
        root = Path(tmp)
        checkout = root / "project"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        executable = install_check(root / "Huggler Qualification.app")
        fixture = str(ROOT / "tests/fixtures/peer-acp.py")
        env = dict(os.environ, PATH="/usr/bin:/bin:/usr/sbin:/sbin",
                   HUGGLER_CLAUDE_ACP=fixture, HUGGLER_CODEX=fixture)
        prompt = "create peers " + json.dumps({
            "clientRequestId": "review", "threads": [{"title": "Reviewer", "prompt": "Review 2+2."}],
        })
        run_window(executable, root / "state", [
            *launch_thread(checkout, "Author", "Claude Agent"), *send_message(prompt),
            {"click": "thread/Reviewer"}, {"see": "Author/Review 2+2."},
            {"click": "permission/allow_once"}, {"click": "permission/allow_once"},
            {"see": "Ready/Reviewer/Completed"}, {"click": "thread/Author"},
            {"see": "Reviewer/2+2=4"}, {"see": "Claude Agent/Received 2+2=4"},
        ], env)
        print("GPUI showed the new reviewer and attributed both sides of the peer exchange.")


if __name__ == "__main__":
    check()
