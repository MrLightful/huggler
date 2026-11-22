#!/usr/bin/env python3
"""Capture the macOS qualification app and optionally compare with a previous run."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile

from desktop_check import ROOT, isolated_env, run_window, send_message


def settings_steps(shot, checkout):
    return [
        {"click": "Settings"}, *shot("settings", "Appearance"),
        {"click": "General"}, *shot("settings-general", "Keep awake while agents run"),
        {"click": "Back to Settings"}, {"click": "Appearance"}, *shot("settings-appearance", "Dark"),
        {"click": "Back to Settings"}, {"click": "Storage"},
        *shot("settings-storage", "Clean up worktrees now"),
        {"click": "Back to Settings"}, {"click": "Projects"},
        *shot("settings-projects", f"Rename project/{checkout.name}"),
        {"click": f"Project actions/{checkout.name}"}, *shot("settings-project-actions", "Add action"),
        {"click": "Add action"}, *shot("project-action-editor", "Action command"),
        {"click": "Cancel action edit"}, {"click": "Back to Projects"},
        {"click": "Back to Settings"}, {"click": "Providers"},
        *shot("settings-providers", "Add provider"),
        {"click": "provider/codex"}, *shot("settings-provider", "model-enabled/fixture-fast"),
        *shot("provider-card", "provider/codex", component="provider/codex"),
        {"click": "Advanced settings"}, *shot("provider-advanced", "CLI executable"),
        {"click": "Add provider"}, *shot("provider-catalog", "Choose a provider"),
        {"click": "provider/claudeAgent"}, *shot("provider-installation", "Installation guide"),
        {"click": "Back to Settings"}, {"click": "Back to workspace"},
    ]


def workspace_steps(shot, checkout):
    return [
        {"resize": [1120, 740]}, *shot("starter", "First message"),
        *shot("starter-message-input", "First message", component="First message"),
        {"click": "Choose project"}, *shot("project-picker", f"project/{checkout.name}"),
        {"key": "escape"}, {"absent": f"project/{checkout.name}"},
        {"key": "cmd-shift-n"}, *shot("conversation-setup", "Git checkout"),
        {"click": "Choose project"}, {"click": f"project/{checkout.name}"},
        {"see": "From main"}, {"click": "Choose base branch"},
        *shot("branch-picker", "branch/main"), {"click": "branch/main"},
        {"click": "Choose workspace"}, *shot("workspace-picker", "Current checkout"),
        {"click": "Current checkout"},
        {"click": "Conversation name"}, {"type": "Review the workspace"}, {"click": "Open conversation"},
        *shot("conversation-ready", "status/Ready"),
        {"click": "model-automatic"}, *shot("model-picker", "model-option/fixture-fast"),
        {"click": "model-option-automatic"},
        *settings_steps(shot, checkout),
        {"click": "All files tab"}, {"click": "Workspace file/src"},
        *shot("file-tree", "Workspace file/src/lib.rs"),
        {"click": "Workspace file/guide.md"}, *shot("markdown-preview", "Rendered Markdown"),
        *shot("markdown-content", "Rendered Markdown", component="Rendered Markdown"),
        {"click": "Document mode/Raw"}, *shot("document-editor", "Raw editor"),
        {"key": "cmd-w"}, {"see": "Message"},
        {"click": "Changes tab"}, *shot("changes", "Changed/src/lib.rs"),
        {"click": "All changes"}, *shot("change-filter", "Change filter/Uncommitted files"),
        {"key": "escape"}, {"click": "Changed/src/lib.rs"},
        *shot("diff", "Diff line/+    42"),
        {"click": "Document mode/Patch"}, *shot("diff-patch", "Document mode/Unified"),
        {"key": "cmd-w"}, {"see": "Message"},
        {"hover": "Changed/src/lib.rs"}, {"click": "Revert/src/lib.rs"},
        *shot("confirm-file-revert", "Confirm file revert"), {"key": "escape"},
        {"click": "Expand terminal"}, {"see": "Terminal grid"},
        {"type": "echo Workspace check passed; echo ready > .git/terminal-ready"}, {"key": "enter"},
        {"file_text": [str(checkout / ".git/terminal-ready"), "ready\n"]},
        {"click": "New terminal"}, {"click": "Terminal tab/Terminal 1"},
        *shot("terminal", "Terminal tab/Terminal 2"),
        *shot("terminal-grid", "Terminal grid", component="Terminal grid"),
        {"click": "Collapse terminal"},
        {"click": "MCP server status"}, *shot("mcp-servers", "MCP/Workspace Tools/3 tools"),
        {"key": "escape"},
        *send_message("command output"), *shot("questions", "Submit answers"),
        {"click": "Command 1 output"}, {"contains": "command-output/"},
        *shot("command-output", "Command 1 output"), {"click": "Command 1 output"},
        {"click": "Message"}, {"type": "Review the result."}, {"click": "Queue message"},
        *shot("running-queue", "1 queued message"), {"click": "Dismiss questions"},
        {"see": "Codex/reply Review the result."},
        *send_message("callbacks"), *shot("permission", "permission/allow_once"),
        {"click": "Stop thread"}, {"see": "status/Interrupted"},
        {"click": "Message"}, {"type": "Review the reference files."},
        {"next_file_selection": [str(checkout / "diagram.png"), str(checkout / "report.pdf")]},
        {"click": "Attach files"}, {"see": "Remove diagram.png"},
        *shot("attachment-draft", "Remove report.pdf"),
        {"click": "Send"}, {"see": "You/Review the reference files."},
        *shot("conversation", "Ready/Review the workspace/Completed"),
        {"resize": [640, 640]}, *shot("conversation-compact", "Message"),
    ]


def capture(executable, output, appearance, case, baseline):
    scenes = set()

    def shot(name, ready, *, component=None):
        if name in scenes:
            raise ValueError(f"Duplicate screenshot name: {name}")
        scenes.add(name)
        options = {"name": name}
        if output is not None:
            options["path"] = str(output / f"{name}.png")
            if name in baseline:
                options["baseline"] = str(baseline[name])
        if component is not None:
            options["component"] = component
        return [{"see": ready}, {"screenshot": options}]

    state = case / "state"
    state.mkdir()
    checkout = case / "atlas"
    env = {key: value for key, value in isolated_env(case).items() if not key.startswith("GIT_")}
    env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
    subprocess.run(["git", "init", "-qb", "main", checkout], check=True, env=env)
    with (checkout / ".git/info/exclude").open("a") as exclude:
        exclude.write("\n/frames.ndjson\n")
    (checkout / "src").mkdir()
    source = checkout / "src/lib.rs"
    source.write_text("pub fn answer() -> u32 {\n    41\n}\n")
    (checkout / "guide.md").write_text("# Workspace guide\n\nRun **cargo test** before each review.\n\n- Create a branch\n- Review the diff\n")
    for name in ("diagram.png", "report.pdf"):
        shutil.copyfile(ROOT / "tests/fixtures/attachments" / name, checkout / name)
    subprocess.run(["git", "-C", checkout, "add", "."], check=True, env=env)
    subprocess.run([
        "git", "-C", checkout, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test",
        "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "-qm", "Fixture",
    ], check=True, env=env)
    source.write_text("pub fn answer() -> u32 {\n    42\n}\n")
    (state / "preferences.json").write_text(json.dumps({
        "appearance": appearance, "askForAgentApproval": True,
        "projects": [{"name": checkout.name, "root": str(checkout)}],
    }))
    codex = case / "codex"
    codex.write_text("""#!/bin/sh
case "$*" in
    'login status') echo 'Logged in using ChatGPT'; exit 0;;
    '--version') echo 'codex-cli 0.170.0'; exit 0;;
esac
exec "$HUGGLER_SCREENSHOT_CODEX_FIXTURE" "$@"
""")
    codex.chmod(0o755)
    shell = case / "shell"
    shell.write_text("#!/bin/sh\nexport PS1='atlas $ '\nexec /bin/bash --noprofile --norc -i\n")
    shell.chmod(0o755)
    env.update(
        HUGGLER_CODEX=str(codex),
        HUGGLER_CLAUDE_ACP=str(case / "missing-claude-agent"),
        HUGGLER_SCREENSHOT_CODEX_FIXTURE=str(ROOT / "tests/fixtures/context-provider.py"),
        SHELL=str(shell),
    )
    first = run_window(executable, state, workspace_steps(shot, checkout), env)

    with sqlite3.connect(state / "huggler.sqlite3") as db:
        message, = db.execute(
            "SELECT id FROM message WHERE role = 'user' AND text LIKE ?", ("Review the reference files.%",),
        ).fetchone()
    attachment = f"attachment/{message}"
    second = run_window(executable, state, [
        {"resize": [1120, 740]}, {"click": f"{attachment}/0/diagram.png"},
        *shot("image-attachment", "image preview/diagram.png"),
        *shot("attachment-image", "image preview/diagram.png", component="image preview/diagram.png"),
        {"click": f"{attachment}/0/diagram.png"}, {"click": f"{attachment}/1/report.pdf"},
        *shot("pdf-attachment", "pdf preview/report.pdf"),
        *shot("attachment-pdf-text", "pdf preview/report.pdf", component="pdf preview/report.pdf"),
    ], env)
    if output is not None:
        actual = {path.stem for path in output.glob("*.png")}
        if missing := scenes - actual:
            raise RuntimeError(f"Missing screenshots: {', '.join(sorted(missing))}")
        if unexpected := actual - scenes:
            raise RuntimeError(f"Unexpected screenshots: {', '.join(sorted(unexpected))}")
    changed = {line.removeprefix("Screenshot changed: ")
               for stdout in (first, second) for line in stdout.splitlines()
               if line.startswith("Screenshot changed: ")}
    return scenes, changed


def report_comparison(current, previous, changed):
    for label, names in [("Added", current - previous), ("Removed", previous - current),
                         ("Changed", changed)]:
        for name in sorted(names):
            print(f"{label}: {name}.png")
    return not (changed or current != previous)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path, help="desktop binary built with --features qualification")
    parser.add_argument("output", type=Path, nargs="?", help="new or empty directory for PNGs")
    parser.add_argument("--appearance", choices=("light", "dark"), default="dark")
    parser.add_argument("--baseline", type=Path, help="previous capture from the same macOS setup")
    parser.add_argument("--check-only", action="store_true", help="verify scene checkpoints without taking screenshots")
    args = parser.parse_args()
    if args.check_only and (args.output or args.baseline):
        parser.error("--check-only does not take an output or baseline directory")
    if not args.check_only and not args.output:
        parser.error("output is required when taking screenshots")
    executable = args.executable.resolve(strict=True)
    output = args.output.resolve() if args.output else None
    baseline = {path.stem: path for path in args.baseline.resolve().glob("*.png")} if args.baseline else {}
    if args.baseline and not baseline:
        parser.error("--baseline must contain a previous screenshot set")
    if output:
        output.mkdir(parents=True, exist_ok=True)
        if any(output.iterdir()):
            parser.error("output must be empty")
    # The fixture path appears in Settings. Reserve the same path for repeatable captures.
    case = Path(tempfile.gettempdir()).resolve() / "huggler-screenshot-fixture"
    try:
        case.mkdir(mode=0o700)
    except FileExistsError:
        parser.exit(1, f"Screenshot fixture directory already exists: {case}\n")
    try:
        scenes, changed = capture(executable, output, args.appearance, case, baseline)
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")
    finally:
        shutil.rmtree(case)
    if args.check_only:
        print(f"Passed {len(scenes)} desktop scene checkpoints")
        return 0
    print(f"Captured {len(scenes)} screenshots in {output}")
    return 0 if not args.baseline or report_comparison(scenes, baseline.keys(), changed) else 1


if __name__ == "__main__":
    sys.exit(main())
