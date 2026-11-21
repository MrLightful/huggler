#!/usr/bin/env python3
"""Deterministic provider peers. Recorded inbound frames keep provider IDs and model data."""
import base64
import json
import os
from pathlib import Path
import subprocess
import sys
from urllib.request import Request, urlopen


def frames(name):
    source = Path(__file__).resolve().with_name(name + ".ndjson")
    return [json.loads(row)["frame"] for row in source.read_text().splitlines()]


def attachment(name):
    return (Path(__file__).resolve().parent.parent / "attachments" / name).read_bytes()


def emit(frame):
    print(json.dumps(frame), flush=True)


def receive():
    return json.loads(input())


def mcp(config, method, params):
    headers = config["headers"]
    if isinstance(headers, list):
        headers = {h["name"]: h["value"] for h in headers}
    headers = {**headers, "Accept": "application/json, text/event-stream", "Content-Type": "application/json"}
    msg = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    with urlopen(Request(config["url"], json.dumps(msg).encode(), headers=headers), timeout=5) as reply:
        result = json.load(reply)["result"]
    assert not result.get("isError"), result
    return result


def check_tools(config):
    assert any(t["name"] == "thread_list" for t in mcp(config, "tools/list", {})["tools"])
    result = mcp(config, "tools/call", {"name": "thread_list", "arguments": {}})
    assert isinstance(result["structuredContent"]["threads"], list)


def acp(flavor):
    recorded_turn = flavor in ("fx", "grok", "kiro", "registry")
    rows = frames(flavor if recorded_turn else "registry")
    init, session = rows[0]["result"], rows[1]["result"]
    startup = None
    if flavor.startswith("cursor") or flavor in ("antigravity-v1", "antigravity-logged-out"):
        provider = flavor.split("-")[0]
        startup = json.loads(Path(__file__).with_name(provider + "-cli-startup.json").read_text())
        init = startup[0]["result"]
    if flavor == "antigravity":
        init = json.loads(Path(__file__).with_name("antigravity-initialize.json").read_text())
    if flavor == "limited":
        init["agentCapabilities"] = {"loadSession": False}
    models = [m["modelId"] for m in session.get("models", {}).get("availableModels", [])]
    for option in session.get("configOptions", []):
        if option["id"] == "model":
            models.extend(m["value"] for m in option["options"] if "value" in m)
    selected = None
    native = session["sessionId"]
    for raw in sys.stdin:
        msg = json.loads(raw)
        method, p = msg["method"], msg["params"]
        if (method, flavor) in (("initialize", "early-login"), ("session/new", "login")):
            emit({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32000, "message": "Authentication required"}})
            continue
        if method == "initialize":
            assert p["protocolVersion"] == (2 if flavor.startswith("antigravity") else init["protocolVersion"])
            if flavor == "incompatible":
                init["protocolVersion"] = 99
            result = init
        elif method in ("session/new", "session/load"):
            assert p["cwd"] == os.getcwd()
            if flavor.endswith("-logged-out"):
                assert method == "session/new"
                emit({**startup[1], "id": msg["id"]})
                continue
            if method == "session/load":
                assert p["sessionId"] == native
            servers = p["mcpServers"]
            for server in servers:
                check_tools(server)
            result = session
        elif method in ("session/set_model", "session/set_config_option"):
            assert p["sessionId"] == native
            if flavor in ("fx", "grok"):
                assert method == "session/set_config_option" and p["configId"] == "model"
            else:
                assert method == "session/set_model"
            selected = p.get("value", p.get("modelId"))
            assert selected == min(models)
            result = {}
        elif method == "session/prompt":
            assert p["sessionId"] == native
            prompt = p["prompt"][0]["text"]
            supports_http = init["agentCapabilities"].get("mcpCapabilities", {}).get("http", False)
            assert [s["name"] for s in servers] == (["huggler"] if supports_http else [])
            if prompt.startswith("interrupt "):
                _, thread, run = prompt.split()
                result = mcp(servers[0], "tools/call", {"name": "thread_interrupt", "arguments": {
                    "threadId": thread, "runId": run, "clientRequestId": run}})
                assert result["structuredContent"]["status"] == "interrupt_requested"
                for frame in rows[2:-1]:
                    emit({"jsonrpc": "2.0", "method": frame["method"], "params": frame["params"]})
                emit({"jsonrpc": "2.0", "id": msg["id"], "result": rows[-1]["result"]})
                continue
            assert selected == (min(models) if models else None)
            assert (prompt == "fresh") == (opened == "session/new")
            if prompt == "hold":
                Path("holding").touch()
                input()
            deny = "stop-this-command" if flavor.startswith("antigravity") else "deny"
            request = {"sessionId": native, "toolCall": {"toolCallId": "fixture", "title": "Run command"},
                       "options": [{"optionId": deny, "name": "Deny", "kind": "reject_once"}]}
            emit({"jsonrpc": "2.0", "id": "permission", "method": "session/request_permission", "params": request})
            assert receive()["result"]["outcome"]["optionId"] == deny
            for f in rows[2:-1]:
                emit({"jsonrpc": "2.0", "method": f["method"], "params": f["params"]})
            result = rows[-1]["result"]
        else:
            raise AssertionError(method)
        if method in ("session/new", "session/load"):
            opened = method
        emit({"jsonrpc": "2.0", "id": msg["id"], "result": result})


def pi():
    rows = frames("pi")
    assert "--mode" in sys.argv and sys.argv[sys.argv.index("--mode") + 1] == "rpc"
    assert ("--extension" in sys.argv) == ("HUGGLER_MCP_TOKEN" in os.environ) == ("HUGGLER_MCP_URL" in os.environ)
    home = Path(os.environ["PI_CODING_AGENT_DIR"])
    home.mkdir(exist_ok=True)
    session = rows[0]["data"]
    if "--session" in sys.argv:
        assert sys.argv[sys.argv.index("--session") + 1] == session["sessionFile"]
    if (home / "wrong-id").exists():
        session["sessionId"] += "-changed"
    if (home / "wrong-path").exists():
        session["sessionFile"] += ".changed"
    for raw in sys.stdin:
        msg = json.loads(raw)
        method = msg["type"]
        if method == "prompt":
            if msg["message"] == "hold":
                Path("holding").touch()
                input()
            if msg["message"].startswith("Attachment fixture"):
                payload, = msg["images"]
                assert payload["type"] == "image" and payload["mimeType"] == "image/png"
                assert base64.b64decode(payload["data"]) == attachment("diagram.png")
            else:
                assert (msg["message"] == "fresh") == ("--session" not in sys.argv)
            if "--extension" in sys.argv:
                subprocess.run(["node", str(Path(__file__).with_name("check-extension.mjs")),
                                sys.argv[sys.argv.index("--extension") + 1]], check=True, stdout=sys.stderr)
            emit({"type": "extension_ui_request", "method": "confirm", "id": "dialog"})
            assert receive() == {"type": "extension_ui_response", "id": "dialog", "cancelled": True}
            for f in rows[2:-1]:
                emit(f)
            continue
        if method == "get_state":
            (home / "started").touch()
            data = session
        elif method == "get_available_models":
            data = rows[1]["data"]
        elif method == "set_model":
            assert msg["provider"] == session["model"]["provider"] and msg["modelId"] == session["model"]["id"]
            data = {}
        else:
            raise AssertionError(method)
        emit({"type": "response", "command": method, "id": msg["id"], "success": True, "data": data})


def muse():
    rows = frames("muse")
    session = rows[1]["result"]["session"]
    session["workspaceRoot"] = os.getcwd()
    for raw in sys.stdin:
        msg = json.loads(raw)
        method, p = msg["method"], msg.get("params", {})
        if method == "initialized":
            continue
        if method == "initialize":
            result = rows[0]["result"]
        elif method == "model/list":
            result = {"models": [{"modelId": session["modelId"]}]}
        elif method in ("session/start", "session/resume"):
            opened = method
            if method == "session/start":
                assert p["workspaceRoot"] == os.getcwd()
            else:
                assert p["sessionId"] == session["sessionId"]
            assert p["modelId"] == session["modelId"]
            check_tools(p["config"]["mcpServers"]["huggler"])
            result = rows[1]["result"]
        elif method == "turn/start":
            prompt = p["input"][0]["text"]
            assert p["sessionId"] == session["sessionId"]
            if prompt.startswith("Attachment fixture"):
                payload = p["input"][1]
                assert payload["type"] == "image" and payload["mediaType"] == "image/png"
                assert base64.b64decode(payload["base64Data"]) == attachment("diagram.png")
            else:
                assert (prompt == "fresh") == (opened == "session/start")
            if prompt == "hold":
                Path("holding").touch()
                input()
            if prompt == "form":
                emit({"jsonrpc": "2.0", "method": "userInput/requested", "params": {"sessionId": session["sessionId"]}})
                continue
            saved = rows[2]["result"]["turnId"]
            for f in [*rows[3:-1], rows[-2], rows[2], rows[-1]]:
                emit(json.loads(json.dumps(f).replace(saved, p["commandId"])))
            continue
        else:
            raise AssertionError(method)
        emit({"jsonrpc": "2.0", "id": msg["id"], "result": result})


if __name__ == "__main__":
    # The installed fixture launcher supplies the selected driver as its first argument.
    flavor = sys.argv[1]
    if "--version" in sys.argv:
        if flavor == "pi":
            assert (Path(os.environ["PI_CODING_AGENT_DIR"]) / "started").is_file()
        print({"pi": "1.0.0", "kiro": "2.32.0", "grok": "1.0.41", "cursor": "2026.11.19-f005e9d"}[flavor])
    elif flavor == "pi":
        pi()
    elif flavor == "muse":
        assert "serve" in sys.argv
        muse()
    else:
        acp(flavor)
