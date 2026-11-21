#!/usr/bin/env python3
"""Page and search a long local conversation in a macOS qualification window."""
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

from desktop_check import ROOT, launch_thread, run_window, send_message


def main():
    executable = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as tmp:
        checkout = Path(tmp)
        subprocess.run(["git", "init", "-q", tmp], check=True)
        state = checkout / "state"
        run_window(executable, state, launch_thread(checkout, "Long history"), os.environ)
        long_message = "a" * 8192 + " a unique ending"
        with sqlite3.connect(state / "huggler.sqlite3") as db:
            thread, = db.execute("SELECT id FROM thread").fetchone()
            for i in range(1000):
                db.execute("INSERT INTO run(id,thread_id,status,ordinal) VALUES(?,?,'completed',?)",
                           (str(i), thread, i + 1))
                db.execute("INSERT INTO message(id,run_id,role,text) VALUES(?,?,'assistant',?)",
                           (str(i), str(i), long_message if i == 5 else f"Answer {i}"))
        steps = [
            {"see": "Codex/Answer 999"}, {"absent": "Codex/Answer 967"},
            {"click": "Older"}, {"see": "Codex/Answer 967"}, {"absent": "Codex/Answer 999"},
            {"click": "Newest"}, {"see": "Codex/Answer 999"},
            {"click": "Search messages"}, {"type": "unique ending"}, {"click": "Search"},
            {"see": "expand/5"}, {"click": "expand/5"}, {"see": "Codex/" + long_message},
            {"click": "Clear search"}, {"see": "Codex/Answer 999"}, {"absent": "Clear search"},
            {"click": "Search messages"}, {"type": "Answer 732"}, {"key": "enter"},
            {"see": "Codex/Answer 732"}, {"absent": "Codex/Answer 999"},
        ]
        run_window(executable, state, steps, os.environ)
        env = dict(os.environ, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"),
                   HUGGLER_PROMPT_SUGGESTIONS="1")
        run_window(executable, checkout / "suggestion-state", [
            *launch_thread(checkout, "Follow-up"), *send_message("First question"),
            {"click": "Suggested reply"}, {"click": "Send"}, {"see": "You/Explain the result."},
            {"see": "Suggested reply"}, {"click": "Message"}, {"type": "My own question"},
            {"click": "Suggested reply"}, {"click": "Send"}, {"see": "You/My own question"},
        ], env)
    print("Native history paging, search, full message and suggested replies passed")


if __name__ == "__main__":
    main()
