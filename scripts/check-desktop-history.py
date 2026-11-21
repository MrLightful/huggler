#!/usr/bin/env python3
"""Page through a long local conversation in a macOS qualification window."""
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
                           (str(i), str(i), long_message if i == 998 else f"Answer {i}"))
        steps = [
            {"see": "Codex/Answer 999"}, {"absent": "Codex/Answer 967"},
            {"click": "Older"}, {"see": "Codex/Answer 967"}, {"absent": "Codex/Answer 999"},
            {"click": "Newest"}, {"see": "Codex/Answer 999"},
            {"absent": "Search messages"}, {"click": "expand/998"}, {"see": "Codex/" + long_message},
        ]
        run_window(executable, state, steps, os.environ)
        env = dict(os.environ, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"),
                   HUGGLER_PROMPT_SUGGESTIONS="1")
        starter_state = checkout / "starter-state"
        first = "Explain this project"
        run_window(executable, starter_state, [
            {"resize": [640, 440]}, {"click": "Conversation options"},
            {"click": "Git checkout"}, {"type": str(checkout)}, {"click": "Conversation options"},
            {"click": "Start without a project"}, {"click": "First message"}, {"type": first},
            {"click": "Send first message"}, {"see": "thread/" + first},
            {"see": "Codex/reply " + first},
        ], env)
        with sqlite3.connect(starter_state / "huggler.sqlite3") as db:
            assert db.execute("SELECT text FROM message WHERE role='user'").fetchall() == [(first,)]
            root, = db.execute("SELECT root FROM project").fetchone()
            assert Path(root).parent == (starter_state / "scratch").resolve()
        run_window(executable, starter_state, [
            {"key": "cmd-n"}, {"click": "Conversation options"}, {"click": "Git checkout"},
            {"type": str(checkout / "missing")}, {"click": "Conversation options"},
            {"click": "First message"}, {"type": "Review my changes"}, {"key": "enter"},
            {"type": "Include the tests."}, {"key": "cmd-enter"}, {"contains": "Could not open"},
            {"click": "Conversation options"}, {"click": "Git checkout"}, {"key": "cmd-a"},
            {"type": str(checkout)}, {"click": "Conversation options"}, {"click": "Send first message"},
            {"see": "thread/Review my changes"}, {"see": "Ready/Review my changes/Completed"},
        ], env)
        with sqlite3.connect(starter_state / "huggler.sqlite3") as db:
            assert db.execute("SELECT text FROM message WHERE role='user' ORDER BY rowid").fetchall() == [
                (first,), ("Review my changes\nInclude the tests.",),
            ]
        run_window(executable, starter_state, [
            {"resize": [640, 440]}, {"click": "Settle/" + first},
            {"in_lower_half": "section/Settled/1"}, {"absent": "thread/" + first},
            {"click": "Settle/Review my changes"}, {"absent": "section/Ready/1"},
            {"in_lower_half": "section/Settled/2"}, {"click": "Unsettle/Review my changes"},
        ], env)
        run_window(executable, starter_state, [
            {"click": "Show settled"}, {"click": "thread/" + first},
            {"see": "Codex/reply " + first}, {"click": "Unsettle/" + first},
            {"see": "Ready/" + first + "/Completed"}, {"click": "Settle/" + first},
            {"see": "Settled/" + first + "/Settled"}, *send_message("Continue"),
            {"see": "Codex/reply Continue"}, {"see": "Ready/" + first + "/Completed"},
            {"absent": "section/Settled/1"},
        ], env)
        run_window(executable, checkout / "suggestion-state", [
            *launch_thread(checkout, "Follow-up"), *send_message("First question"),
            {"click": "Suggested reply"}, {"click": "Send"}, {"see": "You/Explain the result."},
            {"see": "Suggested reply"}, {"click": "Message"}, {"type": "My own question"},
            {"click": "Suggested reply"}, {"click": "Send"}, {"see": "You/My own question"},
        ], env)
    print("Native history paging, full message and suggested replies passed")


if __name__ == "__main__":
    main()
