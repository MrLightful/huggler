#!/usr/bin/env python3
"""A stdio Codex peer. Run a single recorded process against the actual client."""
import json
import os
from pathlib import Path
import sys
import time
from urllib.parse import urlsplit

here = Path(__file__).resolve().parent
case = json.loads((here / "case.json").read_text())
records = [json.loads(line) for line in Path(case["fixture"]).read_text().splitlines()]
index = case["start"]
assert sys.argv[1:] == ["app-server"]
assert records[0]["version"] == "0.156.1"
ids = {}


def substitute(value):
    if isinstance(value, list):
        return [substitute(item) for item in value]
    if isinstance(value, dict):
        return {key: substitute(item) for key, item in value.items()}
    if value == "<workspace>":
        return str(Path.cwd())
    return case.get("substitutions", {}).get(value, value)


while True:
    entry = records[index]
    if entry["type"] == "runtime_exit":
        (here / "cursor").write_text(str(index))
        break
    if entry["type"] == "await_release":
        deadline = time.monotonic() + 10
        while not (here / "release").exists():
            assert time.monotonic() < deadline, "test did not release replay"
            time.sleep(0.01)
        index += 1
        continue
    frame = substitute(entry["frame"])
    if entry["type"] == "expect_outbound":
        actual = json.loads(sys.stdin.readline())
        method = frame.get("method")
        # Provider-initiated requests use their own IDs, echoed by the host.
        if method and "id" in frame:
            assert isinstance(actual["id"], int) and actual["id"] not in ids.values()
            ids[frame["id"]] = actual["id"]
            frame["id"] = actual["id"]
        if method == "initialize":
            # Huggler identifies its own client and doesn't register an MCP Apps renderer.
            frame["params"]["clientInfo"] = {
                "name": "huggler", "title": "Huggler", "version": case["version"]
            }
            del frame["params"]["capabilities"]["extensions"]
        if method == "turn/start":
            # This consumer explicitly runs read-only turns.
            frame["params"]["sandboxPolicy"] = {"type": "readOnly"}
            if case.get("approvals"):
                frame["params"]["approvalPolicy"] = "on-request"
        if method == "thread/start":
            if case.get("agentTools"):
                config = actual["params"]["config"]["mcp_servers.huggler"]
                url = urlsplit(config["url"])
                assert url.scheme == "http" and url.hostname == "127.0.0.1" and url.port
                assert url.path == "/mcp" and os.environ["HUGGLER_MCP_TOKEN"]
                frame["params"]["config"]["mcp_servers.huggler"] = {
                    "url": config["url"], "bearer_token_env_var": "HUGGLER_MCP_TOKEN", "required": True,
                }
            if case.get("approvals"):
                frame["params"].update(approvalPolicy="on-request", sandbox="read-only")
            # The recorder raised this limit to capture a three-level spawn.
            for key in records[0].get("metadata", {}).get("recorderThreadConfigKeys", []):
                del frame["params"]["config"][key]
        assert actual == frame, (index, actual, frame)
    else:
        assert entry["type"] == "emit_inbound"
        if "id" in frame and "method" not in frame:
            frame["id"] = ids[frame["id"]]
        print(json.dumps(frame), flush=True)
    index += 1
