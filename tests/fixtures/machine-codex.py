#!/usr/bin/env python3
"""Pause an accepted Codex turn so the desktop can disconnect while it runs."""
import json
from pathlib import Path
import sys
import time


def emit(**message):
    print(json.dumps(message), flush=True)


assert sys.argv[1:] == ["app-server"]
for line in sys.stdin:
    frame = json.loads(line)
    if "id" not in frame:
        continue
    method = frame.get("method")
    params = frame.get("params", {})
    result = {}
    if method in ("thread/start", "thread/resume"):
        assert Path(params["cwd"]) == Path.cwd()
        result = {"thread": {"id": "machine-session"}}
    if method == "turn/start":
        assert params["input"][0]["text"] == "Write the remote result."
        assert Path(".git").is_dir()
        result = {"turn": {"id": "machine-turn"}}
    emit(id=frame["id"], result=result)
    if method == "turn/start":
        with Path("turns.txt").open("a") as turns:
            turns.write("turn/start\n")
        deadline = time.monotonic() + 90
        while not Path("finish-turn").exists():
            assert time.monotonic() < deadline, "Timed out waiting for the SSH disconnect"
            time.sleep(0.05)
        Path("remote-result.txt").write_text("Written by the remote provider.\n")
        answer = "Wrote the remote result."
        if Path("long-result").exists():
            answer += "\n" + "A fixture result line.\n" * 240 + "End of remote result."
        emit(method="item/completed", params={
            "threadId": "machine-session", "turnId": "machine-turn",
            "item": {"type": "agentMessage", "id": "result", "text": answer},
        })
        emit(method="turn/completed", params={
            "threadId": "machine-session", "turn": {"id": "machine-turn", "status": "completed"},
        })
