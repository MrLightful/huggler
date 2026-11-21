#!/usr/bin/env python3
"""Loopback-only OpenCode peer. Reuse recorded response and event bodies for both APIs."""
import base64
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import sys
import threading
from urllib.parse import parse_qs, urlsplit

from replay import check_tools, frames


class Peer(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, data=None, status=200):
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        if data is not None:
            self.wfile.write(json.dumps(data).encode())

    def incomplete_reply(self, hold):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", "100")
        self.end_headers()
        self.wfile.write(b"{")
        self.wfile.flush()
        if hold:
            Path("holding").touch()
            self.server.stopped.wait(10)

    def handle_request(self):
        url = urlsplit(self.path)
        route = url.path
        query = parse_qs(url.query)
        method = self.command
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length)) if length else {}
        if password and self.headers.get("Authorization") != auth:
            return self.reply({}, 401)

        if route == "/api/info":
            return self.reply({"version": version, "pid": os.getpid(), "urls": [], "paths": {"tmp": "/tmp"}} if v2 else {}, 200 if v2 else 404)
        if not route.startswith("/api/"):
            assert query == {"directory": [os.getcwd()]}
        if route in ("/api/model", "/api/experimental/mcp/huggler"):
            assert query == {"location[directory]": [os.getcwd()]}
        if route == "/global/health" and not v2:
            return self.reply({"version": version, "healthy": True})
        if route == ("/api/model" if v2 else "/provider"):
            return self.reply({"data": [model]} if v2 else {"all": [{"id": model["providerID"], "models": {model["id"]: {}}}]})
        if route == ("/api/experimental/mcp/huggler" if v2 else "/mcp"):
            assert managed and method == ("PUT" if v2 else "POST")
            assert body["config"]["type"] == "remote" and body["config"]["oauth"] is False
            check_tools(body["config"])
            self.server.registered = True
            return self.reply(None, 204) if v2 else self.reply({"huggler": {"status": "connected"}})
        if route == base and method == "POST":
            assert body["title"]
            rules = body["permissions" if v2 else "permission"]
            assert len(rules) == (2 if managed else 1)
            if v2:
                assert body["location"]["directory"] == os.getcwd() and body["model"] == model
            return self.reply(rows[0]["data"])
        if route == session_path and method == "GET":
            return self.reply(rows[0]["data"])
        if route == session_path + "/model" and v2:
            assert body["model"] == model
            return self.reply({})
        if route == "/api/event" and v2:
            assert self.headers["Accept"] == "text/event-stream"
            ready = threading.Event()
            self.server.listener = ready
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            if not ready.wait(10):
                return
            events = [r["event"] for r in rows[2:]]
            recorded = rows[1]["data"]["data"]["id"]
            events = [
                {**e, "data": {**e["data"], "inboxID": self.server.inbox}}
                if e["data"].get("inboxID") == recorded else e for e in events
            ]
            foreign = {"type": "session.execution.failed", "data": {"sessionID": "other"}}
            prompt = self.server.prompt
            if prompt in ("queued-disconnect", "truncated", "wrong-id", "wrong-session", "hold-admission"):
                events = []
            if prompt in ("disconnect", "form", "hold", "joined"):
                events = events[:1]
            if prompt == "form":
                events.append({"type":"form.created","data":{"form":{"sessionID":session["id"]}}})
            if prompt == "joined":
                events.append({"type":"session.inbox.delivered","data":{"sessionID":session["id"],"inboxID":"other"}})
            for event in [foreign, *events]:
                packet = ("data: " + json.dumps(event, ensure_ascii=False) + "\r\n\r\n").encode()
                self.wfile.write(packet)
                if event["type"] in ("session.text.delta", "session.step.ended"):
                    self.wfile.write(packet)
            if prompt == "hold":
                Path("holding").touch()
                self.server.stopped.wait(10)
            return
        if v2 and method == "DELETE" and route == session_path + "/inbox/" + self.server.inbox:
            Path("cancelled-inbox").touch()
            self.server.stopped.set()
            return self.reply(None, 204)
        if route == session_path + ("/interrupt" if v2 else "/abort"):
            Path("aborted").touch()
            self.server.stopped.set()
            return self.reply({"interrupted": True} if v2 else True)
        if route == session_path + ("/prompt" if v2 else "/message"):
            assert self.server.registered == managed
            prompt = body["text"] if v2 else body["parts"][0]["text"]
            self.server.stopped.clear()
            if prompt == "refused":
                return self.reply({}, 409)
            if v2:
                assert body["delivery"] == "queue"
                inbox = body["id"]
                assert inbox.startswith("msg_") and inbox not in self.server.seen
                self.server.seen.add(inbox)
                self.server.inbox = inbox
                self.server.prompt = prompt
                self.server.listener.set()
                if prompt in ("truncated", "hold-admission"):
                    return self.incomplete_reply(prompt == "hold-admission")
                admission = {**rows[1]["data"]["data"], "id": inbox}
                if prompt == "wrong-id":
                    admission["id"] = "msg_other"
                if prompt == "wrong-session":
                    admission["sessionID"] = "other"
                return self.reply({"data": admission})
            assert body["model"] == {"modelID": model["id"], "providerID": model["providerID"]}
            if prompt in ("hold", "truncated"):
                return self.incomplete_reply(prompt == "hold")
            return self.reply({"info":rows[2]["event"]["properties"]["info"], "parts":[rows[1]["event"]["properties"]["part"]]})
        self.reply({}, 404)

    do_GET = do_POST = do_PUT = do_DELETE = handle_request


if __name__ == "__main__":
    flavor = sys.argv[1]
    v2 = flavor == "open2"
    rows = frames(flavor)
    managed = "serve" in sys.argv
    version = "2.0.18" if v2 else "1.14.39"
    session = rows[0]["data"]["data"] if v2 else rows[0]["data"]
    model = {"providerID":"opencode","id":"space-bunny-free"} if v2 else {"providerID":"openai","id":"gpt-5.4-mini"}
    (session["location"] if v2 else session)["directory"] = os.getcwd()
    base = "/api/session" if v2 else "/session"
    session_path = f"{base}/{session['id']}"
    password = os.environ.get("OPENCODE_SERVER_PASSWORD") if managed else ("denied" if "deny" in sys.argv else None)
    if managed:
        assert password and sys.argv[-5:] == ["serve", "--hostname", "127.0.0.1", "--port", "0"]
    auth = "Basic " + base64.b64encode(f"opencode:{password}".encode()).decode()
    with ThreadingHTTPServer(("127.0.0.1", 0), Peer) as server:
        server.seen = set()
        server.registered = False
        server.stopped = threading.Event()
        port = server.server_port
        Path("last-port").write_text(str(port))
        print(f"server listening on http://127.0.0.1:{port}", flush=True)
        server.serve_forever()
