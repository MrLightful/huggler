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
        committed_name = "branch-only.txt"
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
        (checkout / ".git" / "info" / "exclude").write_text("/fixture-tree/\n")
        (checkout / "fixture-tree" / "empty").mkdir(parents=True)
        (checkout / "fixture-tree" / "nested").mkdir()
        (checkout / "fixture-tree" / "nested" / "entry.txt").write_text("Fixture attachment\n")
        markdown = "# Review notes\n\nKeep **source** intact.\n"
        edited_markdown = "# Updated review notes\n\nKeep **source** intact."
        markdown_path = "fixture-tree/nested/review.md"
        (checkout / markdown_path).write_text(markdown)
        (checkout / "fixture-tree" / "linked-entry").symlink_to("nested/entry.txt")
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
            *launch_thread(checkout, "Implementation", current=True),
            {"see": "Ready/Implementation/New conversation"},
            {"resize": [1120, 740]},
            {"center_x": ["Resize conversations", 288]}, {"center_x": ["Resize changes", 784]},
            {"drag_x": ["Resize conversations", 40]}, {"center_x": ["Resize conversations", 328]},
            {"drag_x": ["Resize changes", 40]}, {"center_x": ["Resize changes", 824]},
            {"see": "Changes tab"}, {"absent": "Refresh changes"},
            {"right_of": ["Changes tab", "All files tab"]},
            {"click": "All files tab"}, {"see": "Workspace file/fixture-tree"}, {"absent": "Workspace file/.git"},
            {"hover": "Workspace file/fixture-tree"},
            {"max_height": ["Attach to chat/fixture-tree", 18]}, {"max_height": ["Open in/fixture-tree", 18]},
            {"click": "Open in/fixture-tree"}, {"see": "Open Finder/fixture-tree"},
            {"see": "Open Terminal/fixture-tree"}, {"see": "Open VS Code/fixture-tree"},
            {"key": "escape"}, {"absent": "Open Finder/fixture-tree"}, {"absent": "Workspace file/fixture-tree/nested"},
            {"click": "Workspace file/fixture-tree"}, {"see": "Workspace file/fixture-tree/nested"},
            {"click": "Workspace file/fixture-tree/empty"}, {"see": "Empty folder"},
            {"click": "Workspace file/fixture-tree/nested"}, {"see": "Workspace file/fixture-tree/nested/entry.txt"},
            {"click": f"Workspace file/{markdown_path}"}, {"see": "Rendered Markdown"},
            {"click": "Copy document"}, {"clipboard_text": markdown},
            {"click": "Document mode/Raw"}, {"see": "Raw editor"},
            {"absent": "Rendered Markdown"}, {"click": "Raw editor"}, {"key": "cmd-a"},
            {"type": edited_markdown},
            {"file_size": [str(checkout / markdown_path), len(markdown)]},
            {"click": "Document mode/Preview"}, {"see": "Rendered Markdown"},
            {"click": "Document mode/Raw"}, {"see": "Raw editor"},
            {"click": "Copy document"}, {"clipboard_text": edited_markdown},
            {"key": "cmd-s"}, {"file_size": [str(checkout / markdown_path), len(edited_markdown)]},
            {"click": "Document mode/Preview"}, {"see": "Rendered Markdown"}, {"key": "cmd-w"},
            {"click": "Workspace file/fixture-tree/nested/entry.txt"},
            {"see": "Document text/Fixture attachment\n"}, {"absent": "Message"},
            {"click": "Document mode/Raw"}, {"see": "Raw editor"},
            {"click": "Raw editor"}, {"key": "cmd-a"}, {"type": "Unsaved plain text"},
            {"key": "cmd-w"}, {"see": "Discard document edits"}, {"click": "Keep editing"},
            {"click": "Copy document"}, {"clipboard_text": "Unsaved plain text"},
            {"key": "cmd-w"}, {"click": "Discard document edits"}, {"see": "Message"},
            {"click": "Workspace file/fixture-tree/nested/entry.txt"},
            {"see": "Document text/Fixture attachment\n"},
            {"right_of": ["File tab/fixture-tree/nested/entry.txt", "Chat tab"]},
            {"click": "Chat tab"}, {"see": "Message"}, {"absent": "Close Chat tab"},
            {"click": "Workspace file/fixture-tree/nested/entry.txt"},
            {"see": "Document text/Fixture attachment\n"}, {"absent": "Remove entry.txt"},
            {"click": "Open in/fixture-tree/nested/entry.txt"}, {"see": "Open Finder/fixture-tree/nested/entry.txt"},
            {"hover": "Open Finder/fixture-tree/nested/entry.txt"}, {"see": "Open Finder/fixture-tree/nested/entry.txt"},
            {"key": "escape"}, {"absent": "Open Finder/fixture-tree/nested/entry.txt"},
            {"hover": "Workspace file/fixture-tree/nested/entry.txt"},
            {"click": "Attach to chat/fixture-tree/nested/entry.txt"}, {"see": "Remove entry.txt"},
            {"click": "Close File tab/fixture-tree/nested/entry.txt"},
            {"absent": "File tab/fixture-tree/nested/entry.txt"}, {"see": "Message"},
            {"click": "Remove entry.txt"}, {"absent": "Remove entry.txt"},
            {"click": "Workspace file/fixture-tree"}, {"absent": "Workspace file/fixture-tree/nested/entry.txt"},
            {"drag_x": ["Resize all files", 136]}, {"center_x": ["Resize all files", 960]},
            {"hover": "Workspace file/fixture-tree"},
            {"click": "Open in/fixture-tree"}, {"see": "Open Finder/fixture-tree"},
            {"key": "escape"}, {"absent": "Open Finder/fixture-tree"},
            {"click": "Changes tab"}, {"see": "All changes"}, {"absent": "Workspace file/fixture-tree"},
            {"drag_x": ["Resize changes", -136]}, {"center_x": ["Resize changes", 824]},
            {"click": "Hide changes"}, {"absent": "Changes tab"},
            {"drag_x": ["Resize conversations", 200]}, {"center_x": ["Resize conversations", 528]},
            {"drag_x": ["Resize conversations", -200]}, {"center_x": ["Resize conversations", 328]},
            {"click": "Show changes"}, {"see": "Changes tab"}, {"center_x": ["Resize changes", 824]},
            {"click": "section/Ready/1"}, {"absent": "thread/Implementation"},
            {"click": "section/Ready/1"}, {"see": "thread/Implementation"},
            {"right_of": ["Send", "Attach files"]}, {"absent": "Send plan"},
            {"absent": "Checking PR…"}, {"absent": "Create PR"}, {"right_of": ["Hide changes", "Open in"]},
            {"click": "Message"}, {"type": "Create the answer file."},
            {"click": "All files tab"}, {"hover": "Workspace file/fixture-tree"}, {"click": "Attach to chat/fixture-tree"},
            {"absent": "Workspace file/fixture-tree/nested"}, {"click": "Workspace file/fixture-tree"},
            {"hover": "Workspace file/fixture-tree/linked-entry"}, {"click": "Attach to chat/fixture-tree/linked-entry"},
            {"click": "Changes tab"}, {"click": "Send"},
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
            {"click": "Hide conversations"}, {"absent": "thread/Implementation"},
        ], env)
        assert (checkout / markdown_path).read_text() == edited_markdown
        assert (checkout / "fixture-tree/nested/entry.txt").read_text() == "Fixture attachment\n"
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
            {"see": "thread/Implementation"}, {"see": "Codex/Corrected the answer."}, {"resize": [640, 440]},
            {"absent": "Checking PR…"}, {"absent": "Create PR"}, {"right_of": ["Show changes", "Open in"]},
            {"absent": f"Changed/{name}"}, {"click": "Show changes"}, {"see": f"Changed/{name}"},
            {"click": f"Open in/{name}"}, {"see": f"Open Finder/{name}"},
            {"see": f"Open Terminal/{name}"}, {"see": f"Open VS Code/{name}"},
            {"key": "escape"}, {"absent": f"Open Finder/{name}"},
            {"click": f"Changed/{name}"}, {"see": f"Document/{name}"},
            {"absent": "Changes tab"}, {"see": "Chat tab"},
            {"click": f"Close Diff tab/All changes/{name}"}, {"see": "Message"},
            {"resize": [1120, 740]}, {"click": "Show changes"},
            {"click": f"Changed/{name}"}, {"see": f"Document/{name}"}, {"click": "Chat tab"},
            {"click": "Message"}, {"type": "Wait for my next instruction."},
            {"click": "Create"}, *launch_thread(other, "Review"),
            {"see": "No changes"}, {"absent": f"Changed/{name}"},
            {"absent": f"Diff tab/All changes/{name}"},
            {"click": "All files tab"}, {"see": "No files in this workspace"},
            {"click": "thread/Implementation"}, {"see": f"Workspace file/{name}"},
            {"see": f"Diff tab/All changes/{name}"}, {"see": "Message"},
            {"click": "Changes tab"}, {"see": f"Changed/{name}"}, {"click": "Send"},
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
        expected_patch = subprocess.check_output([
            "git", "-C", checkout, "diff-index", "--patch", "--full-index", "--no-renames",
            "--no-color", "--no-ext-diff", "--no-textconv", "--ignore-submodules=dirty",
            default_commit, "--", committed_name,
        ], text=True)
        run_window(binary, state, [
            {"click": "Resume queue"}, {"see": "Codex/Reviewed the staged result."},
            {"click": "Create"}, {"next_file_selection": [str(checkout)]}, {"click": "Choose project"},
            {"see": "From main (remote)"}, {"click": "First message"}, {"type": "Please start from default @base-only"},
            {"see": "suggestion/base-only.txt"}, {"absent": "suggestion/base-only-link.txt"}, {"key": "enter"},
            {"click": "Send first message"}, {"see": "Codex/Started from the default branch."},
            {"see": "thread/Start from default base-only.txt"},
            {"click": "Create"}, *launch_thread(checkout, "Isolated review", branch="feature/review", base="origin/topic"),
            {"see": "Changed/base-only.txt"},
            {"click": "All changes"}, {"see": "Change filter/Uncommitted files"},
            {"key": "down"}, {"key": "down"}, {"key": "enter"},
            {"see": "Working tree clean"}, {"absent": f"Changed/{name}"},
            {"click": "thread/Implementation"}, {"see": f"Changed/{name}"},
            {"resize": [1120, 740]}, {"see": "All changes"}, {"see": f"Changed/{other_name}"},
            {"see": f"Changed/{committed_name}"}, {"hover": f"Changed/{committed_name}"},
            {"absent": f"Revert/{committed_name}"},
            {"click": f"Open in/{committed_name}"}, {"see": f"Open VS Code/{committed_name}"},
            {"absent": f"Open VS Code/{name}"}, {"key": "escape"},
            {"click": f"Changed/{committed_name}"}, {"see": "Diff line/+committed on topic"},
            {"absent": "Revert document"}, {"click": "Copy document"}, {"clipboard_text": expected_patch},
            {"key": "cmd-w"}, {"see": "Message"},
            {"click": f"Open in/{name}"}, {"see": f"Open VS Code/{name}"},
            {"absent": f"Open VS Code/{committed_name}"}, {"key": "escape"},
            {"click": f"Changed/{name}"}, {"see": f"Diff tab/All changes/{name}"},
            {"see": f"Document/{name}"}, {"contains": "+later edit"},
            {"click": f"Changed/{other_name}"}, {"see": f"Document/{other_name}"},
            {"contains": "+keep this"}, {"key": "cmd-w"},
            {"absent": f"Diff tab/All changes/{other_name}"}, {"see": f"Document/{name}"},
            {"key": "cmd-l"}, {"see": "Message"}, {"see": f"Diff tab/All changes/{name}"},
            {"click": "All changes"}, {"click": "Change filter/Uncommitted files"},
            {"see": "Uncommitted files"}, {"see": f"Changed/{name}"},
            {"absent": f"Changed/{committed_name}"}, {"see": f"Changed/{other_name}"},
            {"click": f"Changed/{name}"}, {"see": f"Diff tab/Uncommitted files/{name}"},
            {"see": "Diff line/-42"}, {"see": "Diff line/+later edit"},
            {"click": "Document mode/Patch"}, {"contains": "-42\n+later edit"},
            {"click": "Document mode/Unified"}, {"see": "Diff line/+later edit"}, {"key": "cmd-w"},
            {"absent": f"Diff tab/Uncommitted files/{name}"}, {"see": f"Document/{name}"},
            {"key": "cmd-w"}, {"absent": f"Diff tab/All changes/{name}"}, {"see": "Message"},
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
            {"click": f"Changed/{other_name}"}, {"see": "Diff line/+keep this"},
            {"click": "Document mode/Raw"}, {"see": "Raw editor"},
            {"click": "Copy document"}, {"clipboard_text": "keep this\n"},
            {"click": "Raw editor"}, {"key": "cmd-a"}, {"type": "edited from the diff"},
            {"click": "Save document"}, {"file_size": [str(checkout / other_name), len("edited from the diff")]},
            {"click": "Document mode/Unified"}, {"see": "Diff line/+edited from the diff"},
            {"click": "Revert document"}, {"see": "Confirm file revert"}, {"click": "Cancel file revert"},
            {"see": "Diff line/+edited from the diff"}, {"click": "Revert document"}, {"click": "Confirm file revert"},
            {"see": "Working tree clean"}, {"see": "This file has no changes in this comparison."},
            {"absent": "Revert document"},
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
            {"click": f"Changed/{other_name}"}, {"see": f"Diff tab/Uncommitted files/{other_name}"},
            {"contains": "+first edit"}, {"key": "cmd-w"}, {"see": "Message"},
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

        (checkout / name).write_text("staged change\n")
        git("add", "--", name)
        (checkout / name).write_text("later edit\n")
        with sqlite3.connect(state / "huggler.sqlite3") as db:
            roots = dict(db.execute("SELECT t.name,p.root FROM thread t JOIN project p ON t.project=p.id"))
        assert Path(roots["Implementation"]) == checkout.resolve()
        for thread, expected_commit, expected_branch in [
            ("Start from default base-only.txt", default_commit, "huggler/start-from-default-base-only-txt"),
            ("Isolated review", topic_commit, "feature/review"),
        ]:
            worktree = Path(roots[thread])
            assert worktree != checkout.resolve() and worktree.joinpath(".git").is_file()
            assert worktree.name.startswith(expected_branch.split("/")[-1] + "-")
            assert subprocess.check_output(["git", "-C", worktree, "rev-parse", "HEAD"], text=True).strip() == expected_commit
            assert subprocess.check_output(["git", "-C", worktree, "branch", "--show-current"], text=True).strip() == expected_branch
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


def check_editor_reload(binary):
    with tempfile.TemporaryDirectory(prefix="huggler desktop editor ") as tmp:
        root = Path(tmp)
        checkout = root / "checkout"
        subprocess.run(["git", "init", "-qb", "main", checkout], check=True)
        path = checkout / "notes.txt"
        path.write_text("Original notes\n")
        env = isolated_env(root)
        env["HUGGLER_CODEX"] = str(ROOT / "tests/fixtures/desktop-coding-peer.py")
        open_notes = {"click": "Workspace file/notes.txt"}
        raw = {"click": "Document mode/Raw"}
        run_window(binary, root / "state", [
            *launch_thread(checkout, "First editor", current=True),
            {"resize": [1120, 740]}, {"click": "All files tab"},
            open_notes, raw, {"see": "Raw editor"},
            {"key": "cmd-,"}, {"click": "Done"}, {"see": "Raw editor"},
            {"key": "cmd-a"}, {"type": "First edit"}, {"key": "cmd-s"},
            {"file_size": [str(path), len("First edit")]},
            {"click": "Document mode/Preview"}, {"see": "Document text/First edit"},
            *launch_thread(checkout, "Second editor", current=True),
            open_notes, raw, {"see": "Raw editor"}, {"click": "Raw editor"},
            {"key": "cmd-a"}, {"type": "Edited from another conversation"}, {"key": "cmd-s"},
            {"file_size": [str(path), len("Edited from another conversation")]},
            {"click": "thread/First editor"}, open_notes,
            {"see": "Document text/Edited from another conversation"},
            raw, {"see": "Raw editor"}, {"click": "Copy document"},
            {"clipboard_text": "Edited from another conversation"},
            {"click": "Raw editor"}, {"key": "cmd-a"}, {"type": "Unsaved draft"},
            {"click": "Document mode/Preview"}, open_notes, raw,
            {"click": "Copy document"}, {"clipboard_text": "Unsaved draft"},
            {"key": "cmd-w"}, {"click": "Discard document edits"}, {"see": "Message"},
        ], env)
        assert path.read_text() == "Edited from another conversation"
    print("Native Raw editor reload, draft preservation and focus restoration passed")


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
            {"absent": "message-duration/Duration unavailable"},
            {"click": "activity/2 tool calls · 2 messages"},
            {"see": "commentary/I’ll start with the conversation layout."}, {"see": "Command 1 output"},
            {"see": "Context compacted"}, {"contains": "Codex/## Review summary"},
            {"see": "activity/2 tool calls · 2 messages"}, {"see": "activity/1 tool call"},
            {"absent": "elapsed-time"},
            {"contains": "message-duration/"},
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
    check_editor_reload(binary)
    check_transcript(binary)
