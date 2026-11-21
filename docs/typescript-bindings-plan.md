# TypeScript bindings for Huggler

Ship an npm package that lets a Node.js project open a Git checkout, run and monitor an agent conversation, answer a provider request, and resume the conversation after a process restart. Start with TypeScript CLIs and long-lived backends. The first release should install from npm on Apple Silicon macOS and x64 Linux with glibc without requiring Cargo or a C compiler on the consumer’s machine. `@huggler/sdk` is the proposed package name, pending npm scope access and availability.

The preview implementation and local tarball installation instructions are in the [TypeScript SDK guide](typescript-sdk.md).

## Architecture

Add a standalone Rust `cdylib` crate in `bindings/node/native` using NAPI-RS and a TypeScript package in `bindings/node`. Give the binding its own manifest and lockfile and use the root Huggler crate as a Cargo dependency. The addon hosts `Huggler` in-process on NAPI-RS’s Tokio runtime. The TypeScript layer supplies a typed API, abortable observers, and structured errors. Rust continues to own the database, migrations, command receipts, provider processes, and agent authorization. The GPUI app continues to embed the Cargo crate directly and runs without Node.js. [Rust runtime](../src/runtime.rs) · [NAPI-RS async runtime](https://napi.rs/docs/concepts/async-runtime)

Use Node-API for the binary interface so each platform build can serve multiple tested Node.js majors. Target Node.js 22.14 or later in the 22 series and test Node.js 24 and 26 before advertising them. The first spike should verify the effective Node-API floor with the pinned NAPI-RS features. Qualify Windows, Intel macOS, Linux arm64, musl, and alternative JS runtimes as separate follow-ups. Browser and edge clients can use a product’s authenticated Node.js backend if they need access. Revisit a standalone service when remote clients are a requirement. The existing `/v1/peer` route is a thread-scoped sharing protocol. [Node-API ABI](https://nodejs.org/download/release/v22.18.0/docs/api/n-api.html) · [NAPI-RS compatibility](https://napi.rs/docs/more/support-compatibility) · [Peer API](../src/remote.rs)

## First release API

Use camelCase methods and fields, typed string IDs, and discriminated unions for events, run states, provider requests, and error details. Preserve driver keys such as `codex`, `claudeAgent`, and `opencode`. Define binding DTOs for the exported surface and generate declarations from those definitions, including unions that need explicit mappings. Check generated declarations and contract fixtures in CI. Represent exact 64-bit values such as event sequences and usage totals as decimal strings so they round-trip through JavaScript and JSON. Treat `EventCursor` and `HistoryCursor` as distinct opaque strings. Validate input at the native boundary and expose an error with `code`, `message`, and typed `detail`. Publish stable codes for cases such as `already_open`, `conflict`, `cursor_expired`, and `history_changed`. Review the diagnostics surfaced to JavaScript for credentials and other internal data. [Rust contracts](../src/contracts.rs) · [Huggler naming conventions](sdk-proposal.md#naming)

| Workflow | Proposed TypeScript entry points | Acceptance |
| --- | --- | --- |
| Configure and open | `Huggler.open`, `listProviderInstances`, `probeProvider`, `openProject`, `close` | Open a project with a named provider instance. Honor custom database and worktree paths, provider homes, and declarative output settings. A competing process gets `already_open`. |
| Run and continue | `launch`, `send`, `wait`, `listThreads`, `readThread` | Launch a root-checkout thread. On reopen, the same Huggler thread resumes the same native Codex session with a new run ID. A repeated command returns the original receipt. Reusing its ID for different intent yields `conflict`. |
| Observe and page | `subscribe`, `readHistory`, `readMessage` | Receive ordered thread-scoped changes, resume from a saved event cursor, and page older messages with the history cursor. On `cursor_expired`, the consumer takes a fresh snapshot. On `history_changed`, the consumer fetches a new first page. |
| Respond and control | `respondToPermission`, `respondToQuestion`, `stopThread` | Answer the specific request on its owning thread and run. A wrong or expired request yields an error. Stopping a thread exposes the resulting run state. |

Keep configuration data-only in the first release. Accept the state directory, storage layout, named provider instances, and declarative agent output options such as name, title, instructions, tool presentation, and provider labels. Defer JavaScript implementations of Rust traits such as `AgentText`. Resolve provider prerequisites through the existing adapters and expose the probe’s version, transport, authentication state, and observed capabilities. The npm package ships Huggler’s binding and the loader. The host configures the provider executable and credentials. [Storage layout](../src/layout.rs) · [Provider configuration](../src/providers/registry.rs) · [Provider probe](../src/providers/probe.rs) · [Agent output](../src/output.rs)

Map `launch` to `Action::LaunchInstance` and `readHistory` to `Huggler::detail`. Let `send` select Codex plan mode through `Action::SendPlan` so a consumer can initiate the recorded structured-question workflow. Expose operation-specific receipts so `send` has a required `runId` and a launch receipt can represent a thread without a run. Use the same database layout and migrations as Cargo hosts. Verify sequential handoff between Rust and Node.js with the same core revision and provider instance configuration. Follow the core’s schema compatibility rules when upgrading. [Runtime API](../src/runtime.rs) · [Receipt contract](../src/contracts.rs) · [Question fixture](../tests/questions.rs)

### Consumer example

Once the preview package is published, the intended install command is `npm install @huggler/sdk@preview`. This sketch assumes `./my-project` is a Git checkout and the Codex CLI is installed and signed in. Persist the thread ID and command IDs with the application job when retries must survive a restart. `wait` returns the terminal run, including failure or interruption states.

```ts
import { Huggler, commandId, threadId } from "@huggler/sdk"

const huggler = await Huggler.open({ stateDir: "./.huggler" })
try {
  const project = await huggler.openProject("./my-project")
  const thread = threadId("example-thread")
  await huggler.launch({
    commandId: commandId("example-launch"),
    threadId: thread,
    projectId: project.id,
    providerInstanceId: "codex",
    name: "Repository walkthrough",
  })
  const receipt = await huggler.send({
    commandId: commandId("example-first-prompt"),
    threadId: thread,
    text: "Summarize the repository structure.",
  })
  const run = await huggler.wait(thread, receipt.runId)
  if (run.status !== "completed") throw new Error(`Run ended as ${run.status}`)
  console.log((await huggler.readThread(thread)).messages.at(-1)?.text)
} finally {
  await huggler.close()
}
```

### Lifecycle and streaming

- Make runtime operations asynchronous. Run synchronous Rust filesystem, Git, and SQLite calls on blocking workers. `close()` should be safe to await more than once and resolve after managed workers and the database owner lock are released. Keep explicit `close()` as the normal shutdown path. Verify cleanup during Node Worker termination while Tokio can still make progress. [Runtime lifecycle](../src/runtime.rs)
- Expose `subscribe(threadId, { after?, signal? })` as a snapshot plus a pull-based async iterable with at most one pending native `next()` per iterator. A fresh subscription starts after the snapshot’s cursor. A resumed iterator replays all changes strictly after `after`, which can include changes already in the returned snapshot. Consumers rebuilding their view from that snapshot apply only changes newer than its cursor. Preserve Rust’s replay order and retention errors. Use the bounded history API for large conversations and measure the cost of the current full subscription snapshot before adding any new Rust projection. [Rust subscription](../src/runtime.rs) · [Event batching](../src/store.rs) · [History](../src/store/history.rs)
- Wire `AbortSignal` and iterator `return()` to native cancellation so a sleeping `next()` or `wait()` can release its Rust future. An aborted `wait` leaves the run active. `stopThread` is the explicit run-control operation. Settle pending readers when the runtime closes and verify that an idle consumer can exit. Resume after cancellation from the last event the consumer actually received. Document that a committed command can still return its original receipt on retry after the caller loses the result. [Rust waits and stop](../src/runtime.rs)

## Implementation and release gates

1. **Prove the boundary on macOS.** Create the isolated addon and wrap `open`, project open, launch, send, wait, thread read, and close. Reuse the recorded Codex restart fixture and expected replies in a separate TypeScript consumer that runs in two Node.js processes. Confirm receipt retry, conflict, native continuation, custom database locking, and sibling isolation. Install the fixture harness only in development and CI. [Recorded scenario](../tests/support/resume.rs) · [Cargo package qualification](stage-7.md)
2. **Finish the API.** Implement the remaining surface and run TypeScript integration tests against scripted or recorded providers. Cover approval and question ownership, abort and close races, Worker teardown, cursor round trips above `Number.MAX_SAFE_INTEGER`, retention expiry, and history paging. Check that ordinary Node callbacks keep running during Rust filesystem work. Run the existing Rust suites if the core changes. Record each supported workflow against the parity inventory with harness, version, transport, and observed result. [Permissions](../tests/permissions.rs) · [Questions](../tests/questions.rs) · [Capability inventory](parity-inventory.md)
3. **Qualify npm installs.** Use one prebuilt `.node` per target with exact-version platform packages as optional dependencies. Build from the same source revision with locked dependencies, include license notices, and document the tested OS and libc floors. In clean consumers, install the packed packages with `--ignore-scripts`, exercise ESM and CommonJS imports and TypeScript type resolution, then repeat the fixture-based lifecycle on both targets. Test the oldest advertised Node.js version and a current patch on each advertised major. Check the loader’s errors for unsupported platforms and missing platform dependencies. Once the maintainer has configured npm ownership and authorized release, publish platform packages first, then the SDK under a preview tag with npm trusted publishing. Re-run the install from the registry before promoting it. [NAPI-RS platform packages](https://napi.rs/docs/cli/pre-publish) · [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/)

After the first release, extend the API by workflow. Add durable queue controls, attachments, history import, context forks, and host observation of cross-provider peer replies and delegated completions. Enable the existing caller-scoped agent tool service for the collaboration cases and keep human approvals on the host API. Then evaluate feature-gated workspace setup, schedules, guest sharing, repository integrations, and host service interfaces. Qualify each workflow through an independent TypeScript consumer before claiming support. [Runtime controls](stage-5.md) · [Cross-provider workflows](stage-3.md) · [Application integrations](stage-6.md)
