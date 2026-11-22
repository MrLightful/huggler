#!/usr/bin/env python3
"""Drive native settings and verify their effects on new and existing conversations."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

from desktop_check import ROOT, isolated_env, launch_thread, run_window


def check(executable):
    with tempfile.TemporaryDirectory(prefix="huggler settings ") as directory:
        root = Path(directory)
        checkout, state = root / "checkout", root / "state"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        env = isolated_env(root)
        fixture = ROOT / "tests/fixtures/context-provider.py"
        prefs = state / "preferences.json"
        run_window(executable, state, [
            *launch_thread(checkout, "Original"),
            {"key": "cmd-n"}, {"key": "cmd-shift-n"}, {"click": "Conversation name"}, {"type": "New default"},
            {"key": "cmd-,"}, {"click": "Dark"}, {"click": "default/claudeAgent"},
            {"click": "Codex executable"}, {"type": "relative/path"}, {"click": "Save path"},
            {"see": "settings-error/Enter the full path to the Codex executable."},
            {"click": "Codex executable"}, {"key": "cmd-a"}, {"type": str(fixture)},
            {"click": "Save path"}, {"click": "Done"},
            {"click": "Open conversation"}, {"see": "status/Ready"},
        ], env)
        assert json.loads(prefs.read_text()) == {
            "appearance": "dark", "provider": "claudeAgent", "codexExecutable": str(fixture),
        }
        # Hold checkout discovery until Settings saves a new appearance. This
        # makes navigation during launch deterministic even on fast filesystems.
        git = root / "git"
        git.write_text("""#!/bin/sh
case "$*" in
  *'rev-parse --show-toplevel')
    for i in $(seq 1 200); do
      grep -q '"light"' "$HUGGLER_STATE_DIR/preferences.json" && break
      sleep 0.1
    done
    grep -q '"light"' "$HUGGLER_STATE_DIR/preferences.json" || exit 1
    ;;
esac
exec /usr/bin/git "$@"
""")
        git.chmod(0o755)
        run_window(executable, state, [
            {"resize": [640, 440]}, {"key": "cmd-n"},
            {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"}, {"type": str(checkout)},
            {"click": "Conversation name"}, {"type": "After restart"},
            {"click": "Open conversation"}, {"key": "cmd-,"}, {"click": "Light"},
            {"see": "thread/After restart"}, {"click": "Done"}, {"see": "status/Ready"},
            {"click": "Show conversations"},
            {"click": "Settings"}, {"click": "Dark"}, {"click": "Done"},
            {"key": "cmd-n"}, {"key": "cmd-shift-n"}, {"click": "Conversation name"}, {"type": "Background"},
            {"click": "Open conversation"}, {"click": "thread/Original"},
            {"click": "Settings"}, {"click": "Light"}, {"see": "thread/Background"}, {"click": "Done"},
            {"click": "Message"}, {"type": "Review my draft"},
            {"click": "Settings"}, {"click": "Done"},
            {"click": "Show changes"}, {"see": "All changes"},
            {"click": "Hide changes"}, {"absent": "Changes tab"}, {"click": "Send"},
            {"see": "Codex/reply Review my draft"},
        ], dict(env, PATH=f"{root}:{env['PATH']}"))
        with sqlite3.connect(state / "huggler.sqlite3") as db:
            assert dict(db.execute("SELECT name,provider_instance FROM thread")) == {
                "Original": "codex", "New default": "claudeAgent", "After restart": "claudeAgent",
                "Background": "claudeAgent",
            }
            selected = (state / "selected-thread").read_text()
            assert db.execute("SELECT name FROM thread WHERE id=?", (selected,)).fetchone() == ("Original",)
        # A process-level override wins over the saved executable.
        run_window(executable, state, [
            {"key": "cmd-,"}, {"see": "Codex executable"}, {"absent": "Save path"},
            {"click": "Done"}, {"click": "Message"}, {"type": "Check override"}, {"click": "Send"},
            {"see": "status/Failed"},
        ], dict(env, HUGGLER_CODEX=str(root / "missing")))
        run_window(executable, state, [
            {"key": "cmd-,"}, {"click": "System"}, {"click": "Codex executable"},
            {"key": "cmd-a"}, {"key": "backspace"}, {"click": "Done"}, {"see": "Message"},
        ], env)
        assert json.loads(prefs.read_text()) == {
            "appearance": "system", "provider": "claudeAgent", "codexExecutable": None,
        }
    print("Native settings, shortcuts, drafts and provider persistence passed")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve())
