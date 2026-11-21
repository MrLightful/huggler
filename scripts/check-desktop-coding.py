#!/usr/bin/env python3
"""Drive a complete local coding cycle in the native macOS qualification window."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from desktop_check import ROOT, isolated_env, launch_thread, run_window, send_message


def check(binary):
    with tempfile.TemporaryDirectory(prefix="huggler desktop coding ") as tmp:
        root = Path(tmp)
        checkout = root / "checkout"
        state = root / "state"
        name = "answer [v1].txt"
        subprocess.run(["git", "init", "-q", checkout], check=True)

        def git(*args):
            return subprocess.check_output(["git", "-C", checkout, *args], text=True).strip()

        git("config", "user.name", "Fixture")
        git("config", "user.email", "fixture@example.test")
        git("config", "commit.gpgsign", "false")
        git("config", "core.hooksPath", os.devnull)
        env = isolated_env(root)
        # A host launched from another repository must still run the provider
        # against each conversation's selected worktree.
        subprocess.run(["git", "init", "-q", root], check=True)
        env.update(GIT_DIR=str(root / ".git"), GIT_WORK_TREE=str(root),
                   GIT_COMMON_DIR=str(root / ".git"), GIT_INDEX_FILE=str(root / ".git/index"))
        env["HUGGLER_CODEX"] = str(ROOT / "tests/fixtures/desktop-coding-peer.py")
        run_window(binary, state, [
            {"see": "First message"}, {"right_of": ["Send first message", "Attach files"]}, {"absent": "Conversation options"},
            *launch_thread(checkout, "Implementation"),
            {"see": "Ready/Implementation/New conversation"},
            {"resize": [1120, 740]},
            {"center_x": ["Resize conversations", 256]}, {"center_x": ["Resize changes", 820]},
            {"drag_x": ["Resize conversations", 40]}, {"center_x": ["Resize conversations", 296]},
            {"drag_x": ["Resize changes", 40]}, {"center_x": ["Resize changes", 860]},
            {"click": "Hide changes"}, {"absent": "Refresh changes"},
            {"drag_x": ["Resize conversations", 200]}, {"center_x": ["Resize conversations", 496]},
            {"drag_x": ["Resize conversations", -200]}, {"center_x": ["Resize conversations", 296]},
            {"click": "Show changes"}, {"center_x": ["Resize changes", 860]},
            {"click": "section/Ready/1"}, {"absent": "thread/Implementation"},
            {"click": "section/Ready/1"}, {"see": "thread/Implementation"},
            {"right_of": ["Send", "Attach files"]}, {"absent": "Send plan"}, {"right_of": ["Create PR", "Open project"]},
            {"right_of": ["Hide changes", "Create PR"]},
            *send_message("Create the answer file."),
            {"see": "writing/Checking the requested filename. I will confirm the path with you before "
                    "creating the answer file in this checkout."}, {"click": "Command 1 output"},
            {"see": "command-output/Checkout ready"},
            {"see": "Needs attention/Implementation/Answer requested"},
            {"click": "answer/filename"}, {"type": name}, {"click": "Create"},
            {"click": "thread/Implementation"}, {"click": "Submit answers"},
            {"see": "Needs attention/Implementation/Approval requested"},
            {"click": "permission/allow_once"}, {"see": "Codex/Created the answer file."},
            {"see": "Ready/Implementation/Completed"}, {"absent": "status/Completed"},
            {"click": "Hide conversations"}, {"absent": "thread/Implementation"},
            {"right_of": [f"Changed/{name}", "Message"]},
            {"click": "Hide changes"}, {"absent": f"Changed/{name}"},
            {"click": "Show conversations"}, {"see": "thread/Implementation"},
            {"click": "Show changes"},
            {"center_x": ["Resize conversations", 296]}, {"center_x": ["Resize changes", 860]},
            {"right_of": [f"Changed/{name}", "Message"]}, {"absent": "Publish"}, {"absent": "Checks"},
            *send_message("Use a single line."), {"see": "Codex/Corrected the answer."},
        ], env)
        assert (checkout / name).read_text() == "42\n"
        git("add", "--", name)
        git("commit", "-qm", "Produce the expected answer")
        (checkout / name).write_text("staged change\n")
        git("add", "--", name)
        (checkout / name).write_text("later edit\n")
        other = root / "other"
        subprocess.run(["git", "init", "-q", other], check=True)
        run_window(binary, state, [
            {"resize": [640, 440]}, {"see": "Codex/Corrected the answer."},
            {"right_of": ["Create PR", "Open project"]},
            {"right_of": ["Show changes", "Create PR"]},
            {"absent": f"Changed/{name}"}, {"click": "Show changes"}, {"see": f"Changed/{name}"},
            {"click": "Hide changes"}, {"resize": [1120, 740]}, {"click": "Show changes"},
            {"click": "Message"}, {"type": "Wait for my next instruction."},
            {"click": "Create"}, *launch_thread(other, "Review"),
            {"see": "Working tree clean"}, {"absent": f"Changed/{name}"},
            {"click": "thread/Implementation"}, {"see": f"Changed/{name}"}, {"click": "Send"},
            {"see": "status/Running"}, {"click": "Message"}, {"type": "Review the staged result."},
            {"click": "Queue message"}, {"see": "1 queued message"},
            {"in_lower_half": "section/Working/1"}, {"see": "Working/Implementation/Running"},
            {"click": "thread/Review"}, {"absent": "thread/Implementation"}, {"click": "section/Working/1"},
            {"see": "Working/Implementation/Running"}, {"click": "section/Working/1"}, {"absent": "thread/Implementation"},
            {"click": "section/Working/1"}, {"click": "thread/Implementation"},
            {"see": "writing/Waiting for your instruction."}, {"click": "Stop thread"},
            {"see": "1 queued message · paused"}, {"see": "Needs attention/Implementation/Queue paused"},
        ], env)
        assert git("show", f":{name}") == "staged change"
        assert (checkout / name).read_text() == "later edit\n"
        run_window(binary, state, [
            {"click": "Resume queue"}, {"see": "Codex/Reviewed the staged result."},
            {"click": "Create"}, *launch_thread(checkout, "Isolated review", branch="feature/review"),
            {"see": "Working tree clean"}, {"absent": f"Changed/{name}"},
            {"click": "thread/Implementation"}, {"click": "Refresh changes"}, {"see": f"Changed/{name}"},
        ], env)
    print("Native conversation, queue and sidebar file list passed with a scripted Codex peer")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve())
