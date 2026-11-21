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
        committed_name = "branch-only.txt"
        subprocess.run(["git", "init", "-q", "-b", "main", checkout], check=True)

        def git(*args):
            return subprocess.check_output(["git", "-C", checkout, *args], text=True).strip()

        git("config", "user.name", "Fixture")
        git("config", "user.email", "fixture@example.test")
        git("config", "commit.gpgsign", "false")
        git("config", "core.hooksPath", os.devnull)
        git("commit", "--allow-empty", "-qm", "Base for review")
        git("switch", "-qc", "topic")
        env = isolated_env(root)
        # A host launched from another repository must still run the provider
        # against each conversation's selected worktree.
        subprocess.run(["git", "init", "-q", root], check=True)
        env.update(GIT_DIR=str(root / ".git"), GIT_WORK_TREE=str(root),
                   GIT_COMMON_DIR=str(root / ".git"), GIT_INDEX_FILE=str(root / ".git/index"))
        env["HUGGLER_CODEX"] = str(ROOT / "tests/fixtures/desktop-coding-peer.py")
        run_window(binary, state, [
            {"see": "First message"}, {"right_of": ["Send first message", "Attach files"]}, {"absent": "Conversation options"},
            *launch_thread(checkout, "Implementation"), {"click": "Show conversations"},
            {"see": "Ready/Implementation/New conversation"},
            {"resize": [1120, 740]},
            {"center_x": ["Resize conversations", 256]}, {"center_x": ["Resize changes", 820]},
            {"drag_x": ["Resize conversations", 40]}, {"center_x": ["Resize conversations", 296]},
            {"drag_x": ["Resize changes", 40]}, {"center_x": ["Resize changes", 860]},
            {"see": "Changes tab"}, {"absent": "Refresh changes"},
            {"click": "Hide changes"}, {"absent": "Changes tab"},
            {"drag_x": ["Resize conversations", 200]}, {"center_x": ["Resize conversations", 496]},
            {"drag_x": ["Resize conversations", -200]}, {"center_x": ["Resize conversations", 296]},
            {"click": "Show changes"}, {"see": "Changes tab"}, {"center_x": ["Resize changes", 860]},
            {"click": "section/Ready/1"}, {"absent": "thread/Implementation"},
            {"click": "section/Ready/1"}, {"see": "thread/Implementation"},
            {"right_of": ["Send", "Attach files"]}, {"absent": "Send plan"}, {"right_of": ["Create PR", "Open in"]},
            {"right_of": ["Hide changes", "Create PR"]},
            *send_message("Create the answer file."),
            {"see": "writing/Checking the requested filename. I will confirm the path with you before "
                    "creating the answer file in this checkout."},
            {"click": "Command 1 output"},
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
        (checkout / committed_name).write_text("committed on topic\n")
        git("add", "--", name, committed_name)
        git("commit", "-qm", "Produce the expected answer")
        (checkout / name).write_text("staged change\n")
        git("add", "--", name)
        (checkout / name).write_text("later edit\n")
        other = root / "other"
        subprocess.run(["git", "init", "-q", other], check=True)
        run_window(binary, state, [
            {"resize": [640, 440]}, {"click": "Show conversations"}, {"see": "Codex/Corrected the answer."},
            {"right_of": ["Create PR", "Open in"]},
            {"right_of": ["Show changes", "Create PR"]},
            {"absent": f"Changed/{name}"}, {"click": "Show changes"}, {"see": f"Changed/{name}"},
            {"click": f"Toggle diff/{name}"}, {"see": f"Diff/{name}"},
            {"click": f"Toggle diff/{name}"}, {"absent": f"Diff/{name}"},
            {"click": "Hide changes"}, {"resize": [1120, 740]}, {"click": "Show changes"},
            {"click": "Message"}, {"type": "Wait for my next instruction."},
            {"click": "Create"}, *launch_thread(other, "Review"),
            {"see": "No changes"}, {"absent": f"Changed/{name}"},
            {"click": "thread/Implementation"}, {"see": f"Changed/{name}"}, {"click": "Send"},
            {"see": "status/Running"}, {"click": "Message"}, {"type": "Review the staged result."},
            {"see": "Stop thread"}, {"absent": "Queue message"}, {"key": "enter"}, {"see": "1 queued message"},
            {"in_lower_half": "section/Working/1"}, {"see": "Working/Implementation/Running"},
            {"click": "thread/Review"}, {"absent": "thread/Implementation"}, {"click": "section/Working/1"},
            {"see": "Working/Implementation/Running"}, {"click": "section/Working/1"}, {"absent": "thread/Implementation"},
            {"click": "section/Working/1"}, {"click": "thread/Implementation"},
            {"see": "writing/Waiting for your instruction."}, {"click": "Stop thread"},
            {"see": "1 queued message · paused"}, {"see": "Needs attention/Implementation/Queue paused"},
        ], env)
        assert git("show", f":{name}") == "staged change"
        assert (checkout / name).read_text() == "later edit\n"
        other_name = "draft.txt"
        (checkout / other_name).write_text("keep this\n")
        run_window(binary, state, [
            {"click": "Show conversations"}, {"click": "Resume queue"}, {"see": "Codex/Reviewed the staged result."},
            {"click": "Create"}, *launch_thread(checkout, "Isolated review", branch="feature/review"),
            {"see": f"Changed/{committed_name}"},
            {"click": "All changes"}, {"see": "Change filter/Uncommitted files"},
            {"key": "down"}, {"key": "enter"},
            {"see": "Working tree clean"}, {"absent": f"Changed/{name}"},
            {"click": "thread/Implementation"}, {"see": f"Changed/{name}"},
            {"resize": [1120, 740]}, {"see": "All changes"}, {"see": f"Changed/{other_name}"},
            {"see": f"Changed/{committed_name}"}, {"hover": f"Changed/{committed_name}"},
            {"absent": f"Revert/{committed_name}"},
            {"click": f"Toggle diff/{name}"}, {"see": f"Diff/{name}"},
            {"click": f"Toggle diff/{name}"}, {"absent": f"Diff/{name}"},
            {"click": f"Changed/{name}"}, {"see": f"Diff/{name}"},
            {"click": "All changes"}, {"click": "Change filter/Uncommitted files"},
            {"see": "Uncommitted files"}, {"see": f"Diff/{name}"},
            {"absent": f"Changed/{committed_name}"}, {"see": f"Changed/{other_name}"},
            {"click": "Uncommitted files"}, {"click": "Change filter/All changes"},
            {"see": f"Changed/{committed_name}"}, {"see": f"Diff/{name}"},
            {"hover": f"Changed/{name}"}, {"click": f"Revert/{name}"},
            {"see": "Confirm file revert"}, {"key": "escape"},
            {"absent": "Confirm file revert"},
            {"file_size": [str(checkout / name), len("later edit\n")]},
            {"hover": f"Changed/{name}"}, {"click": f"Revert/{name}"},
            {"click": "Confirm file revert"}, {"see": f"Changed/{name}"},
            {"click": "All changes"}, {"click": "Change filter/Uncommitted files"},
            {"absent": f"Changed/{name}"}, {"see": f"Changed/{other_name}"},
            {"file_size": [str(checkout / other_name), len("keep this\n")]},
            {"hover": f"Changed/{other_name}"}, {"click": f"Revert/{other_name}"},
            {"click": "Confirm file revert"}, {"see": "Working tree clean"},
        ], env)
        assert git("status", "--short") == ""
        assert (checkout / name).read_text() == "42\n"
        assert not (checkout / other_name).exists()
    print("Native conversation, queue and sidebar file list passed with a scripted Codex peer")


def check_transcript(binary):
    with tempfile.TemporaryDirectory(prefix="huggler desktop transcript ") as tmp:
        root = Path(tmp)
        checkout = root / "checkout"
        state = root / "state"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        env = isolated_env(root)
        env["HUGGLER_CODEX"] = str(ROOT / "tests/fixtures/desktop-transcript-peer.py")
        run_window(binary, state, [
            *launch_thread(checkout, "Transcript review"),
            {"resize": [640, 550]},
            {"click": "Message"}, {"type": "Review the latest changes."}, {"key": "shift-enter"},
            {"type": "Summarize the checks."}, {"key": "enter"},
            {"see": "You/Review the latest changes.\nSummarize the checks."},
            {"see": "activity/2 tool calls · 2 messages"},
            {"contains": "commentary/I’ll check the transcript"}, {"see": "activity-item/docs / lookup"},
            {"resize": [1120, 740]},
            {"absent": "commentary/I’ll start with the conversation layout."},
            {"absent": "Command 1 output"},
            {"max_height": ["elapsed-time", 24]},
            {"click": "activity/2 tool calls · 2 messages"},
            {"see": "commentary/I’ll start with the conversation layout."}, {"see": "Command 1 output"},
            {"see": "Context compacted"}, {"contains": "Codex/## Review summary"},
            {"see": "activity/2 tool calls · 2 messages"}, {"see": "activity/1 tool call"},
            {"absent": "elapsed-time"},
            {"absent": "activity-item/docs / lookup"},
            {"click": "activity/2 tool calls · 2 messages"}, {"see": "activity-item/docs / lookup"},
            {"click": "activity/1 tool call"}, {"see": "activity-item/desktop/src/conversation.rs"},
        ], env)
        run_window(binary, state, [
            {"see": "Context compacted"}, {"contains": "Codex/## Review summary"},
            {"click": "activity/2 tool calls · 2 messages"}, {"click": "Command 1 output"},
            {"see": "command-output/All 12 checks passed."},
        ], env)
    print("Native transcript activity grouping and saved command output passed")


if __name__ == "__main__":
    binary = Path(sys.argv[1]).resolve()
    check(binary)
    check_transcript(binary)
