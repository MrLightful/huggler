# Huggler orchestration SDK

**Design proposal** based on T3 Code [`338352d`](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9), reviewed 20 November 2026.

The proposed `huggler` npm package lets a Node application give agents persistent conversations, run them on different coding harnesses, and let them request or delegate work across threads. T3 Code’s command processor, SQLite store, provider runtimes and scoped agent tools supply those behaviors. This repository documents the extraction ahead of implementation. [Orchestrator][src-1] · [Server composition][src-2] · [Agent tool service][src-3]

**Start the extraction** with one Codex conversation that survives a runtime restart. The first milestone has a recording you can inspect with Git and Python, two upstream test commands, and the expected results for a consumer of the packed SDK. [First milestone](first-milestone.md)

## What an application gets

| Application need | Package behavior |
| --- | --- |
| Create agents | Create or open a project, choose a workspace, select a configured provider instance and model, launch an empty thread or start it with a prompt. A thread ID is its durable address across turns, restarts and supported provider changes. |
| Coordinate work | Address a peer thread directly, or delegate a task with a child, completion delivery and cancellation. Use a separate local Git worktree when children need independent changes. A harness can also spawn its own native subagents. |
| Show status or ask for input | Inspect a particular run, pending questions and approvals, delegated work, queue and provider availability. Subscribe to an environment summary or to one conversation. A host can answer an authorized request while a run waits. |
| Control execution | Queue, edit, reorder, steer, restart, interrupt and stop work. Report how a particular message was delivered. Support provider-dependent operations such as compaction, selection changes and stopping a native child. |
| Preserve or change context | Resume sessions, fork at a stable point, merge context back, change providers, inspect checkpoint diffs and roll back eligible conversation or workspace state. |
| Build an agent application | Add workspace setup, imports, scoped MCP and authenticated RPC, linked environments, schedules/webhooks, guest access, secrets, PR/issue workflows, preview, terminal and device integration. The full profile needs implementations of these host services. |

## Documentation

| When you need to… | Open |
| --- | --- |
| Design an application that creates agents, sends messages, waits and handles human requests | [Proposed SDK, TypeScript interfaces and a two-harness example](sdk-proposal.md) |
| Trace a command from intake through SQLite to the provider and back to a subscriber | [Runtime architecture and per-harness differences](upstream-orchestration.md) |
| Check coverage for a command, read, stream, effect or MCP tool | [Parity inventory](parity-inventory.md) |
| Pick the next implementation task and see what proves it works | [Seven-stage extraction plan](extraction-plan.md) |
| Find the upstream owner of a behavior and review changes since the last port | [Run an upstream review](upstream-watch-guide.md#run-a-review) · [Component map](upstream-watch-guide.md#find-the-implementation-by-behavior) |

[src-1]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.ts
[src-2]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/runtimeLayer.ts
[src-3]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.ts
