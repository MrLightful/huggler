#!/usr/bin/env python3
"""Exercise the native terminal through the desktop sidebar and a local shell."""
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

from desktop_check import ROOT, install_check, isolated_env, launch_thread, run_window


def command(text):
    return [{"type": text}, {"key": "enter"}]


def check(binary, root):
    first = root / "first checkout"
    second = root / "second checkout"
    for checkout in (first, second):
        subprocess.run(["git", "init", "-q", checkout], check=True)
    env = isolated_env(root)
    env.update(
        HUGGLER_CODEX=str(ROOT / "tests/fixtures/desktop-coding-peer.py"),
        SHELL="/bin/bash",
        GIT_DIR=str(second / ".git"),
        GIT_WORK_TREE=str(second),
    )
    run_window(binary, root / "state", [
        *launch_thread(first, "First shell", current=True),
        {"resize": [1120, 740]},
        {"see": "Expand terminal"}, {"absent": "Terminal grid"},
        {"click": "Expand terminal"}, {"see": "Terminal grid"},
        {"above": ["Changes tab", "Resize terminal"]}, {"in_lower_half": "Terminal grid"},
        {"max_height": ["Terminal grid", 210]},
        *command('pwd > pwd.txt; git rev-parse --show-toplevel > git-root.txt; echo $$ > shell.pid'),
        {"file_text": [str(first / "pwd.txt"), f"{first.resolve()}\n"]},
        {"file_text": [str(first / "git-root.txt"), f"{first.resolve()}\n"]},
        *command('stty size > initial-size.txt; export HUGGLER_TEST_SHELL=first'),
        {"see": "Close Terminal 1"},
        {"drag_y": ["Resize terminal", -130]}, {"min_height": ["Terminal grid", 310]},
        *command('stty size > resized-size.txt'),
        {"click": "New terminal"}, {"see": "Terminal tab/Terminal 2"},
        *command('pwd > second-tab-pwd.txt; echo "${HUGGLER_TEST_SHELL:-fresh}" > second-tab.txt; '
                 'export HUGGLER_TEST_SHELL=second; echo $$ > second-tab.pid'),
        {"file_text": [str(first / "second-tab-pwd.txt"), f"{first.resolve()}\n"]},
        {"file_text": [str(first / "second-tab.txt"), "fresh\n"]},
        {"click": "Terminal tab/Terminal 1"},
        *command('echo "$HUGGLER_TEST_SHELL" > after-tab-switch.txt'),
        {"file_text": [str(first / "after-tab-switch.txt"), "first\n"]},
        {"click": "New terminal"}, {"see": "Terminal tab/Terminal 3"},
        {"right_of": ["New terminal", "Close Terminal 3"]},
        *command('echo $$ > third-tab.pid; echo ready > third-tab.txt'),
        {"file_text": [str(first / "third-tab.txt"), "ready\n"]},
        {"click": "Close Terminal 3"}, {"absent": "Terminal tab/Terminal 3"},
        *command('echo "$HUGGLER_TEST_SHELL" > after-active-close.txt'),
        {"file_text": [str(first / "after-active-close.txt"), "second\n"]},
        {"click": "Terminal tab/Terminal 1"}, {"click": "Close Terminal 2"},
        {"absent": "Terminal tab/Terminal 2"},
        *command('echo "$HUGGLER_TEST_SHELL" > after-inactive-close.txt'),
        {"file_text": [str(first / "after-inactive-close.txt"), "first\n"]},
        {"click": "All files tab"}, {"see": "All files tab"}, {"see": "Terminal grid"},
        {"click": "Collapse terminal"}, {"absent": "Terminal grid"},
        {"click": "Expand terminal"},
        *command('echo "$HUGGLER_TEST_SHELL" > after-collapse.txt'),
        {"file_text": [str(first / "after-collapse.txt"), "first\n"]},
        *launch_thread(second, "Second shell", current=True),
        {"see": "Expand terminal"}, {"absent": "Terminal grid"},
        {"click": "Expand terminal"},
        *command('pwd > pwd.txt; echo "${HUGGLER_TEST_SHELL:-fresh}" > session.txt; echo $$ > shell.pid'),
        {"file_text": [str(second / "pwd.txt"), f"{second.resolve()}\n"]},
        {"file_text": [str(second / "session.txt"), "fresh\n"]},
        *command("sleep 60 & echo $! > job.pid; echo started > job-ready.txt"),
        {"file_text": [str(second / "job-ready.txt"), "started\n"]},
        {"click": "Close Terminal 1"}, {"see": "No terminals open"},
        {"absent": "Terminal grid"},
        {"click": "Collapse terminal"}, {"click": "Expand terminal"},
        {"see": "No terminals open"},
        {"click": "New terminal"}, {"see": "Terminal tab/Terminal 2"},
        *command('echo $$ > restarted.pid; exit 7'),
        {"see": "Shell exited with code 7"}, {"click": "Restart terminal"},
        {"see": "Terminal tab/Terminal 2"}, {"absent": "Restart terminal"},
        *command('echo $$ > after-restart.pid; echo ready > after-restart.txt'),
        {"file_text": [str(second / "after-restart.txt"), "ready\n"]},
        {"click": "thread/First shell"}, {"see": "Terminal grid"},
        {"click": "Terminal grid"},
        *command('echo "$HUGGLER_TEST_SHELL" > after-switch.txt'),
        {"file_text": [str(first / "after-switch.txt"), "first\n"]},
        {"clipboard_write": "echo pasted > paste.txt"},
        {"key": "cmd-v"}, {"key": "enter"},
        {"file_text": [str(first / "paste.txt"), "pasted\n"]},
        *command('echo running > started.txt; sleep 60; echo $? > sleep-status.txt'),
        {"file_text": [str(first / "started.txt"), "running\n"]},
        {"key": "ctrl-c"},
        {"file_text": [str(first / "sleep-status.txt"), "130\n"]},
        *command(r"""printf '\033[31m\124erminal output\033[0m\n'; echo resumed > ctrl-c.txt"""),
        {"file_text": [str(first / "ctrl-c.txt"), "resumed\n"]},
        {"key": "cmd-a"}, {"key": "cmd-c"}, {"clipboard_contains": "Terminal output"},
        *command("""trap '' HUP; echo ready > quit-ready.txt"""),
        {"file_text": [str(first / "quit-ready.txt"), "ready\n"]},
    ], env)
    initial = (first / "initial-size.txt").read_text().split()
    resized = (first / "resized-size.txt").read_text().split()
    assert int(resized[0]) > int(initial[0]) > 0
    assert int(resized[1]) == int(initial[1]) > 0
    closed = int((second / "shell.pid").read_text())
    restarted = int((second / "restarted.pid").read_text())
    assert closed != restarted
    for pid in (closed, restarted, int((second / "after-restart.pid").read_text()),
                *(int((first / name).read_text()) for name in ("shell.pid", "second-tab.pid", "third-tab.pid"))):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            continue
        raise AssertionError(f"Terminal shell PID {pid} is still running")
    job = int((second / "job.pid").read_text())
    deadline = time.monotonic() + 2
    while True:
        try:
            os.kill(job, 0)
        except ProcessLookupError:
            break
        if time.monotonic() >= deadline:
            os.kill(job, signal.SIGTERM)
            raise AssertionError(f"Background job PID {job} survived Close shell")
        time.sleep(0.05)


def check_project_actions(binary, root):
    root.mkdir()
    checkout = root / "project"
    subprocess.run(["git", "init", "-qb", "main", checkout], check=True)
    subprocess.run(["git", "-C", checkout, "-c", "user.name=Huggler", "-c",
                    "user.email=check@example.test", "-c", "commit.gpgsign=false",
                    "commit", "--allow-empty", "-qm", "Base"], check=True)
    env = isolated_env(root)
    env.update(HUGGLER_CODEX=str(ROOT / "tests/fixtures/desktop-coding-peer.py"), SHELL="/bin/bash")
    state = root / "state"

    def add_action(name, text):
        return [
            {"click": "Add action"}, {"click": "Action name"}, {"type": name},
            {"click": "Action command"}, {"type": text}, {"click": "Save action"},
        ]

    original = "echo unexpected > unconfirmed.txt"
    multiline = "echo first > action.txt\necho second >> action.txt"
    run_window(binary, state, [
        *launch_thread(checkout, "Action project", current=True),
        {"resize": [1120, 740]}, {"click": "Expand terminal"},
        {"same_line": ["New terminal", "Terminal project actions"]},
        {"right_of": ["New terminal", "Close Terminal 1"]},
        {"right_of": ["Terminal project actions", "New terminal"]},
        {"click": "Terminal project actions"}, {"click": "Configure project actions"},
        *add_action("Run", original),
        *add_action("Check", multiline),
        {"click": "Back to workspace"},
        {"click": "Prefill Run"}, {"see": "Prefilled terminal command"},
        {"click": "Collapse terminal"}, {"absent": "Prefilled terminal command"},
        {"click": "Expand terminal"}, {"see": "Prefilled terminal command"},
        {"key": "cmd-a"}, {"key": "cmd-c"}, {"clipboard_text": original},
        {"type": "echo edited > edited.txt"}, {"key": "enter"},
        {"file_text": [str(checkout / "edited.txt"), "edited\n"]},
        {"click": "Terminal project actions"}, {"click": "Terminal action/Check"},
        {"see": "Prefilled terminal command"}, {"key": "escape"},
        {"absent": "Prefilled terminal command"},
        {"click": "Terminal project actions"}, {"click": "Configure project actions"},
        {"click": "Make default/Check"},
        {"click": "Edit action/Run"}, {"click": "Action name"}, {"key": "cmd-a"},
        {"type": "Run project"}, {"click": "Save action"}, {"click": "Back to workspace"},
        {"click": "Prefill Check"}, {"key": "cmd-a"}, {"key": "cmd-c"},
        {"clipboard_text": multiline}, {"key": "enter"},
        {"file_text": [str(checkout / "action.txt"), "first\nsecond\n"]},
    ], env)
    assert not (checkout / "unconfirmed.txt").exists()

    run_window(binary, state, [
        *launch_thread(checkout, "Action worktree", branch="action-worktree"),
        {"click": "Expand terminal"}, {"click": "Prefill Check"},
        {"see": "Prefilled terminal command"}, {"key": "cmd-a"}, {"key": "cmd-c"},
        {"clipboard_text": multiline}, {"key": "enter"}, {"absent": "Prefilled terminal command"},
        {"see": "Shell exited"},
        {"click": "Terminal project actions"}, {"see": "Terminal action/Run project"},
        {"click": "Configure project actions"}, {"click": "Remove action/Run project"},
        {"click": "Back to workspace"}, {"click": "Terminal project actions"},
        {"see": "Terminal action/Check"}, {"absent": "Terminal action/Run project"},
    ], env)
    output, = (state / "worktrees").glob("*/action.txt")
    assert output.read_text() == "first\nsecond\n"

    alternate = root / "alternate"
    subprocess.run(["git", "-C", checkout, "worktree", "add", "-qb", "alternate", alternate], check=True)
    alternate_alias = root / "alternate-link"
    alternate_alias.symlink_to(alternate, target_is_directory=True)
    run_window(binary, state, [
        {"click": "Settings"}, {"click": "Projects"},
        {"next_file_selection": [str(alternate)]}, {"click": "Add project"},
        {"click": "Project actions/alternate"},
        *add_action("Alternate", "echo alternate > alternate.txt"),
        {"click": "Back to Projects"}, {"click": "Back to Settings"}, {"click": "Back to workspace"},
        *launch_thread(alternate_alias, "Alternate action worktree", branch="alternate-action-worktree"),
        {"click": "Expand terminal"}, {"see": "Prefill Alternate"}, {"absent": "Prefill Check"},
    ], env)
    run_window(binary, state, [
        {"click": "Expand terminal"}, {"click": "Prefill Alternate"},
        {"see": "Prefilled terminal command"}, {"key": "enter"}, {"see": "Shell exited"},
    ], env)
    alternate_output, = (state / "worktrees").glob("*/alternate.txt")
    assert alternate_output.read_text() == "alternate\n"
    assert not (alternate / "alternate.txt").exists()


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="huggler desktop terminal ") as tmp:
        root = Path(tmp)
        binary = install_check(root / "Huggler.app")
        check(binary, root)
        check_project_actions(binary, root / "project actions")
    print("Desktop terminal workflow passed")
