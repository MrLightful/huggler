#!/usr/bin/env python3
"""Exercise a live denial or a recorded Codex grandchild approval in the installed Tauri UI."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("installer", ROOT / "scripts/install-desktop.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


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
        "start": 1, "approvals": True
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
    identifier = "dev.huggler.permission." + uuid.uuid4().hex
    with tempfile.TemporaryDirectory(prefix="huggler permission ") as tmp:
        root = Path(tmp)
        checkout = root / "project"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        prompt = "Use the Write tool to create approval-check.txt containing approved. If permission is denied, finish without trying another tool."
        name = "Permission qualification"
        if replay:
            path, name, prompt = prepare_replay(root)
        elif provider == "codex":
            prompt = ("This is a permission UI test in an empty checkout. Request permission via exec_command "
                      "with sandbox_permissions=require_escalated to run exactly 'touch approval-check.txt'. "
                      "If denied, finish without trying a second time.")
        ui = root / "ui"
        shutil.copytree(ROOT / "desktop/ui", ui)
        driver = """
(async () => {
  const el = id => document.getElementById(id);
  el("root").value = INPUT.root;
  el("provider").value = INPUT.provider;
  el("name").value = INPUT.name;
  el("launch").requestSubmit();
  let sent = false;
  let decided = false;
  setInterval(() => {
    if (!sent && !el("conversation").hidden) {
      sent = true;
      el("prompt").value = INPUT.prompt;
      el("send").requestSubmit();
    }
    const panel = el("permissions");
    const button = panel.querySelector(INPUT.replay ? "button[data-kind=allow_once]" : "button[data-kind=reject_once]");
    if (button && !decided) {
      if (INPUT.replay && !panel.querySelector("legend").textContent.includes("/root/relay/writer")) {
        el("error").textContent = "Permission did not identify the requesting grandchild";
      } else {
        decided = true;
        button.click();
      }
    }
    if (el("error").textContent || ["failed","completed","interrupted"].includes(el("status").dataset.status)) el("quit").click();
  }, 100);
})();
"""
        (ui / "qualification.js").write_text("const INPUT = " + json.dumps({
            "provider": provider, "root": str(checkout), "prompt": prompt, "name": name, "replay": replay
        }) + ";\n" + driver)
        html = ui / "index.html"
        html.write_text(html.read_text().replace("</head>", '<script defer src="qualification.js"></script></head>'))
        executable = installer.install(root / "Huggler Qualification.app", identifier=identifier, ui=ui)
        state = Path.home() / "Library/Application Support" / identifier
        env = dict(os.environ, PATH="/usr/bin:/bin:/usr/sbin:/sbin")
        env[variable] = str(Path(path).resolve(strict=True))
        try:
            subprocess.run([executable], cwd="/", env=env, timeout=150, check=True)
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
                    print("Codex 0.156.1 replay: Tauri identified and approved /root/relay/writer, both children returned Written.")
                    return
                choice = next(o for o in request["options"] if o["optionId"] == selected)
                assert choice["kind"] == "reject_once"
                expected = "interrupted" if selected == "cancel" else "completed"
                assert runs[0][1] == expected, runs
                assert not (checkout / "approval-check.txt").exists()
                print(f"{provider}: Tauri selected {selected!r}, same run {expected}, file absent")
        finally:
            if state.is_dir():
                shutil.rmtree(state)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["codex", "claudeAgent", "recorded-codex"])
    check(parser.parse_args().mode)
