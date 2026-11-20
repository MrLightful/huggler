# Huggler SDK proposal

An application opens a Huggler runtime and owns its state directory. Its agents get scoped tools for addressing peers and delegating tasks. The npm package name is `huggler`. The exports, TypeScript interfaces and MCP names below describe the proposed SDK.

## Naming

| Surface | Huggler convention |
| --- | --- |
| npm and TypeScript | Import `openHuggler`, `Huggler` and `HugglerOptions` from `huggler`. Use domain names such as `ThreadId` and `ProviderAdapter` inside the package namespace. |
| Files and exports | Name files for their purpose, for example `src/open-huggler.ts`, `src/providers/claude-agent.ts` and `src/profiles/full.ts`. The driver configuration ID is `claudeAgent` and the corresponding import is `huggler/providers/claude-agent`. |
| Agent tools | Register the MCP server as `huggler`. Tool names describe operations, e.g. `thread_send` and `delegate_task`. A harness that qualifies names as `mcp__<server>__<tool>` exposes `mcp__huggler__thread_send`. Use the same tool catalog for discovery, dispatch, provider instructions and examples. |
| Runtime assets and configuration | Use `HUGGLER_*` for Huggler-owned environment variables, e.g. `HUGGLER_MCP_URL`. Use Huggler filenames for generated assets such as `pi-huggler-mcp-extension.ts`. Keep harness-defined settings and protocol identifiers in their vendor spelling. |

The [upstream inventory](parity-inventory.md#agent-facing-tools) maps the pinned source identifiers to Huggler tools. Keep original identifiers in source references, recorded upstream evidence and readers for historical input. Preserve the applicable license notices when porting code.

## Package exports

Use one Node ESM package with explicit subpath exports. Applications should have a small public API even when the internal implementation spans several modules. Keep Effect's scoped resources, streams, SQL transactions and layers inside the runtime. Offer Promises and cancellable async iterables at the application boundary.

| Import | Intended consumer |
| --- | --- |
| `huggler` | Trusted application server. Open/close the runtime, manage providers and projects, launch threads, send, read, wait and respond. |
| `huggler/contracts` | Browser or server needing versioned wire types, codecs, IDs, capabilities, legal commands and stream frames. Keep Node, SQLite and provider SDKs out of this export. |
| `huggler/providers/<driver-module>` | Register one driver or a custom driver implementing the provider extension API. A driver kind selects an implementation. An instance selects its configured account, environment, options and continuation identity. |
| `huggler/mcp`, `huggler/rpc` | Scoped tools for agents and authorized remote clients, and a typed application transport for dashboards. Carry authenticated identity through every handler. |
| `huggler/integrations/*`, `huggler/profiles/full` | Opt into host services individually or load the full set of host integrations. Report which integrations are active and which prerequisite is missing. |
| `huggler/testing` | Build deterministic applications and driver conformance tests with replay transports and isolated state. |

## Two agents in one application

The proposed host API below assumes a Node service with an accessible Git checkout, two installed and authenticated harnesses, and a host that can handle human requests. Model resolution consults each instance's workspace catalog. The correlation key comes from the application's durable job. A retry of the same job uses the same key.

```ts
import { openHuggler, type Huggler } from "huggler";
import { codex } from "huggler/providers/codex";
import { claudeAgent } from "huggler/providers/claude-agent";

// Call once at service startup. The host invokes its shutdown hooks after draining intake.
export async function configureReviews(onShutdown: (close: () => Promise<void>) => void) {
  const engine = await openHuggler({
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

async function startReview(engine: Huggler, jobId: string, checkout: string) {
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
    message: "Draft the changelog. Use thread_send to ask thread "
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

## Proposed agent calls

These examples use the proposed Huggler MCP catalog and the argument shapes from the upstream contracts. Replace example IDs with values returned by thread discovery, the package host, or `orchestrator_capabilities`. The running writer's MCP credential determines who sent the message. The receiver gets the message with sender provenance and can use the same tool to reply. [Agent tool schemas][src-4]

```json
{
  "name": "thread_send",
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

## Interface contracts for the SDK

Proposed signatures below use the upstream schema names in shortened form. The document names the important fields. The published contracts export would supply the complete unions, including attachment types and provider options.

```ts
declare function openHuggler(options: HugglerOptions): Promise<Huggler>;

interface Huggler {
  readonly projects: Projects;
  readonly providers: Providers;
  readonly threads: Threads;
  readonly humanRequests: HumanRequests;
  close(): Promise<void>;
}

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
- **Authority.** The trusted application owns runtime configuration and human decisions. Agent-facing access goes through a credential bound to the live caller. Expose agent delegation through that authenticated context. Accepting an arbitrary `parentThreadId` from an app client would bypass the runtime’s ownership checks.
- **Installation.** Ship compiled JavaScript, declarations and runtime assets. Load an SDK or CLI only when its driver is selected. Report setup, version and login state by instance. The compatible Node range and supported OS/architecture combinations must be qualified from the packed release.

[src-4]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/orchestratorMcp.ts
[src-5]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/orchestrator/tools.ts
