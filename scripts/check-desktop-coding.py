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
        env["HUGGLER_CODEX"] = str(ROOT / "tests/fixtures/desktop-coding-peer.py")
        command = """value=$(cat 'answer [v1].txt'); if [ "$value" = 42 ]; then echo OK; else echo "expected 42, got $value"; exit 7; fi"""
        run_window(binary, state, [
            *launch_thread(checkout, "Implementation"),
            {"click": "Message"}, {"type": "Plan the answer file."}, {"click": "Send plan"},
            {"see": "Codex/Create the answer file and check its value."},
            *send_message("Create the answer file."),
            {"see": "writing/Checking the requested filename."}, {"click": "Command 1 output"},
            {"see": "command-output/Checkout ready"},
            {"click": "answer/filename"}, {"type": name}, {"click": "Submit answers"},
            {"click": "permission/allow_once"}, {"see": "Codex/Created the answer file."},
            {"click": "Message"}, {"type": "Use a single line."},
            {"click": "Changes"}, {"see": f"Unstaged/{name}"}, {"contains": "\n+41\n"}, {"click": "Stage file"},
            {"see": f"Staged/{name}"}, {"click": "Unstage file"}, {"see": f"Unstaged/{name}"},
            {"click": "Checks"}, {"click": "Check command"}, {"type": command}, {"click": "Run check"},
            {"see": "check/Failed · exit 7"}, {"see": "output/expected 42, got 41\n"},
            {"click": "Discuss check"}, {"click": "Send"}, {"see": "Codex/Corrected the answer."},
            {"click": "Checks"}, {"click": "Run check"}, {"see": "check/Passed · exit 0"},
            {"see": "output/OK\n"}, {"click": "Changes"}, {"see": f"Unstaged/{name}"},
            {"click": "Stage file"}, {"see": f"Staged/{name}"}, {"click": "Commit message"},
            {"type": "Produce the expected answer"}, {"click": "Commit staged"}, {"see": "Working tree clean"},
        ], env)
        assert git("log", "-1", "--format=%s") == "Produce the expected answer"
        assert git("show", f"HEAD:{name}") == "42"
        assert git("status", "--porcelain") == ""
        # Stage one version, then edit it again. The desktop commit must consume only the index.
        (checkout / name).write_text("staged change\n")
        git("add", "--", name)
        (checkout / name).write_text("later edit\n")
        run_window(binary, state, [
            {"resize": [640, 440]}, {"see": "Codex/Corrected the answer."}, {"click": "Changes"},
            {"click": f"Staged/{name}"}, {"contains": "\n+staged change\n"},
            {"click": f"Unstaged/{name}"}, {"contains": "\n+later edit\n"},
            {"click": "Commit message"}, {"type": "Commit only the index"}, {"click": "Commit staged"},
            {"absent": f"Staged/{name}"}, {"see": f"Unstaged/{name}"},
            {"click": "Conversation"}, {"click": "Message"}, {"type": "Wait for my next instruction."},
            {"click": "Checks"}, {"click": "Check command"}, {"type": "echo checking; exec sleep 120"},
            {"click": "Run check"}, {"see": "output/checking\n"}, {"click": "New conversation"},
            {"resize": [900, 660]}, *launch_thread(checkout, "Review"),
            {"click": "Message"}, {"type": "Separate review draft"}, {"click": "Checks"},
            {"absent": "output/checking\n"}, {"click": "thread/Implementation"}, {"click": "Checks"},
            {"see": "output/checking\n"}, {"click": "Stop check"}, {"see": "check/Stopped"},
            {"click": "Conversation"}, {"click": "Send"}, {"see": "You/Wait for my next instruction."},
            {"see": "status/Running"}, {"click": "Message"}, {"type": "Review the staged result."},
            {"click": "Queue message"}, {"see": "1 queued message"},
            {"see": "writing/Waiting for your instruction."}, {"click": "Stop thread"},
            {"see": "1 queued message · paused"},
        ], env)
        assert git("log", "-1", "--format=%s") == "Commit only the index"
        assert git("show", f"HEAD:{name}") == "staged change"
        assert (checkout / name).read_text() == "later edit\n"
        # A second conversation starts at the committed base in its own named
        # worktree, even while the original checkout holds uncommitted work.
        origin = root / "review remote.git"
        subprocess.run(["git", "init", "--bare", "-q", origin], check=True)
        git("remote", "add", "origin", str(origin))
        original = git("symbolic-ref", "HEAD")
        base = git("rev-parse", "HEAD")
        branch = "feature/isolated-review"
        run_window(binary, state, [
            {"see": "1 queued message · paused"}, {"click": "Resume queue"},
            {"see": "Codex/Reviewed the staged result."},
            {"click": "New conversation"}, *launch_thread(checkout, "Isolated review", branch=branch),
            {"click": "Changes"}, {"see": "Working tree clean"},
            {"click": "Checks"}, {"click": "Check command"},
            {"type": "echo verified > review.txt && git branch --show-current"}, {"click": "Run check"},
            {"see": "check/Passed · exit 0"}, {"see": f"output/{branch}\n"},
            {"click": "Changes"}, {"see": "Unstaged/review.txt"}, {"click": "Stage file"},
            {"see": "Staged/review.txt"}, {"click": "Commit message"}, {"type": "Verify isolated checkout"},
            {"click": "Commit staged"}, {"see": "Working tree clean"},
            {"click": "Publish"}, {"see": "remote/origin"}, {"click": "Push branch"},
            {"see": f"Pushed {branch} to origin."},
        ], env)
        assert git("symbolic-ref", "HEAD") == original and git("rev-parse", "HEAD") == base
        assert not (checkout / "review.txt").exists()
        assert (checkout / name).read_text() == "later edit\n"
        assert subprocess.check_output(["git", "-C", origin, "show", f"{branch}:review.txt"], text=True) == "verified\n"
        assert git("rev-parse", f"{branch}@{{upstream}}") == git("rev-parse", branch)
    print("Native coding cycle passed with a scripted Codex peer")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve())
