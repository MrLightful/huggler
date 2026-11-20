#!/usr/bin/env python3
"""A stdio Codex peer. Run a single recorded process against the actual client."""
import json
from pathlib import Path
import sys
import time

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
        if "id" in frame:
            assert isinstance(actual["id"], int) and actual["id"] not in ids.values()
            ids[frame["id"]] = actual["id"]
            frame["id"] = actual["id"]
        if frame["method"] == "initialize":
            # Huggler identifies its own client and doesn't register an MCP Apps renderer.
            frame["params"]["clientInfo"] = {
                "name": "huggler", "title": "Huggler", "version": case["version"]
            }
            del frame["params"]["capabilities"]["extensions"]
        if frame["method"] == "turn/start":
            # This consumer explicitly runs read-only turns.
            frame["params"]["sandboxPolicy"] = {"type": "readOnly"}
        if frame["method"] == "thread/start":
            # The recorder raised this limit to capture a three-level spawn.
            for key in records[0].get("metadata", {}).get("recorderThreadConfigKeys", []):
                del frame["params"]["config"][key]
        assert actual == frame, (index, actual, frame)
    else:
        assert entry["type"] == "emit_inbound"
        if "id" in frame:
            frame["id"] = ids[frame["id"]]
        print(json.dumps(frame), flush=True)
    index += 1
