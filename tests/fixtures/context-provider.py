#!/usr/bin/env python3
"""Scripted Codex app-server and ACP peers for context and input assertions."""
import json
from pathlib import Path
import sys
import time
from uuid import uuid4


if sys.argv[1:3] == ["mcp", "login"]:
    Path("mcp-login.json").write_text(json.dumps(sys.argv[1:]))
    print("Fixture MCP login completed", flush=True)
    sys.exit(0)


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
        if frame["id"] == "approve":
            assert frame["result"]["decision"] in ("accept", "decline")
            continue
        assert frame["id"] == "ask"
        if prompt == "command output":
            emit(method="item/completed", params={"threadId": session, "turnId": turn, "item": {
                "type": "commandExecution", "id": "build", "aggregatedOutput": "final report"}})
            for item in [
                {"type": "contextCompaction", "id": "compact"},
                {"type": "mcpToolCall", "id": "lookup", "server": "docs", "tool": "lookup",
                 "status": "failed", "error": {"message": "Document unavailable"}},
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
        result = {"data": [{
            "id": model,
            "displayName": "Fixture Deep" if model == "fixture-deep" else "Fixture Fast",
            "isDefault": model == "fixture-fast",
            "defaultReasoningEffort": "medium",
            "supportedReasoningEfforts": [{"reasoningEffort": "medium"}, {"reasoningEffort": "high"}],
            **({"serviceTiers": [{"id": "priority", "name": "Fast"}]} if model == "fixture-deep" else {}),
        } for model in models]}
    elif method == "config/read" and Path("codex-fixture-config.json").exists():
        result = {"config": json.loads(Path("codex-fixture-config.json").read_text())}
    elif method == "skills/list" and not acp:
        assert params["cwds"] == [str(Path.cwd())]
        if Path("skill-list-unavailable").exists():
            emit(id=frame["id"], error={"code": -32601, "message": "unsupported"})
            continue
        result = {"data": [
            {"cwd": str(Path.cwd().parent), "skills": [
                {"name": "other-checkout", "enabled": True, "path": "/other/SKILL.md"},
            ]},
            {"cwd": str(Path.cwd()), "skills": [
                {"name": "build-guide", "enabled": True, "path": str(Path.cwd() / "build" / "SKILL.md"),
                 "shortDescription": "Build the workspace"},
                {"name": "lint-guide", "enabled": True, "path": str(Path.cwd() / "lint" / "SKILL.md"),
                 "interface": {"shortDescription": "Run the linter"}},
                {"name": "disabled-guide", "enabled": False, "path": "/disabled/SKILL.md"},
                {"name": "shared-skill", "enabled": True, "path": str(Path.cwd() / "repo" / "SKILL.md"),
                 "scope": "repo"},
                {"name": "shared-skill", "enabled": True, "path": str(Path.cwd() / "user" / "SKILL.md"),
                 "scope": "user"},
            ]},
        ]}
    elif method == "initialize" and acp:
        result = {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}
        if Path("acp-vision").exists():
            result["agentCapabilities"]["promptCapabilities"] = {"image": True}
    elif method == "mcpServerStatus/list" and not acp:
        assert ephemeral and params["threadId"] == session
        assert params["detail"] == "toolsAndAuthOnly"
        if Path("mcp-status-unavailable").exists():
            emit(id=frame["id"], error={"code": -32601, "message": "unsupported"})
            continue
        if params.get("cursor") is None:
            if Path("mcp-status-delay").exists():
                time.sleep(2)
            result = {"nextCursor": "second-page", "data": [
                {"name": "Workspace Tools", "runtimeStatus": "connected",
                 "tools": {"search": {}, "read": {}, "summarize": {}}},
                {"name": "Notes", "runtimeStatus": "failed", "authStatus": "notLoggedIn", "tools": {}},
                {"name": "Reports", "runtimeStatus": "disabled", "tools": {}},
            ]}
        else:
            assert params["cursor"] == "second-page"
            result = {"data": [
                {"name": "Calendar", "toolsError": "MCP startup failed", "tools": {}},
                {"name": "Archive", "serverInfo": {"name": "Archive", "version": "1"}, "tools": {}},
                {"name": "Reference", "authStatus": "notLoggedIn", "tools": {"find": {}}},
            ]}
    elif method in ("thread/start", "session/new", "thread/resume", "session/load", "thread/fork"):
        ephemeral = params.get("ephemeral", False)
        if ephemeral:
            assert params["cwd"] == str(Path.cwd())
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
        if prompt == "trace ACP tools":
            for call in [
                {"sessionUpdate": "tool_call", "toolCallId": "weather", "title": "Checking the forecast",
                 "kind": "execute", "status": "in_progress",
                 "_meta": {"claudeCode": {"toolName": "mcp__weather__get_weather"}}},
                {"sessionUpdate": "tool_call_update", "toolCallId": "weather", "title": "Checking the forecast",
                 "kind": "execute", "status": "completed"},
                {"sessionUpdate": "tool_call_update", "toolCallId": "weather", "rawOutput": {"temperature": 12}},
                {"sessionUpdate": "tool_call", "toolCallId": "navigation", "title": "MCP Tool navigate_page",
                 "kind": "execute", "status": "completed",
                 "rawInput": {"ServerName": "browser", "ToolName": "navigate_page", "Arguments": {}}},
                {"sessionUpdate": "tool_call", "toolCallId": "standard", "title": "Search the checkout",
                 "kind": "search", "status": "completed",
                 "content": [{"type": "content", "content": {"type": "text", "text": "Found README.md"}}]},
            ]:
                emit(method="session/update", params={"sessionId": session, "update": call})
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
            emit(method="item/started", params={"threadId": session, "turnId": turn, "item": {
                "type": "mcpToolCall", "id": "lookup", "server": "docs", "tool": "lookup",
                "status": "inProgress", "arguments": {"query": "build report"}}})
            for target, item, delta in [(turn, "build", "prefix" + "界" * 20000 + "end"),
                                        ("prior", "foreign", "wrong")]:
                emit(method="item/commandExecution/outputDelta", params={
                    "threadId": session, "turnId": target, "itemId": item, "delta": delta})
    emit(id=frame["id"], result=result)
    if method == "turn/start":
        if prompt == "callbacks":
            emit(id="approve", method="item/commandExecution/requestApproval", params={
                "threadId": session, "turnId": turn, "command": "make check",
                "availableDecisions": ["accept", "decline"]})
        if prompt.startswith("quota "):
            window = prompt.split()[1]
            if window != "unknown":
                reset = int(time.time()) + (2 if window == "soon" else -30)
                emit(method="account/rateLimits/updated", params={"rateLimits": {"limitId": "codex",
                     "primary": {"usedPercent": 100, "resetsAt": reset}}})
            emit(method="turn/completed", params={"threadId": session, "turn": {"id": turn, "status": "failed",
                 "error": {"message": "Quota exhausted", "codexErrorInfo": "usageLimitExceeded"}}})
        elif prompt in ("ask", "ask one question", "callbacks", "command output"):
            questions = [{"id": "one"}]
            if prompt != "ask one question":
                questions.append({"id": "two"})
            emit(id="ask", method="item/tool/requestUserInput", params={"threadId": session, "turnId": turn,
                 "questions": questions})
        else:
            completed()

# EOF proves the adapter has finished processing the sample response.
if not acp and "prompt" in globals() and prompt in ("delayed sample", "sample blank", "sample unavailable"):
    Path(prompt + ".finished").touch()
