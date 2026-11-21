#!/usr/bin/env python3
"""Page and search a long local conversation in a macOS qualification window."""
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

from desktop_check import launch_thread, run_window


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
        ]
        run_window(executable, state, steps, os.environ)
    print("Native history paging, search and full message passed")


if __name__ == "__main__":
    main()
