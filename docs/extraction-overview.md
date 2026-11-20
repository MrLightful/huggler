# Huggler orchestration crate

**Design proposal** based on T3 Code [`338352d`](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9), reviewed 20 November 2026.

Huggler’s goal is one agent orchestration runtime that applications written in different programming languages can embed. Build and distribute the Rust crate through Cargo first, then expose that same runtime through language bindings. The Tauri desktop app is the first host and calls the crate directly from Rust.

The core gives agents persistent conversations, runs them on different coding harnesses, and lets them message peers or delegate work. SQLite stores Huggler state. Provider adapters talk to local executables or external servers. The base desktop app needs no Node.js runtime for Huggler.

Use T3 Code to identify capabilities and regression scenarios. Huggler can choose its own API, persistence schema and provider transports as long as the intended workflow is covered. Some outcomes depend on provider-native features, and some require a host integration. Record those prerequisites alongside the evidence. [Orchestrator][src-1] · [Server composition][src-2] · [Agent tool service][src-3]

Start with a Codex conversation that survives reopening the Tauri app. Then qualify a second provider with a live approval in the UI and prove cross-provider messaging and delegation. [First milestone](first-milestone.md) · [Two-harness scenarios](extraction-plan.md#3-prove-agent-to-agent-operation-with-two-harnesses)

## What an application gets

| Application need | Proposed capability |
| --- | --- |
| Create agents | Create or open a project, choose a workspace, select a configured provider instance and model, launch an empty thread or start it with a prompt. A thread ID is its durable address across turns, restarts and supported provider changes. |
| Coordinate work | Address a peer thread directly, or delegate a task with a child, completion delivery and cancellation. Use a separate local Git worktree when children need independent changes. A harness can also spawn its own native subagents. |
| Show status or ask for input | Inspect a particular run, pending questions and approvals, delegated work, queue and provider availability. Subscribe to an environment summary or to one conversation. A host can answer an authorized request while a run waits. |
| Control execution | Queue, edit, reorder, steer, restart, interrupt and stop work. Report how a particular message was delivered. Support provider-dependent operations such as compaction, selection changes and stopping a native child. |
| Preserve or change context | Resume sessions, fork at a stable point, merge context back, change providers, inspect checkpoint diffs and roll back eligible conversation or workspace state. |
| Build an agent application | Add workspace setup, imports, scoped MCP and optional authenticated remote RPC, linked environments, schedules/webhooks, guest access, secrets, PR/issue workflows, preview, terminal and device integration. These capabilities need their selected host services. |

## Documentation

| When you need to… | Open |
| --- | --- |
| Design an application that creates agents, sends messages, waits and handles human requests | [Proposed Rust API, Tauri integration and a two-harness example](sdk-proposal.md) |
| Trace a command from intake through SQLite to the provider and back to a subscriber | [Runtime architecture and per-harness differences](upstream-orchestration.md) |
| Look up the reference behavior for a command, read, stream, effect or MCP tool | [T3 Code capability inventory](parity-inventory.md) |
| Pick the next implementation task and see what proves it works | [Seven-stage extraction plan](extraction-plan.md) |
| Find the upstream owner of a behavior and review changes since the last port | [Run an upstream review](upstream-watch-guide.md#run-a-review) · [Component map](upstream-watch-guide.md#find-the-implementation-by-behavior) |

[src-1]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.ts
[src-2]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/runtimeLayer.ts
[src-3]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.ts
