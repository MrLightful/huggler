#!/usr/bin/env python3
"""Replay a Markdown image response through Codex app-server stdio."""
import json
import os
import sys
import time

assert sys.argv[1:] == ["app-server"]
thread = "image-fixture"
turn = "image-turn"
item = "image-reply"


def emit(**frame):
    print(json.dumps(frame), flush=True)


def notify(method, **params):
    emit(method=method, params={"threadId": thread, "turnId": turn, **params})


for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request.get("method")
    result = {
        "thread/start": {"thread": {"id": thread}},
        "thread/resume": {"thread": {"id": thread}},
        "model/list": {"data": []},
        "turn/start": {"turn": {"id": turn}},
    }.get(method, {})
    emit(id=request["id"], result=result)
    if method != "turn/start":
        continue
    text = os.environ["HUGGLER_FIXTURE_IMAGE_RESPONSE"]
    notify("item/agentMessage/delta", itemId=item, delta=os.environ["HUGGLER_FIXTURE_IMAGE_STREAM"])
    time.sleep(8)
    notify("item/completed", item={
        "id": item, "type": "agentMessage", "phase": "final_answer", "text": text,
    })
    notify("turn/completed", turn={"id": turn, "status": "completed"})
