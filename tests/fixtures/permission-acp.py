#!/usr/bin/env python3
"""Protocol fixture for a single manual edit permission, with load replay."""
import json
import sys
from pathlib import Path

session = None
prompt = None


def send(**frame):
    print(json.dumps({"jsonrpc": "2.0", **frame}), flush=True)


def update(text):
    send(method="session/update", params={"sessionId": session, "update": {
        "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}
    }})


for line in sys.stdin:
    frame = json.loads(line)
    method = frame.get("method")
    params = frame.get("params", {})
    request_id = frame["id"]
    if method == "initialize":
        send(id=request_id, result={"protocolVersion": 1, "agentCapabilities": {"loadSession": True}})
    elif method in ("session/new", "session/load"):
        assert Path(params["cwd"]).resolve() == Path.cwd()
        session = params.get("sessionId", "fixture-session")
        if method == "session/load":
            update("replayed history")
        send(id=request_id, result={"sessionId": session, "modes": {"currentModeId": "auto"}})
    elif method == "session/set_mode":
        assert params == {"modeId": "default", "sessionId": session}
        send(id=request_id, result={})
    elif method == "session/prompt":
        prompt = request_id
        persistent_only = params["prompt"][0]["text"] == "Offer only a persistent grant"
        send(id="provider-1", method="session/request_permission", params={"sessionId": session,
             "toolCall": {"toolCallId": "edit-1", "title": "Create approval-check.txt",
                          "rawInput": {"file_path": "approval-check.txt", "content": "approved"}},
             "options": [{"optionId": "always", "name": "Allow always", "kind": "allow_always"}]
                         if persistent_only else
                        [{"optionId": "yes", "name": "Allow once", "kind": "allow_once"},
                         {"optionId": "no", "name": "Deny once", "kind": "reject_once"}]})
    else:
        assert request_id == "provider-1"
        outcome = frame["result"]["outcome"]
        if persistent_only:
            assert outcome == {"outcome": "cancelled"}
            update("persistent approval cancelled")
            send(id=prompt, result={"stopReason": "end_turn"})
            continue
        assert outcome["outcome"] == "selected" and outcome["optionId"] in ("yes", "no")
        if outcome["optionId"] == "yes":
            Path("approval-check.txt").write_text("approved")
        update("operation " + ("approved" if outcome["optionId"] == "yes" else "denied"))
        send(id=prompt, result={"stopReason": "end_turn"})
