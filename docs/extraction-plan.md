# Extracting and qualifying Huggler

Start with one installed consumer that can run a conversation across a restart. Then prove that two independently configured harnesses can collaborate through the same public entry points a real app will use. Keep those scenarios running as the remaining providers and host integrations move over. The [SDK proposal](sdk-proposal.md) defines the application-facing interfaces. The [architecture guide](upstream-orchestration.md) traces its dependencies. The [parity inventory](parity-inventory.md) indexes upstream operations and variants to verify through their Huggler equivalents.

These are implementation tasks and proposed acceptance tests. The reference is T3 Code commit `338352dee2f43079482e054e8229a2ba055641d9`. A test cited below is an entry point to port or extend. Test results and live compatibility evidence belong to the implementation phase.

## Choose a stage

| Stage | Ready when | Evidence to keep |
| --- | --- | --- |
| [1. Capture the reference](#1-establish-the-comparison-before-moving-code) | A checkout contains the pinned source | Transcripts, expected projections and focused test results. Begin with the [Codex restart recipe](first-milestone.md). |
| [2. Run one durable thread](#2-extract-contracts-and-run-one-durable-thread) | The reference case is reproducible | An installed consumer resumes the same conversation and a retried command returns its original receipt. |
| [3. Connect two harnesses](#3-prove-agent-to-agent-operation-with-two-harnesses) | One driver works through the public API | A peer reply with sender identity and a delegated completion that wakes its parent. |
| [4. Qualify all drivers](#4-qualify-every-provider-instance-and-capability) | Two harnesses share a conformance suite | Results per driver, provider version, instance configuration and supported operation. |
| [5. Complete runtime behavior](#5-complete-control-history-and-recovery) | Stage 2 works, with qualified drivers available for provider-specific cases | Results for history, commands, requests, stream recovery, process loss and migration. |
| [6. Assemble the full profile](#6-assemble-the-full-application-profile) | Agent tools and runtime controls work | An authorized second application completes a workflow with the host integrations enabled. |
| [7. Qualify a release](#7-validate-the-npm-distribution-and-future-upgrades) | Begin tarball checks in stage 2 and finish after stages 3–6 | Install and upgrade results for the shipped package, plus a comparison to the reviewed upstream SHA. |

## 1. Establish the comparison before moving code

**Prerequisite.** A separate checkout at the reference commit. Use its pinned Node 26.13.1 and pnpm 12.14.0 for the reference build. Give it a temporary state directory, provider homes and Git repository. Read the upstream testing guide and run the focused suites from the checkout. [Repository toolchain][src-toolchain] · [Test commands][src-agents] · [Testing strategy][src-strategy]

```sh
vp test run apps/server/src/orchestration-v2/testkit/CodexReplayFixtures.integration.test.ts
vp test run apps/server/src/orchestration-v2/Orchestrator.receipt-race.test.ts
vp test run apps/server/src/mcp/OrchestratorMcpToolkit.integration.test.ts
vp test run apps/server/src/orchestration-v2/testkit/OrchestratorReplayRecovery.integration.test.ts
```

1. Use the staged commands and expected outcomes in `OrchestratorScenario`, the real adapters wired by `ProviderReplayHarness`, and the versioned transcripts in the fixture directories. Carry over deterministic clock, IDs and workspace path substitution. Keep both inbound frames and expected outbound calls. [Scenario runner][src-1] · [Adapter harness][src-2] · [Scenario catalog][src-3] · [Transport utilities][src-4]
2. For each inventory row, record the public entry point, required caller/host, source test or missing case, and an observable assertion. Save reference receipts, ordered lifecycle transitions, final conversations, relations between IDs, cursors, provider calls and relevant file changes. Normalize temporary paths, times and random identifiers consistently while preserving ordering and equalities. Keep representative v1/v2 databases made from test data for migration tests.
3. Compare the architecture notes to current registration and executable tests. Record differences by SHA. Keep provider versions alongside recorded transcripts and remove credentials or personal content before storing new captures. The recording scripts and tool-use eval cases cover different questions. A replay verifies runtime behavior. An eval checks whether a model can choose the right tool from its instructions. [Architecture notes][src-5] · [Recorders][src-6] · [Tool-use evals][src-7]

**Acceptance.** A second developer can repeat the reference runs, match a selected receipt to its projection and provider transcript, and identify which rows have only source evidence. Use the same scenario inputs on both sides of the extraction.

## 2. Extract contracts and run one durable thread

**Depends on stage 1.** Make this the first package change. Use Codex because the upstream replay suite already exercises opening, continuing and reopening one of its sessions.

1. Port the contract dependency closure, including branded IDs, schemas, codecs and tagged errors. Separate the browser-safe wire export from runtime construction. Preserve decoding of saved records, including optional fields introduced after older records were written. Compile a minimal TypeScript consumer against the public declarations. [Domain and wire shapes][src-8] · [Agent contracts][src-9]
2. Follow `runtimeLayer.ts` to lift the command executor, policy, event sink, projections, receipts, effect outbox/worker and session/turn management together. Bring the production SQLite client and migration chain. Use a private state directory with one engine owner. Extract dependencies on global server config into explicit options for paths, credentials, process execution and integration services. Share the event sink, viewer registry and provider instance registry across writers and subscribers. [Service composition][src-10] · [SQLite and migrations][src-11] · [Transaction tests][src-12]
3. Open the runtime with `openHuggler` and release its resources with `runtime.close()`. Follow the server's startup order for recovery, worker startup and admitting commands. Provide project open, root-checkout launch, send, read, wait and subscribe. Use the production Codex driver/adapter with its replay transport. Report unavailable host features explicitly until their integration is wired. [Startup][src-13] · [Launch tests][src-14] · [Management][src-15]
4. Pack an initial build and install it in a scratch consumer. Launch an empty thread, subscribe, send the first turn and wait for its final answer. Close the engine, reopen the same state, read the first answer, then send the second turn on the original thread. Exercise a second thread in the same process to catch instance or event-stream leakage.

**Acceptance.** The consumer sees the same command receipt semantics, ordered messages, terminal status, stable thread ID and resumed conversation as the reference. A repeated command on the winning thread returns its original sequence. Two concurrent submissions of one command ID against different threads produce one success and one `OrchestratorCommandIdConflictError`. Closing the engine releases its SQLite workers and process resources so the consumer exits. [Race test][src-16] · [Restart scenario][src-17]

## 3. Prove agent-to-agent operation with two harnesses

**Depends on stage 2.** Add Claude Agent SDK alongside Codex, per-instance configuration and workspace model discovery. Port MCP session credentials, caller resolution, revocation, access checks, deferred discovery and tool handlers. Register them on the `huggler` server and update tool references in model instructions, provider bridges, tool search results and eval expectations. Run each case below through a provider-facing MCP endpoint and inspect the result through the host SDK. [Huggler naming conventions](sdk-proposal.md#naming) · [MCP service][src-18] · [Toolkit integration tests][src-19] · [Access races][src-20]

| Case | Arrange and act | Required observation |
| --- | --- | --- |
| Independent peers | Launch A on Codex and B on Claude. A uses `thread_send` to request a review. Let A's first turn finish. B reads A's address from the received origin and sends back its findings. Repeat B's send with the same request key. | B receives the request with A as sender. A receives one reply with B as sender and can start a later run. Each send has a receipt and actual delivery mode. Reopening either thread preserves the exchange. |
| Owned delegation | A delegates an async review to B's provider instance. Let the parent go idle. Finish the child, observe it from a dashboard and allow completion delivery to the parent. Repeat with an explicit parent acknowledgement, a cancelled child and a grandchild. | Each task retains its parent, child address and result. A passive read leaves delivery intact. An eligible completion wakes the idle parent with the result once. An acknowledgement or cancellation has the upstream effect on later delivery. |
| Native child | Replay a harness-native subagent, including one that outlives the root turn. Inspect the execution tree and, where supported, interrupt the child. | The tree shows the native ancestry and correct owning run. The package exposes the controls the adapter supports. Native child addressability follows the provider capability. |
| Caller constraints | Issue two caller-bound credentials, exercise an allowed send, then revoke one. Try an owned-task read as a different parent, a mode escalation, a user question and a provider approval. | The live caller owns the recorded action. Forbidden calls return the expected error and cause no write. A user question and a human permission decision reach their respective responders. |

Add local batch thread creation, shared checkout versus isolated worktree, queued child follow-up, status/timeout, concurrent completions and stop propagation to these cases. Run the upstream `review-round-uses-thread-send` eval when testing agent instructions. That case expects **another delegation** when the user requested a second delegated review round. A separate positive case should explicitly request an independent peer reply. [Completion tests][src-21] · [Native and delegated cancellation][src-22] · [Review eval inputs][src-23]

**Acceptance.** Capture the actual MCP requests and host-visible thread/task results for all four cases. A two-agent demo can display who asked whom, which run produced the answer, who can stop the work and which request requires a human.

## 4. Qualify every provider instance and capability

**Depends on stage 3 for the common conformance suite.** Move the built-in driver registry, usage-reader order, provider host, credential/environment resolution, model catalog, installation/version probes and maintenance hooks. For each driver keep a configuration boundary and a scoped session adapter. Verify continuation identity when using multiple instances of the same kind. Preserve restrictions on providers sharing a machine login or external server. [Registration and reader order][src-24] · [Driver contract][src-25] · [Session contract][src-26] · [Host implementation][src-27]

1. Run the per-driver cases in the [provider table](upstream-orchestration.md#provider-support-at-the-pinned-revision). Qualify Codex and Claude first, then the shared ACP transport and its Grok, fx, Kiro, Antigravity and ACP Registry flavors. Exercise Cursor, Pi, Muse and both OpenCode majors through their actual adapter entry points. Partition the suite by driver so a basic import works on a machine with a single harness.
2. For each supported version and ownership mode, test discovery, login state, session start, prompt, streamed output, settlement, interrupt, restart/resume, requests and MCP round-trip. For forks, rollback, steering, model changes, compaction, native children and background work, verify each advertised capability or the actual negative outcome. Capture the static declaration and the effective session capability after startup. Exercise the branch where an ACP handshake omits fork and the branch where a managed OpenCode 2.x server registers MCP by directory.
3. Use upstream replay files where present. Add a representative Antigravity recording that goes through adapter, ingestion and projection. It has dedicated adapter/driver/protocol tests, but the shared runner registers the other ten drivers. For ACP Registry record the registry agent and protocol version used. Qualify managed and external OpenCode separately. Check aggregate usage against overlapping and absent records. [Replay registry][src-28] · [Antigravity adapter tests][src-29] · [Antigravity protocol tests][src-30] · [Usage aggregation tests][src-31]

**Acceptance.** Publishable evidence has a row per driver, harness version, OS, architecture and operating mode, with separate columns for replay, live smoke test and unsupported operations. A missing SDK/login or an incompatible version yields an actionable instance status. Other configured instances stay usable. Claims about live operation require a run with that harness.

## 5. Complete control, history and recovery

**Depends on stage 2.** Use the qualified adapters from stages 3–4 for provider-specific paths. Move the remaining command families, project mutations, queries and streams in the [inventory](parity-inventory.md). Test the following behaviors through application APIs.

| Area | Tasks and observable acceptance |
| --- | --- |
| Queue, dispatch and stop | Exercise auto/queue/steer/restart delivery, editing and reordering queued work, fresh context, prepared-run failure/retry, interrupt, force stop, background waits, usage-limit reset and automatic continuation. Stop a thread with queued work and a live child. Confirm the queue stays held until resumed and stale completions cannot start the wrong run. [Queue finalization][src-32] · [Stop][src-33] · [Limit recovery][src-34] |
| Human input and attachments | Route approval/denial, questions, per-question attachments and dismissal to the owning session/node. Lose a live-only callback during restart and verify its projected outcome and retry behavior. Exercise upload preparation, claim, cleanup and duplicate send. [Request tests][src-35] · [Claims][src-36] |
| Model, context and files | Test live selection changes and changes that restart a session. Run native fork, prior-turn fork, portable fork, merge-back and cross-provider handoff. Check the stable prefix, attribution, history budget and the first outbound prompt after each transfer. Use temporary repositories for rollback with and without file restore, dirty state and shared-worktree ownership. [Switch][src-37] · [Fork][src-38] · [Merge-back][src-39] · [Restore safety][src-40] |
| Reads and transport | Serve all 23 orchestration RPC methods and five HTTP reads with the original payload/access semantics. Exercise shell vs detail, bounded history paging, search, diffs, workflow/native child reads, command output and prompt suggestions. Disconnect, replay from a retained cursor, and reconnect with an expired cursor. Compare the rebuilt client state with a fresh authoritative snapshot. Run the upstream stream budgets against long histories and large output. [Stream budget][src-41] · [Bounded snapshots][src-42] · [Transport performance][src-43] |
| Crash and process loss | Terminate the runtime before commit, after commit/before claiming an effect, and after external I/O/before recording completion. Reopen the same file-backed database. Verify committed intent, outbox disposition by kind, state reconciliation, outstanding delegated completions and how uncertainty is reported. No second outbound prompt or approval may result from blindly replaying a process-bound row. Retried cleanup tolerates an already-removed resource. [Outbox recovery][src-44] · [Runtime recovery regressions][src-45] |
| Existing data | Exercise v1 cutover on a copy, lazy history import and first continuation. Scan/import Claude, Codex and Pi transcripts. Import an ACP Registry session through its separate route. Repeat, change or remove a source, and inspect history, provenance and continuation eligibility each time. [V1 cutover test][src-46] · [External import][src-47] · [Registry sessions][src-48] |

**Acceptance.** Each command/event/effect group has an assertion on its external result or resulting state. Historical data still decodes. Retention, coalescing and pagination produce a consistent client view across a disconnect. Scope denial and unsupported capabilities produce distinguishable errors.

## 6. Assemble the full application profile

**Depends on stages 3 and 5.** Keep explicit integration imports for small hosts. Wire each reference integration into the full profile, including worker lifetime and cleanup. Publish its prerequisites and use at least one positive and one unavailable/denied case. The package needs a host implementation for capabilities that call an external service.

| Integration | Acceptance scenario |
| --- | --- |
| Projects and workspaces | Launch in the root, an existing worktree and a newly allocated worktree. Fail a setup script, inspect preparation status, retry and release the run. Exercise clone, scratch promotion and cleanup with a sibling still using the checkout. [Launch][src-14] · [Setup][src-49] |
| Linked environments | Start two runtimes with independent state. Link with approval, send and delegate remotely, disconnect the follower, reconnect and complete the task. Preserve the remote address, sender and local parent result. Revoking a grant blocks subsequent writes. Reject unsupported remote worktree isolation. [Remote delegation][src-50] · [Durable remote task][src-51] |
| Guests and authenticated RPC | Redeem a share, inspect the guest view, submit a message, accept/dismiss it from the host and close the share. Verify another thread remains inaccessible. Exercise read-only and operating RPC identities on the same command. Qualify a relay-backed route separately if offered. [Guest service tests][src-52] · [RPC authorization][src-53] |
| Schedules and webhooks | With a controlled clock, exercise interval, fixed-time and one-shot schedules around a timezone boundary. Verify new-thread/continue/fresh-session behavior. Deliver a signed webhook twice, then an invalid signature or filtered event. Compare run history, deduplication and cleanup on thread deletion. [Scheduling and webhook tests][src-54] |
| SCM and issue trackers | Resolve project bindings through the seven registered SCM drivers. Run per-driver supported operations against recorded service responses. Verify link/unlink, lists, CI/check reads, mutations and watch-triggered wakes. Repeat a stale sync and inspect its effect. Include the Linear integration and unavailable credentials. [SCM definitions][src-55] · [PR watch][src-56] · [Linear driver][src-57] |
| Secrets and policy | Request a secret, approve/deny through the host and inspect provider delivery, expiry and persisted/audited fields. Exercise configured secret references, including 1Password, and managed policy limits on a live agent. [Secret requests][src-58] · [1Password][src-59] · [Managed policy][src-60] |
| Terminal, preview, devices, HTML and MCP Apps | Stream terminal output through disconnect/close, open and close a thread-owned preview, navigate/capture it, select/open/capture/close a device, and render HTML under the host policy. Service an MCP App tool/resource and carry allowed model context to the next turn. Test cleanup and loss of host availability for each service. [Terminal manager][src-61] · [Browser hosts][src-62] · [Device tests][src-63] · [HTML renderer][src-64] · [MCP App requests][src-65] |

**Acceptance.** Match each of the 92 upstream MCP declarations to its Huggler tool name. Compare toolkit, input/result/error schema and exposed operations for representative identities. Exercise deferred discovery, user messaging, environment preferences and theme alongside the integrations above. Drive one complete workflow through a second application using the published contracts and full profile.

## 7. Validate the npm distribution and future upgrades

**Depends on all stages for a full-parity release.** Start this packaging check in stage 2 and repeat it for every driver or integration added.

1. Compile Node ESM and declarations, preserve subpath exports and resolve internal `workspace:`/catalog references. The upstream CLI bundles private TS-source workspace packages. For each dependency choose compiled internal code, a publishable dependency or a lazily loaded optional runtime. Audit the pnpm patches that affect the chosen profile. The reference pins Effect 4.1.3 and patches `@effect/platform-node-shared@4.1.2`, `@cursor/sdk@1.4.1` and `@muse-code/sdk@1.4.6`. OpenCode 2's protocol/schema require compatible Effect identity. A consumer's npm install must preserve that behavior independently of the upstream workspace overrides. [Workspace pins and patches][src-66] · [Server manifest][src-67] · [Bundler][src-68]
2. Test runtime asset resolution from the installed package. Execute the inline SQLite workers, Claude history worker, Pi MCP extension and ACP MCP stdio bridge. For the bridge, supply a package-owned executable invocation that works inside a different application's Node process. Check model-manifest JSON, process spawning, SDK binaries, optional platform modules and terminal PTY assets. Exercise a path containing spaces and start from an unrelated cwd. [SQLite worker source][src-69] · [History worker][src-70] · [Pi extension][src-71] · [ACP bridge][src-72] · [Model manifest][src-73]
3. Run `npm pack --json`, check that the packed manifest names the package `huggler`, and inspect its exports, declarations and runtime assets for upstream-branded names. Install the resulting `.tgz` in a fresh project outside any workspace with a fresh dependency tree. Type-check and run the [two-agent SDK example](sdk-proposal.md#two-agents-in-one-application), install with optional dependencies omitted for a minimal profile, import the contracts in a browser bundle, and run the full-profile smoke scenarios. Check that public declaration imports also resolve. The upstream CLI declares Node `^22.16 || ^23.11 || >=24.10`. Set the package support range from installed-consumer runs across supported Node/OS/architecture combinations.
4. Include the upstream MIT notice (copyright 2026 T3 Tools Inc.) with copied code. Generate third-party notices from the actual distribution and verify redistribution terms for any bundled SDK/binary. Keep external harness installation and account requirements documented per instance. [MIT license][src-74] · [Upstream notice configuration][src-75]
5. For subsequent upstream updates, use the [source map and watch workflow](upstream-watch-guide.md) to compare the last reviewed SHA with a pinned candidate. Record which changes affect the package and which tests demonstrate parity. Re-run the inventory, differential scenarios and packed-install suite. Version the public SDK and wire contracts deliberately, and exercise migration from the previous package release using a copy of its database.

**Release evidence.** Retain the tarball manifest, version/OS table, attribution, baseline comparison and a link to a passing scenario or explicit limitation for every inventory group. A clean application should be able to install the tarball, create and inspect agents, complete both forms of cross-harness collaboration, restart, and recover the expected results.

[src-toolchain]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/package.json
[src-agents]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/AGENTS.md
[src-strategy]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/docs/orchestration-v2/testing-strategy.md

[src-1]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/OrchestratorScenario.ts
[src-2]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/ProviderReplayHarness.ts
[src-3]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures
[src-4]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-testing
[src-5]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/docs/orchestration-v2
[src-6]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/scripts
[src-7]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/evals/mcp-tool-use/README.md
[src-8]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/orchestrationV2.ts
[src-9]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/orchestratorMcp.ts
[src-10]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/runtimeLayer.ts
[src-11]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/persistence
[src-12]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/EventSink.test.ts
[src-13]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/serverRuntimeStartup.ts
[src-14]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ThreadLaunchService.test.ts
[src-15]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ThreadManagementService.ts
[src-16]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.receipt-race.test.ts
[src-17]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/OrchestratorReplayRecovery.integration.test.ts
[src-18]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.ts
[src-19]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpToolkit.integration.test.ts
[src-20]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/McpToolAccess.race.test.ts
[src-21]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/DelegatedCompletionDelivery.test.ts
[src-22]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/TaskCancelNativeSubagent.integration.test.ts
[src-23]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/evals/mcp-tool-use/cases/orchestration.ndjson
[src-24]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/builtInDrivers.ts
[src-25]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-core/src/server/driver.ts
[src-26]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-core/src/server/ProviderAdapter.ts
[src-27]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/ProviderHostLive.ts
[src-28]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/ProviderReplayFixtures.ts
[src-29]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Adapters/AntigravityAdapterV2.test.ts
[src-30]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/acp/AntigravityProtocol.test.ts
[src-31]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/usage/usageAggregation.test.ts
[src-32]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/RunFinalizationQueue.test.ts
[src-33]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ThreadStop.test.ts
[src-34]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/UsageLimitRecoveryWorker.test.ts
[src-35]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/RuntimeRequestService.test.ts
[src-36]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/AttachmentClaims.test.ts
[src-37]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/ProviderSwitch.integration.test.ts
[src-38]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/ThreadFork.integration.test.ts
[src-39]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/ThreadMergeBack.integration.test.ts
[src-40]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/CheckpointRestoreSafety.test.ts
[src-41]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/LiveStreamBudget.test.ts
[src-42]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/boundedSnapshotTransport.test.ts
[src-43]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ThreadTransportPerformance.test.ts
[src-44]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/EffectOutbox.ts
[src-45]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ProviderRuntimeRecoveryService.regression.test.ts
[src-46]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/legacy/LegacyV1Cutover.integration.test.ts
[src-47]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/project/AgentSessionImporter.test.ts
[src-48]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/AcpRegistrySessions.ts
[src-49]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/project/ProjectSetupScriptRunner.test.ts
[src-50]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/peer/RemoteDelegation.test.ts
[src-51]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/RemoteDelegatedTask.test.ts
[src-52]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/guest/GuestShareService.test.ts
[src-53]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/auth/RpcAuthorization.ts
[src-54]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/scheduledTasks
[src-55]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/sourceControl/sourceControlDefinitions.ts
[src-56]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/pullRequestWatch.test.ts
[src-57]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/issue-tracker-linear
[src-58]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/secrets/SecretRequests.test.ts
[src-59]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/packages/secret-provider-onepassword
[src-60]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/policy/ManagedPolicy.test.ts
[src-61]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/terminal/Manager.test.ts
[src-62]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/preview
[src-63]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/device/DeviceService.test.ts
[src-64]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/htmlRender/HtmlRender.test.ts
[src-65]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcpApps/McpAppRequests.test.ts
[src-66]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/pnpm-workspace.yaml
[src-67]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/package.json
[src-68]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/vite.config.ts
[src-69]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/shared/src/nodeSqliteClient.ts
[src-70]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/claude-history-worker.ts
[src-71]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-pi/src/server/mcpExtensionSource.ts
[src-72]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/AcpMcpStdioBridge.ts
[src-73]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/model-manifest.json
[src-74]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/LICENSE
[src-75]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/third-party-licenses.config.json
