#!/usr/bin/env python3
"""Accept one turn, then stay silent until the parent closes stdin."""
import json
import os
from pathlib import Path
import sys

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
        result = {"turn": {"id": "accepted"}}
    print(json.dumps({"id": frame["id"], "result": result}), flush=True)
