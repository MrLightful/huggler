"""Shared controls for installed macOS desktop checks."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("installer", ROOT / "scripts/install-desktop.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


def install_check(app):
    return installer.install(app, identifier="dev.huggler.qualify." + uuid.uuid4().hex, qualification=True)


def isolated_env(case, *, codex_login=False):
    home = case / "home"
    home.mkdir()
    env = {k: v for k, v in os.environ.items() if not k.startswith("HUGGLER_")}
    env.update(HOME=str(home), PATH="/usr/bin:/bin:/usr/sbin:/sbin")
    if codex_login:
        env["CODEX_HOME"] = os.environ.get("CODEX_HOME") or str(Path.home() / ".codex")
    else:
        env.pop("CODEX_HOME", None)
    assert not shutil.which("node", path=env["PATH"])
    return env


def launch_thread(checkout, name, provider="Codex", *, branch=None):
    return [
        {"click": "Git checkout"}, {"type": str(checkout)}, {"click": provider},
        {"click": "Conversation name"}, {"type": name},
        *([{"click": "New worktree branch (optional)"}, {"type": branch}] if branch else []),
        {"click": "Open conversation"},
        {"see": "status/Ready"},
    ]


def send_message(prompt):
    return [{"click": "Message"}, {"type": prompt}, {"click": "Send"}]


def run_window(executable, state, steps, env):
    state.mkdir(parents=True, exist_ok=True)
    script = state / "workflow.json"
    script.write_text(json.dumps([*steps, {"key": "cmd-q"}]))
    env = dict(env, HUGGLER_UI_SCRIPT=str(script), HUGGLER_STATE_DIR=str(state))
    result = subprocess.run(
        [executable], cwd="/", env=env,
        check=True, timeout=150, stdout=subprocess.PIPE, text=True
    )
    if "GPUI workflow passed" not in result.stdout.splitlines():
        raise RuntimeError("The window closed before the workflow finished")
