#!/usr/bin/env python3
"""Check Mermaid previews and source controls in the macOS transcript."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from desktop_check import ROOT, launch_thread, run_window, send_message


def main(executable):
    source = "flowchart LR\n    A[Read request] --> B[Render diagram]"
    streaming = f"```mermaid\n{source}"
    response = f"```mermaid\nunknown diagram\n```\n\n{streaming}\n```"
    with tempfile.TemporaryDirectory() as tmp:
        checkout = Path(tmp) / "checkout"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        env = dict(
            os.environ,
            HUGGLER_CODEX=str(ROOT / "tests/fixtures/desktop-images-peer.py"),
            HUGGLER_FIXTURE_IMAGE_STREAM=streaming,
            HUGGLER_FIXTURE_IMAGE_RESPONSE=response,
        )
        state = checkout.parent / "state"
        run_window(executable, state, [
            *launch_thread(checkout, "Mermaid preview"),
            *send_message("Draw the workflow"),
            {"see": "writing/" + streaming},
            {"absent": "Mermaid diagram"},
            {"see": "Ready/Mermaid preview/Completed"},
            {"see": "Diagram unavailable"},
            {"see": "Mermaid diagram"},
            {"min_height": ["Mermaid diagram", 30]},
            {"click": "Show source"},
            {"absent": "Mermaid diagram"},
            {"click": "Copy Mermaid source"},
            {"clipboard_text": source},
            {"click": "Show diagram"},
            {"see": "Mermaid diagram"},
        ], env)
        run_window(executable, state, [
            {"resize": [640, 540]},
            {"see": "Mermaid diagram"},
            {"min_height": ["Mermaid diagram", 20]},
        ], env)


if __name__ == "__main__":
    main(Path(sys.argv[1]).resolve())
