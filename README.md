# T3 orchestration as an npm package

**Design proposal · source baseline `338352d` · 20 November 2026**

An application should be able to create two persistent agent threads, give them different coding harnesses, let one ask the other for help, and recover both conversations after its server restarts. T3 Code already implements each part. The extraction should reuse its command processor, SQLite state, provider runtimes and scoped agent tools together. Packaging the adapters alone would leave the application responsible for queueing, permissions, child completion and recovery. [Orchestrator][src-1] [Server composition][src-2] [Agent tool service][src-3]

This repository currently contains the **design and extraction materials**. TypeScript interfaces and import paths below are proposed. The quoted MCP tool names are existing T3 interfaces. The package name is a working name.

| For this decision | Read |
| --- | --- |
| What should an application import, and how do its agents communicate? | This overview |
| What owns the state, which provider differences matter, and what must move? | [Architecture and harness analysis](docs/upstream-orchestration.md) |
| Which existing commands, reads, streams, effects and agent tools need coverage? | [Protocol and capability inventory](docs/parity-inventory.md) |
| What should the first extraction change contain, and what proves full parity? | [Ordered implementation and verification plan](docs/extraction-plan.md) |
| Where does each component live in T3 Code, and which changes should I port? | [Upstream source map and watch workflow](docs/upstream-watch-guide.md) · [Git path list](docs/t3code-watch.paths) |

## What an application gets

| Application need | Package behavior |
| --- | --- |
| Create agents | Create or open a project, choose a workspace, select a configured provider instance and model, launch an empty thread or start it with a prompt. A thread ID is its durable address across turns, restarts and supported provider changes. |
| Coordinate work | Address a peer thread directly, or delegate a task with a child, completion delivery and cancellation. Use a separate local Git worktree when children need independent changes. A harness can also spawn its own native subagents. |
| Show status or ask for input | Inspect a particular run, pending questions and approvals, delegated work, queue and provider availability. Subscribe to an environment summary or to one conversation. A host can answer an authorized request while a run waits. |
| Control execution | Queue, edit, reorder, steer, restart, interrupt and stop work. Report how a particular message was delivered. Support provider-dependent operations such as compaction, selection changes and stopping a native child. |
| Preserve or change context | Resume sessions, fork at a stable point, merge context back, change providers, inspect checkpoint diffs and roll back eligible conversation or workspace state. |
| Build a T3-like host | Add workspace setup, imports, scoped MCP and authenticated RPC, linked environments, schedules/webhooks, guest access, secrets, PR/issue workflows, preview, terminal and device integration. The full profile needs implementations of these host services. |

### Proposed package surface

Use one Node ESM package with explicit subpath exports. Applications should have a small public API even when the internal implementation spans several modules. Keep Effect's scoped resources, streams, SQL transactions and layers inside the runtime. Offer Promises and cancellable async iterables at the application boundary.

| Import | Intended consumer |
| --- | --- |
| `t3-orchestration` | Trusted application server. Open/close the runtime, manage providers and projects, launch threads, send, read, wait and respond. |
| `t3-orchestration/contracts` | Browser or server needing versioned wire types, codecs, IDs, capabilities, legal commands and stream frames. Keep Node, SQLite and provider SDKs out of this export. |
| `t3-orchestration/providers/<kind>` | Register one driver or a custom driver implementing the provider extension API. A driver kind selects an implementation. An instance selects its configured account, environment, options and continuation identity. |
| `t3-orchestration/mcp`, `/rpc` | Scoped tools for agents and authorized remote clients, and a typed application transport for dashboards. Carry authenticated identity through every handler. |
| `t3-orchestration/integrations/*`, `/profiles/t3` | Opt into host services individually or assemble the full T3 configuration. Report which integrations are active and which prerequisite is missing. |
| `t3-orchestration/testing` | Build deterministic applications and driver conformance tests with replay transports and isolated state. |

### Two agents in one application

The proposed host API below assumes a Node service with an accessible Git checkout, two installed and authenticated harnesses, and a host that can handle human requests. Model resolution consults each instance's workspace catalog. The correlation key comes from the application's durable job. A retry of the same job uses the same key.

```ts
import { openOrchestrator, type Orchestrator } from "t3-orchestration";
import { codex } from "t3-orchestration/providers/codex";
import { claudeAgent } from "t3-orchestration/providers/claudeAgent";

// Call once at service startup. The host invokes its shutdown hooks after draining intake.
export async function configureReviews(onShutdown: (close: () => Promise<void>) => void) {
  const engine = await openOrchestrator({
    stateDir: "./agent-state",
    drivers: [codex(), claudeAgent()],
    instances: [
      { id: "writer", driver: "codex" },
      { id: "reviewer", driver: "claudeAgent" },
    ],
    agentTools: { transport: "local", toolkits: ["orchestrator", "thread"] },
  });

  onShutdown(() => engine.close());
  return (jobId: string, checkout: string) => startReview(engine, jobId, checkout);
}

async function startReview(engine: Orchestrator, jobId: string, checkout: string) {
  const project = await engine.projects.open({ workspaceRoot: checkout });
  const launch = async (instanceId: string, title: string) => {
    const modelSelection = await engine.providers.resolveSelection({ instanceId, cwd: checkout });
    return engine.threads.launch({
      projectId: project.id, title, modelSelection,
      runtimeMode: "approval-required", interactionMode: "default",
      clientRequestId: jobId + "/launch/" + instanceId,
    });
  };

  const reviewer = await launch("reviewer", "Reviewer");
  const writer = await launch("writer", "Author");
  const sent = await engine.threads.send({
    threadId: writer.threadId,
    message: "Draft the changelog. Use t3_thread_send to ask thread "
      + reviewer.threadId + " for a review. Ask it to send its findings back here.",
    mode: "auto",
    clientRequestId: jobId + "/draft",
  });
  const observed = await engine.threads.wait({
    threadId: writer.threadId, runId: sent.runId, timeoutMs: 10_000,
  });
  return { writer, reviewer, sent, observed };
}
```

The ten-second wait observes the author's first run. A review can arrive later and start a **new run in the same author thread**. The host can follow the exchange with `threads.subscribe({ threadId })` and read current state with `threads.status(threadId)`. Use a delegated task when the parent needs an owned result and a completion notification.

### What one agent actually calls

These examples show **existing MCP tool arguments**. Replace example IDs with values returned by thread discovery, the package host, or `orchestrator_capabilities`. The running writer's MCP credential determines who sent the message. The receiver gets the message with sender provenance and can use the same tool to reply.

```json
{
  "name": "t3_thread_send",
  "arguments": {
    "threadId": "<reviewer thread ID>",
    "message": "Review the draft in CHANGELOG.md and send your findings back to my thread.",
    "mode": "auto",
    "clientRequestId": "release-42/changelog-review"
  }
}
```

Use delegation when completion should wake the parent. A writer can ask Claude to do a review even if the writer runs on Codex. Include the material the child needs in the task or a shared file. A child starts with the task and optional role. Use `isolation: "worktree"` if it should start from the parent commit in a separate checkout. Uncommitted edits are available in shared isolation.

```json
{
  "name": "delegate_task",
  "arguments": {
    "task": "Review the draft in CHANGELOG.md and return the corrections.",
    "target": { "providerInstanceId": "reviewer" },
    "role": "review",
    "isolation": "shared",
    "mode": "async",
    "clientRequestId": "release-42/owned-review"
  }
}
```

The first call yields a message ID, run ID and actual delivery. The second yields a task ID and local child thread ID or remote address. Its run ID can initially be null. The parent can continue working and receive a completion. Reading the completed task with `task_status({ taskId })` can acknowledge the result. A bounded `mode: "wait"` can return on a question, approval or timeout. A host displaying progress should use ordinary observation APIs to avoid consuming a parent's delivery. [MCP input and result schemas][src-4] [Tool declarations][src-5]

### Interface contracts for the SDK

Proposed signatures below use the upstream schema names in shortened form. The document names the important fields. The published contracts export would supply the complete unions, including attachment types and provider options.

```ts
interface Threads {
  launch(input: LaunchThreadInput): Promise<{ threadId: ThreadId; runId: RunId | null }>;
  send(input: SendInput): Promise<{
    threadId: ThreadId;
    messageId: MessageId;
    runId: RunId;
    taskId?: TaskId;
    delivery: "started" | "queued" | "steered" | "restarted";
    status: RunStatus;
  }>;
  status(id: ThreadId): Promise<ThreadStatusView>;
  wait(input: {
    threadId: ThreadId; runId?: RunId; timeoutMs?: number;
    untilDelegatedWorkSettled?: boolean; signal?: AbortSignal;
  }): Promise<WaitResult>;
  subscribe(input: {
    threadId: ThreadId; afterSequence?: number; signal?: AbortSignal;
  }): AsyncIterable<ThreadStreamFrame>;
}

interface HumanRequests {
  list(threadId: ThreadId): Promise<PendingRequest[]>;
  respond(input: AuthorizedRequestResponse): Promise<CommandReceipt>;
}
```

- **Identity.** Give IDs distinct types. A thread identifies a conversation, a run identifies one unit of application work, an attempt identifies an execution of it, and a task identifies a parent–child relationship. A process restart can replace a provider session while the application thread survives.
- **Reads.** `ThreadStatusView` should expose current and latest terminal runs, held/queued work, unanswered requests, active children, background work and settlement. `WaitResult` should expose `timedOut`, the observed run/status and, for a settled family, the run containing the result. Clients can read paged activity for its answer.
- **Writes.** Use a stable command ID or a caller-scoped request key for retried operations. Distinguish committed intent, provider acceptance and terminal outcome in the result types. Expose typed reasons for unsupported capabilities, insufficient authority, stale targets, unavailable instances and storage failures.
- **Subscriptions.** Return the upstream snapshot/delta semantics. Persist a cursor **after** applying its updates. Cancellation of a subscription or wait releases the observer. Execution stops through explicit run, thread or task controls.
- **Authority.** The trusted application owns runtime configuration and human decisions. Agent-facing access goes through a credential bound to the live caller. Expose agent delegation through that authenticated context. Accepting an arbitrary `parentThreadId` from an app client would bypass T3's ownership checks.
- **Installation.** Ship compiled JavaScript, declarations and runtime assets. Load an SDK or CLI only when its driver is selected. Report setup, version and login state by instance. The compatible Node range and supported OS/architecture combinations must be qualified from the packed release.

**First implementation milestone.** A clean consumer using SQLite and one real adapter with recorded transport can launch, send, observe a final answer, restart and read it again. The next milestone proves both a two-way exchange between independent Codex and Claude threads and a delegated result that wakes an idle parent. The remaining stages extend that proof to every registered driver and the full integration profile. [Implementation plan](docs/extraction-plan.md)

[src-1]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.ts
[src-2]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/runtimeLayer.ts
[src-3]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.ts
[src-4]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/orchestratorMcp.ts
[src-5]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/orchestrator/tools.ts
