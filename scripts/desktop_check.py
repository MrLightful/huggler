"""Shared controls for installed macOS desktop checks."""
import importlib.util
import json
from pathlib import Path
import subprocess
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("installer", ROOT / "scripts/install-desktop.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


def install_check(app):
    return installer.install(app, identifier="dev.huggler.qualify." + uuid.uuid4().hex, qualification=True)


def launch_thread(checkout, name, provider="Codex"):
    return [
        {"click": "Git checkout"}, {"type": str(checkout)}, {"click": provider},
        {"click": "Conversation name"}, {"type": name}, {"click": "Open conversation"},
        {"see": "status/Ready"},
    ]


def send_message(prompt):
    return [{"click": "Message"}, {"type": prompt}, {"click": "Send"}]


def run_window(executable, state, steps, env):
    state.mkdir(parents=True, exist_ok=True)
    script = state / "workflow.json"
    script.write_text(json.dumps([*steps, {"click": "Quit"}]))
    result = subprocess.run(
        [executable], cwd="/", env=dict(env, HUGGLER_STATE_DIR=str(state), HUGGLER_UI_SCRIPT=str(script)),
        check=True, timeout=150, stdout=subprocess.PIPE, text=True
    )
    if "GPUI workflow passed" not in result.stdout.splitlines():
        raise RuntimeError("The window closed before the workflow finished")
