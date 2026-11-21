#!/usr/bin/env python3
"""Exercise a live denial or a recorded Codex grandchild approval in the installed GPUI window."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile

from desktop_check import ROOT, install_check, isolated_env, launch_thread, run_window, send_message


def prepare_replay(root):
    source = ROOT / "tests/fixtures/codex-subagents/nested-approval.ndjson"
    records = [json.loads(line) for line in source.read_text().splitlines()]
    # The desktop launch uses the provider's default model. Configure the peer
    # to return the same model that the recording selected explicitly.
    for entry in records:
        if entry.get("label") == "config/read" and entry["type"] == "emit_inbound":
            entry["frame"]["result"]["config"]["model"] = records[0]["metadata"]["model"]
    fixture = root / "replay.ndjson"
    fixture.write_text("\n".join(map(json.dumps, records)))
    metadata = subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=ROOT
    )
    (root / "case.json").write_text(json.dumps({
        "fixture": str(fixture), "version": json.loads(metadata)["packages"][0]["version"],
        "start": 1, "approvals": True, "workspaceWrite": True, "agentTools": True
    }))
    peer = root / "codex"
    shutil.copyfile(ROOT / "tests/fixtures/replay-codex.py", peer)
    peer.chmod(0o755)
    def params(method):
        return next(r["frame"]["params"] for r in records
                    if r.get("label") == method and r["type"] == "expect_outbound")
    return peer, params("thread/name/set")["name"], params("turn/start")["input"][0]["text"]


def check(mode):
    replay = mode == "recorded-codex"
    provider = "codex" if replay else mode
    program, variable = {
        "codex": ("codex", "HUGGLER_CODEX"),
        "claudeAgent": ("claude-agent-acp", "HUGGLER_CLAUDE_ACP")
    }[provider]
    path = os.environ.get(variable) or shutil.which(program)
    if not replay and not path:
        raise SystemExit(f"Install {program} or set {variable} to its absolute path")
    with tempfile.TemporaryDirectory(prefix="huggler permission ") as tmp:
        root = Path(tmp)
        checkout = root / "project"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        target = (checkout / "approval-check.txt").resolve()
        prompt = (f"Use the Write tool to create the file at this exact absolute path: {target}\n"
                  "Set its content to approved. If permission is denied, finish without trying another tool.")
        name = "Permission qualification"
        if replay:
            path, name, prompt = prepare_replay(root)
        elif provider == "codex":
            prompt = ("This is a permission UI test in an empty checkout. Request permission via exec_command "
                      "with sandbox_permissions=require_escalated to run exactly 'touch approval-check.txt'. "
                      "If denied, finish without trying a second time.")
        executable = install_check(root / "Huggler Qualification.app")
        state = root / "state"
        if provider == "codex":
            env = isolated_env(root, codex_login=not replay)
        else:
            # The separately installed Claude ACP adapter uses its logged-in home.
            env = dict(os.environ, PATH="/usr/bin:/bin:/usr/sbin:/sbin")
        env[variable] = str(Path(path).resolve(strict=True))
        controls = launch_thread(checkout, name, "Codex" if provider == "codex" else "Claude Agent")
        controls.extend(send_message(prompt))
        if replay:
            controls.extend([{"permission_from": "/root/relay/writer"}, {"click": "permission/allow_once"},
                             {"see": f"Ready/{name}/Completed"}])
        else:
            controls.extend([{"click": "permission/reject_once"},
                             {"see_any": [f"Ready/{name}/Completed", "status/Interrupted"]}])
        run_window(executable, state, controls, env)
        with sqlite3.connect(state / "huggler.sqlite3") as db:
            runs = db.execute("SELECT id,status,error FROM run").fetchall()
            choices = db.execute("SELECT run_id,request,decision FROM permission").fetchall()
            assert len(runs) == 1, runs
            assert len(choices) == 1 and choices[0][0] == runs[0][0], "Expected one permission on the completed run"
            _, request, selected = choices[0]
            request = json.loads(request)
            if replay:
                assert selected == "accept" and runs[0][1] == "completed", (selected, runs)
                children = [json.loads(row[0]) for row in db.execute(
                    "SELECT projection FROM native_child WHERE run_id=? ORDER BY rowid", (runs[0][0],))]
                assert len(children) == 2 and all(c["result"] == "Written." for c in children)
                writer = children[1]
                assert writer["path"] == "/root/relay/writer"
                assert request["native_child"] == {k: writer[k] for k in ("thread_id", "turn_id")}
                print("Codex 0.156.1 replay: GPUI identified and approved /root/relay/writer, both children returned Written.")
                return
            choice = next(o for o in request["options"] if o["optionId"] == selected)
            assert choice["kind"] == "reject_once"
            if provider == "claudeAgent":
                assert request["tool_call"]["name"] == "Write"
                proposed_write = request["tool_call"]["rawInput"]
                assert proposed_write["content"].strip() == "approved"
                proposed = Path(proposed_write["file_path"])
                assert (checkout / proposed).resolve() == target, proposed
            expected = "interrupted" if selected == "cancel" else "completed"
            assert runs[0][1] == expected, runs
            assert not target.exists()
            print(f"{provider}: GPUI selected {selected!r}, same run {expected}, file absent")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["codex", "claudeAgent", "recorded-codex"])
    check(parser.parse_args().mode)
