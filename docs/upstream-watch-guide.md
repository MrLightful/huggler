# Maintaining parity with T3 Code

Use this guide when deciding whether an upstream change belongs in the npm package. The source map is pinned to **338352dee2f43079482e054e8229a2ba055641d9** on T3 Code’s `main` branch. The [watch list](t3code-watch.paths) contains Git pathspecs for comparing that revision with a later one. For the expected behavior of each operation, use the [capability inventory](parity-inventory.md). For the SDK that exposes it, see the [package overview](../README.md).

To browse newer commits on GitHub, start with the live histories for the component you maintain. [Engine history](https://github.com/pingdotgg/t3code/commits/main/apps/server/src/orchestration-v2) · [Agent tool history](https://github.com/pingdotgg/t3code/commits/main/apps/server/src/mcp) · [Driver registration history](https://github.com/pingdotgg/t3code/commits/main/apps/server/src/provider/builtInDrivers.ts)

## Find the implementation by behavior

Paths in the middle column are relative to the linked directories. Most engine services have a sibling `.test.ts` file. For changes spanning several services, start with an integration test that drives the public entry point.

| When the upstream change affects… | Start in T3 Code | What to check in the package |
| --- | --- | --- |
| Commands, IDs, statuses, errors and wire compatibility | In [contracts][packages/contracts/src], `orchestrationV2.ts` defines the domain, `orchestrationDispatch.ts` defines dispatch errors, `orchestratorMcp.ts` describes agent inputs/results, and `rpc.ts` and `environmentHttp.ts` define remote APIs. | Diff discriminants, field defaults and codecs. Decode an old event, command receipt and snapshot, then exercise the affected operation through the SDK or remote transport. |
| Boot, shutdown and which services the host supplies | [server.ts][apps/server/src/server.ts] composes production dependencies. [runtimeLayer.ts][apps/server/src/orchestration-v2/runtimeLayer.ts] wires the engine and production integrations. [serverRuntimeStartup.ts][apps/server/src/serverRuntimeStartup.ts] orders startup and recovery. | Update runtime construction and resource cleanup. Open an existing state directory, recover outstanding work, send, close and reopen. |
| Creating a thread, choosing its worktree, sending or steering | In the [engine][apps/server/src/orchestration-v2], start with `ThreadMessageIntake`. It routes launches through `ThreadLaunchService` and sends through `ThreadManagementService`. For a dispatched command, read `ThreadCommandExecutor`, `Orchestrator` and `CommandPolicy`. | Launch empty and with a first message. Send to an idle and a busy thread. Check the selected delivery mode, run ownership, held queue and receipt when retrying the same request. |
| SQLite, deduplication and restart behavior | In the [engine][apps/server/src/orchestration-v2], read `EventSink`, `EventStore`, `ProjectionStore`, `CommandReceiptStore`, `EffectOutbox`, `EffectWorker` and `ProviderRuntimeRecoveryService`. Read [persistence][apps/server/src/persistence] for `Migrations/`, migration registration and database initialization. Also watch [the SQLite worker][packages/shared/src/nodeSqliteClient.ts] and [legacy import][apps/server/src/orchestration-v2/legacy]. | Compare transactions, replay-safe effect kinds and process-bound effects. Upgrade a copy of an old database. Retry a committed command and simulate process loss around provider I/O. Check for duplicate turns, missing results and incorrect terminal states. |
| Provider calls, streaming events, questions and approvals | In the [engine][apps/server/src/orchestration-v2], follow `ProviderSessionManager` → `ProviderTurnStartService` → driver adapter → `ProviderEventIngestor` and `ProviderCommitBatch`. `RuntimeRequestService` and `ProviderTurnControlService` handle responses and controls. | Replay the affected protocol frames through the real adapter. Compare output order, pending request IDs, callback delivery, cancellation and final state. |
| Agents finding, messaging or delegating to each other | [OrchestratorMcpService.ts][apps/server/src/mcp/OrchestratorMcpService.ts] implements agent operations. [Toolkits][apps/server/src/mcp/toolkits] declare tools in `tools.ts` and bind them in `handlers.ts`. Read [threadAccess.ts][apps/server/src/mcp/threadAccess.ts], [McpSessionRegistry.ts][apps/server/src/mcp/McpSessionRegistry.ts] and [model-facing instructions][packages/provider-core/src/server/orchestrationInstructions.ts] together. | Exercise peer send/reply and parent-owned delegation through a scoped agent. Check the advertised schema and tool availability, sender provenance, duplicate request handling, and denial for another thread or an expired caller. |
| Child completion, status and waking a parent | In the [engine][apps/server/src/orchestration-v2], inspect `RunFinalizationService`, `NotificationMailbox`, `DelegatedTaskQuestionReactor`, `NativeChildWorkQuery` and `ThreadSettlementService`. Both client and server depend on [pending background work][packages/shared/src/orchestrationV2PendingBackgroundWork.ts]. | Let a parent finish its turn before the child. Check what status a host observes, when the parent wakes, which run owns the result, and when a delivery is acknowledged. Include a native child and a T3-delegated child. |
| Resume, provider switch, fork, merge and rollback | In the [engine][apps/server/src/orchestration-v2], follow `ProviderContinuationService`, `ProviderSwitchService`, `ThreadForkService`, `ContextHandoffService`, `ContextHandoffDelivery` and `CheckpointRollbackService`. [Checkpointing][apps/server/src/checkpointing] owns the workspace side. | Compare retained conversation, continuation identity and actual files after a transfer or rewind. Exercise native and portable paths plus a refusal to restore a shared checkout. |
| Status in a second application, history and reconnects | [ws.ts][apps/server/src/ws.ts] and [RPC authorization][apps/server/src/auth/RpcAuthorization.ts] expose scoped operations. In the [engine][apps/server/src/orchestration-v2], inspect `ShellStream`, `ThreadStream`, `WireProjection`, `CommandOutputHub` and `http.ts`. [Client state][packages/client-runtime/src/state] contains the consuming reducers, snapshot readers and history merge tests. | Compare a fresh snapshot with state reconstructed from deltas. Disconnect, resume with a recent cursor, then try an expired cursor and page older history. Verify both authorization and the status the app renders. |

## Locate a harness change

Start at [builtInDrivers.ts][apps/server/src/provider/builtInDrivers.ts]. Its two arrays register **11 execution drivers** and **8 aggregate usage readers** at this revision. The order of usage readers matters when records overlap. The [provider-core server API][packages/provider-core/src/server] defines `driver.ts` for provisioning and discovery, `ProviderAdapter.ts` for session and turn operations, and `ProviderHost.ts` for services an embedded host must supply. The [server provider directory][apps/server/src/provider] owns instance resolution, credentials, model discovery, `model-manifest.json` and the production host implementation.

| Driver kinds | Where the implementation lives | Changes that warrant a consumer test |
| --- | --- | --- |
| `codex`, `claudeAgent`, `antigravity` | [Server drivers][apps/server/src/provider/Drivers] and [server adapters][apps/server/src/orchestration-v2/Adapters]. Codex also uses [the app-server transport][packages/effect-codex-app-server]. Antigravity has [server ACP support][apps/server/src/provider/acp]. | A session-start field, native child event, callback or transport change can affect resumed conversations. Check CLI/SDK requirements and the corresponding adapter tests. |
| `acpRegistry`, `fx`, `grok`, `kiro` | [Registry][packages/provider-acp-registry], [FX][packages/provider-fx], [Grok][packages/provider-grok] and [Kiro][packages/provider-kiro]. Follow shared changes into [provider-acp][packages/provider-acp] and [effect-acp][packages/effect-acp]. | Run each affected ACP driver's fixture suite after a shared handshake, permission or session-negotiation change. Include a registry agent with different advertised capabilities. |
| `cursor`, `muse` | [Cursor][packages/provider-cursor] and [Muse][packages/provider-muse]. Inspect the upstream [SDK patches][patches] alongside the adapter versions. | Verify SDK callbacks, event translation and cancellation from an installed tarball. |
| `opencode`, `pi` | [OpenCode][packages/provider-opencode] contains both `src/server/adapter.ts` and [the 2.x adapter][packages/provider-opencode/src/server/v2/adapter.ts]. [Pi][packages/provider-pi] ships a [generated MCP extension][packages/provider-pi/src/server/mcpExtensionSource.ts]. | Test the selected OpenCode runtime generation and server lifetime. For Pi, load the extension from a packed consumer and call a scoped tool in a resumed session. |

A new driver registration calls for a new inventory row, install/status check and replay scenario. A change in a shared transport can affect several existing rows without changing the registry. The [provider matrix](upstream-orchestration.md#provider-support-at-the-pinned-revision) identifies which capabilities to exercise per driver.

## Follow a change beyond the engine

| Area | Upstream locations | Evidence to carry into the extraction |
| --- | --- | --- |
| Workspaces, worktrees and imported conversations | [Project][apps/server/src/project], [workspace][apps/server/src/workspace], [VCS][apps/server/src/vcs], [Git][apps/server/src/git], [checkpoints][apps/server/src/checkpointing], and [ACP registry sessions][apps/server/src/provider/AcpRegistrySessions.ts]. | Setup/cleanup, selected checkout, transcript provenance and first continuation after import. |
| Remote agents, guests and grants | [Peer forwarding and delegation][apps/server/src/peer], [auth][apps/server/src/auth], [guest access][apps/server/src/guest], [server relay][apps/server/src/relay] and [hosted relay][infra/relay]. | Test two runtimes with separate databases. Check the remote address, reconnect, cancellation and revoked grants. |
| Automation and external services | [Scheduled tasks and webhook ingress][apps/server/src/scheduledTasks], [secret requests][apps/server/src/secrets], [policy][apps/server/src/policy], [SCM registration][apps/server/src/sourceControl/sourceControlDefinitions.ts], [PR watch reactor][apps/server/src/orchestration-v2/PullRequestWatchReactor.ts] and [issue tracking][apps/server/src/issue]. The corresponding `packages/source-control-*`, `issue-tracker-*` and `secret-provider-*` packages implement backends. | Compare external side effects, credentials required, deduplication and the run that receives a wake or approval. |
| Interactive hosts | [Terminal][apps/server/src/terminal], [server preview][apps/server/src/preview], [desktop preview host][apps/desktop/src/preview], [devices][apps/server/src/device], [HTML rendering][apps/server/src/htmlRender] and [MCP Apps][apps/server/src/mcpApps]. | Exercise host availability, ownership, disconnect and resource disposal in the full profile. |
| Tests, recordings and intended behavior | [Engine testkit and fixtures][apps/server/src/orchestration-v2/testkit], [shared replay utilities][packages/provider-testing], [capture scripts][apps/server/scripts], [tool-use evals][evals/mcp-tool-use] and [architecture notes][docs/orchestration-v2]. | Preserve the recorded inputs and expected provider calls. At this revision Antigravity has adapter/protocol tests but lacks a shared replay-fixture runner entry. Capture that gap explicitly when qualifying it. |
| npm distribution | [Server package][apps/server/package.json], [bundler][apps/server/vite.config.ts], [workspace dependency pins][pnpm-workspace.yaml], [lockfile][pnpm-lock.yaml], [patches][patches] and [release tooling][scripts]. | Compare exported types, bundled worker/bridge/extension assets, runtime version constraints and notices. Install the resulting tarball outside the monorepo and exercise the changed driver or host. |

## Review changes since the last package update

Run the following in Bash or Zsh from the **package checkout**. Set `t3_repo` to your T3 Code clone. On the first review, use the analysis SHA below. On later reviews, use the last **reviewed** upstream SHA and carry forward the list of deferred changes. Keep a separate record of the last package release that incorporated each change.

```bash
watch_file="$(git rev-parse --show-toplevel)/docs/t3code-watch.paths"
t3_repo="/Users/mrlightful/Projects/Personal/t3code"
base=338352dee2f43079482e054e8229a2ba055641d9

git -C "$t3_repo" fetch origin
candidate="$(git -C "$t3_repo" rev-parse --verify 'refs/remotes/origin/main^{commit}')"
git -C "$t3_repo" merge-base --is-ancestor "$base" "$candidate"
```

Continue after the ancestry check returns success. If it fails, verify the remote and previous SHA and inspect the divergence before selecting a new baseline. Fetching the remote-tracking branch works with local working-tree edits and leaves the checkout at its current revision.

```bash
watch_specs=()
while IFS= read -r spec; do
  case "$spec" in
    ''|'#'*) continue ;;
  esac
  watch_specs+=("$spec")
done < "$watch_file"

git -C "$t3_repo" log --oneline "$base..$candidate" -- "${watch_specs[@]}"
git -C "$t3_repo" diff --find-renames --name-status "$base" "$candidate" -- "${watch_specs[@]}"
git -C "$t3_repo" diff --find-renames --name-status --diff-filter=ADR "$base" "$candidate"
```

The first command after the loop lists candidate commits. The second shows the net watched changes. The third scans the **whole repository** for added, deleted or renamed paths. Use it to spot a new driver family, a service moved out of the server, or fixtures added under a new root. Update the path list when ownership moves. The list intentionally includes the server and provider trees broadly so new siblings are picked up. Presentation-only edits elsewhere in the web, mobile and desktop clients generally fall outside this filter. For a change first reported in a client, inspect that client's path history directly.

For each relevant commit, read the whole patch with `git -C "$t3_repo" show --find-renames <commit>`, replacing `<commit>` with the SHA from the log. Follow changed tests and imports outside the watch list. Compare the resulting behavior to a row in the [inventory](parity-inventory.md) and run the corresponding [extraction acceptance scenario](extraction-plan.md) against the candidate and the installed package. For wire, storage or shared protocol changes, include old data or another affected driver.

Record the **upstream SHA, affected capability, disposition, package change or reason, and regression test**. Useful dispositions are *ported*, *already covered*, *host-only* and *deferred*. Link a deferred item to its outstanding work. Advance the review SHA after accounting for the range. Update the package's claimed parity baseline only to the extent supported by passing comparisons and documented exceptions.

### Worked example — a background child delegates after its parent finishes

Upstream commit `3a60d0996a` changed which run can own a delegation. A native background subagent can still be working after its parent's foreground turn completes. The fix uses the matching pending background subagent and the latest owning run when the caller has no active foreground run. The registration and name of `delegate_task` stayed the same. [Upstream patch](https://github.com/pingdotgg/t3code/commit/3a60d0996ae483952aec13a1ec0d8654f17c1421)

With the same `t3_repo` and `watch_specs` as above, compare the two views of this commit.

```bash
git -C "$t3_repo" diff --name-only '3a60d0996a^' 3a60d0996a -- "${watch_specs[@]}"
git -C "$t3_repo" diff --name-only '3a60d0996a^' 3a60d0996a
```

Five files match the watch list — `OrchestratorMcpService.ts`, `threadAccess.ts`, `Orchestrator.ts`, `orchestrationV2PendingBackgroundWork.ts` and `TaskCancelNativeSubagent.integration.test.ts`. The whole diff also shows changes to the web timeline logic and its test. Port the caller check, owning-run selection and shared background-state predicate together. Use the regression named **“a native background subagent delegates after its parent's run completes”** to verify the package, then compare what a client shows for that child. [Upstream regression test][apps/server/src/mcp/TaskCancelNativeSubagent.integration.test.ts]

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
