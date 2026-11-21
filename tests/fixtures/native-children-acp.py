#!/usr/bin/env python3
"""Representative Claude Agent ACP 0.109.0 v1 stdio frames, one managed process per turn."""
import json
import sys


ROOT, RELAY, WRITER = "root", "agent-42", "agent-99"
RUNNING = {"state": "running"}
WAITING = {"state": "requires_action"}
DONE = {"state": "idle", "stopReason": "end_turn"}
CANCEL = {"cancel": {}}


def read():
    return json.loads(input())


def send(**frame):
    print(json.dumps({"jsonrpc": "2.0", **frame}), flush=True)


def update(session, kind, **fields):
    send(method="session/update", params={
        "sessionId": session, "update": {"sessionUpdate": kind, **fields},
    })


def child(parent, session, **fields):
    update(parent, "subagent_update", sessionId=session, **fields)


def chunk(session, text, message="answer", **fields):
    update(session, "agent_message_chunk", content={"type": "text", "text": text},
           messageId=message, **fields)


def message(sender, recipient, text):
    for session, link in [(sender, "recipientMessageId"), (recipient, "senderMessageId")]:
        update(session, "session_message", senderSessionId=sender, recipientSessionId=recipient,
               messageId="tool-launch", content=[{"type": "text", "text": text}],
               **{link: "tool-launch"})


def permission(session, wire):
    send(id=wire, method="session/request_permission", params={
        "sessionId": session, "toolCall": {"title": wire, "toolCallId": wire},
        "options": [{"optionId": "yes", "name": "Allow", "kind": "allow_once"}],
    })


def expect_selected(wire):
    reply = read()
    assert reply["id"] == wire and reply["result"]["outcome"] == {
        "outcome": "selected", "optionId": "yes",
    }


def announce():
    child(ROOT, RELAY, title="Relay", state=RUNNING, capabilities=CANCEL)
    child(RELAY, WRITER, title="Writer", state=RUNNING, capabilities=CANCEL)


init = read()
assert init["method"] == "initialize" and init["params"]["clientCapabilities"]["subagents"] == {}
send(id=init["id"], result={"protocolVersion": 1, "agentCapabilities": {"loadSession": True}})
setup = read()
if setup["method"] == "session/load":
    assert setup["params"]["sessionId"] == ROOT
    announce()
    chunk(WRITER, "old transcript")
    child(RELAY, WRITER, state=DONE)
    child(ROOT, RELAY, state=DONE)
    chunk(ROOT, "old root")
else:
    assert setup["method"] == "session/new"
send(id=setup["id"], result={"sessionId": ROOT, "modes": {"currentModeId": "default"}})
if setup["method"] == "session/load":
    child(ROOT, RELAY, state={"state": "idle"})
    child(RELAY, WRITER, state={"state": "idle"})
prompt = read()
assert prompt["method"] == "session/prompt" and prompt["params"]["sessionId"] == ROOT
scenario = prompt["params"]["prompt"][0]["text"]
if scenario == "background":
    announce()
    message(ROOT, RELAY, "private instructions")
    chunk(ROOT, "flattened child", _meta={"claudeCode": {"parentToolUseId": "tool-launch"}})
    child(ROOT, "orphaned", title="Detached", state=RUNNING, capabilities=CANCEL)
    child(ROOT, "orphaned", state={"state": "unknown"}, capabilities=None)
    child("unannounced", "stranger", state=RUNNING, capabilities=CANCEL)
    permission("stranger", "foreign")
    denied = read()
    assert denied["id"] == "foreign" and denied["error"]["code"] == -32602
    permission(WRITER, "old request")
    child(RELAY, WRITER, state=WAITING)
    permission(ROOT, "finish root")
    expect_selected("finish root")
    chunk(ROOT, "root answer")
    send(id=prompt["id"], result={"stopReason": "end_turn"})
    expect_selected("old request")
    child(RELAY, WRITER, state=RUNNING)
    permission(WRITER, "withdrawn")
    child(RELAY, WRITER, state=WAITING)
    send(method="$/cancel_request", params={"requestId": "withdrawn"})
    reply = read()
    assert reply["id"] == "withdrawn" and reply["result"]["outcome"] == {"outcome": "cancelled"}
    child(RELAY, WRITER, state=RUNNING)
    permission(WRITER, "background request")
    child(RELAY, WRITER, state=WAITING)
    expect_selected("background request")
    child(RELAY, WRITER, state=RUNNING)
    assert read() == {"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": WRITER}}
    permission(RELAY, "finish relay")
    expect_selected("finish relay")
    child(RELAY, WRITER, state={"state": "idle", "stopReason": "cancelled"})
    message(RELAY, ROOT, "relay report")
    child(ROOT, RELAY, state=DONE)
elif scenario == "lost":
    announce()
    send(id=prompt["id"], result={"stopReason": "end_turn"})
    permission(WRITER, "lost request")
    child(RELAY, WRITER, state=WAITING)
elif scenario == "states":
    child(ROOT, WRITER, title="Writer", state=RUNNING, capabilities=CANCEL)
    child(ROOT, WRITER, capabilities=None, title=None)
    permission(ROOT, "inspect writer")
    expect_selected("inspect writer")
    permission(WRITER, "ended request")
    child(ROOT, WRITER, state=WAITING)
    child(ROOT, WRITER, state={"state": "idle"})
    reply = read()
    assert reply["id"] == "ended request" and reply["result"]["outcome"] == {"outcome": "cancelled"}
    child(ROOT, WRITER, state=RUNNING, capabilities=CANCEL)
    child(ROOT, WRITER, state={"state": "idle", "stopReason": "error",
                               "error": {"code": -32603, "message": "review failed"}})
    chunk(ROOT, "finished")
    send(id=prompt["id"], result={"stopReason": "end_turn"})
elif scenario in ("first", "resume"):
    if scenario == "first":
        announce()
        child(ROOT, RELAY, state=DONE)
    else:
        child(RELAY, WRITER, state=RUNNING)
        permission(ROOT, "inspect resumed writer")
        expect_selected("inspect resumed writer")
    chunk(WRITER, "earlier progress", message="progress")
    chunk(WRITER, scenario + " ")
    chunk(WRITER, "finding")
    child(RELAY, WRITER, state=DONE)
    chunk(ROOT, scenario)
    send(id=prompt["id"], result={"stopReason": "end_turn"})
else:
    raise ValueError(scenario)
