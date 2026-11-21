#!/usr/bin/env python3
"""Scripted Codex app-server and ACP peers for context and input assertions."""
import json
from pathlib import Path
import sys
from uuid import uuid4


def emit(**frame):
    if acp:
        frame["jsonrpc"] = "2.0"
    print(json.dumps(frame), flush=True)


def completed():
    emit(method="item/completed", params={"threadId": session, "turnId": turn,
         "item": {"type": "agentMessage", "text": "reply " + prompt[-30:]}})
    emit(method="turn/completed", params={"threadId": session, "turn": {"id": turn, "status": "completed"}})


acp = "app-server" not in sys.argv
for line in sys.stdin:
    frame = json.loads(line)
    method = frame.get("method")
    params = frame.get("params", {})
    with Path("frames.ndjson").open("a") as out:
        print(json.dumps(frame), file=out)
    if not method:
        assert frame["id"] == "ask"
        completed()
        continue
    if "id" not in frame:
        continue
    result = {}
    if method == "initialize" and acp:
        result = {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}
    elif method in ("thread/start", "session/new", "thread/resume", "session/load", "thread/fork"):
        session = str(uuid4()) if method in ("thread/fork", "thread/start", "session/new") else params.get("threadId", params.get("sessionId"))
        result = {"sessionId": session} if acp else {"thread": {"id": session}}
        if method == "thread/fork":
            boundary = "wrong" if Path("bad-boundary").exists() else params["lastTurnId"]
            result["thread"].update(forkedFromId=params["threadId"], turns=[{"id": boundary}])
    elif method == "session/prompt":
        prompt = params["prompt"][0]["text"]
        emit(method="session/update", params={"sessionId": session, "update": {"sessionUpdate": "agent_message_chunk",
             "content": {"type": "text", "text": "portable reply"}}})
        result = {"stopReason": "end_turn"}
    elif method == "turn/start":
        prompt = params["input"][0]["text"]
        turn = str(uuid4())
        result = {"turn": {"id": turn}}
    emit(id=frame["id"], result=result)
    if method == "turn/start":
        if prompt == "ask":
            emit(id="ask", method="item/tool/requestUserInput", params={"threadId": session, "turnId": turn,
                 "questions": [{"id": "one"}, {"id": "two"}]})
        else:
            completed()
