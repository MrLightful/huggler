#!/usr/bin/env python3
"""Drive native settings and verify their effects on new and existing conversations."""
import json
from pathlib import Path
import sqlite3
import stat
import subprocess
import sys
import tempfile

from desktop_check import ROOT, isolated_env, launch_thread, run_window


def check_general(executable, state, env):
    env = dict(env, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"))
    general = [{"key": "cmd-,"}, {"click": "General"}]
    notification = "conversation-completed/Shortcut check"
    question_prompt = "ask one question"
    run_window(executable, state, [
        {"resize": [640, 440]}, *general,
        {"click": "⌘Enter"}, {"click": "Keep awake while agents run"}, {"click": "Back to Settings"}, {"click": "Back to workspace"},
        {"click": "First message"}, {"type": "Shortcut check"}, {"key": "enter"},
        {"type": "Second line"}, {"see": "First message"}, {"key": "cmd-enter"},
        {"contains": "Codex/reply"}, {"absent": notification},
        {"resize": [1120, 740]},
        {"click": "Message"}, {"type": question_prompt}, {"key": "cmd-enter"}, {"see": "Dismiss questions"},
        {"click": "Message"}, {"type": "Queued follow-up"}, {"click": "Queue message"},
        {"see": "1 queued message"}, {"click": "Dismiss questions"},
        {"see": "Codex/reply Queued follow-up"}, {"absent": notification},
        *general, {"click": "Steer"}, {"click": "Agent notifications"}, {"click": "Back to Settings"}, {"click": "Back to workspace"},
        {"click": "Message"}, {"type": question_prompt}, {"key": "cmd-enter"}, {"see": "Dismiss questions"},
        {"click": "Message"}, {"type": "Steered follow-up"}, {"click": "Send follow-up"},
        {"see": "You/Steered follow-up"}, {"absent": "1 queued message"},
        {"click": "Message"}, {"type": "Steered by shortcut"}, {"key": "cmd-enter"},
        {"see": "You/Steered by shortcut"}, {"absent": "1 queued message"},
        {"click": "Dismiss questions"}, {"see": notification},
    ], env)
    prefs = state / "preferences.json"
    saved = json.loads(prefs.read_text())
    assert {key: saved[key] for key in ("sendShortcut", "followUp", "agentNotifications", "keepAwake")} == {
        "sendShortcut": "commandEnter", "followUp": "steer", "agentNotifications": True, "keepAwake": True,
    }
    with sqlite3.connect(state / "huggler.sqlite3") as db:
        messages = db.execute("SELECT text, run_id FROM message WHERE role='user' ORDER BY rowid").fetchall()
        assert [text for text, _ in messages] == [
            "Shortcut check\nSecond line", question_prompt, "Queued follow-up", question_prompt,
            "Steered follow-up", "Steered by shortcut",
        ]
        assert messages[1][1] != messages[2][1]
        assert messages[3][1] == messages[4][1] == messages[5][1]
    run_window(executable, state, [
        {"absent": notification},
        {"click": "Message"}, {"type": "After relaunch"}, {"key": "enter"}, {"type": "Second line"},
        {"key": "cmd-enter"}, {"see": "You/After relaunch\nSecond line"}, {"see": notification},
    ], env)


def check_storage(executable, state, env):
    run_window(executable, state, [
        {"resize": [640, 440]}, {"key": "cmd-,"}, {"center_x": ["Appearance", 320]},
        {"absent": "Done"}, {"same_line": ["Show conversations", "Back to workspace"]},
        *({"right_of": [f"settings-chevron/{name}", f"settings-summary/{name}"]}
          for name in ("Appearance", "Providers", "Storage")),
        {"click": "Storage"},
        {"same_line": ["Show conversations", "Back to Settings"]},
        {"see": "automatic-cleanup/Off"}, {"click": "Clean up worktrees now"},
        {"see": "No unused worktrees found."}, {"click": "automatic-cleanup/On"},
        {"click": "Back to Settings"}, {"see": "Providers"}, {"click": "Back to workspace"},
    ], env)
    assert json.loads((state / "preferences.json").read_text())["automaticWorktreeCleanup"] is True


def check_projects(executable, root, env):
    root.mkdir()
    root = root.resolve()
    state = root / "state"
    first, second, local = (root / name for name in ("atlas", "beacon ", "local "))
    for checkout in (first, second, local):
        subprocess.run(["git", "init", "-qb", "main", checkout], check=True)
    nested = first / "nested"
    nested.mkdir()
    env = dict(env, HUGGLER_CODEX=str(ROOT / "tests/fixtures/context-provider.py"))
    run_window(executable, state, [
        {"above": ["Choose project", "First message"]},
        {"click": "Choose project"}, {"next_file_selection": [str(nested)]}, {"click": "Add project"},
        {"see": "Choose workspace"}, {"click": "Choose project"}, {"see": "project/atlas"},
        {"next_file_selection": [str(first)]}, {"click": "Add project"},
        {"see": "Choose workspace"}, {"click": "Choose project"},
        {"next_file_selection": [str(second)]}, {"click": "Add project"},
        {"click": "Choose project"}, {"see": "project/beacon "}, {"key": "escape"},
        {"click": "Settings"}, {"click": "Projects"}, {"click": "Rename project/atlas"},
        {"key": "cmd-a"}, {"type": "Atlas App"}, {"key": "enter"},
        {"see": "Rename project/Atlas App"},
    ], env)
    projects = json.loads((state / "preferences.json").read_text())["projects"]
    assert projects == [{"name": "Atlas App", "root": str(first)}, {"name": "beacon ", "root": str(second)}]

    run_window(executable, state, [
        {"resize": [640, 440]}, {"click": "Choose project"}, {"see": "project/Atlas App"},
        {"type": "BEACON"}, {"absent": "project/Atlas App"}, {"see": "project/beacon "}, {"key": "enter"},
        {"absent": "project/beacon "}, {"click": "First message"}, {"type": "Reuse Beacon"},
        {"click": "Send first message"}, {"see": "Codex/reply Reuse Beacon"},
        {"key": "cmd-shift-n"}, {"click": "Choose project"}, {"click": "project/Atlas App"},
        {"click": "Conversation name"}, {"type": "Reuse Atlas"}, {"click": "Open conversation"},
        {"see": "Ready/Reuse Atlas/New conversation"},
        {"click": "Settings"}, {"click": "Projects"}, {"click": "Remove project/Atlas App"},
        {"absent": "Rename project/Atlas App"}, {"click": "Back to Settings"}, {"click": "Back to workspace"},
        {"key": "cmd-shift-n"}, {"next_file_selection": [str(local)]}, {"click": "Browse"},
        {"see": "Choose workspace"}, {"click": "Conversation name"}, {"type": "One-time checkout"},
        {"click": "Open conversation"}, {"see": "Ready/One-time checkout/New conversation"},
    ], env)
    with sqlite3.connect(state / "huggler.sqlite3") as db:
        roots = dict(db.execute("SELECT thread.name, project.root FROM thread JOIN project ON thread.project = project.id"))
        assert roots == {"Reuse Beacon": str(second), "Reuse Atlas": str(first), "One-time checkout": str(local)}
    assert first.is_dir()
    assert json.loads((state / "preferences.json").read_text())["projects"] == [
        {"name": "beacon ", "root": str(second)},
    ]


def check_provider_setup(executable, root, env):
    state = root / "provider-setup"
    codex = root / "test-codex"
    codex.write_text("""#!/bin/sh
set -eu
if test "$*" = '--version'; then
    printf 'codex-cli %s\n' "${FIXTURE_CODEX_VERSION:-0.169.0}"
    exit 0
fi
test "$*" = 'login status'
# The composer startup check and the Providers page both see a signed-out account.
if test -f "$HOME/status-checked-in-settings"; then
    echo 'Logged in using ChatGPT' >&2
else
    if test -f "$HOME/status-checked"; then
        touch "$HOME/status-checked-in-settings"
    fi
    touch "$HOME/status-checked"
    echo 'Not logged in' >&2
    exit 1
fi
""")
    codex.chmod(0o755)
    run_window(executable, state, [
        {"resize": [640, 440]},
        {"click": "model-automatic"}, {"see": "model-status/Connect a provider in Settings."},
        {"absent": "provider/Codex"}, {"see": "Edit"}, {"key": "escape"},
        {"key": "cmd-,"}, {"see": "Appearance"}, {"see": "Providers"},
        {"absent": "provider/codex"}, {"absent": "Dark"},
        {"click": "Providers"}, {"right_of": ["Add provider", "provider/codex"]},
        {"absent": "provider/claudeAgent"},
        {"min_height": ["provider/codex", 64]}, {"max_height": ["provider/codex", 80]},
        {"above": ["provider/codex", "Provider title"]},
        {"absent": "CLI executable"},
        {"see": "Sign-in needed"}, {"see": "v0.169.0"}, {"absent": "Update to v0.170.0"},
        {"above": ["Back to Settings", "v0.169.0"]}, {"above": ["Back to Settings", "Refresh setup"]},
        {"above": ["Back to Settings", "Provider title"]}, {"above": ["Provider title", "provider-status-message"]},
        {"see": "Copy sign-in command"}, {"see": "Advanced settings"}, {"absent": "CLI executable"},
        {"absent": "API key"}, {"absent": "Sign in with ChatGPT"},
        {"click": "Refresh setup"}, {"see": "Connected"},
        {"right_of": ["v0.169.0", "Provider title"]}, {"right_of": ["Connected", "v0.169.0"]},
        {"above": ["Provider title", "Models"]},
        {"absent": "provider-status-message"},
        {"absent": "Copy sign-in command"}, {"absent": "Codex sign-in guide"},
        {"click": "Add provider"}, {"see": "Choose a provider"}, {"absent": "Provider title"},
        {"see": "provider-card-status/claudeAgent/Installation needed"},
        {"see": "provider/codex"}, {"click": "provider/claudeAgent"},
        {"absent": "provider/claudeAgent"}, {"absent": "Choose a provider"},
        {"absent": "provider-status-message"}, {"absent": "Copy sign-in command"},
        {"above": ["Provider title", "Installation guide"]},
        {"see": "Installation guide"}, {"absent": "Models"}, {"absent": "Environment"}, {"absent": "Advanced settings"},
        {"absent": "Refresh setup"}, {"absent": "CLI executable"},
        {"click": "Custom executable"}, {"see": "CLI executable"}, {"absent": "Save path"},
        {"click": "provider/codex"}, {"see": "Connected"}, {"absent": "CLI executable"},
        {"click": "Back to Settings"},
        {"see": "Appearance"}, {"absent": "provider/codex"}, {"click": "Back to workspace"},
    ], dict(env, HUGGLER_CODEX=str(codex)))
    assert (Path(env["HOME"]) / "status-checked").exists()
    managed = state / "providers/codex/bin/codex"
    managed.parent.mkdir(parents=True)
    managed.write_bytes(codex.read_bytes())
    managed.chmod(0o755)
    prefs = state / "preferences.json"
    prefs.write_text(json.dumps({"codexExecutable": str(managed)}))
    run_window(executable, state, [
        {"key": "cmd-,"}, {"click": "Providers"}, {"absent": "provider/codex"},
        {"click": "Add provider"}, {"click": "provider/codex"}, {"see": "Install Codex"},
        {"click": "Back to Settings"},
    ], env)
    managed.with_name("codex-code-mode-host").write_bytes(codex.read_bytes())
    managed.with_name("codex-code-mode-host").chmod(0o755)
    run_window(executable, state, [
        {"resize": [640, 440]}, {"key": "cmd-,"}, {"click": "Providers"}, {"click": "provider/codex"},
        {"above": ["Back to Settings", "v0.169.0"]}, {"right_of": ["v0.169.0", "Provider title"]},
        {"right_of": ["Update to v0.170.0", "v0.169.0"]}, {"absent": "Connected"},
        {"above": ["Back to Settings", "Reset to defaults"]},
        {"click": "Back to Settings"},
    ], env)
    run_window(executable, state, [
        {"key": "cmd-,"}, {"click": "Providers"}, {"click": "provider/codex"},
        {"right_of": ["Connected", "v0.170.0"]}, {"absent": "Update to v0.170.0"}, {"click": "Back to Settings"},
    ], dict(env, FIXTURE_CODEX_VERSION="0.170.0"))

    # Block the provider directory so installation fails before any download.
    broken = root / "provider-install-error"
    broken.mkdir()
    (broken / "providers").touch()
    (broken / "preferences.json").write_text(json.dumps({
        "codexExecutable": str(broken / "providers/codex/bin/codex"),
    }))
    error = "provider-setup-error/Could not open the provider folder."
    run_window(executable, broken, [
        {"key": "cmd-,"}, {"click": "Providers"}, {"click": "Add provider"}, {"click": "provider/codex"},
        {"click": "Install Codex"}, {"see": "Setup failed"},
        {"right_of": ["Setup failed", "Provider title"]},
        {"above": ["Provider title", error]}, {"above": [error, "Install Codex"]},
        {"click": "Back to Settings"},
    ], env)


def check_model_refresh(executable, root, fixture, env):
    state = root / "model-refresh"
    codex = root / "codex-refresh-fixture"
    codex.write_text("""#!/bin/sh
case "$*" in
    'login status') echo 'Logged in using ChatGPT' >&2; exit 0;;
    '--version') echo 'codex-cli 0.170.0'; exit 0;;
esac
if test -f model-list-requested; then
    touch empty-model-list
else
    touch model-list-requested
fi
exec "$MODEL_FIXTURE" "$@"
""")
    codex.chmod(0o755)
    run_window(executable, state, [
        {"key": "cmd-,"}, {"click": "Providers"}, {"click": "provider/codex"},
        {"see": "Connected"}, {"see": "model-enabled/fixture-fast"},
        {"absent": "Refresh models"}, {"click": "Refresh setup"},
        {"see": "Connected"}, {"see": "No models"},
        {"absent": "model-enabled/fixture-fast"}, {"absent": "Refresh models"},
        {"click": "Back to Settings"},
    ], dict(env, HUGGLER_CODEX=str(codex), MODEL_FIXTURE=str(fixture)))


def check_models_and_runtime(executable, root, checkout, fixture, env):
    state = root / "models-and-runtime"
    home = root / "codex config"
    home.mkdir()
    codex = root / "codex-model-fixture"
    codex.write_text("""#!/bin/sh
if test "$*" = 'login status'; then
    echo 'Logged in using ChatGPT' >&2
    exit 0
fi
if test "$#" -gt 1 && test "$1" = '--profile'; then
    test "$2" = 'fixture profile' || exit 2
    test "$CODEX_HOME" = "$EXPECTED_CODEX_HOME" || exit 2
    touch "$CODEX_HOME/received-runtime-options"
fi
if test "${FIXTURE_PROVIDER_VARIABLE-}" = 'value with spaces=1'; then
    touch "$EXPECTED_CODEX_HOME/received-environment"
fi
exec "$MODEL_FIXTURE" "$@"
""")
    codex.chmod(0o755)
    env = dict(env, HUGGLER_CODEX=str(codex), MODEL_FIXTURE=str(fixture), EXPECTED_CODEX_HOME=str(home))
    open_provider = [
        {"key": "cmd-,"}, {"click": "Providers"}, {"click": "provider/codex"},
        {"see": "model-enabled/fixture-fast"}, {"see": "model-enabled/fixture-deep"},
    ]
    run_window(executable, state, [
        *launch_thread(checkout, "Models check"),
        {"click": "model-automatic"},
        {"above": ["model-option/fixture-fast", "model-option/fixture-deep"]},
        {"click": "model-option-automatic"},
        *open_provider,
        {"absent": "Launch arguments"}, {"absent": "CLI executable"},
        {"above": ["model-enabled/fixture-fast", "model-enabled/fixture-deep"]},
        {"click": "model-up/fixture-deep"},
        {"above": ["model-enabled/fixture-deep", "model-enabled/fixture-fast"]},
        {"click": "model-enabled/fixture-deep"}, {"absent": "model-up/fixture-deep"},
        {"click": "Back to Settings"}, {"click": "Back to workspace"}, {"click": "model-automatic"},
        {"see": "model-option/fixture-fast"}, {"absent": "model-option/fixture-deep"},
        {"click": "model-option-automatic"},
    ], env)
    run_window(executable, state, [
        {"resize": [1120, 840]},
        *open_provider,
        {"absent": "model-up/fixture-deep"}, {"click": "model-enabled/fixture-deep"},
        {"above": ["model-enabled/fixture-deep", "model-enabled/fixture-fast"]},
        {"click": "Advanced settings"}, {"click": "Codex config directory"}, {"type": str(home)},
        {"click": "Launch arguments"}, {"type": "--profile 'fixture profile'"},
        {"click": "Save runtime settings"}, {"click": "Advanced settings"},
        {"see": "Environment"}, {"click": "Add variable"},
        {"click": "variable-value/0"}, {"type": "value with spaces=1"},
        {"click": "Back to Settings"},
        {"see": "settings-error/Start variable names with a letter or underscore and use only letters, numbers, and underscores."},
        {"key": "cmd-n"}, {"see": "variable-name/0"},
        {"click": "variable-name/0"}, {"type": "FIXTURE_PROVIDER_VARIABLE"},
        {"click": "thread/Models check"},
        {"click": "model-automatic"},
        {"above": ["model-option/fixture-deep", "model-option/fixture-fast"]},
        {"click": "model-option-automatic"},
    ], env)
    prefs = state / "preferences.json"
    preferences = json.loads(prefs.read_text())
    assert preferences["runtimeOverrides"]["codex"] == {"home": str(home), "args": ["--profile", "fixture profile"]}
    assert preferences["providerEnvironment"] == {"codex": {"FIXTURE_PROVIDER_VARIABLE": "value with spaces=1"}}
    assert stat.S_IMODE(prefs.stat().st_mode) == 0o600
    assert not (home / "received-environment").exists()
    run_window(executable, state, [
        *open_provider, {"see": "environment-variable/FIXTURE_PROVIDER_VARIABLE"},
        {"click": "Edit variables"}, {"click": "remove-variable/0"}, {"click": "Cancel variable changes"},
        {"see": "environment-variable/FIXTURE_PROVIDER_VARIABLE"}, {"click": "Back to Settings"},
    ], env)
    assert (home / "received-runtime-options").is_file()
    assert (home / "received-environment").is_file()
    run_window(executable, state, [
        *open_provider, {"above": ["model-enabled/fixture-deep", "model-enabled/fixture-fast"]},
        {"click": "Reset to defaults"}, {"click": "Cancel reset"},
        {"above": ["model-enabled/fixture-deep", "model-enabled/fixture-fast"]},
        {"click": "Reset to defaults"}, {"click": "Restore defaults"},
        {"above": ["model-enabled/fixture-fast", "model-enabled/fixture-deep"]},
        {"absent": "environment-variable/FIXTURE_PROVIDER_VARIABLE"}, {"click": "Back to Settings"},
    ], env)
    assert json.loads((state / "preferences.json").read_text()) == {
        "appearance": "system", "codexExecutable": None, "automaticWorktreeCleanup": False,
    }


def check(executable):
    with tempfile.TemporaryDirectory(prefix="huggler settings ") as directory:
        root = Path(directory)
        checkout, state = root / "checkout", root / "state"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        # Model discovery writes the fixture's protocol log in the checkout.
        with (checkout / ".git/info/exclude").open("a") as exclude:
            exclude.write("\n/frames.ndjson\n")
        env = dict(isolated_env(root), HUGGLER_CLAUDE_ACP=str(root / "missing-claude-agent"))
        check_general(executable, root / "general", env)
        check_storage(executable, root / "storage", env)
        check_projects(executable, root / "projects", env)
        check_provider_setup(executable, root, env)
        fixture = ROOT / "tests/fixtures/context-provider.py"
        check_model_refresh(executable, root, fixture, env)
        prefs = state / "preferences.json"
        state.mkdir()
        prefs.write_text(json.dumps({"codexExecutable": str(state / "providers/codex/bin/codex")}))
        run_window(executable, state, [
            *launch_thread(checkout, "Original"),
            {"key": "cmd-n"}, {"key": "cmd-shift-n"}, {"click": "Conversation name"}, {"type": "After setup"},
            {"key": "cmd-,"}, {"click": "Appearance"}, {"absent": "provider/codex"},
            {"click": "Dark"}, {"click": "Back to Settings"}, {"absent": "Dark"},
            {"click": "Providers"}, {"absent": "CLI executable"},
            {"see": "No providers detected"}, {"absent": "provider/codex"}, {"absent": "provider/claudeAgent"},
            {"click": "Add provider"}, {"see": "Choose a provider"}, {"see": "provider/claudeAgent"},
            {"click": "provider/codex"}, {"see": "Install Codex"},
            {"absent": "Models"}, {"absent": "Advanced settings"}, {"absent": "CLI executable"},
            {"click": "Custom executable"}, {"click": "CLI executable"}, {"key": "cmd-a"}, {"type": "relative/path"}, {"click": "Save path"},
            {"see": "settings-error/Enter the full path to the executable."},
            {"click": "Add provider"}, {"see": "CLI executable"}, {"see": "Install Codex"},
            {"click": "CLI executable"}, {"key": "cmd-a"}, {"type": str(fixture)},
            {"click": "Add provider"}, {"click": "provider/claudeAgent"},
            {"see": "Installation guide"}, {"absent": "CLI executable"},
            {"click": "Back to Settings"}, {"click": "Back to workspace"},
            {"click": "model-automatic"}, {"absent": "provider/Claude Agent"},
            {"see": "Edit"}, {"key": "escape"},
            {"click": "Open conversation"}, {"see": "status/Ready"},
        ], env)
        saved = {
            "appearance": "dark", "codexExecutable": str(fixture), "automaticWorktreeCleanup": False,
        }
        assert json.loads(prefs.read_text()) == saved
        prefs.write_text(json.dumps(dict(saved, provider="claudeAgent")))
        # Hold checkout discovery until Settings saves a new appearance. This
        # makes navigation during launch deterministic even on fast filesystems.
        git = root / "git"
        git.write_text("""#!/bin/sh
case "$*" in
  *'rev-parse --show-toplevel')
    for i in $(seq 1 200); do
      grep -q '"light"' "$HUGGLER_STATE_DIR/preferences.json" && break
      sleep 0.1
    done
    grep -q '"light"' "$HUGGLER_STATE_DIR/preferences.json" || exit 1
    ;;
esac
exec /usr/bin/git "$@"
""")
        git.chmod(0o755)
        run_window(executable, state, [
            {"resize": [640, 440]}, {"see": "Hide conversations"}, {"key": "cmd-n"},
            {"key": "cmd-shift-n"}, {"click": "Git checkout"}, {"key": "cmd-a"}, {"type": str(checkout)},
            {"click": "Conversation name"}, {"type": "After restart"},
            {"click": "Open conversation"}, {"key": "cmd-,"}, {"see": "Show conversations"},
            {"click": "Appearance"}, {"click": "Light"},
            {"resize": [1120, 740]},
            {"see": "thread/After restart"}, {"click": "Back to Settings"}, {"click": "Back to workspace"}, {"resize": [640, 440]}, {"see": "status/Ready"},
            {"see": "thread/After restart"},
            {"click": "Settings"}, {"resize": [1120, 740]}, {"click": "Appearance"}, {"click": "Dark"}, {"click": "Back to Settings"}, {"click": "Back to workspace"},
            {"key": "cmd-n"}, {"key": "cmd-shift-n"}, {"click": "Conversation name"}, {"type": "Background"},
            {"click": "Open conversation"}, {"click": "thread/Original"},
            {"click": "Settings"}, {"click": "Appearance"}, {"click": "Light"}, {"see": "thread/Background"}, {"click": "Back to Settings"}, {"click": "Back to workspace"},
            {"click": "Message"}, {"type": "Review my draft"},
            {"click": "Settings"}, {"click": "Back to workspace"},
            {"see": "All changes"},
            {"click": "Hide changes"}, {"absent": "Changes tab"}, {"click": "Send"},
            {"see": "Codex/reply Review my draft"},
        ], dict(env, PATH=f"{root}:{env['PATH']}"))
        with sqlite3.connect(state / "huggler.sqlite3") as db:
            assert dict(db.execute("SELECT name,provider_instance FROM thread")) == {
                "Original": "codex", "After setup": "codex", "After restart": "codex",
                "Background": "codex",
            }
            selected = (state / "selected-thread").read_text()
            assert db.execute("SELECT name FROM thread WHERE id=?", (selected,)).fetchone() == ("Original",)
        # A process-level override wins over the saved executable.
        run_window(executable, state, [
            {"key": "cmd-,"}, {"click": "Providers"}, {"click": "Add provider"}, {"click": "provider/codex"},
            {"click": "Custom executable"}, {"see": "CLI executable"}, {"absent": "Save path"},
            {"click": "Back to Settings"}, {"click": "Back to workspace"}, {"click": "Message"}, {"type": "Check override"}, {"click": "Send"},
            {"see": "status/Failed"},
        ], dict(env, HUGGLER_CODEX=str(root / "missing")))
        run_window(executable, state, [
            {"key": "cmd-,"}, {"click": "Appearance"}, {"click": "System"},
            {"click": "Back to Settings"}, {"click": "Providers"}, {"click": "provider/codex"},
            {"click": "Advanced settings"}, {"click": "CLI executable"},
            {"key": "cmd-a"}, {"key": "backspace"}, {"click": "Add provider"}, {"click": "provider/claudeAgent"},
            {"see": "provider/codex"}, {"click": "Back to Settings"}, {"click": "Back to workspace"}, {"see": "Message"},
        ], env)
        assert json.loads(prefs.read_text()) == {
            "appearance": "system", "codexExecutable": None, "automaticWorktreeCleanup": False,
        }
        claude_env = {key: value for key, value in env.items() if key != "HUGGLER_CLAUDE_ACP"}
        claude = root / "claude"
        claude.write_text('#!/bin/sh\ntest "$*" = "auth status --json" && echo \'{"loggedIn":true}\'\n')
        claude.chmod(0o755)
        open_claude = [{"key": "cmd-,"}, {"click": "Providers"}, {"click": "provider/claudeAgent"}]
        prefs.write_text(json.dumps(dict(json.loads(prefs.read_text()), claudeAgentExecutable=env["HUGGLER_CLAUDE_ACP"])))
        run_window(executable, state, [
            {"key": "cmd-,"}, {"click": "Providers"}, {"absent": "provider/claudeAgent"},
            {"click": "Add provider"}, {"click": "provider/claudeAgent"}, {"see": "Installation guide"},
            {"click": "Custom executable"}, {"click": "CLI executable"}, {"key": "cmd-a"}, {"type": str(claude)}, {"click": "Save path"}, {"click": "Back to Settings"},
        ], claude_env)
        assert json.loads(prefs.read_text())["claudeAgentExecutable"] == str(claude)
        run_window(executable, state, [
            *open_claude, {"see": "Connected"}, {"see": "Models"}, {"see": "Advanced settings"},
            {"absent": "Custom executable"}, {"right_of": ["provider/claudeAgent", "provider/codex"]},
            {"right_of": ["Add provider", "provider/claudeAgent"]},
            {"click": "Add provider"}, {"see": "Choose a provider"}, {"see": "provider/opencode"},
            {"click": "provider/claudeAgent"}, {"see": "Connected"}, {"click": "Back to Settings"},
        ], dict(claude_env, HUGGLER_CODEX=str(fixture)))
        check_models_and_runtime(executable, root, checkout, fixture, env)
    print("Native settings, shortcuts, drafts and provider persistence passed")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve())
