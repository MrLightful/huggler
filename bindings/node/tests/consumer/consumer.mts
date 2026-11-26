import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { appendFileSync, existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { once } from "node:events"
import { Worker } from "node:worker_threads"
import { Huggler, HugglerError, commandId, threadId, eventCursor, runId, providerInstanceId,
  type Run, type OpenOptions, type Subscription, type Change, type EventCursor, type HistoryCursor } from "@huggler/sdk"

const [, , phase, repo] = process.argv
const recorded = providerInstanceId("recorded")
const approval = providerInstanceId("approval")
interface FixtureOptions extends OpenOptions {
  stateDir: string
}
const options: FixtureOptions = {
  stateDir: resolve("state"),
  name: "Fixture Studio",
  slug: "fixture-app",
  storage: { worktreeWords: ["cedar", "maple"], databasePath: resolve("custom.sqlite"), worktreeRoot: { kind: "shared", path: resolve("trees") } },
  output: { version: "fixture-sdk" },
  instances: [
    { id: recorded, driver: "codex", executable: resolve("codex"), home: resolve("codex-home") },
    { id: approval, driver: "claudeAgent", executable: join(repo, "tests/fixtures/permission-acp.py") },
  ],
}
const checkout = realpathSync("project with spaces")
const thread = threadId("conversation")
const sibling = threadId("sibling")
const model = "gpt-6-luna"
const name = "Replay fixture: provider_thread_resume"
const native = "01a0d5ef-62bd-7c20-861b-0528ff7d86bd"
const answer = "provider thread resume fixture first turn complete"
const first = { threadId: thread, commandId: commandId("send-first"), text: "Respond with exactly: " + answer }
const second = "Using the conversation history available in this resumed thread, first repeat the exact final answer you gave in the previous turn. Then on a new line write exactly: provider thread resume fixture second turn complete"
const resumeRecording = join(repo, "knowledge/reference/codex-resume/codex_transcript.ndjson")
function recording(start: number, fixture = resumeRecording, substitutions: Record<string, string> = {}) {
  writeFileSync("case.json", JSON.stringify({ start, fixture, substitutions,
    clientInfo: { name: "fixture-app", title: "Fixture Studio", version: "fixture-sdk" } }))
}
const code = (expected: HugglerError["code"]) => (error: unknown) =>
  error instanceof HugglerError && error.code === expected

// Compile native results as object literals against the installed declarations.
function sample(type: string, value: unknown) {
  appendFileSync("wire-contract.mts", `(${JSON.stringify(value)}) satisfies Wire<sdk.${type}>;\n`)
}

async function until<T extends Change["type"]>(sub: Subscription, type: T) {
  while (true) {
    const { value, done } = await sub.events.next()
    assert(!done)
    sample("ThreadEvent", value)
    if (value.change.type === type) return value.change as Extract<Change, { type: T }>
    if (value.change.type === "run") assert.notEqual(value.change.run.status, "failed", value.change.run.error ?? "")
  }
}
async function drain(sub: Subscription, after: EventCursor, through: EventCursor) {
  let cursor = after
  const statuses = []
  while (BigInt(cursor) < BigInt(through)) {
    const { value, done } = await sub.events.next()
    assert(!done)
    sample("ThreadEvent", value)
    assert.equal(value.threadId, sub.snapshot.id)
    assert.equal(typeof value.sequence, "string")
    assert(BigInt(value.sequence) > BigInt(cursor))
    cursor = value.sequence
    if (value.change.type === "run") statuses.push(value.change.run.status)
  }
  assert.equal(cursor, through)
  return statuses
}
async function check(rt: Huggler) {
  const project = await rt.openProject(checkout)
  sample("Project", project)
  const launch = (id: typeof thread, providerInstanceId = recorded, title = name) =>
    rt.launch({ commandId: commandId("launch-" + id), projectId: project.id, threadId: id,
      name: title, providerInstanceId, ...(providerInstanceId === "recorded" && { model }) })
  if (phase === "first") {
    assert(existsSync("custom.sqlite"))
    execFileSync(process.execPath, ["consumer.mjs", "locked", repo], { stdio: "inherit", timeout: 15000 })
    const instances = await rt.listProviderInstances()
    for (const instance of instances) sample("ProviderInstance", instance)
    assert(instances.some(p => p.id === recorded && p.home === resolve("codex-home")))
    assert.equal(instances.find(p => p.id === recorded)?.workspaceWrite, false)
    recording(1)
    assert.equal((await launch(thread)).runId, null)
    await launch(sibling)
    const sub = await rt.subscribe(thread)
    const sent = await rt.send(first)
    sample("SendReceipt", sent)
    const run = await rt.wait(thread, sent.runId)
    sample("Run", run)
    assert.equal(run.status, "completed", run.error ?? "")
    const snapshot = await rt.readThread(thread)
    sample("Thread", snapshot)
    assert.equal(snapshot.messages.at(-1)?.text, answer)
    assert.equal(snapshot.nativeThread, native)
    assert.deepEqual(await drain(sub, sub.snapshot.cursor, snapshot.cursor), ["starting", "running", "waiting", "completed"])
    assert.equal((await rt.readThread(sibling)).messages.length, 0)
    writeFileSync("saved.json", JSON.stringify({ sent, initial: sub.snapshot.cursor, cursor: snapshot.cursor }))
  } else if (phase === "second") {
    const saved = JSON.parse(readFileSync("saved.json", "utf8"))
    assert.deepEqual(await rt.send(first), saved.sent)
    await assert.rejects(rt.send({ ...first, text: "different intent" }), code("conflict"))
    const replay = await rt.subscribe(thread, { after: saved.initial })
    assert.deepEqual(await drain(replay, saved.initial, saved.cursor), ["starting", "running", "waiting", "completed"])
    const detail = await rt.readHistory(thread, { limit: 1 })
    sample("ConversationDetail", detail)
    assert.equal(detail.model, model)
    assert.equal(detail.foregroundRun, null)
    assert.equal(detail.queuedCount, "0")
    assert.equal(detail.queueHeld, false)
    assert.equal(detail.streamingText, "")
    assert.deepEqual(detail.questions, [])
    const history = detail.history
    assert(history.older)
    assert.equal((await rt.readHistory(thread, { before: history.older })).history.messages[0].message.id, saved.sent.messageId)
    recording(41)
    const sent = await rt.send({ threadId: thread, commandId: commandId("second"), text: second })
    assert.notEqual(sent.runId, saved.sent.runId)
    sample("SendReceipt", sent)
    const run = await rt.wait(thread, sent.runId)
    sample("Run", run)
    assert.equal(run.status, "completed", run.error ?? "")
    assert.equal(run.ordinal, "2")
    const snapshot = await rt.readThread(thread)
    sample("Thread", snapshot)
    assert.equal(snapshot.nativeThread, native)
    const last = snapshot.messages.at(-1)!
    assert.equal(last.text, answer + "\nprovider thread resume fixture second turn complete")
    assert.deepEqual(await rt.readMessage(thread, last.id), last)
    await assert.rejects(rt.readHistory(thread, { before: history.older }), code("history_changed"))
    assert.deepEqual(await rt.readProviderUsage(), [{ providerInstanceId: "recorded", reportedRuns: "2", inputTokens: "42897", outputTokens: "50" }])
    recording(1, resumeRecording, { [native]: "another-native-thread" })
    const other = await rt.send({ ...first, threadId: sibling, commandId: commandId("sibling-send") })
    assert.equal((await rt.wait(sibling, other.runId)).status, "completed")
    assert.equal((await rt.readThread(sibling)).nativeThread, "another-native-thread")
    assert.equal((await rt.readThread(thread)).cursor, snapshot.cursor)
    assert.equal((await rt.listThreads()).length, 2)
  } else if (phase === "requests") {
    const defaults = await Huggler.open({ name: "Fixture App" })
    try { assert(existsSync(resolve("home/.fixture-app/userdata/huggler.sqlite3"))) }
    finally { await defaults.close() }
    const slug = await Huggler.open({ name: "Fixture App / Lab", slug: "fixture_lab" })
    try { assert(existsSync(resolve("home/.fixture_lab/userdata/huggler.sqlite3"))) }
    finally { await slug.close() }
    const slugOnly = await Huggler.open({ slug: "fixture-cli" })
    try { assert(existsSync(resolve("home/.fixture-cli/userdata/huggler.sqlite3"))) }
    finally { await slugOnly.close() }
    const explicitState = resolve("explicit state")
    const explicit = await Huggler.open({ stateDir: explicitState })
    const explicitThread = threadId("explicit-state-thread")
    try {
      assert(existsSync(join(explicitState, "huggler.sqlite3")))
      const project = await explicit.openProject(checkout)
      await explicit.launch({ commandId: commandId("explicit-state-launch"), threadId: explicitThread,
        projectId: project.id, providerInstanceId: "codex", name: "Existing conversation" })
    }
    finally { await explicit.close() }
    const named = await Huggler.open({ name: "Fixture App", slug: "fixture_legacy", stateDir: explicitState })
    try { assert.equal((await named.readThread(explicitThread)).name, "Existing conversation") }
    finally { await named.close() }
    await assert.rejects(Huggler.open({ name: "../outside" }), code("invalid"))
    await assert.rejects(Huggler.open({ name: "Fixture App", slug: "../outside" }), code("invalid"))
    await assert.rejects(Huggler.open({}), code("invalid"))
    const probe = await rt.probeProvider(approval, checkout)
    sample("ProviderReport", probe)
    assert.deepEqual(probe, { transport: "ACP v1", version: null, authenticated: null, resume: true, agentTools: false, models: [] })
    // @ts-expect-error JavaScript callers can pass a value of the wrong type
    await assert.rejects(rt.probeProvider(123, checkout), code("invalid"))
    await assert.rejects(Huggler.open({ ...options, storage: { worktreeRoot: { kind: "projectRelative", path: checkout } } }), code("invalid"))
    await assert.rejects(Huggler.open({ ...options, storage: { worktreeWords: [] } }), code("invalid"))
    await assert.rejects(Huggler.open({ ...options, storage: { worktreeWords: ["cedar", "cedar"] } }), code("invalid"))
    const invalidJSON = { stateDir: "state", output: { version: 1n } }
    // @ts-expect-error BigInt cannot be serialized as an SDK option
    await assert.rejects(Huggler.open(invalidJSON), code("invalid"))
    const id = threadId("permissions")
    await launch(id, approval, "Approve edits")
    const sub = await rt.subscribe(id)
    const send = (key: string) => rt.send({ threadId: id, commandId: commandId(key), text: "Create approval-check.txt" })
    const sent = await send("approval-send")
    const { request } = await until(sub, "permissionRequested")
    sample("Thread", await rt.readThread(id))
    const choice = { threadId: id, runId: sent.runId, requestId: request.id, option: "yes" }
    const controller = new AbortController()
    const pending = rt.wait(id, sent.runId, { signal: controller.signal })
    controller.abort(new Error("observer left"))
    await assert.rejects(pending, /observer left/)
    assert.equal((await rt.readThread(id)).permissions[0].id, request.id)
    await assert.rejects(rt.respondToPermission({ ...choice, threadId: sibling }))
    await assert.rejects(rt.respondToPermission({ ...choice, option: "missing" }))
    await rt.respondToPermission(choice)
    await assert.rejects(rt.respondToPermission(choice))
    assert.equal((await rt.wait(id, sent.runId)).status, "completed")
    assert.equal(readFileSync(join(checkout, "approval-check.txt"), "utf8"), "approved")
    const active = await send("stop-me")
    await until(sub, "permissionRequested")
    await rt.stopThread({ threadId: id, commandId: commandId("stop") })
    assert.equal((await rt.wait(id, active.runId)).status, "interrupted")

    const fixture = join(repo, "tests/fixtures/codex-plan-questions/codex_transcript.ndjson")
    recording(1, fixture)
    const planner = threadId("planner")
    await launch(planner, recorded, "Replay fixture: plan_questions")
    const questions = await rt.subscribe(planner)
    const rows = readFileSync(fixture, "utf8").trim().split("\n").map(line => JSON.parse(line))
    const text = rows.find(r => r.type === "expect_outbound" && r.label === "turn/start").frame.params.input[0].text
    const plan = await rt.send({ threadId: planner, commandId: commandId("plan"), text, mode: "plan" })
    const q = (await until(questions, "questionRequested")).request
    sample("Thread", await rt.readThread(planner))
    const detail = await rt.readHistory(planner)
    sample("ConversationDetail", detail)
    assert.equal(detail.foregroundRun?.id, plan.runId)
    assert.deepEqual(detail.questions, [q])
    const response = { threadId: planner, runId: plan.runId, requestId: q.id, answers: { schema_preference: ["Strict schemas (Recommended)"] } }
    await assert.rejects(rt.respondToQuestion({ ...response, runId: runId("wrong") }), code("not_found"))
    await rt.respondToQuestion(response)
    assert.equal((await rt.wait(planner, plan.runId)).status, "completed")
    await assert.rejects(rt.respondToQuestion(response), code("already_answered"))
    const events = (await rt.subscribe(planner)).events
    const next = events.next()
    await assert.rejects(events.next(), code("invalid"))
    await events.return()
    assert.equal((await next).done, true)
    const abort = new AbortController()
    const sleeping = (await rt.subscribe(planner, { signal: abort.signal })).events.next()
    abort.abort(new Error("cancel subscription"))
    await assert.rejects(sleeping, /cancel subscription/)
    const last = await send("close-active")
    await until(sub, "permissionRequested")
    const waiter = rt.wait(id, last.runId).then(
      run => assert.equal(run.status, "interrupted"),
      error => assert(code("closed")(error)),
    )
    const closing = (await rt.subscribe(planner)).events.next()
    const closed = assert.rejects(closing, code("closed"))
    await Promise.all([rt.close(), rt.close(), closed, waiter])
    await assert.rejects(rt.listThreads(), code("closed"))
  } else if (phase === "precision") {
    const sub = await rt.subscribe(thread, { after: eventCursor("9007199254740992") })
    assert(BigInt(sub.snapshot.cursor) > BigInt(Number.MAX_SAFE_INTEGER))
    const event = await sub.events.next()
    assert(!event.done)
    assert.equal(event.value.sequence, "9007199254740993")
    const { history } = await rt.readHistory(thread, { limit: 1 })
    assert.equal(history.sequence, sub.snapshot.cursor)
    assert(history.older)
    assert.equal((await rt.readHistory(thread, { before: history.older, limit: 1 })).history.messages.length, 1)
    await assert.rejects(rt.subscribe(thread, { after: eventCursor("0") }), code("cursor_expired"))
  }
}

// Compile-time regression check for cursors persisted by a consumer.
function outputText(change: Extract<Change, { type: "outputDelta" }>): string {
  return change.text
}
function completedOrdinal(run: Extract<Run, { status: "completed" }>): string {
  return run.ordinal
}
function cursorTypes(rt: Huggler, before: HistoryCursor) {
  // @ts-expect-error a history cursor belongs to readHistory
  return rt.subscribe(thread, { after: before })
}

if (phase === "locked") {
  await assert.rejects(Huggler.open({ ...options, stateDir: resolve("other-state") }), code("already_open"))
} else if (phase === "responsive") {
  // The Git process can finish only after the Node event loop acknowledges it.
  mkdirSync("bin")
  writeFileSync("bin/git", `#!/usr/bin/env python3
import os, sys, time
from pathlib import Path
signals = Path(__file__).resolve().parent
(signals / "git-started").touch()
deadline = time.monotonic() + 10
while not (signals / "git-ack").exists():
    if time.monotonic() > deadline:
        sys.exit(1)
    time.sleep(.01)
os.environ["PATH"] = os.environ["PATH"].split(os.pathsep, 1)[1]
os.execvp("git", ["git", *sys.argv[1:]])
`, { mode: 0o755 })
  process.env.PATH = resolve("bin") + ":" + process.env.PATH
  const rt = await Huggler.open(options)
  const timer = setInterval(() => {
    if (existsSync("bin/git-started")) writeFileSync("bin/git-ack", "")
  }, 10)
  try { assert.equal((await rt.openProject(checkout)).root, checkout) }
  finally { clearInterval(timer); await rt.close() }
} else if (phase === "diagnostics") {
  // Script a rejected handshake and verify both the live and stored error surfaces.
  writeFileSync("diagnostic.py", `import json, sys
request = json.loads(sys.stdin.readline())
print(json.dumps({"id": request["id"], "error": {"code": -1, "message": "private fixture diagnostic"}}), flush=True)
`)
  const diagnostic = providerInstanceId("diagnostic")
  const id = threadId("failed-handshake")
  let savedCursor: EventCursor
  const rt = await Huggler.open({ ...options, instances: [
    ...options.instances!, { id: diagnostic, driver: "codex", executable: "python3", args: [resolve("diagnostic.py")], workspaceWrite: true },
  ] })
  try {
    assert.equal((await rt.listProviderInstances()).find(p => p.id === diagnostic)?.workspaceWrite, true)
    const missing = join(checkout, "private-missing-checkout")
    await assert.rejects(rt.probeProvider(approval, missing), (error: unknown) => {
      assert(error instanceof HugglerError && error.code === "checkout_unavailable")
      assert(!error.message.includes("private-missing-checkout") && !error.detail.includes("private-missing-checkout"))
      return true
    })
    await assert.rejects(rt.probeProvider(diagnostic, checkout), (error: unknown) => {
      assert(error instanceof HugglerError && error.code === "provider")
      assert.doesNotMatch(error.message + error.detail, /private fixture diagnostic/)
      return true
    })
    const project = await rt.openProject(checkout)
    await rt.launch({ threadId: id, commandId: commandId("diagnostic-launch"), projectId: project.id, name: "Failed handshake", providerInstanceId: diagnostic })
    const sent = await rt.send({ threadId: id, commandId: commandId("diagnostic-send"), text: "fixture" })
    const run = await rt.wait(id, sent.runId)
    assert.equal(run.status, "failed")
    assert(run.error)
    assert.doesNotMatch(run.error, /private fixture diagnostic/)
    const snapshot = await rt.readThread(id)
    assert.equal(snapshot.runs.at(-1)?.error, run.error)
    savedCursor = snapshot.cursor
  } finally { await rt.close() }

  // Replay diagnostic-bearing events that another Cargo host could have written.
  const diagnosticText = "private fixture diagnostic"
  const historical = [
    { type: "steer", run_id: "r", command_id: "c", outcome: { rejected: diagnosticText } },
    { type: "native_child", child: { run_id: "r", parent_thread_id: "p", thread_id: "c",
      path: "child", status: "failed", result: diagnosticText } },
    { type: "task", task: { taskId: "t", parentThreadId: id, parentRunId: "r", childThreadId: "c",
      childRunId: "cr", resultRunId: "cr", result: diagnosticText, status: "failed", delivery: "pending" } },
  ]
  execFileSync("python3", ["-c", `import json, sqlite3, sys
with sqlite3.connect("custom.sqlite") as db:
    for change in json.load(sys.stdin):
        db.execute("INSERT INTO event(thread_id,change) VALUES(?,?)", (sys.argv[1], json.dumps(change)))`, id],
    { input: JSON.stringify(historical) })
  const reader = await Huggler.open(options)
  try {
    const sub = await reader.subscribe(id, { after: savedCursor })
    for (const expected of ["steer", "nativeChild", "task"]) {
      const { value, done } = await sub.events.next()
      assert(!done)
      assert.equal(value.change.type, expected)
      assert(!JSON.stringify(value).includes(diagnosticText))
      sample("ThreadEvent", value)
    }
  } finally { await reader.close() }
} else if (phase === "worker") {
  const worker = new Worker(`const { parentPort, workerData } = require('node:worker_threads')
    require('@huggler/sdk').Huggler.open(workerData).then(async rt => {
      const sub = await rt.subscribe('conversation')
      parentPort.postMessage('open')
      await sub.events.next()
    })`, { eval: true, workerData: options })
  assert.deepEqual(await once(worker, "message"), ["open"])
  await worker.terminate()
  await (await Huggler.open(options)).close()
} else if (phase === "idle") {
  await Huggler.open(options)
} else {
  const rt = await Huggler.open(options)
  try { await check(rt) } finally { await rt.close() }
}
