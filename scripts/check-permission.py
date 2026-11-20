#!/usr/bin/env python3
"""Exercise a live one-shot denial from the installed Tauri UI in a fresh checkout."""
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


def check(provider):
    program, variable = {
        "codex": ("codex", "HUGGLER_CODEX"),
        "claudeAgent": ("claude-agent-acp", "HUGGLER_CLAUDE_ACP")
    }[provider]
    path = os.environ.get(variable) or shutil.which(program)
    if not path:
        raise SystemExit(f"Install {program} or set {variable} to its absolute path")
    identifier = "dev.huggler.permission." + uuid.uuid4().hex
    with tempfile.TemporaryDirectory(prefix="huggler permission ") as tmp:
        root = Path(tmp)
        checkout = root / "project"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        prompt = "Use the Write tool to create approval-check.txt containing approved. If permission is denied, finish without trying another tool."
        if provider == "codex":
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
  el("name").value = "Permission qualification";
  el("launch").requestSubmit();
  let sent = false;
  let decided = false;
  setInterval(() => {
    if (!sent && !el("conversation").hidden) {
      sent = true;
      el("prompt").value = INPUT.prompt;
      el("send").requestSubmit();
    }
    const deny = el("permissions").querySelector("button[data-kind=reject_once]");
    if (deny && !decided) {decided = true; deny.click();}
    if (el("error").textContent || ["failed","completed","interrupted"].includes(el("status").dataset.status)) el("quit").click();
  }, 100);
})();
"""
        (ui / "qualification.js").write_text("const INPUT = " + json.dumps({
            "provider": provider, "root": str(checkout), "prompt": prompt
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
                choice = next(o for o in json.loads(request)["options"] if o["optionId"] == selected)
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
    parser.add_argument("provider", choices=["codex", "claudeAgent"])
    check(parser.parse_args().provider)
