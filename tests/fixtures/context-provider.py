#!/usr/bin/env python3
"""Scripted Codex app-server and ACP peers for context and input assertions."""
import json
from pathlib import Path
import sys
import time
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
        if method == "session/load" and session == "archived":
            if Path("removed").exists():
                emit(id=frame["id"], error={"code": -32602, "message": "unknown session"})
                continue
            for role, text in [("user", "old question"), ("agent", "old "), ("agent", "answer")]:
                emit(method="session/update", params={"sessionId": session, "update": {
                    "sessionUpdate": role + "_message_chunk", "content": {"type": "text", "text": text}}})
            if Path("changed").exists():
                emit(method="session/update", params={"sessionId": session, "update": {
                    "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": " changed"}}})
        if method == "thread/fork":
            boundary = "wrong" if Path("bad-boundary").exists() else params["lastTurnId"]
            result["thread"].update(forkedFromId=params["threadId"], turns=[])
    elif method == "thread/turns/list":
        assert params["threadId"] == session and params["limit"] == 1
        result = {"data": [{"id": boundary}], "nextCursor": None}
    elif method == "session/prompt":
        prompt = params["prompt"][0]["text"]
        emit(method="session/update", params={"sessionId": session, "update": {"sessionUpdate": "agent_message_chunk",
             "content": {"type": "text", "text": "portable reply"}}})
        result = {"stopReason": "end_turn"}
    elif method == "turn/steer":
        assert params["threadId"] == session and params["expectedTurnId"] == turn
        text = params["input"][0]["text"]
        if text == "reject":
            emit(id=frame["id"], error={"code": -32602, "message": "turn refused"})
            continue
        if text == "finish before ack":
            completed()
            continue
        if text == "drop":
            sys.exit(0)
        result = {"turnId": turn}
    elif method == "turn/start":
        prompt = params["input"][0]["text"]
        turn = str(uuid4())
        result = {"turn": {"id": turn}}
    emit(id=frame["id"], result=result)
    if method == "turn/start":
        if prompt == "callbacks":
            emit(id="approve", method="item/commandExecution/requestApproval", params={
                "threadId": session, "turnId": turn, "availableDecisions": ["accept", "decline"]})
        if prompt.startswith("quota "):
            window = prompt.split()[1]
            if window != "unknown":
                reset = int(time.time()) + (2 if window == "soon" else -30)
                emit(method="account/rateLimits/updated", params={"rateLimits": {"limitId": "codex",
                     "primary": {"usedPercent": 100, "resetsAt": reset}}})
            emit(method="turn/completed", params={"threadId": session, "turn": {"id": turn, "status": "failed",
                 "error": {"message": "Quota exhausted", "codexErrorInfo": "usageLimitExceeded"}}})
        elif prompt in ("ask", "callbacks"):
            emit(id="ask", method="item/tool/requestUserInput", params={"threadId": session, "turnId": turn,
                 "questions": [{"id": "one"}, {"id": "two"}]})
        else:
            completed()
