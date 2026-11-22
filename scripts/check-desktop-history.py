#!/usr/bin/env python3
"""Page through a long local conversation in a macOS qualification window."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

from desktop_check import ROOT, launch_thread, run_window, send_message


def check_turn_rail(executable, checkout, env):
    state = checkout / "turn-rail-state"
    run_window(executable, state, launch_thread(checkout, "Turn navigation"), env)
    with sqlite3.connect(state / "huggler.sqlite3") as db:
        thread, = db.execute("SELECT id FROM thread").fetchone()
        for turn in range(1, 5):
            run = f"rail-{turn}"
            db.execute("INSERT INTO run(id,thread_id,status,ordinal) VALUES(?,?,'completed',?)",
                       (run, thread, turn))
            answer = f"## Response {turn}\n\nStart with the **upload handler**.\n\n" + "\n\n".join(
                f"Step {step}. Check the selected files and report which upload succeeded."
                for step in range(1, 14)
            )
            db.executemany("INSERT INTO message(id,run_id,role,text) VALUES(?,?,?,?)", [
                (f"{run}-question", run, "user", f"How should we handle upload {turn}?"),
                (f"{run}-answer", run, "assistant", answer),
            ])
        db.executemany("INSERT INTO run_activity(run_id,item_id,item,sequence) VALUES('rail-1',?,?,?)", [
            (f"record-{i}", json.dumps({
                "id": f"record-{i}",
                "kind": "commentary" if i == 0 else "tool",
                "title": "Checking upload handlers" if i == 0 else f"Read file {i}",
                "status": "completed",
            }), i + 1)
            for i in range(71)
        ])

    summary = "activity/70 tool calls · 1 message"
    run_window(executable, state, [
        {"see": "active-turn/4"}, {"absent": "Scroll to end"},
        {"resize": [640, 640]}, {"see": "thread/Turn navigation"},
        {"click": "Hide conversations"}, {"absent": "thread/Turn navigation"},
        {"absent": "turn/5"}, {"max_height": ["turn/1", 10]},
        {"hover": "Show conversations"}, {"absent": "wide-turn/4"},
        {"hover": "turn/1"}, {"see": "wide-turn/1"}, {"absent": "wide-turn/4"},
        {"see": "turn-question/How should we handle upload 1?"},
        {"contains": "turn-response/Response 1 Start with the upload handler."},
        {"click": "turn/1"}, {"in_upper_half": "You/How should we handle upload 1?"}, {"see": "active-turn/1"},
        {"see": summary}, {"absent": "Earlier activity omitted"},
        {"click": summary}, {"see": "Earlier activity omitted"}, {"click": summary},
        {"hover": "Show conversations"}, {"absent": "wide-turn/1"},
        {"click": "turn/3"}, {"in_upper_half": "You/How should we handle upload 3?"}, {"see": "active-turn/3"},
        {"center_x": ["Scroll to end", 320]}, {"click": "Scroll to end"},
        {"see": "active-turn/4"}, {"absent": "Scroll to end"},
        {"click": "Show conversations"}, {"see": "thread/Turn navigation"},
        {"click": "Hide conversations"}, {"see": "turn/3"},
    ], env)


def main():
    executable = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as tmp:
        checkout = Path(tmp)
        subprocess.run(["git", "init", "-q", tmp], check=True)
        check_turn_rail(executable, checkout, os.environ)
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
            {"see": "Codex/Answer 999"}, {"absent": "Codex/Answer 967"}, {"see": "active-turn/32"},
            {"click": "Older"}, {"see": "Codex/Answer 967"}, {"absent": "Codex/Answer 999"},
            {"see": "active-turn/1"}, {"see": "Newest"}, {"click": "Scroll to end"},
            {"see": "Codex/Answer 999"}, {"see": "active-turn/32"}, {"absent": "Scroll to end"},
            {"absent": "Search messages"}, {"click": "expand/998"}, {"see": "Codex/" + long_message},
        ]
        run_window(executable, state, steps, os.environ)
        run_window(executable, state, [
            {"resize": [640, 440]},
            {"center_x": ["Resize conversations", 240]}, {"click": "Create"},
            {"see": "First message"}, {"click": "thread/Long history"}, {"see": "active-turn/32"},
        ], os.environ)
        env = dict(os.environ, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"),
                   HUGGLER_PROMPT_SUGGESTIONS="1")
        starter_state = checkout / "starter-state"
        first = "Explain this project"
        run_window(executable, starter_state, [
            {"resize": [640, 440]}, {"key": "cmd-shift-n"},
            {"click": "Git checkout"}, {"type": str(checkout)}, {"click": "Back to message"},
            {"click": "Choose project"}, {"click": "Start without a project"}, {"click": "model-automatic"},
            {"see": "provider/Codex"}, {"see": "⌘L to focus"}, {"key": "cmd-l"},
            {"absent": "provider/Codex"}, {"absent": "⌘L to focus"},
            {"click": "model-automatic"}, {"see": "provider/Codex"},
            {"click": "Hide conversations"}, {"absent": "provider/Codex"}, {"see": "⌘L to focus"},
            {"key": "cmd-l"}, {"absent": "⌘L to focus"}, {"type": first}, {"click": "Show conversations"},
            {"click": "Send first message"}, {"see": "thread/" + first},
            {"see": "Codex/reply " + first},
        ], env)
        with sqlite3.connect(starter_state / "huggler.sqlite3") as db:
            assert db.execute("SELECT text FROM message WHERE role='user'").fetchall() == [(first,)]
            root, = db.execute("SELECT root FROM project").fetchone()
            assert Path(root).parent == (starter_state / "scratch").resolve()
        run_window(executable, starter_state, [
            {"key": "cmd-n"}, {"key": "cmd-shift-n"}, {"click": "Git checkout"},
            {"key": "cmd-l"}, {"type": str(checkout / "missing")}, {"click": "Back to message"},
            {"click": "First message"}, {"type": "Review my changes"}, {"key": "shift-enter"},
            {"type": "Include the tests."}, {"key": "enter"}, {"contains": "Could not open"},
            {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"},
            {"type": str(checkout)}, {"click": "Back to message"}, {"click": "Send first message"},
            {"see": "thread/Review my changes"}, {"see": "Ready/Review my changes/Completed"},
        ], env)
        with sqlite3.connect(starter_state / "huggler.sqlite3") as db:
            assert db.execute("SELECT text FROM message WHERE role='user' ORDER BY rowid").fetchall() == [
                (first,), ("Review my changes\nInclude the tests.",),
            ]
        run_window(executable, starter_state, [
            {"resize": [640, 440]},
            {"hover": "thread/" + first}, {"click": "Settle/" + first},
            {"in_lower_half": "section/Settled/1"}, {"absent": "thread/" + first},
            {"hover": "thread/Review my changes"}, {"click": "Settle/Review my changes"},
            {"absent": "section/Ready/1"}, {"in_lower_half": "section/Settled/2"},
            {"hover": "thread/Review my changes"}, {"click": "Unsettle/Review my changes"},
        ], env)
        run_window(executable, starter_state, [
            {"click": "section/Settled/1"}, {"click": "thread/" + first},
            {"see": "Codex/reply " + first}, {"hover": "thread/" + first}, {"click": "Unsettle/" + first},
            {"see": "Ready/" + first + "/Completed"}, {"hover": "thread/" + first}, {"click": "Settle/" + first},
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
        first_note = f"attachment/{messages[1][0]}/0/notes.txt"
        second_note = f"attachment/{messages[1][0]}/1/notes.txt"
        plain = 'Text attachment "ordinary.txt"'
        run_window(executable, attachment_state, [
            {"resize": [1080, 640]},
            {"see": first_file}, {"see": "You/Review"}, {"absent": "You/"},
            {"right_of": [first_note, "You/Review"]},
            {"same_line": [first_note, "You/Review"]},
            {"same_line": [second_note, "You/Review"]},
            {"absent": "file preview/attachment fixture"}, {"click": first_file},
            {"see": "file preview/attachment fixture"}, {"click": first_file},
            {"absent": "file preview/attachment fixture"},
            {"remember_x": second_note}, {"click": second_note}, {"see": "file preview/second file"},
            {"same_x": second_note},
            {"click": first_note}, {"see": "file preview/attachment fixture"},
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
        caption = "Request 🖼️\n\n" + "Ｗ" * 48 + "\nFiles:"
        caption_control = "You/" + caption
        run_window(executable, mixed_state, [
            {"click": "First message"}, {"type": caption},
            {"next_file_selection": [str(image), str(pdf), str(binary)]},
            {"click": "Attach files"}, {"see": "Remove diagram.png"}, {"see": "Remove report.pdf"},
            {"see": "Remove payload.bin"}, {"click": "Send first message"},
            {"see": "Ready/Request 🖼️/Completed"},
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
            {"resize": [640, 640]}, {"same_line": [prefix + "/0/diagram.png", caption_control]},
            {"right_of": [prefix + "/0/diagram.png", caption_control]},
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
            {"click": "Suggested reply"}, {"key": "tab"}, {"resize": [640, 440]},
            {"click": "Show changes"}, {"see": "Changes tab"}, {"see": "Resize changes"}, {"key": "cmd-l"},
            {"absent": "Changes tab"}, {"key": "cmd-right"},
            {"type": " about the tests"}, {"click": "Send"}, {"see": "You/My own question about the tests"},
        ], env)
    print("Native turn navigation, history paging, full message and suggested replies passed")


if __name__ == "__main__":
    main()
