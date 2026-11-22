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
            {"center_x": ["Resize conversations", 288]}, {"center_x": ["Resize changes", 784]},
            {"drag_x": ["Resize conversations", 40]}, {"center_x": ["Resize conversations", 328]},
            {"drag_x": ["Resize changes", 40]}, {"center_x": ["Resize changes", 824]},
            {"see": "Changes tab"}, {"absent": "Refresh changes"},
            {"click": "Hide changes"}, {"absent": "Changes tab"},
            {"drag_x": ["Resize conversations", 200]}, {"center_x": ["Resize conversations", 528]},
            {"drag_x": ["Resize conversations", -200]}, {"center_x": ["Resize conversations", 328]},
            {"click": "Show changes"}, {"see": "Changes tab"}, {"center_x": ["Resize changes", 824]},
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
            {"center_x": ["Resize conversations", 328]}, {"center_x": ["Resize changes", 824]},
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
            {"click": f"Open in/{name}"}, {"see": f"Open Finder/{name}"},
            {"see": f"Open Terminal/{name}"}, {"see": f"Open VS Code/{name}"},
            {"key": "escape"}, {"absent": f"Open Finder/{name}"},
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
            {"key": "down"}, {"key": "down"}, {"key": "enter"},
            {"see": "Working tree clean"}, {"absent": f"Changed/{name}"},
            {"click": "thread/Implementation"}, {"see": f"Changed/{name}"},
            {"resize": [1120, 740]}, {"see": "All changes"}, {"see": f"Changed/{other_name}"},
            {"see": f"Changed/{committed_name}"}, {"hover": f"Changed/{committed_name}"},
            {"absent": f"Revert/{committed_name}"},
            {"click": f"Open in/{committed_name}"}, {"see": f"Open VS Code/{committed_name}"},
            {"absent": f"Open VS Code/{name}"}, {"key": "escape"},
            {"click": f"Open in/{name}"}, {"see": f"Open VS Code/{name}"},
            {"absent": f"Open VS Code/{committed_name}"}, {"key": "escape"},
            {"absent": f"Diff/{name}"},
            {"click": "All changes"}, {"click": "Change filter/Uncommitted files"},
            {"see": "Uncommitted files"}, {"see": f"Changed/{name}"},
            {"absent": f"Changed/{committed_name}"}, {"see": f"Changed/{other_name}"},
            {"click": "Uncommitted files"}, {"click": "Change filter/All changes"},
            {"see": f"Changed/{committed_name}"}, {"see": f"Changed/{name}"},
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

        (checkout / other_name).write_text("first edit\n")
        run_window(binary, state, [
            {"see": "All changes"}, {"see": f"Changed/{committed_name}"},
            {"hover": f"Changed/{committed_name}"}, {"click": f"Approve/{committed_name}"},
            {"see": f"Changed/{other_name}"}, {"absent": f"Changed/{committed_name}"},
            {"click": "All changes"}, {"click": "Change filter/Uncommitted files"},
            {"see": f"Changed/{other_name}"},
            {"hover": f"Changed/{other_name}"}, {"click": f"Approve/{other_name}"},
            {"see": "No changes to review"},
            {"click": "Uncommitted files"}, {"click": "Change filter/All changes"},
            {"see": f"Changed/{name}"}, {"absent": f"Changed/{other_name}"},
            {"click": "All changes"}, {"click": "Change filter/Approved"},
            {"see": f"Changed/{committed_name}"}, {"see": f"Changed/{other_name}"},
            {"click": f"Open in/{other_name}"}, {"see": f"Open Finder/{other_name}"},
            {"key": "escape"}, {"absent": f"Open Finder/{other_name}"},
        ], env)
        assert git("status", "--short") == f"?? {other_name}"
        assert (checkout / other_name).read_text() == "first edit\n"
        (checkout / other_name).write_text("later edit\n")
        run_window(binary, state, [
            {"see": f"Changed/{other_name}"}, {"absent": f"Changed/{committed_name}"},
            {"click": "All changes"}, {"click": "Change filter/Approved"},
            {"see": f"Changed/{committed_name}"}, {"absent": f"Changed/{other_name}"},
            {"hover": f"Changed/{committed_name}"}, {"click": f"Undo approval/{committed_name}"},
            {"see": "No approved files"},
            {"click": "Approved"}, {"click": "Change filter/All changes"},
            {"see": f"Changed/{committed_name}"}, {"see": f"Changed/{other_name}"},
        ], env)
        assert git("status", "--short") == f"?? {other_name}"

        (checkout / name).write_text("bulk staged edit\n")
        git("add", "--", name)
        (checkout / name).write_text("bulk working edit\n")
        protected = "approved-only.txt"
        (checkout / protected).write_text("keep this approved file\n")
        run_window(binary, state, [
            {"see": "All changes"}, {"see": f"Changed/{committed_name}"},
            {"click": "All changes"}, {"click": "Change filter/Uncommitted files"},
            {"see": "Uncommitted files"}, {"absent": f"Changed/{committed_name}"},
            {"see": f"Changed/{name}"}, {"see": f"Changed/{other_name}"},
            {"click": "File actions"}, {"click": "Approve all"},
            {"see": "No changes to review"},
            {"click": "Uncommitted files"}, {"click": "Change filter/Approved"},
            {"see": "Approved"}, {"see": f"Changed/{name}"}, {"see": f"Changed/{other_name}"},
            {"see": f"Changed/{protected}"}, {"absent": f"Changed/{committed_name}"},
            {"click": "File actions"}, {"click": "Undo all approvals"},
            {"see": "No approved files"},
            {"click": "Approved"}, {"click": "Change filter/Uncommitted files"},
            {"see": "Uncommitted files"}, {"hover": f"Changed/{protected}"},
            {"click": f"Approve/{protected}"}, {"absent": f"Changed/{protected}"},
            {"click": "File actions"}, {"click": "Revert all"},
            {"see": "Confirm revert all"}, {"click": "Cancel file revert"},
            {"absent": "Confirm revert all"},
            {"file_size": [str(checkout / name), len("bulk working edit\n")]},
            {"file_size": [str(checkout / other_name), len("later edit\n")]},
            {"click": "File actions"}, {"click": "Revert all"},
            {"click": "Confirm revert all"}, {"see": "No changes to review"},
        ], env)
        assert git("status", "--short") == f"?? {protected}"
        assert git("show", f":{name}") == "42"
        assert (checkout / name).read_text() == "42\n"
        assert not (checkout / other_name).exists()
        assert (checkout / protected).read_text() == "keep this approved file\n"
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
