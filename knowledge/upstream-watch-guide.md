# Tracking T3 Code behavior

Use this guide to decide which upstream changes affect Huggler and how to reproduce their observable behavior. The source map is pinned to **338352dee2f43079482e054e8229a2ba055641d9** on T3 Code’s `main` branch. The [watch list](upstream-watch.paths) contains Git pathspecs for the review. For expected workflow behavior, see the [capability inventory](parity-inventory.md).

## Run a review

From the Huggler checkout, run the review with Git and Python 3. Fetching refreshes the clone’s remote-tracking ref. The review reads committed trees and prints the exact range.

```sh
upstream_repo="/Users/mrlightful/Projects/Personal/t3code"
git -C "$upstream_repo" fetch origin
python3 scripts/review-upstream.py "$upstream_repo"
```

The default range starts at `reviewedThrough` and ends at the local `origin/main`. Use `--base` and `--to` to select a different range from the local checkout. [Review command](../scripts/review-upstream.py) · [Review record](upstream-review.json)

| Report section | How to use it |
| --- | --- |
| Runtime and providers | Inspect command semantics, session and run state, MCP tools, provider registration and adapters, migrations, wire shapes, consuming reducers and replay assertions. New siblings under the engine and new `provider-*` packages match automatically. |
| Host integrations and package distribution | Check the affected host capability or shipped artifact. An optional integration can change observable behavior in an otherwise identical core runtime. |
| Supporting changes | Skim the commit subjects for new services, contracts, shared helpers, lockfile-only upgrades, CLI semantics and user-facing evidence. For example, an OpenCode scenario checks that a background result reaches a settled thread and can be read with the CLI. [Upstream scenario][background-scenario] |
| Outside the watch list | Presentation-only CSS, translations, native client layout and marketing usually need no crate work. For a client-reported behavior or a dependency outside the map, use `--all` for every commit and net changed path. Open an individual commit with `git -C "$upstream_repo" show --find-renames <sha>` to see its companion files. |
| New component directories | Inspect new root directories and new immediate children of `apps/`, `apps/server/src/`, `packages/`, `infra/` and `evals/`. Promote relevant owners into the path list, then rerun the same range. Git-detected renames show both names when either side matches a focused path. |

File sections show the net difference between the two trees. Commit lists also surface work introduced and reverted inside the interval. The map prioritizes locations. Features introduced entirely in existing unwatched files require the full-repository view, changed imports or upstream release notes to discover. Edit the first applicable section when an owner moves. [Watch paths](upstream-watch.paths)

To browse newer commits on GitHub, start with the live histories for the component you maintain. [Engine history](https://github.com/pingdotgg/t3code/commits/main/apps/server/src/orchestration-v2) · [Agent tool history](https://github.com/pingdotgg/t3code/commits/main/apps/server/src/mcp) · [Driver registration history](https://github.com/pingdotgg/t3code/commits/main/apps/server/src/provider/builtInDrivers.ts)

## Find the implementation by behavior

Paths in the middle column are relative to the linked directories. Most engine services have a sibling `.test.ts` file. For changes spanning several services, start with an integration test that drives the public entry point.

| When the upstream change affects… | Start in T3 Code | What to check in Huggler |
| --- | --- | --- |
| Commands, IDs, statuses, errors and wire compatibility | In [contracts][packages/contracts/src], `orchestrationV2.ts` defines the domain, `orchestrationDispatch.ts` defines dispatch errors, `orchestratorMcp.ts` describes agent inputs/results, and `rpc.ts` and `environmentHttp.ts` define remote APIs. | Diff discriminants, field defaults and codecs. Check changes to older Huggler records or supported import formats where relevant. Exercise the affected operation through the crate or host transport. |
| Boot, shutdown and which services the host supplies | [server.ts][apps/server/src/server.ts] composes production dependencies. [runtimeLayer.ts][apps/server/src/orchestration-v2/runtimeLayer.ts] wires the engine and production integrations. [serverRuntimeStartup.ts][apps/server/src/serverRuntimeStartup.ts] orders startup and recovery. | Update runtime construction and resource cleanup. Open an existing state directory, recover outstanding work, send, close and reopen. |
| Creating a thread, choosing its worktree, sending or steering | In the [engine][apps/server/src/orchestration-v2], start with `ThreadMessageIntake`. It routes launches through `ThreadLaunchService` and sends through `ThreadManagementService`. For a dispatched command, read `ThreadCommandExecutor`, `Orchestrator` and `CommandPolicy`. | Launch empty and with a first message. Send to an idle and a busy thread. Check the selected delivery mode, run ownership, held queue and receipt when retrying the same request. |
| SQLite, deduplication and restart behavior | In the [engine][apps/server/src/orchestration-v2], read `EventSink`, `EventStore`, `ProjectionStore`, `CommandReceiptStore`, `EffectOutbox`, `EffectWorker` and `ProviderRuntimeRecoveryService`. Read [persistence][apps/server/src/persistence] for `Migrations/`, migration registration and database initialization. Also watch [the SQLite worker][packages/shared/src/nodeSqliteClient.ts] and [legacy import][apps/server/src/orchestration-v2/legacy]. | Compare transactions, replay-safe effect kinds and process-bound effects. Upgrade a copy of an older Huggler database or run the supported upstream import. Retry a committed command and simulate process loss around provider I/O. Check for duplicate turns, missing results and incorrect terminal states. |
| Provider calls, streaming events, questions and approvals | In the [engine][apps/server/src/orchestration-v2], follow `ProviderSessionManager` → `ProviderTurnStartService` → driver adapter → `ProviderEventIngestor` and `ProviderCommitBatch`. `RuntimeRequestService` and `ProviderTurnControlService` handle responses and controls. | Replay the affected protocol frames through the real adapter. Compare output order, pending request IDs, callback delivery, cancellation and final state. |
| Agents finding, messaging or delegating to each other | [OrchestratorMcpService.ts][apps/server/src/mcp/OrchestratorMcpService.ts] implements agent operations. [Toolkits][apps/server/src/mcp/toolkits] declare tools in `tools.ts` and bind them in `handlers.ts`. Read [threadAccess.ts][apps/server/src/mcp/threadAccess.ts], [McpSessionRegistry.ts][apps/server/src/mcp/McpSessionRegistry.ts] and [model-facing instructions][packages/provider-core/src/server/orchestrationInstructions.ts] together. | Exercise peer send/reply and parent-owned delegation through a scoped agent. Check the advertised schema and tool availability, sender provenance, duplicate request handling, and denial for another thread or an expired caller. |
| Child completion, status and waking a parent | In the [engine][apps/server/src/orchestration-v2], inspect `RunFinalizationService`, `NotificationMailbox`, `DelegatedTaskQuestionReactor`, `NativeChildWorkQuery` and `ThreadSettlementService`. Both client and server depend on [pending background work][packages/shared/src/orchestrationV2PendingBackgroundWork.ts]. | Let a parent finish its turn before the child. Check what status a host observes, when the parent wakes, which run owns the result, and when a delivery is acknowledged. Include a native child and a child created through Huggler delegation. |
| Resume, provider switch, fork, merge and rollback | In the [engine][apps/server/src/orchestration-v2], follow `ProviderContinuationService`, `ProviderSwitchService`, `ThreadForkService`, `ContextHandoffService`, `ContextHandoffDelivery` and `CheckpointRollbackService`. [Checkpointing][apps/server/src/checkpointing] owns the workspace side. | Compare retained conversation, continuation identity and actual files after a transfer or rewind. Exercise native and portable paths plus a refusal to restore a shared checkout. |
| Status in a second application, history and reconnects | [ws.ts][apps/server/src/ws.ts] and [RPC authorization][apps/server/src/auth/RpcAuthorization.ts] expose scoped operations. In the [engine][apps/server/src/orchestration-v2], inspect `ShellStream`, `ThreadStream`, `WireProjection`, `CommandOutputHub` and `http.ts`. [Client state][packages/client-runtime/src/state] contains the consuming reducers, snapshot readers and history merge tests. | Compare a fresh snapshot with state reconstructed from deltas. Disconnect, resume with a recent cursor, then try an expired cursor and page older history. Verify both authorization and the status the app renders. |

## Locate a harness change

Start at [builtInDrivers.ts][apps/server/src/provider/builtInDrivers.ts]. Its two arrays register **11 execution drivers** and **8 aggregate usage readers** at this revision. The order of usage readers matters when records overlap. The [provider-core server API][packages/provider-core/src/server] defines `driver.ts` for provisioning and discovery, `ProviderAdapter.ts` for session and turn operations, and `ProviderHost.ts` for services an embedded host must supply. The [server provider directory][apps/server/src/provider] owns instance resolution, credentials, model discovery, `model-manifest.json` and the production host implementation.

| Driver kinds | Where the implementation lives | Changes that warrant a consumer test |
| --- | --- | --- |
| `codex`, `claudeAgent`, `antigravity` | [Server drivers][apps/server/src/provider/Drivers] and [server adapters][apps/server/src/orchestration-v2/Adapters]. Codex also uses [the app-server transport][packages/effect-codex-app-server]. Antigravity has [server ACP support][apps/server/src/provider/acp]. | A session-start field, native child event, callback or transport change can affect resumed conversations. Check CLI/SDK requirements and the corresponding adapter tests. |
| `acpRegistry`, `fx`, `grok`, `kiro` | [Registry][packages/provider-acp-registry], [FX][packages/provider-fx], [Grok][packages/provider-grok] and [Kiro][packages/provider-kiro]. Follow shared changes into [provider-acp][packages/provider-acp] and [effect-acp][packages/effect-acp]. | Run each affected ACP driver's fixture suite after a shared handshake, permission or session-negotiation change. Include a registry agent with different advertised capabilities. |
| `cursor`, `muse` | [Cursor][packages/provider-cursor] and [Muse][packages/provider-muse]. Inspect the upstream [SDK patches][patches] alongside the adapter versions. | Verify equivalent live requests, event translation and cancellation through Huggler’s selected transport. Inspect upstream patches when behavior differs. |
| `opencode`, `pi` | [OpenCode][packages/provider-opencode] contains both `src/server/adapter.ts` and [the 2.x adapter][packages/provider-opencode/src/server/v2/adapter.ts]. [Pi][packages/provider-pi] ships a [generated MCP extension][packages/provider-pi/src/server/mcpExtensionSource.ts]. | Test the selected OpenCode runtime generation and server lifetime. For Pi, verify any chosen extension from an installed host and call a scoped tool in a resumed session. |

A new driver registration calls for an inventory entry, install/status check and replay scenario. A change in a shared transport can affect several drivers. Check which operations each affected driver advertises. [Huggler provider reference](../docs/content/docs/reference/providers.mdx)

## Follow a change beyond the engine

| Area | Upstream locations | Evidence to carry into Huggler |
| --- | --- | --- |
| Workspaces, worktrees and imported conversations | [Project][apps/server/src/project], [workspace][apps/server/src/workspace], [VCS][apps/server/src/vcs], [Git][apps/server/src/git], [checkpoints][apps/server/src/checkpointing], and [ACP registry sessions][apps/server/src/provider/AcpRegistrySessions.ts]. | Setup/cleanup, selected checkout, transcript provenance and first continuation after import. |
| Remote agents, guests and grants | [Peer forwarding and delegation][apps/server/src/peer], [auth][apps/server/src/auth], [guest access][apps/server/src/guest], [server relay][apps/server/src/relay] and [hosted relay][infra/relay]. | Test two runtimes with separate databases. Check the remote address, reconnect, cancellation and revoked grants. |
| Automation and external services | [Scheduled tasks and webhook ingress][apps/server/src/scheduledTasks], [secret requests][apps/server/src/secrets], [policy][apps/server/src/policy], [SCM registration][apps/server/src/sourceControl/sourceControlDefinitions.ts], [PR watch reactor][apps/server/src/orchestration-v2/PullRequestWatchReactor.ts] and [issue tracking][apps/server/src/issue]. The corresponding `packages/source-control-*`, `issue-tracker-*` and `secret-provider-*` packages implement backends. | Compare external side effects, credentials required, deduplication and the run that receives a wake or approval. |
| Interactive hosts | [Terminal][apps/server/src/terminal], [server preview][apps/server/src/preview], [desktop preview host][apps/desktop/src/preview], [devices][apps/server/src/device], [HTML rendering][apps/server/src/htmlRender] and [MCP Apps][apps/server/src/mcpApps]. | Exercise host availability, ownership, disconnect and resource disposal in the full profile. |
| Tests, recordings and intended behavior | [Engine testkit and fixtures][apps/server/src/orchestration-v2/testkit], [shared replay utilities][packages/provider-testing], [capture scripts][apps/server/scripts], [tool-use evals][evals/mcp-tool-use] and [architecture notes][docs/orchestration-v2]. | Preserve the recorded inputs and expected provider calls. At this revision Antigravity has adapter/protocol tests but lacks a shared replay-fixture runner entry. Capture that gap explicitly when qualifying it. |
| Upstream distribution | [Server package][apps/server/package.json], [bundler][apps/server/vite.config.ts], [workspace dependency pins][pnpm-workspace.yaml], [lockfile][pnpm-lock.yaml], [patches][patches] and [release tooling][scripts]. | Identify changes to runtime assets, provider compatibility and licensing. If a change applies to Huggler, exercise it from a packaged crate or installed GPUI build. |

## Adapt and merge a relevant change

1. **Identify the observable change.** Open the full upstream patch. Find the assertion that distinguishes the old and new behavior and the identity that observes it (host, calling agent, parent or child). Use `git -C "$upstream_repo" show <sha>:<path>` to read the implementation and regression at the candidate revision. Look up the matching command, event, tool or integration. [Parity inventory](parity-inventory.md)
2. **Follow the behavior across boundaries.** Check the contract and handler, adapter and replay fixture, or writer, projection and migration as applicable. If upstream only changes presentation, check whether the crate already returns the underlying data. A tool description can change how an agent behaves even when the schema stays identical.
3. **Translate the behavior.** Record the upstream commit and the corresponding Rust API, adapter or host integration. Preserve the accepted workflow across API and transport differences. Carry across reusable fixtures and preserve license notices. Bundle dependent upstream changes when one fix relies on another. [Huggler naming conventions](README.md#naming)
4. **Verify at the consumer boundary.** Run or inspect the upstream regression at the candidate. Drive the same scenario through the crate or GPUI host and compare its observable result. Use a prior Huggler database for migrations, another adapter for shared protocol changes, or a clean install for runtime asset changes. Capture a fresh replay when the transport differs. [Verification guide](verification.md)
5. **Close the review interval.** In the crate change or review notes, record the range, capability, and disposition (*ported*, *already covered*, *host-only* or *deferred*) with the commit, test or reason. Set `reviewedThrough` to the full candidate SHA after accounting for the interval. For outstanding work, append an object to `deferred` with `upstream` (full commit SHA) and `followUp` (capability plus issue, task link or concrete pending check). The command prints those entries on subsequent runs, including empty ranges. Remove an entry once its disposition is recorded. Base release coverage on passing crate or host evidence. [Review record](upstream-review.json)

### Worked example — a background child delegates after its parent finishes

Upstream commit `3a60d0996a` changed which run can own a delegation. A native background subagent can still be working after its parent's foreground turn completes. The fix uses the matching pending background subagent and the latest owning run when the caller has no active foreground run. The registration and name of `delegate_task` stayed the same. [Upstream patch](https://github.com/pingdotgg/t3code/commit/3a60d0996ae483952aec13a1ec0d8654f17c1421)

To reproduce the classification and inspect the full patch, use the clone from the quick start.

```sh
python3 scripts/review-upstream.py "$upstream_repo" --base '3a60d0996a^' --to 3a60d0996a
git -C "$upstream_repo" show --find-renames 3a60d0996a
```

Five files match the watch list — `OrchestratorMcpService.ts`, `threadAccess.ts`, `Orchestrator.ts`, `orchestrationV2PendingBackgroundWork.ts` and `TaskCancelNativeSubagent.integration.test.ts`. The whole diff also shows changes to the web timeline logic and its test. Port the caller check, owning-run selection and shared background-state predicate together. Use the regression named **“a native background subagent delegates after its parent's run completes”** to verify the crate, then compare what a client shows for that child. [Upstream regression test][apps/server/src/mcp/TaskCancelNativeSubagent.integration.test.ts]

[packages/contracts/src]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src
[apps/server/src/server.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/server.ts
[apps/server/src/orchestration-v2/runtimeLayer.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/runtimeLayer.ts
[apps/server/src/serverRuntimeStartup.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/serverRuntimeStartup.ts
[apps/server/src/orchestration-v2]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2
[apps/server/src/persistence]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/persistence
[packages/shared/src/nodeSqliteClient.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/shared/src/nodeSqliteClient.ts
[apps/server/src/orchestration-v2/legacy]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/legacy
[apps/server/src/mcp/OrchestratorMcpService.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.ts
[apps/server/src/mcp/toolkits]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits
[apps/server/src/mcp/threadAccess.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/threadAccess.ts
[apps/server/src/mcp/McpSessionRegistry.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/McpSessionRegistry.ts
[packages/provider-core/src/server/orchestrationInstructions.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-core/src/server/orchestrationInstructions.ts
[packages/shared/src/orchestrationV2PendingBackgroundWork.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/shared/src/orchestrationV2PendingBackgroundWork.ts
[apps/server/src/checkpointing]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/checkpointing
[apps/server/src/ws.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/ws.ts
[apps/server/src/auth/RpcAuthorization.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/auth/RpcAuthorization.ts
[packages/client-runtime/src/state]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/client-runtime/src/state
[apps/server/src/provider/builtInDrivers.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/builtInDrivers.ts
[packages/provider-core/src/server]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-core/src/server
[apps/server/src/provider]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider
[apps/server/src/provider/Drivers]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/Drivers
[apps/server/src/orchestration-v2/Adapters]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Adapters
[packages/effect-codex-app-server]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/effect-codex-app-server
[apps/server/src/provider/acp]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/acp
[packages/provider-acp-registry]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-acp-registry
[packages/provider-fx]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-fx
[packages/provider-grok]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-grok
[packages/provider-kiro]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-kiro
[packages/provider-acp]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-acp
[packages/effect-acp]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/effect-acp
[packages/provider-cursor]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-cursor
[packages/provider-muse]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-muse
[patches]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/patches
[packages/provider-opencode]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-opencode
[packages/provider-opencode/src/server/v2/adapter.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-opencode/src/server/v2/adapter.ts
[packages/provider-pi]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-pi
[packages/provider-pi/src/server/mcpExtensionSource.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-pi/src/server/mcpExtensionSource.ts
[apps/server/src/project]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/project
[apps/server/src/workspace]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/workspace
[apps/server/src/vcs]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/vcs
[apps/server/src/git]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/git
[apps/server/src/provider/AcpRegistrySessions.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/AcpRegistrySessions.ts
[apps/server/src/peer]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/peer
[apps/server/src/auth]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/auth
[apps/server/src/guest]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/guest
[apps/server/src/relay]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/relay
[infra/relay]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/infra/relay
[apps/server/src/scheduledTasks]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/scheduledTasks
[apps/server/src/secrets]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/secrets
[apps/server/src/policy]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/policy
[apps/server/src/sourceControl/sourceControlDefinitions.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/sourceControl/sourceControlDefinitions.ts
[apps/server/src/orchestration-v2/PullRequestWatchReactor.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/PullRequestWatchReactor.ts
[apps/server/src/issue]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/issue
[apps/server/src/terminal]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/terminal
[apps/server/src/preview]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/preview
[apps/desktop/src/preview]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/desktop/src/preview
[apps/server/src/device]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/device
[apps/server/src/htmlRender]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/htmlRender
[apps/server/src/mcpApps]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcpApps
[apps/server/src/orchestration-v2/testkit]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit
[packages/provider-testing]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-testing
[apps/server/scripts]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/scripts
[evals/mcp-tool-use]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/evals/mcp-tool-use
[docs/orchestration-v2]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/docs/orchestration-v2
[apps/server/package.json]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/package.json
[apps/server/vite.config.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/vite.config.ts
[pnpm-workspace.yaml]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/pnpm-workspace.yaml
[pnpm-lock.yaml]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/pnpm-lock.yaml
[scripts]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/scripts
[apps/server/src/mcp/TaskCancelNativeSubagent.integration.test.ts]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/TaskCancelNativeSubagent.integration.test.ts

[background-scenario]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/scripts/scenarios/opencode-background-result-wakes-thread.e2e.ts
