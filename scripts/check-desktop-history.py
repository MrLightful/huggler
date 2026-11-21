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
            {"resize": [640, 440]}, {"key": "cmd-shift-n"},
            {"click": "Git checkout"}, {"type": str(checkout)}, {"click": "Back to message"},
            {"click": "Start without a project"}, {"click": "First message"}, {"type": first},
            {"click": "Send first message"}, {"see": "thread/" + first},
            {"see": "Codex/reply " + first},
        ], env)
        with sqlite3.connect(starter_state / "huggler.sqlite3") as db:
            assert db.execute("SELECT text FROM message WHERE role='user'").fetchall() == [(first,)]
            root, = db.execute("SELECT root FROM project").fetchone()
            assert Path(root).parent == (starter_state / "scratch").resolve()
        run_window(executable, starter_state, [
            {"key": "cmd-n"}, {"key": "cmd-shift-n"}, {"click": "Git checkout"},
            {"type": str(checkout / "missing")}, {"click": "Back to message"},
            {"click": "First message"}, {"type": "Review my changes"}, {"key": "enter"},
            {"type": "Include the tests."}, {"key": "cmd-enter"}, {"contains": "Could not open"},
            {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"},
            {"type": str(checkout)}, {"click": "Back to message"}, {"click": "Send first message"},
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
            {"click": "section/Settled/1"}, {"click": "thread/" + first},
            {"see": "Codex/reply " + first}, {"click": "Unsettle/" + first},
            {"see": "Ready/" + first + "/Completed"}, {"click": "Settle/" + first},
            {"see": "Settled/" + first + "/Settled"}, *send_message("Continue"),
            {"see": "Codex/reply Continue"}, {"see": "Ready/" + first + "/Completed"},
            {"absent": "section/Settled/1"},
        ], env)
        attachment = checkout / "notes.txt"
        attachment.write_text("attachment fixture")
        other = checkout / "other" / "notes.txt"
        other.parent.mkdir()
        other.write_text("second file")
        attachment_state = checkout / "attachment-state"
        select_file = [{"next_file_selection": [str(attachment)]}, {"click": "Attach files"}]
        run_window(executable, attachment_state, [
            *select_file, {"click": "Remove notes.txt"}, {"absent": "Remove notes.txt"},
            *select_file, {"see": "Remove notes.txt"}, {"click": "Send first message"},
            {"see": "Ready/notes.txt/Completed"}, {"absent": "Remove notes.txt"},
            *select_file, {"see": "Remove notes.txt"}, {"click": "Create"},
            {"absent": "Remove notes.txt"}, {"click": "thread/notes.txt"}, {"see": "Remove notes.txt"},
            {"next_file_selection": [str(other)]}, {"click": "Attach files"},
            *send_message("Review"), {"absent": "Remove notes.txt"},
            {"see": "Ready/notes.txt/Completed"},
        ], env)
        with sqlite3.connect(attachment_state / "huggler.sqlite3") as db:
            messages = db.execute("SELECT id,text FROM message WHERE role='user' ORDER BY rowid").fetchall()
            expected = 'Text attachment "notes.txt"\nattachment fixture'
            assert [text.strip() for _, text in messages] == [
                expected, "Review\n\n" + expected + '\n\nText attachment "notes.txt"\nsecond file',
            ]
        attachment.unlink()
        other.unlink()
        first_file = f"attachment/{messages[0][0]}/0/notes.txt"
        second_message = f"attachment/{messages[1][0]}"
        plain = 'Text attachment "ordinary.txt"'
        run_window(executable, attachment_state, [
            {"see": first_file}, {"see": "You/Review"}, {"absent": "You/"},
            {"absent": "file preview/attachment fixture"}, {"click": first_file},
            {"see": "file preview/attachment fixture"}, {"click": first_file},
            {"absent": "file preview/attachment fixture"},
            {"click": second_message + "/1/notes.txt"}, {"see": "file preview/second file"},
            {"click": second_message + "/0/notes.txt"}, {"see": "file preview/attachment fixture"},
            {"absent": "file preview/second file"}, *send_message(plain), {"see": "You/" + plain},
        ], env)

        mixed_state = checkout / "mixed-attachment-state"
        fixtures = ROOT / "tests/fixtures/attachments"
        image = checkout / "diagram.png"
        pdf = checkout / "report.pdf"
        image.write_bytes((fixtures / image.name).read_bytes())
        pdf.write_bytes((fixtures / pdf.name).read_bytes())
        binary = checkout / "payload.bin"
        binary.write_bytes(b"\x00\xff")
        run_window(executable, mixed_state, [
            {"next_file_selection": [str(image), str(pdf), str(binary)]},
            {"click": "Attach files"}, {"see": "Remove diagram.png"}, {"see": "Remove report.pdf"},
            {"see": "Remove payload.bin"}, {"click": "Send first message"},
            {"see": "Ready/diagram.png/Completed"},
        ], env)
        image.unlink()
        pdf.unlink()
        binary.unlink()
        with sqlite3.connect(mixed_state / "huggler.sqlite3") as db:
            message, = db.execute("SELECT id FROM message WHERE role='user'").fetchone()
        prefix = f"attachment/{message}"
        saved_pdf = checkout / "saved.pdf"
        saved_pdf.write_bytes(b"replace me")
        pdf_bytes = (fixtures / "report.pdf").read_bytes()
        saved_binary = checkout / "saved.bin"
        run_window(executable, mixed_state, [
            {"click": prefix + "/0/diagram.png"}, {"see": "image preview/diagram.png"},
            {"see": "Save diagram.png"}, {"click": prefix + "/0/diagram.png"},
            {"click": prefix + "/1/report.pdf"}, {"see": "pdf preview/report.pdf"},
            {"next_save_path": str(saved_pdf)}, {"click": "Save report.pdf"},
            {"file_size": [str(saved_pdf), len(pdf_bytes)]}, {"click": prefix + "/1/report.pdf"},
            {"click": prefix + "/2/payload.bin"}, {"next_save_path": str(saved_binary)},
            {"click": "Save payload.bin"}, {"file_size": [str(saved_binary), 2]},
        ], env)
        assert saved_pdf.read_bytes() == pdf_bytes
        assert saved_binary.read_bytes() == b"\x00\xff"

        run_window(executable, checkout / "suggestion-state", [
            *launch_thread(checkout, "Follow-up"), *send_message("First question"),
            {"click": "Suggested reply"}, {"click": "Send"}, {"see": "You/Explain the result."},
            {"see": "Suggested reply"}, {"click": "Message"}, {"type": "My own question"},
            {"click": "Suggested reply"}, {"click": "Send"}, {"see": "You/My own question"},
        ], env)
    print("Native history paging, full message and suggested replies passed")


if __name__ == "__main__":
    main()
