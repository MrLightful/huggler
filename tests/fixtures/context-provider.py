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
    if method == "initialize" and Path("fail-before-prompt").exists():
        sys.exit(1)
    if not method:
        assert frame["id"] == "ask"
        if prompt == "command output":
            emit(method="item/completed", params={"threadId": session, "turnId": turn, "item": {
                "type": "commandExecution", "id": "build", "aggregatedOutput": "final report"}})
            for item in [
                {"type": "contextCompaction", "id": "compact"},
                {"type": "mcpToolCall", "id": "lookup", "server": "docs", "tool": "lookup", "status": "failed"},
                {"type": "agentMessage", "id": "progress", "phase": "commentary", "text": "Checking the report."},
            ]:
                emit(method="item/completed", params={"threadId": session, "turnId": turn, "item": item})
        completed()
        continue
    if "id" not in frame:
        continue
    result = {}
    if method == "model/list" and not acp:
        models = [] if Path("empty-model-list").exists() else ["fixture-fast", "fixture-deep"]
        result = {"data": [{"id": model} for model in models]}
    elif method == "initialize" and acp:
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
    elif method == "thread/sample":
        assert params["threadId"] == session and params["expectedTurnId"] == turn and params["effort"] == "low"
        if prompt == "sample unavailable":
            emit(id=frame["id"], error={"code": -32601, "message": "unsupported"})
            continue
        if prompt == "delayed sample":
            while not Path("release-sample").exists():
                time.sleep(.01)
        result = {"text": "  " if prompt == "sample blank" else " Explain the result. "}
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
        if prompt == "command output":
            emit(method="item/started", params={"threadId": session, "turnId": turn, "item": {
                "type": "commandExecution", "id": "build", "command": "make check", "status": "inProgress"}})
            for target, item, delta in [(turn, "build", "prefix" + "界" * 20000 + "end"),
                                        ("prior", "foreign", "wrong")]:
                emit(method="item/commandExecution/outputDelta", params={
                    "threadId": session, "turnId": target, "itemId": item, "delta": delta})
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
        elif prompt in ("ask", "callbacks", "command output"):
            emit(id="ask", method="item/tool/requestUserInput", params={"threadId": session, "turnId": turn,
                 "questions": [{"id": "one"}, {"id": "two"}]})
        else:
            completed()

# EOF proves the adapter has finished processing the sample response.
if not acp and "prompt" in globals() and prompt in ("delayed sample", "sample blank", "sample unavailable"):
    Path(prompt + ".finished").touch()
