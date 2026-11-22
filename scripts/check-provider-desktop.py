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
    checkout = tmp.resolve() / "model-checkout"
    state = tmp / "model-state"
    subprocess.run(["git", "init", "--quiet", checkout], check=True)
    env = dict(env, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"))
    default = "model-automatic"
    run_window(executable, state, [
        {"click": default}, {"see": "provider/available"}, {"see": "provider/Codex"},
        {"absent": "provider/Claude Agent"}, {"absent": "provider/unavailable"},
        {"key": "escape"}, {"click": "First message"},
        {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"}, {"type": str(checkout)},
        {"see": "provider-status/Codex app-server · version unknown · login unverified · 2 models"},
        {"click": default}, {"see": "model-option-automatic/name/Fixture Fast"},
        {"right_of": ["Edit", "Model search and results"]},
        {"see": "model-option/fixture-fast"}, {"type": "Fixture Deep"},
        {"absent": "model-option/fixture-fast"}, {"key": "enter"}, {"see": "model/fixture-deep"},
        {"click": "model/fixture-deep"}, {"click": "provider/available"}, {"see": default},
        {"see": "model-option/grok-build"}, {"absent": "model-option/fixture-deep"},
        {"key": "escape"}, {"click": default}, {"click": "provider/Codex"},
        {"click": "model-option/fixture-deep"}, {"click": "model/fixture-deep"},
        {"click": "Effort"}, {"click": "effort/high"}, {"click": "Fast mode off"}, {"see": "Fast mode on"},
        {"key": "escape"}, {"click": "Conversation name"}, {"type": "Models"},
        {"click": "Open conversation"}, {"see": "status/Ready"}, {"see": "model/fixture-deep"},
        *send_message("First turn"), {"see": "Ready/Models/Completed"},
        {"click": "model/fixture-deep"}, {"see": "Fast mode on"},
        {"click": "Fast mode on"}, {"see": "Fast mode off"},
        {"click": "Edit"}, {"see": "Models"}, {"see": "provider/codex"},
        {"click": "Done"}, *send_message("Standard turn"), {"see": "Codex/reply Standard turn"},
        {"see": "Ready/Models/Completed"},
        {"click": "Message"}, {"type": "Second turn"}, {"click": "model/fixture-deep"},
        {"click": "model-option/fixture-fast"}, {"see": "model/fixture-fast"},
        {"click": "model/fixture-fast"}, {"see": "Effort"}, {"absent": "Fast mode on"},
        {"absent": "Fast mode off"}, {"key": "escape"},
        {"click": "Send"}, {"see": "Codex/reply Second turn"}, {"see": "Ready/Models/Completed"},
    ], env)
    frames_path = checkout / "frames.ndjson"
    frames = [json.loads(line) for line in frames_path.read_text().splitlines()]
    assert sum(frame.get("method") == "model/list" for frame in frames) == 1
    run_window(executable, state, [
        {"see": "model/fixture-fast"}, {"click": "model/fixture-fast"},
        {"click": "model-option-automatic"}, {"see": default},
        *send_message("Default turn"), {"see": "Codex/reply Default turn"},
        {"see": "Ready/Models/Completed"},
    ], env)
    frames = [json.loads(line) for line in (checkout / "frames.ndjson").read_text().splitlines()]
    for methods in (("thread/start", "thread/resume"), ("turn/start",)):
        assert [f["params"].get("model") for f in frames if f.get("method") in methods] == [
            "fixture-deep", "fixture-deep", "fixture-fast", None]
    turns = [f["params"] for f in frames if f.get("method") == "turn/start"]
    assert [p.get("effort") for p in turns] == ["high", "high", None, None]
    assert [p.get("serviceTierForTurn") for p in turns] == ["priority", "default", None, None]
    assert turns[0]["collaborationMode"]["settings"]["reasoning_effort"] == "high"
    config = checkout / "codex-fixture-config.json"
    config.write_text(json.dumps({"model": "fixture-deep", "model_reasoning_effort": "high", "service_tier": "priority"}))
    run_window(executable, state, [
        {"click": default}, {"see": "model-option-automatic/name/Fixture Deep"},
        {"see": "Fast mode on"}, {"click": "Effort"},
        {"click": "effort/medium"}, {"click": "Effort"}, {"click": "effort/default"},
        {"key": "escape"}, *send_message("Configured defaults"),
        {"see": "Codex/reply Configured defaults"}, {"see": "Ready/Models/Completed"},
    ], env)
    frames = [json.loads(line) for line in frames_path.read_text().splitlines()]
    configured = [f["params"] for f in frames if f.get("method") == "turn/start"][-1]
    assert configured["model"] == "fixture-deep"
    assert configured["effort"] == configured["collaborationMode"]["settings"]["reasoning_effort"] == "high"
    assert "serviceTierForTurn" not in configured
    config.unlink()
    (checkout / "empty-model-list").touch()
    run_window(executable, state, [
        {"click": default}, {"see": "model-status/This provider listed no models."},
        {"see": "model-option-automatic/name/Automatic"},
        {"see": "Retry models"}, {"click": "model-option-automatic"},
        *send_message("Empty catalog"), {"see": "Codex/reply Empty catalog"},
        {"see": "Ready/Models/Completed"},
    ], env)


def check_suggestions(executable, tmp, env):
    checkout = tmp.resolve() / "suggestion-checkout"
    state = tmp / "suggestion-state"
    subprocess.run(["git", "init", "--quiet", checkout], check=True)
    (checkout / "docs").mkdir()
    (checkout / "docs" / "Setup (local).md").touch()
    (checkout / "docs" / "Setup.md").touch()
    (checkout / ".gitignore").write_text("private.md\n")
    (checkout / "private.md").touch()
    env = dict(env, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"))
    first = "$build-guide Read [Setup (local).md](docs/Setup%20%28local%29.md) first"
    followup = (
        "🛠 Use $lint-guide please with [Setup.md](docs/Setup.md) and "
        f"[$shared-skill]({checkout / 'user' / 'SKILL.md'}) again"
    )
    ready = "Ready/$build-guide Read Setup (local).md first/Completed"
    run_window(executable, state, [
        {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"},
        {"type": str(checkout / "docs")}, {"click": "Back to message"},
        {"click": "First message"}, {"type": "/build"}, {"see": "suggestion//build-guide"},
        {"key": "enter"}, {"type": "Read @setup"},
        {"see": "suggestion/Setup.md"}, {"see": "suggestion/Setup (local).md"},
        {"key": "down"}, {"key": "tab"}, {"type": "first"},
        {"key": "enter"}, {"see": ready},
        {"click": "Message"}, {"type": "Use /guide"},
        {"see": "suggestion//build-guide"}, {"see": "suggestion//lint-guide"},
        {"absent": "suggestion//disabled-guide"}, {"absent": "suggestion//other-checkout"},
        {"key": "escape"}, {"absent": "suggestion//lint-guide"},
        {"key": "cmd-a"}, {"type": "🛠 Use /lint please"},
        {"key": "alt-left"}, {"key": "left"}, {"see": "suggestion//lint-guide"},
        {"click": "suggestion//lint-guide"},
        {"key": "cmd-right"}, {"type": " with @private"}, {"see": "No matching files"},
        *[{"key": "backspace"}] * len("private"), {"type": "setup"},
        {"see": "suggestion/Setup.md"}, {"key": "enter"}, {"type": "and /shared"},
        {"see": "suggestion//shared-skill"}, {"key": "down"}, {"key": "enter"},
        {"type": "again"},
        {"key": "enter"}, {"see": f"You/{followup}"}, {"see": ready},
    ], env)
    frames = [json.loads(line) for line in (checkout / "frames.ndjson").read_text().splitlines()]
    assert [frame["params"]["input"][0]["text"] for frame in frames if frame.get("method") == "turn/start"] == [first, followup]


def check_mcp_status(executable, tmp, env):
    state = tmp / "mcp-state"
    state.mkdir()
    (state / "mcp-status-delay").touch()
    checkout = tmp.resolve() / "mcp-checkout"
    subprocess.run(["git", "init", "--quiet", checkout], check=True)
    env = dict(env, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"))
    trigger = "MCP server status"
    run_window(executable, state, [
        {"see": "First message"}, {"click": trigger},
        {"see": "MCP/Archive/0 tools"}, {"see": "MCP/Reference/1 tool"},
        {"see": "MCP/Notes/Needs auth"}, {"see": "MCP/Calendar/Error"},
        {"above": ["MCP servers", trigger]},
        {"key": "tab"}, "release_enter", {"see": "Checking MCP servers…"},
        {"key": "escape"}, {"absent": "MCP servers"},
        *launch_thread(checkout, "MCP status"),
        {"resize": [640, 440]}, {"click": trigger}, {"see": "MCP/Workspace Tools/3 tools"},
        {"above": ["MCP servers", trigger]}, {"key": "escape"}, {"absent": "MCP servers"},
    ], env)
    for cwd in (state, checkout):
        frames = [json.loads(line) for line in (cwd / "frames.ndjson").read_text().splitlines()]
        assert any(frame.get("method") == "mcpServerStatus/list" for frame in frames)
        assert all(frame.get("method") != "turn/start" for frame in frames)


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
            {"click": "model-automatic"}, {"absent": "provider/unavailable"},
            {"absent": "provider/Codex"}, {"see": "provider/available"},
            {"see": "model-status/Choose a provider to load models."},
            {"click": "Edit"}, {"see": "Installation guide"}, {"absent": "Models"},
            {"click": "Done"}, {"click": "model-automatic"},
            {"see": "model-status/Choose a provider to load models."}, {"click": "provider/available"},
            {"see": "model-option-automatic/name/Grok Build"},
            {"click": "model-option-automatic"}, {"absent": "model-option-automatic"},
            {"click": "Check provider"},
            {"see": status + "ACP v1 · 1 · login unverified · 2 models"},
        ]
        env = dict(isolated_env(tmp), HUGGLER_PROVIDERS=str(config), HUGGLER_CODEX=str(missing))
        check_models(executable, tmp, env)
        check_suggestions(executable, tmp, env)
        check_mcp_status(executable, tmp, env)
        state = tmp / "state"
        run_window(executable, state, steps, env)
        checkout = tmp / "original"
        moved = tmp / "moved"
        subprocess.run(["git", "init", "--quiet", checkout], check=True)
        # Launching from a Git hook must still register the selected checkout.
        subprocess.run(["git", "init", "--quiet", tmp], check=True)
        config.write_text(json.dumps([{"id": "unavailable", "driver": "cursor", "executable": str(missing)}]))
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
