#!/usr/bin/env python3
"""Deterministic app-server peer for the desktop edit, test and commit scenario."""
import json
from pathlib import Path
import subprocess
import sys
import time
from uuid import uuid4

file = Path("answer [v1].txt")


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
        assert Path(subprocess.check_output(
            ["git", "rev-parse", "--show-toplevel"], text=True).rstrip("\n")) == Path.cwd()
        assert params["approvalPolicy"] == "on-request"
        session = params["threadId"] if method == "thread/resume" else str(uuid4())
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
            text = ("Checking the requested filename. I will confirm the path with you before "
                    "creating the answer file in this checkout.")
            for character in text:
                notify("item/agentMessage/delta", delta=character)
            notify("item/commandExecution/outputDelta", itemId="inspect", delta="Checkout ready")
            emit(id="confirm", method="item/tool/requestUserInput", params={
                "threadId": session, "turnId": turn, "questions": [
                    {"id": "filename", "question": "Which file should contain the answer?"}]})
        elif prompt == "Wait for my next instruction.":
            notify("item/agentMessage/delta", delta="Waiting for your instruction.")
        elif prompt == "Review the staged result.":
            complete("Reviewed the staged result.")
        elif prompt == "Watch for the review signal.":
            deadline = time.monotonic() + 30
            while not Path(".git/huggler-review-signal").exists():
                assert time.monotonic() < deadline, "Review signal timed out"
                time.sleep(.05)
            emit(method="turn/completed", params={"threadId": session, "turn": {
                "id": turn, "status": "failed", "error": {"message": "Review check failed"}}})
        else:
            assert prompt == "Use a single line."
            assert file.read_text() == "41\n"
            file.write_text("42\n")
            complete("Corrected the answer.")
    else:
        emit(id=frame["id"], result={})
