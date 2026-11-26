"""Run against an installed wheel using the repository's protocol fixtures."""

import asyncio
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from threading import Event, Thread
import unittest
from unittest.mock import patch

from huggler import Attachment, Huggler, HugglerError, Peer, asgi_app


ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "tests/fixtures"
NATIVE_THREAD = "01a0d5ef-62bd-7c20-861b-0528ff7d86bd"
FIRST_REPLY = "provider thread resume fixture first turn complete"
FIRST_PROMPT = "Respond with exactly: " + FIRST_REPLY
SECOND_PROMPT = (
    "Using the conversation history available in this resumed thread, first repeat the exact "
    "final answer you gave in the previous turn. Then on a new line write exactly: "
    "provider thread resume fixture second turn complete"
)


async def changed(events, kind):
    async with asyncio.timeout(10):
        async for event in events:
            change = event["change"]
            if change["type"] == kind:
                return change
            if change["type"] == "run" and change["run"]["status"] == "failed":
                raise AssertionError(change["run"]["error"])
    raise AssertionError(f"stream closed before {kind}")


def command(rt, thread, id, kind, **fields):
    return rt.dispatch({"id": id, "thread_id": thread, "action": {"type": kind, **fields}})


class Consumer(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.checkout = self.root / "project with spaces"
        self.git("init", "-q", self.checkout)
        self.git("-C", self.checkout, "-c", "user.name=Fixture", "-c",
                 "user.email=fixture@example.test", "commit", "-qm", "base", "--allow-empty")
        self.codex = self.root / "codex"
        shutil.copyfile(FIXTURES / "replay-codex.py", self.codex)
        self.codex.chmod(0o755)

    def git(self, *args):
        subprocess.run(["git", *args], check=True, capture_output=True)

    def recording(self, start=1, fixture=None):
        if fixture is None:
            fixture = ROOT / "knowledge/reference/codex-resume/codex_transcript.ndjson"
        (self.root / "case.json").write_text(json.dumps({
            "start": start, "fixture": str(fixture), "version": "0.0.1",
        }))

    async def open(self, state="state", *, config=None, **kwargs):
        rt = await Huggler.open(self.root / state, config=config, **kwargs)
        self.addAsyncCleanup(self.close, rt)
        return rt

    async def close(self, rt):
        await rt.close()

    def rejects(self, code):
        return self.assertRaisesRegex(HugglerError, self.REASONS[code])

    REASONS = {"forbidden": "lacks access"}

    async def test_recorded_codex_resume_and_replay(self):
        self.recording()
        config = {"codex": {"executable": str(self.codex)}}
        async with await self.open(config=config) as rt:
            project = rt.open_project(self.checkout)["id"]
            rt.launch("launch", "thread", project, "Replay fixture: provider_thread_resume", "gpt-6-luna")
            rt.launch("sibling", "sibling", project, "Independent thread")
            start, events = rt.subscribe("thread")
            receipt = rt.send("first", "thread", FIRST_PROMPT)
            run = await asyncio.wait_for(rt.wait("thread", receipt["run_id"]), 10)
            self.assertEqual(run["status"], "completed", run["error"])
            snapshot = rt.read("thread")
            self.assertEqual(snapshot["native_thread"], NATIVE_THREAD)
            self.assertEqual(snapshot["messages"][-1]["text"], FIRST_REPLY)
            expected = []
            async with asyncio.timeout(5):
                while not expected or expected[-1]["sequence"] < snapshot["cursor"]:
                    expected.append(await anext(events))
            self.assertEqual(expected[-1]["sequence"], snapshot["cursor"])
            self.assertEqual(rt.read("sibling")["messages"], [])
            page = rt.detail("thread", limit=1)["history"]
            older = rt.detail("thread", before=page["older"])["history"]
            self.assertEqual(older["messages"][0]["message"]["id"], receipt["message_id"])
            with self.assertRaises(HugglerError) as overlap:
                await Huggler.open(self.root / "state", config=config)
            self.assertEqual(overlap.exception.code, "already_open")

        self.recording(41)
        async with await self.open(config=config) as rt:
            self.assertEqual(rt.send("first", "thread", FIRST_PROMPT), receipt)
            with self.assertRaises(HugglerError) as conflict:
                rt.send("first", "thread", "different intent")
            self.assertEqual(conflict.exception.code, "conflict")
            self.assertEqual(conflict.exception.detail["original_thread"], "thread")
            _, replay = rt.subscribe("thread", start["cursor"])
            self.assertEqual([await asyncio.wait_for(anext(replay), 5) for _ in expected], expected)
            receipt2 = rt.send("second", "thread", SECOND_PROMPT)
            run = await asyncio.wait_for(rt.wait("thread", receipt2["run_id"]), 10)
            self.assertEqual(run["status"], "completed", run["error"])
            self.assertEqual(run["ordinal"], 2)
            self.assertEqual(rt.read("thread")["native_thread"], NATIVE_THREAD)
            with self.assertRaises(HugglerError) as stale:
                rt.detail("thread", before=page["older"])
            self.assertEqual(stale.exception.code, "history_changed")
            rt.retain_events("thread", 1)
            with self.assertRaises(HugglerError) as expired:
                rt.subscribe("thread", start["cursor"])
            self.assertEqual(expired.exception.code, "cursor_expired")

    async def test_large_integer_cursors_and_cancelled_stream(self):
        async with await self.open() as rt:
            project = rt.open_project(self.checkout)["id"]
            rt.launch("launch", "thread", project, "Large cursor")
        large = 2**53 + 1
        with sqlite3.connect(self.root / "state/huggler.sqlite3") as db:
            db.execute("UPDATE event SET sequence = ?", (large,))
        async with await self.open() as rt:
            snapshot, stream = rt.subscribe("thread", 0)
            self.assertEqual(snapshot["cursor"], large)
            self.assertEqual((await anext(stream))["sequence"], large)
            with self.assertRaises(TimeoutError):
                await asyncio.wait_for(anext(stream), 0.02)
            command(rt, "thread", "model", "set_model", model="another")
            self.assertEqual((await asyncio.wait_for(anext(stream), 5))["sequence"], large + 1)
            await rt.close()
            with self.assertRaises(StopAsyncIteration):
                await anext(stream)

    async def test_cancelled_delivery_preserves_the_event(self):
        async with await self.open() as rt:
            project = rt.open_project(self.checkout)["id"]
            rt.launch("launch", "thread", project, "Cancelled delivery")
            snapshot, stream = rt.subscribe("thread")
            command(rt, "thread", "model", "set_model", model="fixture-model")
            loop = asyncio.get_running_loop()
            schedule = loop.call_soon_threadsafe
            delivery_queued = Event()

            def observe_delivery(*args, **kwargs):
                handle = schedule(*args, **kwargs)
                delivery_queued.set()
                return handle

            with patch.object(loop, "call_soon_threadsafe", observe_delivery):
                pending = asyncio.ensure_future(stream.next())
                await asyncio.sleep(0)
                # The bridge has queued delivery, but Python has yet to run it.
                self.assertTrue(delivery_queued.wait(5))
                self.assertTrue(pending.cancel())
                with self.assertRaises(asyncio.CancelledError):
                    await pending

            event = await asyncio.wait_for(anext(stream), 5)
            self.assertEqual(event["sequence"], snapshot["cursor"] + 1)
            self.assertEqual(event["change"]["model"], "fixture-model")

    async def test_implicit_close_allows_a_worker_callback_to_finish(self):
        fixture = FIXTURES / "context-provider.py"
        (self.checkout / "fail-before-prompt").touch()
        # A subprocess keeps a regression in Python object destruction from
        # hanging the installed-wheel suite.
        worker = """
import asyncio
from pathlib import Path
import sys
from threading import Event

from huggler import Huggler, HugglerError

entered, release = Event(), Event()

class Text:
    def run_error(self, diagnostic):
        entered.set()
        if not release.wait(5):
            raise RuntimeError("runtime destruction stalled the callback")
        return "scripted failure"

async def main():
    path = Path.cwd() / "state"
    runtime = await Huggler.open(path, config={"codex": {"executable": sys.argv[1]}}, agent_text=Text())
    project = runtime.open_project(Path.cwd())["id"]
    runtime.launch("launch", "thread", project, "Shutdown")
    runtime.send("send", "thread", "Fail in the fixture")
    assert entered.wait(5), "expected a callback under the storage lock"
    del runtime
    release.set()

    async with asyncio.timeout(5):
        while True:
            try:
                reopened = await Huggler.open(path)
                break
            except HugglerError as error:
                if error.code != "already_open":
                    raise
                await asyncio.sleep(0.01)
    async with reopened:
        assert reopened.read("thread")["runs"][0]["error"] == "scripted failure"

asyncio.run(main())
"""
        result = await asyncio.to_thread(
            subprocess.run, [sys.executable, "-c", worker, str(fixture)],
            cwd=self.checkout, capture_output=True, text=True, timeout=20,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    async def test_acp_permission_cancelled_wait_and_stop(self):
        config = {"claude_agent": {"executable": str(FIXTURES / "permission-acp.py")}}
        async with await self.open(config=config) as rt:
            project = rt.open_project(self.checkout)["id"]
            report = await rt.probe_provider("claudeAgent", self.checkout)
            self.assertEqual(report["transport"], "ACP v1")
            self.assertIsNone(report["version"])
            self.assertIsNone(report["authenticated"])
            self.assertTrue(report["resume"])
            command(rt, "approval", "launch", "launch_provider", project=project,
                    name="Approval", provider="claudeAgent")
            _, stream = rt.subscribe("approval")
            receipt = rt.send("permission", "approval", "Ask to edit")
            request = (await changed(stream, "permission_requested"))["request"]
            with self.assertRaises(TimeoutError):
                await asyncio.wait_for(rt.wait("approval", receipt["run_id"]), 0.02)
            self.assertEqual(rt.read("approval")["permissions"][0]["id"], request["id"])
            rt.respond_to_permission("approval", receipt["run_id"], request["id"], "no")
            run = await asyncio.wait_for(rt.wait("approval", receipt["run_id"]), 5)
            self.assertEqual(run["status"], "completed", run["error"])
            self.assertFalse((self.checkout / "approval-check.txt").exists())

            receipt = rt.send("stop", "approval", "Ask again")
            await changed(stream, "permission_requested")
            rt.stop_thread("interrupt", "approval")
            run = await asyncio.wait_for(rt.wait("approval", receipt["run_id"]), 5)
            self.assertEqual(run["status"], "interrupted", run["error"])

    async def test_recorded_codex_question(self):
        fixture = FIXTURES / "codex-plan-questions/codex_transcript.ndjson"
        self.recording(fixture=fixture)
        frames = [json.loads(line) for line in fixture.read_text().splitlines()]
        prompt = next(row["frame"]["params"]["input"][0]["text"] for row in frames
                      if row["type"] == "expect_outbound" and row["label"] == "turn/start")
        async with await self.open(config={"codex": {"executable": str(self.codex)}}) as rt:
            project = rt.open_project(self.checkout)["id"]
            rt.launch("launch", "planner", project, "Replay fixture: plan_questions", "gpt-6-luna")
            _, stream = rt.subscribe("planner")
            receipt = command(rt, "planner", "plan", "send_plan", text=prompt)
            request = (await changed(stream, "question_requested"))["request"]
            self.assertEqual(request["run_id"], receipt["run_id"])
            await rt.respond_to_question("planner", receipt["run_id"], request["id"],
                                         {"schema_preference": ["Strict schemas (Recommended)"]})
            self.assertTrue((await changed(stream, "question_closed"))["answered"])
            run = await asyncio.wait_for(rt.wait("planner", receipt["run_id"]), 10)
            self.assertEqual(run["status"], "completed", run["error"])

    async def test_managed_workspace_and_repository_links(self):
        async with await self.open(config={"storage": {"worktree_words": ["cedar"]}}) as rt:
            rt.set_preferences({"theme": "dark", "reduce_motion": True})
            self.assertEqual(rt.preferences(), {"theme": "dark", "reduce_motion": True})
            project = rt.open_project(self.checkout)["id"]
            worktree = rt.allocate_workspace_on_branch(project, "HEAD", {"exact": "fixture"})
            self.assertEqual(Path(worktree["root"]).name, "cedar")
            self.assertEqual(rt.allocated_workspaces(project), [worktree])
            self.assertEqual(await rt.setup_workspace(worktree["id"], "/usr/bin/true", []), "Ready")
            self.assertEqual(rt.workspace_setup_status(worktree["id"]), "Ready")
            rt.release_workspace(worktree["id"])
            self.assertEqual(rt.allocated_workspaces(project), [])
            named = rt.allocate_workspace_named(project, "HEAD", "orchid", {"exact": "manual-fixture"})
            self.assertEqual(Path(named["root"]).name, "orchid")
            rt.release_workspace(named["id"])
            self.assertEqual(rt.inspect_orphaned_worktrees()["entries"], [])
            rt.launch("launch", "thread", project, "Repository watch")
            repo = rt.bind_thread_repository("thread", "GitHub", "https://api.github.com", "demo/repo")
            self.assertEqual(repo["project"], project)
            link = rt.link_repository_item("thread", repo["id"], "7")
            self.assertEqual(rt.repository_links("thread")[0]["item"], "7")
            watch = rt.watch_repository_item("thread", link)
            self.assertIsNone(rt.sync_repository_watch(watch, 1, "open"))
            self.assertIsNone(rt.sync_repository_watch(watch, 1, "closed"))
            self.assertTrue(rt.unwatch_repository_item("thread", watch))
            self.assertTrue(rt.unlink_repository_item("thread", link))
            rt.delete_thread("thread")
            self.assertEqual(rt.threads(), [])

    async def test_scheduler_and_attachment_with_fixture_provider(self):
        fixture = FIXTURES / "context-provider.py"
        async with await self.open(config={"codex": {"executable": str(fixture)}}) as rt:
            project = rt.open_project(self.checkout)["id"]
            rt.launch("launch", "thread", project, "Schedules")
            spec = {"trigger": {"Webhook": {"event": "push"}}, "destination": "Continue",
                    "prompt": "Run scheduled fixture"}
            key = b"fixture signing key"
            schedule = rt.create_schedule("thread", spec, 100, key)
            self.assertEqual(rt.schedules("thread")[0][0], schedule)
            body = b'{"id":"fixture-event","event":"push"}'
            sig = hmac.new(key, body, hashlib.sha256).hexdigest()
            with self.rejects("forbidden"):
                rt.deliver_webhook(schedule, key, "00", body)
            occurrence = rt.deliver_webhook(schedule, key, sig, body)
            self.assertEqual(rt.deliver_webhook(schedule, key, sig, body), occurrence)
            self.assertEqual(rt.schedule_history(schedule), [occurrence])
            run = await asyncio.wait_for(rt.wait(occurrence["thread"], occurrence["run"]), 5)
            self.assertEqual(run["status"], "completed", run["error"])
            self.assertTrue(rt.delete_schedule(schedule))

            contents = b"captured file\n"
            upload = rt.prepare_upload("thread", "fixture.txt", "text/plain", contents)
            sent = command(rt, "thread", "attachment", "send_attachments", text="Inspect this",
                           uploads=[upload])
            run = await asyncio.wait_for(rt.wait("thread", sent["run_id"]), 5)
            self.assertEqual(run["status"], "completed", run["error"])
            file = rt.read_attachment_file("thread", sent["message_id"], 0)
            self.assertEqual((file.name, file.mime, file.bytes), ("fixture.txt", "text/plain", contents))
            file = Attachment.from_bytes("second.txt", contents)
            Attachment.validate_batch([file])
            receipt = command(rt, "thread", "enqueue-file", "enqueue_files",
                              text="Inspect another", files=[file.as_record()])
            run = await asyncio.wait_for(rt.wait("thread", receipt["run_id"]), 5)
            self.assertEqual(run["status"], "completed", run["error"])
            self.assertEqual(rt.read_attachment_file("thread", receipt["message_id"], 0).bytes, contents)
            receipt = rt.enqueue_files("native-files", "thread", "Inspect the file", [file])
            run = await asyncio.wait_for(rt.wait("thread", receipt["run_id"]), 5)
            self.assertEqual(run["status"], "completed", run["error"])
            self.assertEqual(rt.read_attachment_file("thread", receipt["message_id"], 0).bytes, contents)

    async def test_secret_requests_and_provider_environment(self):
        config = {
            "agent_tools": True,
            "agent_policy": {
                "allowed_tools": ["tool_search", "orchestrator_capabilities", "send_to_user", "request_secret"],
                "defer_tools": True,
                "secret_ttl_seconds": 30,
            },
            "instances": [{
                "id": "fixture", "driver": "claudeAgent",
                "executable": str(FIXTURES / "peer-acp.py"),
                "environment": {"HUGGLER_PYTHON_FIXTURE": "enabled"},
            }],
        }
        async with await self.open(config=config) as rt:
            self.assertEqual(rt.provider_environment("fixture"), {"HUGGLER_PYTHON_FIXTURE": "enabled"})
            project = rt.open_project(self.checkout)["id"]
            command(rt, "thread", "launch", "launch_instance", project=project,
                    name="Requests", instance="fixture")
            sent = rt.send("secret", "thread", "managed secret approved")
            async with asyncio.timeout(10):
                while not (requests := rt.secret_requests("thread")):
                    await asyncio.sleep(0.02)
            self.assertEqual(requests[0]["run"], sent["run_id"])
            secret = b"fixture signing value"
            rt.answer_secret("thread", requests[0]["id"], secret)
            run = await asyncio.wait_for(rt.wait("thread", sent["run_id"]), 5)
            self.assertEqual(run["status"], "completed", run["error"])
            reference = rt.read("thread")["messages"][-1]["text"]
            self.assertEqual(rt.consume_secret(project, reference), secret)
            with self.rejects("forbidden"):
                rt.consume_secret(project, reference)
            self.assertEqual(rt.user_notices("thread"), [(sent["run_id"], "approved")])

    async def test_repository_client_preserves_the_pull_request_snapshot(self):
        requests = []
        pull = {
            "id": "PR-fixture", "number": 7, "url": "https://example.test/pull/7",
            "state": "OPEN", "isDraft": False, "baseRefName": "main", "headRefOid": "a" * 40,
            "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN", "canBeRebased": True,
            "viewerCanEnableAutoMerge": True, "isMergeQueueEnabled": False,
            "isCrossRepository": False, "repository": {
                "nameWithOwner": "demo/repo", "viewerPermission": "WRITE",
                "mergeCommitAllowed": False, "squashMergeAllowed": True, "rebaseMergeAllowed": False,
            },
            "commits": {"nodes": []},
        }

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                requests.append((self.headers["Authorization"], body))
                if body["query"].startswith("mutation"):
                    data = {"mergePullRequest": {"pullRequest": {"id": pull["id"]}}}
                else:
                    data = {"repository": {"ref": {"associatedPullRequests": {"nodes": [pull]},
                                                   "recent": {"nodes": []}}, "recent": {"nodes": []}}}
                response = json.dumps({"data": data}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(response)))
                self.end_headers()
                self.wfile.write(response)

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        worker = Thread(target=server.serve_forever)
        worker.start()
        self.addCleanup(worker.join)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        async with await self.open() as rt:
            project = rt.open_project(self.checkout)["id"]
            bound = rt.bind_repository(project, "GitHub", f"http://127.0.0.1:{server.server_port}", "demo/repo")
            client = rt.repository_client(project, bound["id"], "Bearer fixture")
            result = await client.github_pull_requests("fixture")
            snapshot, = result.open
            self.assertEqual(result.recent, [])
            self.assertEqual(snapshot.number, 7)
            self.assertEqual(snapshot.state, "open")
            self.assertTrue(snapshot.can_merge_now)
            self.assertTrue(snapshot.allows_merge_method("squash"))
            await client.update_github_pull_request(snapshot, {"type": "merge", "method": "squash"})
            self.assertTrue(all(header == "Bearer fixture" for header, _ in requests))
            self.assertEqual(requests[-1][1]["variables"], {
                "id": "PR-fixture", "head": snapshot.head_commit, "method": "SQUASH",
            })
            pull["headRefOid"] = "b" * 40
            with self.assertRaises(HugglerError):
                await client.update_github_pull_request(snapshot, {"type": "merge", "method": "squash"})
            self.assertEqual(sum(body["query"].startswith("mutation") for _, body in requests), 1)

    async def test_host_callbacks_and_terminal(self):
        closed = []

        class Resource:
            def call(self, action):
                return {"type": "bytes", "mime": "text/plain", "data": action["type"].encode()}

            def close(self):
                closed.append(True)

        class DeviceHost:
            def open(self, thread, checkout, request):
                if (thread, checkout, request) != (
                    "thread", self_checkout, {"type": "device", "id": "fixture"}
                ):
                    raise AssertionError((thread, checkout, request))
                return Resource()

            def devices(self):
                return ["fixture"]

        class Connection:
            def request(self, thread, method, params):
                return {"thread": thread, "method": method, "params": params}

        self_checkout = self.checkout
        rt = await self.open()
        project = rt.open_project(self.checkout)["id"]
        rt.launch("launch", "thread", project, "Services")
        rt.set_service_host("device", DeviceHost())
        self.assertEqual(rt.devices("thread"), ["fixture"])
        device = rt.open_service("thread", {"type": "device", "id": "fixture"})
        self.assertEqual(rt.service_handles("thread", "device"), [device])
        self.assertEqual(rt.service_call("thread", device, {"type": "capture"}),
                         {"type": "bytes", "mime": "text/plain", "data": b"capture"})
        rt.set_service_host("device", None)
        self.assertEqual(closed, [True])

        rt.install_mcp_app_host(Connection(), {"ui://fixture"}, {"fixture"})
        app = rt.open_service("thread", {"type": "mcp_app", "resource": "ui://fixture"})
        resource = rt.service_call("thread", app, {"type": "read_resource", "uri": "ui://fixture"})
        self.assertEqual(resource["value"]["method"], "resources/read")
        rt.approve_app_context("thread", app, resource)
        with self.rejects("forbidden"):
            rt.service_call("thread", app, {"type": "call_tool", "name": "unknown", "arguments": {}})
        rt.close_service("thread", app)

        rt.install_terminal_host()
        terminal = rt.open_service("thread", {
            "type": "terminal", "executable": "/usr/bin/printf", "args": ["line\n"],
        })
        cursor, output = None, bytearray()
        async with asyncio.timeout(5):
            while True:
                chunk = rt.service_call("thread", terminal, {"type": "output", "after": cursor})
                cursor = chunk["next"]
                output.extend(chunk["data"])
                if chunk["eof"]:
                    break
                await asyncio.sleep(0.01)
        self.assertIn(b"line", output)
        rt.close_service("thread", terminal)

    async def test_guests_and_peers_use_the_native_routes(self):
        class Text:
            def guest_message(self, name, message):
                return f"{name} said {message}"

        config = {"codex": {"executable": str(FIXTURES / "context-provider.py")}}
        async with await self.open(config=config, agent_text=Text()) as rt:
            project = rt.open_project(self.checkout)["id"]
            rt.launch("target", "target", project, "Target")
            sender = await self.open("sender")
            sender_project = sender.open_project(self.checkout)["id"]
            sender.launch("sender", "sender", sender_project, "Sender")
            router = rt.remote_router()
            app = asgi_app(router)

            async def request(path, bearer, payload):
                sent = []
                message = {"type": "http.request", "body": json.dumps(payload).encode()}

                async def receive():
                    return message

                async def send(response):
                    sent.append(response)

                await app({"type": "http", "method": "POST", "path": path, "headers": [
                    (b"authorization", f"Bearer {bearer}".encode()),
                    (b"content-type", b"application/json"),
                ]}, receive, send)
                body = sent[1]["body"]
                return sent[0]["status"], json.loads(body) if body else None

            invite = rt.invite_guest("target")
            status, session = await request("/v1/guest", invite["bearer"], {"method": "redeem", "name": "A guest"})
            self.assertEqual(status, 200)
            status, submitted = await request("/v1/guest", session["bearer"], {
                "method": "submit", "thread": "target", "key": "first", "text": "Hello",
            })
            self.assertEqual(status, 200)
            self.assertEqual(rt.guest_inbox("target")[0]["id"], submitted["id"])
            self.assertIsNone(rt.moderate_guest("target", submitted["id"], False))
            self.assertEqual(rt.guest_inbox("target"), [])
            status, second = await request("/v1/guest", session["bearer"], {
                "method": "submit", "thread": "target", "key": "accepted", "text": "Accepted text",
            })
            self.assertEqual(status, 200)
            run_id = rt.moderate_guest("target", second["id"], True)
            self.assertEqual((await asyncio.wait_for(rt.wait("target", run_id), 5))["status"], "completed")
            self.assertEqual(rt.read("target")["messages"][0]["text"], "A guest said Accepted text")
            self.assertTrue(rt.close_share(invite["id"]))

            address = rt.address("target")
            grant = rt.approve_link("target", sender.address("sender"), "read")
            payload = {"method": "read", "target": address, "before": None}
            status, response = await request("/v1/peer", grant["bearer"], payload)
            self.assertEqual((status, response["type"]), (200, "history"))
            self.assertTrue(rt.revoke_link(grant["id"]))
            self.assertEqual((await request("/v1/peer", grant["bearer"], payload))[0], 403)
            with self.assertRaises(HugglerError):
                Peer.loopback("192.0.2.1:8930", "fixture")

    async def test_asgi_preserves_native_routing_and_body_limits(self):
        async with await self.open() as rt:
            app = asgi_app(rt.remote_router())
            oversized = b"x" * (2 * 1024 * 1024 + 2)

            async def call(method, body, raw_path):
                responses = []

                async def receive():
                    return {"type": "http.request", "body": body}

                async def send(response):
                    responses.append(response)

                await app({
                    "type": "http", "method": method, "path": "/v1/peer", "raw_path": raw_path,
                    "headers": [(b"content-type", b"application/json")],
                }, receive, send)
                return responses[0]["status"]

            self.assertEqual(await call("POST", oversized, b"/v1/peer"), 413)
            self.assertEqual(await call("GET", oversized, b"/v1/peer"), 405)
            self.assertEqual(await call("POST", b"{}", b"/v1/%70eer"), 404)


if __name__ == "__main__":
    unittest.main()
