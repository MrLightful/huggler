#!/usr/bin/env python3
"""ACP harness using the HTTP MCP transport on both session/new and session/load."""
import json
import sys
import time
import uuid
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen


def emit(**frame):
    print(json.dumps({"jsonrpc": "2.0", **frame}), flush=True)


def mcp(method, params, headers=None):
    frame = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    req = Request(server["url"], json.dumps(frame).encode(), headers={
        "Content-Type": "application/json", "Accept": "application/json, text/event-stream",
        **(auth if headers is None else headers),
    })
    with urlopen(req, timeout=5) as response:
        return json.load(response)


def call(name, **arguments):
    result = mcp("tools/call", {"name": name, "arguments": arguments})
    assert "error" not in result, result
    return result["result"]["structuredContent"]


def deliver(thread, key, text, **extra):
    return call("thread_send", threadId=thread, clientRequestId=key, message=text, **extra)


def delegate(task, key):
    args = dict(task=task, title="Reviewer", mode="async", isolation="shared",
                target={"providerInstanceId": "claudeAgent"}, clientRequestId=key)
    created = call("delegate_task", **args)
    assert call("delegate_task", **args)["taskId"] == created["taskId"]
    assert call("delegate_task", **{**args, "task": "different"})["code"] == "conflict"
    return created


def hold():
    emit(id="permission", method="session/request_permission", params={
        "sessionId": session, "toolCall": {"toolCallId": "hold", "title": "Hold this turn"},
        "options": [{"optionId": "done", "name": "Continue", "kind": "allow_once"}],
    })


def finish(answer):
    emit(method="session/update", params={"sessionId": session, "update": {
        "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": answer}
    }})
    emit(id=prompt_id, result={"stopReason": "end_turn"})


for line in sys.stdin:
    frame = json.loads(line)
    method = frame.get("method")
    params = frame.get("params", {})
    if method == "initialize":
        emit(id=frame["id"], result={"protocolVersion": 1, "agentCapabilities": {
            "loadSession": True, "mcpCapabilities": {"http": True}}})
    elif method in ("session/new", "session/load"):
        session = params.get("sessionId", str(uuid.uuid4()))
        server, = params["mcpServers"]
        assert server["name"] == "huggler" and server["type"] == "http"
        auth = {h["name"]: h["value"] for h in server["headers"]}
        init = mcp("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                  "clientInfo": {"name": "peer-fixture", "version": "1"}})
        assert init["result"]["serverInfo"]["name"] == "huggler"
        emit(id=frame["id"], result={"sessionId": session, "modes": {"currentModeId": "default"}})
    elif method == "session/prompt":
        prompt_id = frame["id"]
        text = params["prompt"][0]["text"]
        tools = mcp("tools/list", {})["result"]["tools"]
        assert {"thread_list", "thread_send"} <= {t["name"] for t in tools}
        if text.startswith("Use the huggler"):
            peers = call("thread_list")["threads"]
            assert [t["id"] for t in peers] == ["reviewer"]
            assert deliver("outside", "review-1", "unreachable")["code"] == "forbidden"
            bad = mcp("tools/call", {"name": "thread_send", "arguments": {
                "threadId": "reviewer", "message": "bad", "clientRequestId": "review-1", "mode": "admin"}})
            assert "error" in bad or bad["result"]["isError"]
            deliver(peers[0]["id"], "review-1", "Review 2+2")
            finish("Review requested")
        elif text.startswith("delegate "):
            action = text.removeprefix("delegate ")
            child = {"nested": "delegate leaf", "subtree": "branch review",
                     "settled": "immediate review"}.get(action, "held review")
            task = delegate(child, "review")
            task_id = task["taskId"]
            if action == "siblings":
                delegate("held review", "review-again")
            if action == "followup":
                followup = deliver(task["childThreadId"], "revise", "revised finding")
                assert followup["delivery"] == "queued"
            if action in ("ack", "cancel", "recover", "siblings", "subtree", "settled"):
                hold()
            else:
                finish("Delegated for review")
        elif text == "branch review":
            leaf = delegate("held review", "leaf")
            assert deliver(leaf["childThreadId"], "revise", "revised finding")["delivery"] == "queued"
            hold()
        elif text == "held review":
            hold()
        elif text.startswith("wait "):
            case = text.removeprefix("wait ")
            review = {"ready": "immediate review", "approval": "held review",
                      "timeout": "gated review", "revoke": "gated review"}[case]
            args = dict(clientRequestId="wait-review", task=review)
            status = call("delegate_task", mode="wait", timeoutMs=20 if case == "timeout" else 4000, **args)
            if case == "revoke":
                assert status["code"] == "forbidden", status
            else:
                assert status["timedOut"] == (case == "timeout"), status
                assert status["awaitingApproval"] == (case == "approval"), status
                assert status["delivery"] == ("acknowledged" if case == "ready" else "pending"), status
                assert call("delegate_task", mode="async", **args)["taskId"] == status["taskId"]
            finish("wait returned " + case)
        elif text in ("gated review", "immediate review"):
            if text == "gated review":
                while not Path("release-review").exists():
                    time.sleep(0.02)
            finish("2+2=4")
        elif text.startswith("inspect "):
            task_id = text.removeprefix("inspect ")
            assert call("task_status", taskId=task_id)["code"] == "forbidden"
            assert call("task_cancel", taskId=task_id)["code"] == "forbidden"
            finish("Access denied")
        elif text.startswith("retry stop "):
            task_id = text.removeprefix("retry stop ")
            assert call("task_cancel", taskId=task_id, clientRequestId="stop-review")["delivery"] == "disposed"
            finish("Stop retried")
        elif text.startswith("Huggler delegated task results\n"):
            results = json.loads(text.split("\n", 1)[1])
            finish("Received: " + ", ".join(r["result"] for r in results))
        elif text == "hold":
            hold()
        elif text == "queue-two":
            for key in ("first", "second"):
                assert deliver("author", key, key)["delivery"] == "queued"
            hold()
        elif text.startswith("Peer message from Huggler "):
            attribution, message = text.split("\n\n", 1)
            sender = json.loads(attribution.removeprefix("Peer message from Huggler "))["thread_id"]
            if message == "Review 2+2":
                first = deliver(sender, "reply-1", "2+2=4")
                retry = deliver(sender, "reply-1", "2+2=4")
                assert first == retry
                assert deliver(sender, "reply-1", "different")["code"] == "conflict"
                finish(first["messageId"] + " " + retry["messageId"])
            else:
                finish("Received " + message)
        else:
            raise AssertionError("unknown fixture prompt")
    elif frame["id"] == "permission":
        if text == "queue-two":
            try:
                deliver("author", "revoked", "unreachable")
            except HTTPError as e:
                assert e.code == 401
            else:
                raise AssertionError("revoked caller retained access")
        if text == "delegate ack":
            status = call("task_status", taskId=task_id)
            assert status["result"] == "2+2=4" and status["delivery"] == "acknowledged"
        elif text == "delegate cancel":
            assert call("task_cancel", taskId=task_id)["delivery"] == "disposed"
        elif text in ("delegate subtree", "delegate settled"):
            assert call("task_cancel", taskId=task_id, clientRequestId="stop-review")["delivery"] == "disposed"
        finish("2+2=4" if text == "held review" else "released")
