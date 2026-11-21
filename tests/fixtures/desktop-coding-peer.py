#!/usr/bin/env python3
"""Deterministic app-server peer for the desktop edit, test and commit scenario."""
import json
from pathlib import Path
import sys

file = Path("answer [v1].txt")
session = "coding-cycle"


def emit(**frame):
    print(json.dumps(frame), flush=True)


def notify(method, **params):
    emit(method=method, params=dict(threadId=session, turnId=turn, **params))


def complete(text, kind="agentMessage"):
    notify("item/completed", item={"type": kind, "id": turn + "-result", "text": text})
    emit(method="turn/completed", params={"threadId": session, "turn": {"id": turn, "status": "completed"}})


assert sys.argv[1:] == ["app-server"]
for line in sys.stdin:
    frame = json.loads(line)
    params = frame.get("params", {})
    method = frame.get("method")
    if "id" not in frame:
        continue
    if frame["id"] == "confirm":
        assert frame["result"] == {"answers": {"filename": {"answers": [file.name]}}}
        emit(id="write", method="item/fileChange/requestApproval",
             params={"threadId": session, "turnId": turn, "itemId": "edit", "reason": "Create the answer file"})
    elif frame["id"] == "write":
        assert frame["result"] == {"decision": "accept"}
        file.write_text("41\n")
        complete("Created the answer file.")
    elif method in ("thread/start", "thread/resume"):
        assert Path(params["cwd"]) == Path.cwd()
        assert params["approvalPolicy"] == "on-request"
        if method == "thread/resume":
            assert params["threadId"] == session
        sandbox = params["sandbox"]
        emit(id=frame["id"], result={"thread": {"id": session}, "model": "fixture-model"})
    elif method == "turn/start":
        prompt = params["input"][0]["text"]
        if params["collaborationMode"]["mode"] == "plan":
            assert prompt == "Plan the answer file."
            assert sandbox == "read-only" and params["sandboxPolicy"] == {"type": "readOnly"}
            turn = "plan"
            emit(id=frame["id"], result={"turn": {"id": turn}})
            complete("Create the answer file and check its value.", "plan")
            continue
        assert sandbox == "workspace-write"
        assert params["sandboxPolicy"] == {"type": "workspaceWrite", "networkAccess": False}
        turn = "revise" if file.exists() else "implement"
        emit(id=frame["id"], result={"turn": {"id": turn}})
        if prompt == "Create the answer file.":
            notify("item/agentMessage/delta", delta="Checking the requested filename.")
            notify("item/commandExecution/outputDelta", itemId="inspect", delta="Checkout ready")
            emit(id="confirm", method="item/tool/requestUserInput", params={
                "threadId": session, "turnId": turn, "questions": [
                    {"id": "filename", "question": "Which file should contain the answer?"}]})
        elif prompt == "Wait for my next instruction.":
            notify("item/agentMessage/delta", delta="Waiting for your instruction.")
        elif prompt == "Review the staged result.":
            complete("Reviewed the staged result.")
        else:
            assert prompt.startswith("Use a single line.\n\n")
            assert "Failed · exit 7" in prompt and "expected 42, got 41" in prompt
            assert file.read_text() == "41\n"
            file.write_text("42\n")
            complete("Corrected the answer.")
    else:
        emit(id=frame["id"], result={})
