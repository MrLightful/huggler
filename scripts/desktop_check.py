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


def launch_thread(checkout, name, provider="Codex", *, branch=None, base=None, current=False):
    return [
        {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"}, {"type": str(checkout)},
        *([{"click": "model-automatic"}, {"click": f"provider/{provider}"},
           {"click": "model-option-automatic"}] if provider != "Codex" else []),
        {"click": "Conversation name"}, {"type": name},
        *([{"click": "New worktree branch (optional)"}, {"type": branch}] if branch else []),
        *([{"see": "From main (remote)"}, {"click": "Choose base branch"},
           {"see": f"branch/{base}"}, {"type": base}, {"click": f"branch/{base}"}] if base else []),
        *([{"click": "Choose workspace"}, {"click": "Current checkout"}] if current else []),
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
    with subprocess.Popen(
        [executable], cwd="/", env=env,
        stdout=subprocess.PIPE, text=True
    ) as app:
        print(f"Checking Huggler (PID {app.pid})", flush=True)
        try:
            stdout, _ = app.communicate(timeout=150)
        finally:
            if app.poll() is None:
                app.terminate()
                try:
                    app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait()
            print(f"Huggler PID {app.pid} exited ({app.returncode})", flush=True)
        if app.returncode:
            raise subprocess.CalledProcessError(app.returncode, app.args, output=stdout)
    if "GPUI workflow passed" not in stdout.splitlines():
        raise RuntimeError("The window closed before the workflow finished")
    return stdout
