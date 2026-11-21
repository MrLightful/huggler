#!/usr/bin/env python3
"""Accept one turn, then stay silent until the parent closes stdin."""
import fcntl
import json
import os
from pathlib import Path
import sys
import time

Path("pid").write_text(str(os.getpid()))
for line in sys.stdin:
    frame = json.loads(line)
    if "id" not in frame:
        continue
    result = {}
    if frame["method"] == "thread/start":
        result = {"thread": {"id": "persisted"}}
    if frame["method"] == "turn/start":
        with open("invocations", "a") as log:
            log.write("turn/start\n")
        # Model a provider helper that keeps working after its launcher dies.
        if Path("model.lock").exists():
            with open("model.lock", "r+") as lease:
                fcntl.flock(lease, fcntl.LOCK_EX)
                if os.fork() == 0:
                    time.sleep(10)
                    os._exit(0)
        result = {"turn": {"id": "accepted"}}
    print(json.dumps({"id": frame["id"], "result": result}), flush=True)
    if frame["method"] == "turn/start" and Path("invalid-frame").exists():
        print("invalid frame", flush=True)
        break
