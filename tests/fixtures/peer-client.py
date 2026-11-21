"""Exercise the operating-host HTTP contract from a process without a Huggler runtime."""
import json
import sys
import time
from urllib.request import ProxyHandler, Request, build_opener


def main():
    config = json.load(sys.stdin)
    http = build_opener(ProxyHandler({}))

    def call(method, **params):
        body = {"method": method, "target": config["target"], **params}
        req = Request(config["url"], data=json.dumps(body).encode(), headers={
            "Content-Type": "application/json", "Authorization": "Bearer " + config["bearer"],
        })
        with http.open(req, timeout=5) as response:
            return json.load(response)

    prompt = "please review"
    write = dict(sender_run=config["sender_run"], key="message",
                 action={"operation": "send", "text": prompt})
    accepted = call("write", **write)
    assert accepted["type"] == "accepted" and accepted["address"] == config["target"]
    assert call("write", **write) == accepted
    deadline = time.monotonic() + 10
    while True:
        result = call("result", run=accepted["receipt"]["runId"])
        if result["status"] == "completed":
            break
        assert result["status"] in ("queued", "starting", "running", "waiting")
        assert time.monotonic() < deadline, "timed out waiting for remote reply"
        time.sleep(.025)
    assert result["text"].startswith("reply ") and result["text"].endswith(prompt), result
    history = call("read")
    assert history["address"] == config["target"]
    assert [m["message"]["text"] for m in history["history"]["messages"]] == [prompt, result["text"]]


if __name__ == "__main__":
    main()
