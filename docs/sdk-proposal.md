# Huggler Rust crate proposal

Huggler is an async Rust library for durable agent orchestration. The Rust crate is the shared implementation. Deliver it through Cargo first, then build bindings so applications in other languages can use the same runtime. The first host is a native desktop app built with Zed’s GPUI framework. It opens the crate in-process and owns its lifecycle. Provider-specific runtime or bridge dependencies belong to their adapters. The base desktop app runs without Node.js.

T3 Code supplies reference scenarios for user-visible behavior. Huggler can define its own Rust types, storage schema and transport payloads. Provider-native operations depend on the selected harness and version. [T3 Code behavior inventory](parity-inventory.md)

## Naming

| Surface | Huggler convention |
| --- | --- |
| Cargo and Rust | Name the library crate `huggler`. Use paths such as `src/providers/claude_agent.rs`, types such as `HugglerOptions` and methods such as `Huggler::open`. Use newtypes for distinct IDs. |
| Driver configuration | Keep stable serialized IDs such as `codex` and `claudeAgent`. Use Rust paths such as `huggler::providers::codex` and `huggler::providers::claude_agent`. |
| Agent tools | Register the MCP server as `huggler`. Use operation names such as `thread_send` and `delegate_task`. A harness that qualifies tool names can expose `mcp__huggler__thread_send`. Use the same catalog for discovery, dispatch and model instructions. |
| Assets and settings | Use `HUGGLER_*` for Huggler-owned environment variables and filenames such as `pi-huggler-mcp-extension.ts` for provider-specific assets. Preserve vendor spellings in provider protocols, historical data readers and source references. |

Preserve applicable license notices when reusing code or fixtures. [Agent tool mapping](parity-inventory.md#agent-facing-tools)

## Crate and host boundaries

| Proposed Rust module | Responsibility |
| --- | --- |
| `huggler` | Open and close the runtime, configure provider instances, manage projects, launch threads, send, read, wait and answer requests. |
| `huggler::contracts` | Typed IDs, commands, events, capabilities and errors. The desktop host uses these types directly. Serialize explicit DTOs for remote APIs. |
| `huggler::providers` | Driver configuration, discovery and health, continuation identity, session lifecycle and normalized provider events. |
| `huggler::mcp` and `huggler::rpc` | Scope agent tools to a live caller. Provide an authenticated remote application transport when a host needs one. |
| `huggler::integrations` | Optional implementations and host interfaces for workspace preparation, Git, schedules, source control, secrets, terminal, previews and other integrations. |
| `huggler::testing` | Replay transports, deterministic clock and IDs, temporary state and provider conformance scenarios. |

The host provides a writable app-data directory, provider configuration and the integrations it enables. The runtime owns migrations, command receipts, event publication, provider processes it starts and their cleanup. An instance identifies its own account, environment and continuation policy.

## Embed in GPUI

1. Open one `Huggler` with the app-data directory on a Tokio runtime. Give the GPUI conversation view a handle to the core.
2. Call the crate directly to open a project, launch or select a thread, send a prompt and answer a permission request. Render the snapshot from `subscribe` and refresh the view when its stream advances. Drop the subscription when switching conversations.
3. On app shutdown, await `Huggler::close()` while Tokio is still running. On the next launch, open the same directory and restore the selected conversation.

The UI lives in the desktop package. Other consumers can use the Cargo crate independently. Provider executables and their credentials belong to the relevant adapter. [Desktop entry point](../desktop/src/main.rs) · [Conversation view](../desktop/src/conversation.rs) · [Provider qualification](extraction-plan.md#4-qualify-every-provider-instance-and-capability)

## Two agents in one application

The example sketches the proposed API. Assume the host has opened project `project_id`, configured a Codex instance named `writer` and a second instance named `reviewer`, and has a durable `job_id` for this request.

```rust
use huggler::{DeliveryMode, LaunchThread, SendMessage};

let author = huggler.threads().launch(LaunchThread::new(project_id.clone(), "writer")
    .with_client_request_id(format!("{job_id}/launch/writer"))).await?;
let reviewer = huggler.threads().launch(LaunchThread::new(project_id, "reviewer")
    .with_client_request_id(format!("{job_id}/launch/reviewer"))).await?;

let message = format!(
    "Draft the changelog. Use thread_send to request a review from thread {}. Ask it to reply here.",
    reviewer.thread_id
);
let sent = huggler.threads().send(
    SendMessage::new(author.thread_id, message)
        .with_mode(DeliveryMode::Auto)
        .with_client_request_id(format!("{job_id}/draft")),
).await?;
```

Use `sent.run_id` to observe the run that handles that message. A later peer reply can start another run on the same author thread. The host observes both through a subscription. Use an owned delegation when the parent needs a result and a completion notification. [Peer and delegation scenarios](extraction-plan.md#3-prove-agent-to-agent-operation-with-two-harnesses)

## Proposed agent calls

The caller's live MCP session determines sender identity and task ownership. Resolve a peer thread through discovery or a host-supplied ID.

```json
{
  "name": "thread_send",
  "arguments": {
    "threadId": "<reviewer thread ID>",
    "message": "Review CHANGELOG.md and send your findings back to my thread.",
    "mode": "auto",
    "clientRequestId": "release-42/changelog-review"
  }
}
```

For owned work, supply the task and provider instance. Shared isolation lets the child read current workspace edits. Worktree isolation starts from a checked-out commit.

```json
{
  "name": "delegate_task",
  "arguments": {
    "task": "Review CHANGELOG.md and return the corrections.",
    "target": { "providerInstanceId": "reviewer" },
    "role": "review",
    "isolation": "shared",
    "mode": "async",
    "clientRequestId": "release-42/owned-review"
  }
}
```

A send returns message and run IDs plus actual delivery. Delegation returns a task ID and child address. The child run ID may become available later. A bounded wait may return early for human input. Passive host reads leave delivery pending. An authorized parent read or explicit acknowledgement can consume it. [Upstream agent schemas][src-4] · [Tool declarations][src-5]

## Public API behavior

| Operation | Proposed behavior |
| --- | --- |
| `Huggler::open`, `close` | Open one runtime per private state directory. Recover before accepting commands. Close releases subscriptions, workers, storage and managed provider processes. |
| `projects().open`, `providers().resolve_selection` | Resolve workspace roots, then validate the selected instance and its workspace-specific model and options. |
| `threads().launch`, `send` | Return typed thread, message and run IDs and the actual delivery outcome. Reuse a caller-scoped request key for retry and expose conflicts across distinct intents. Distinguish committed intent from provider acceptance and final completion. |
| `threads().status`, `wait`, `subscribe` | Report active and latest terminal work, queued or held work and pending requests. Wait for a particular run or for a delegated family. Send snapshot and cursor-based delta frames. Dropping the wait or stream releases the observation. Execution controls stop or interrupt the underlying work explicitly. |
| `human_requests().list`, `respond` | Resolve the owning request and live session. Route a typed approval, denial or answer from an authorized host and report a lost or expired callback explicitly. |

A Huggler thread, application run, execution attempt, delegated task and provider session each have their own identity. Preserve these distinctions in DTOs. An agent action derives its identity from the authenticated caller, with scope and ownership rechecked at dispatch. [Identity and status reference](upstream-orchestration.md#which-identity-or-status-to-expose)

When a new transport can preserve the workflow with different mechanics, describe the result precisely. A history-based fork creates a fresh provider session with selected transcript context. Git restoration is a separate workspace action. Advertise a native operation only when the selected adapter exposes it, and distinguish a fallback from a provider-native result in the API. [Context and recovery reference](upstream-orchestration.md#recovery-streams-and-context)

[src-4]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/orchestratorMcp.ts
[src-5]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/orchestrator/tools.ts
