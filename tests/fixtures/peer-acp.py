#!/usr/bin/env python3
"""ACP harness using the HTTP MCP transport on both session/new and session/load."""
import json
import sys
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
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


def interrupt(thread, run, key="stop-first"):
    return call("thread_interrupt", threadId=thread, runId=run, clientRequestId=key,
                reason="Continue with the second review")


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


def approve_mcp(name):
    tool_name = "mcp__huggler__" + name
    emit(id="approve-mcp", method="session/request_permission", params={
        "sessionId": session, "toolCall": {"toolCallId": str(uuid.uuid4()), "title": tool_name,
            "_meta": {"claudeCode": {"toolName": tool_name, "mcpServer": {"name": "huggler"}}}},
        "options": [{"optionId": "allow-once", "name": "Yes", "kind": "allow_once"}],
    })
    decision = json.loads(next(sys.stdin))
    assert decision["id"] == "approve-mcp"
    assert decision["result"]["outcome"] == {"outcome": "selected", "optionId": "allow-once"}


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
        client = params["clientInfo"]
        assert client["name"] in ("huggler", "atlas")
        emit(id=frame["id"], result={"protocolVersion": 1, "agentCapabilities": {
            "loadSession": True, "mcpCapabilities": {"http": True}}})
    elif method in ("session/new", "session/load"):
        assert Path(params["cwd"]).resolve() == Path.cwd()
        session = params.get("sessionId", str(uuid.uuid4()))
        server, = params["mcpServers"]
        assert server["name"] == client["name"] and server["type"] == "http"
        auth = {h["name"]: h["value"] for h in server["headers"]}
        init = mcp("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                  "clientInfo": {"name": "peer-fixture", "version": "1"}})
        info = init["result"]["serverInfo"]
        assert info["name"] == client["name"]
        if client["name"] == "atlas":
            assert info["version"] == client["version"] == "7"
            assert info["title"] == client["title"] == "Atlas Studio"
            assert init["result"]["instructions"] == "Use the directory to find collaborators."
        emit(id=frame["id"], result={"sessionId": session, "modes": {"currentModeId": "default"}})
    elif method == "session/prompt":
        prompt_id = frame["id"]
        text = params["prompt"][0]["text"]
        tools = mcp("tools/list", {})["result"]["tools"]
        if client["name"] == "atlas":
            if text == "exercise catalog":
                assert {t["name"] for t in tools} == {"find", "capabilities"}
                available = call("capabilities")
                assert available["deferred"] and set(available["tools"]) == {
                    "find", "capabilities", "start_peer", "deliver"}
                assert call("deliver") == {"failure": "denied"}
                assert call("find", query="task_cancel")["tools"] == []
                entry, = call("find", query="deliver")["tools"]
                assert (entry["name"], entry["title"], entry["description"]) == (
                    "deliver", "Send a note", "Send the original Huggler wording.")
                assert entry["inputSchema"]["properties"]["clientRequestId"]["description"] == "Retry key"
                assert call("thread_send") == {"failure": "denied"}
                call("find", query="start_peer")
                peer, = call("start_peer", clientRequestId="make", threads=[{}])["threads"]
                receipt = call("deliver", threadId=peer["threadId"], clientRequestId="note",
                               message="Keep Huggler in this message.")
                finish(json.dumps(receipt))
            else:
                finish(text)
            continue
        if text == "direct notice":
            assert {t["name"] for t in tools} == {"orchestrator_capabilities", "send_to_user"}
            assert call("orchestrator_capabilities")["deferred"] is False
            assert call("tool_search", query="request_secret")["code"] == "forbidden"
            assert call("send_to_user", clientRequestId="notice", text=text)["delivered"]
            finish(text)
            continue
        if text.startswith("managed secret "):
            assert call("orchestrator_capabilities")["deferred"] is True
            names = {t["name"] for t in tools}
            assert names == {"tool_search", "orchestrator_capabilities"}, names
            case = text.split()[-1]
            def secret():
                return call("request_secret", clientRequestId=case, label="Build credential")
            assert secret()["code"] == "forbidden"
            assert call("tool_search", query="thread_send")["tools"] == []
            assert deliver("sibling", "key", "no")["code"] == "forbidden"
            for name in ("request_secret", "send_to_user"):
                assert call("tool_search", query=name)["tools"][0]["name"] == name
            assert "request_secret" in {t["name"] for t in mcp("tools/list", {})["result"]["tools"]}
            for _ in range(2):
                assert call("send_to_user", clientRequestId="notice", text=case)["delivered"]
            request = secret()
            while request["state"] == "requested":
                time.sleep(.03)
                request = secret()
            finish(request["reference"] or request["state"])
            continue
        assert {"thread_list", "thread_send"} <= {t["name"] for t in tools}
        if text.startswith("Use the huggler"):
            peers = call("thread_list")["threads"]
            assert [t["id"] for t in peers] == ["reviewer"]
            assert deliver("outside", "review-1", "unreachable")["code"] == "forbidden"
            bad = mcp("tools/call", {"name": "thread_send", "arguments": {
                "threadId": "reviewer", "message": "bad", "clientRequestId": "review-1", "mode": "admin"}})
            assert "error" in bad or bad["result"]["isError"]
            review = text.split('quoted text: "', 1)[1].split('". After sending', 1)[0]
            deliver(peers[0]["id"], "review-1", review)
            finish("Review requested")
        elif text.startswith("checkout task "):
            args = json.loads(text.removeprefix("checkout task "))
            result = call("delegate_task", mode="wait", timeoutMs=4000, **args)
            finish(json.dumps(result))
        elif text.startswith("edit tracked "):
            tracked = Path("tracked.txt")
            original = tracked.read_text()
            tracked.write_text(text.removeprefix("edit tracked "))
            finish(original)
        elif text.startswith("create peers "):
            assert "create_threads" in {t["name"] for t in tools}
            args = json.loads(text.removeprefix("create peers "))
            with ThreadPoolExecutor(max_workers=2) as pool:
                calls = [pool.submit(call, "create_threads", **args) for _ in range(2)]
                first, retry = [c.result() for c in calls]
            assert first == retry, (first, retry)
            finish(json.dumps(first))
        elif text.startswith("check question "):
            case = json.loads(text.removeprefix("check question "))
            target = {"threadId": "planner"}
            question = {**target, "requestId": case["id"]}
            answers = {"schema_preference": "Strict schemas (Recommended)"}
            def pending(action, **args):
                return call("pending_request", action=action, **args)
            assert pending("list") == {"requestIds": []}
            if case["outside"]:
                assert pending("list", **target)["code"] == "forbidden"
                assert pending("read", **question)["code"] == "forbidden"
                assert pending("respond", answers=answers, **question)["code"] == "forbidden"
            else:
                assert pending("list", **target) == {"requestIds": [case["id"]]}
                data = pending("read", **question)
                assert data == {"requestId": case["id"], "questions": case["questions"]}
                permission = {"threadId": "held", "requestId": case["permission"]}
                assert pending("list", threadId="held") == {"requestIds": []}
                assert pending("read", **permission)["code"] == "not_found"
                assert pending("respond", answers={"decision": "done"}, **permission)["code"] == "not_found"
                assert pending("respond", answers={"wrong": "value"}, **question)["code"] == "invalid"
                def respond():
                    return pending("respond", answers=answers, **question)
                with ThreadPoolExecutor(max_workers=2) as pool:
                    replies = [future.result() for future in [pool.submit(respond), pool.submit(respond)]]
                assert {"requestId": case["id"]} in replies and {"code": "already_answered"} in replies, replies
                assert respond()["code"] == "already_answered"
                assert pending("list", **target) == {"requestIds": []}
            finish("Checked question")
        elif text == "start branch review":
            delegate("branch review", "branch")
            finish("Delegated for review")
        elif text.startswith("delegate "):
            action = text.removeprefix("delegate ")
            child = {"nested": "delegate leaf", "subtree": "branch review",
                     "settled": "immediate review", "rounds": "immediate review"}.get(action, "held review")
            task = delegate(child, "review")
            task_id = task["taskId"]
            if action == "siblings":
                delegate("held review", "review-again")
            if action == "followup":
                followup = deliver(task["childThreadId"], "revise", "revised finding")
                assert followup["delivery"] == "queued"
            if action in ("ack", "cancel", "recover", "siblings", "subtree", "settled", "rounds"):
                hold()
            else:
                finish("Delegated for review")
        elif text.startswith("nested followups "):
            target = text.split()[-1]
            for label, delivery in [("alpha", "started"), ("beta", "queued")]:
                assert deliver(target, label, "nested " + label)["delivery"] == delivery
            hold()
        elif text.startswith("report "):
            hold()
        elif text.startswith("outside interrupt "):
            target, run = text.split()[-2:]
            assert interrupt(target, run)["code"] == "forbidden"
            finish("Access denied")
        elif text.startswith("retry interrupt "):
            target, first, second = text.split()[-3:]
            assert interrupt(target, first)["status"] == "interrupted"
            assert interrupt(target, second)["code"] == "conflict"
            finish("Interrupt retried")
        elif text.startswith(("followups ", "retry followups ", "interrupt followups ")):
            target = text.split()[-1]
            running = deliver(target, "review", "interrupt this review")
            queued = deliver(target, "next", "second review")
            assert running["delivery"] == "started" and queued["delivery"] == "queued"
            assert running["taskId"] != queued["taskId"]
            assert running == deliver(target, "review", "interrupt this review")
            assert queued == deliver(target, "next", "second review")
            assert deliver(target, "next", "different")["code"] == "conflict"
            if text.startswith("retry "):
                finish("Retried both follow-ups")
            else:
                hold()
        elif text == "branch review":
            leaf = delegate("held review", "leaf")
            assert deliver(leaf["childThreadId"], "revise", "revised finding")["delivery"] == "queued"
            hold()
        elif text.startswith("First call the huggler thread_list"):
            approve_mcp("thread_list")
            call("thread_list")
            finish("2+2=4")
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
        elif text.startswith("echo "):
            with Path("delivery-log").open("a") as log:
                print(text, file=log)
            finish(text)
        elif text == "hold":
            hold()
        elif text == "queue-two":
            for key in ("first", "second"):
                assert deliver("author", key, key)["delivery"] == "queued"
            hold()
        elif text.startswith("Peer message from Huggler "):
            attribution, message = text.split("\n\n", 1)
            sender = json.loads(attribution.removeprefix("Peer message from Huggler "))["thread_id"]
            if message == "read batch marker":
                finish(Path("batch-marker").read_text())
            elif message.startswith("nested "):
                label = message.split()[-1]
                delegate("report " + label, label)
                hold()
            elif message in ("interrupt this review", "second review"):
                hold()
            elif message.startswith("Review 2+2."):
                approve_mcp("thread_send")
                first = deliver(sender, "reply-1", "2+2=4")
                approve_mcp("thread_send")
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
        if text.startswith("Peer message from Huggler "):
            if message == "interrupt this review":
                emit(id=prompt_id, result={"stopReason": "cancelled"})
            elif message == "second review":
                finish("Second finding")
            elif message.startswith("nested "):
                finish("Waiting for nested review")
            else:
                raise AssertionError("unexpected held message")
            continue
        if text in ("delegate ack", "delegate rounds"):
            status = call("task_status", taskId=task_id)
            assert status["result"] == "2+2=4" and status["delivery"] == "acknowledged"
        elif text == "delegate cancel":
            assert call("task_cancel", taskId=task_id)["delivery"] == "disposed"
        elif text in ("delegate subtree", "delegate settled"):
            assert call("task_cancel", taskId=task_id, clientRequestId="stop-review")["delivery"] == "disposed"
        elif text.startswith("interrupt followups "):
            first, second = running["runId"], queued["runId"]
            assert interrupt(target, second, "queued")["code"] == "invalid"
            assert interrupt("author", first)["code"] == "not_found"
            assert interrupt(target, first) == {"threadId": target, "runId": first,
                                                "status": "interrupt_requested"}
            while call("task_status", taskId=queued["taskId"])["status"] == "queued":
                time.sleep(0.01)
            assert interrupt(target, first)["status"] == "interrupted"
            assert interrupt(target, second)["code"] == "conflict"
        if text.startswith("report "):
            finish(text.split()[-1])
        else:
            finish("2+2=4" if text == "held review" else "released")
