#!/usr/bin/env python3
"""Drive a complete local coding cycle in the native macOS qualification window."""
import os
from pathlib import Path
import sqlite3
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
        subprocess.run(["git", "init", "-q", "-b", "main", checkout], check=True)

        def git(*args):
            return subprocess.check_output(["git", "-C", checkout, *args], text=True).strip()

        git("config", "user.name", "Fixture")
        git("config", "user.email", "fixture@example.test")
        git("config", "commit.gpgsign", "false")
        git("config", "core.hooksPath", os.devnull)
        (checkout / "base-only.txt").touch()
        (checkout / "base-only-link.txt").symlink_to("base-only.txt")
        git("add", "base-only.txt", "base-only-link.txt")
        git("commit", "-qm", "Default branch")
        default_commit = git("rev-parse", "HEAD")
        git("update-ref", "refs/remotes/origin/main", default_commit)
        git("symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main")
        git("switch", "-qc", "topic")
        git("rm", "-q", "base-only.txt", "base-only-link.txt")
        git("commit", "-qm", "Topic branch")
        topic_commit = git("rev-parse", "HEAD")
        git("update-ref", "refs/remotes/origin/topic", topic_commit)
        env = isolated_env(root)
        # A host launched from another repository must still run the provider
        # against each conversation's selected worktree.
        subprocess.run(["git", "init", "-q", root], check=True)
        env.update(GIT_DIR=str(root / ".git"), GIT_WORK_TREE=str(root),
                   GIT_COMMON_DIR=str(root / ".git"), GIT_INDEX_FILE=str(root / ".git/index"))
        env["HUGGLER_CODEX"] = str(ROOT / "tests/fixtures/desktop-coding-peer.py")
        env["HUGGLER_PROMPT_SUGGESTIONS"] = "1"
        run_window(binary, state, [
            {"see": "First message"}, {"right_of": ["Send first message", "Attach files"]}, {"absent": "Conversation options"},
            *launch_thread(checkout, "Implementation", current=True), {"click": "Show conversations"},
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
        git("add", "--", name)
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
            {"click": "Hide changes"}, {"resize": [1120, 740]}, {"click": "Show changes"},
            {"click": "Message"}, {"type": "Wait for my next instruction."},
            {"click": "Create"}, *launch_thread(other, "Review"),
            {"see": "Working tree clean"}, {"absent": f"Changed/{name}"},
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
        run_window(binary, state, [
            {"click": "Show conversations"}, {"click": "Resume queue"}, {"see": "Codex/Reviewed the staged result."},
            {"click": "Create"}, {"next_file_selection": [str(checkout)]}, {"click": "Choose project"},
            {"see": "From main (remote)"}, {"click": "First message"}, {"type": "Start from default @base-only"},
            {"see": "suggestion/base-only.txt"}, {"absent": "suggestion/base-only-link.txt"}, {"key": "enter"},
            {"click": "Send first message"}, {"see": "Codex/Started from the default branch."},
            {"click": "Create"}, *launch_thread(checkout, "Isolated review", branch="feature/review", base="origin/topic"),
            {"see": "Working tree clean"}, {"absent": f"Changed/{name}"},
            {"click": "thread/Implementation"}, {"click": "Refresh changes"}, {"see": f"Changed/{name}"},
        ], env)
        with sqlite3.connect(state / "huggler.sqlite3") as db:
            roots = dict(db.execute("SELECT t.name,p.root FROM thread t JOIN project p ON t.project=p.id"))
        assert Path(roots["Implementation"]) == checkout.resolve()
        for thread, expected_commit in [("Start from default [base-only.txt](base-only.txt)", default_commit),
                                        ("Isolated review", topic_commit)]:
            worktree = Path(roots[thread])
            assert worktree != checkout.resolve() and worktree.joinpath(".git").is_file()
            assert subprocess.check_output(["git", "-C", worktree, "rev-parse", "HEAD"], text=True).strip() == expected_commit
        assert git("branch", "--show-current") == "topic"
        assert git("rev-parse", "feature/review") == topic_commit
        git("branch", "checkout-review")
        conflicting = root / "conflicting"
        git("worktree", "add", "-qb", "ignored-file-conflict", str(conflicting))
        (conflicting / "base-only.txt").write_text("tracked on the target branch\n")
        git("-C", str(conflicting), "add", "base-only.txt")
        git("-C", str(conflicting), "commit", "-qm", "Track the local filename")
        git("worktree", "remove", str(conflicting))
        (checkout / ".git/info/exclude").write_text("base-only.txt\n")
        private_file = checkout / "base-only.txt"
        private_file.write_text("keep this local file\n")
        run_window(binary, state, [
            {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"}, {"type": str(checkout)},
            {"click": "Conversation name"}, {"type": "Current branch review"},
            {"see": "From main (remote)"}, {"click": "Choose workspace"}, {"click": "Current checkout"},
            {"click": "Choose checkout branch"}, {"see": "branch/checkout-review"},
            {"type": "checkout-review"}, {"key": "down"}, {"key": "enter"},
            {"see": "Switch to checkout-review"}, {"click": "Open conversation"}, {"see": "status/Ready"},
            {"key": "cmd-shift-n"}, {"click": "Conversation name"}, {"type": "Conflicting branch"},
            {"see": "From main (remote)"}, {"click": "Choose workspace"}, {"click": "Current checkout"},
            {"click": "Choose checkout branch"}, {"see": "branch/checkout-review"},
            {"type": "ignored-file-conflict"}, {"click": "branch/ignored-file-conflict"},
            {"see": "Switch to ignored-file-conflict"}, {"click": "Open conversation"},
            {"contains": "untracked working tree files would be overwritten"},
        ], env)
        with sqlite3.connect(state / "huggler.sqlite3") as db:
            roots = dict(db.execute("SELECT t.name,p.root FROM thread t JOIN project p ON t.project=p.id"))
        assert Path(roots["Current branch review"]) == checkout.resolve()
        assert "Conflicting branch" not in roots
        assert git("branch", "--show-current") == "checkout-review"
        assert private_file.read_text() == "keep this local file\n"
        assert git("show", f":{name}") == "staged change"
        assert (checkout / name).read_text() == "later edit\n"
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
