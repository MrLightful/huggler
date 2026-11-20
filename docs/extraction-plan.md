# Building and qualifying the Huggler crate

Start with a Rust consumer that resumes a Codex conversation after reopening its state. Then exercise the same flow in an installed Tauri app and qualify a second provider through a live approval and cross-provider collaboration. Use T3 Code as the behavioral reference across the broader capability inventory. The Rust crate can use its own API, schema and transports. Record whether each workflow is supported directly, through a documented fallback, or unavailable for the selected provider or host. [Crate proposal](sdk-proposal.md) · [Upstream architecture](upstream-orchestration.md) · [Capability inventory](parity-inventory.md)

## Stage handoff

**Stage 3 is in progress.** The installed macOS arm64 Tauri app displayed a live Codex CLI 0.170.0 permission request. The scripted UI selected **Deny and stop turn**. The requesting run reported `interrupted` and the file was absent. Reproduce with `python3 scripts/check-permission.py codex` using a signed-in Codex CLI. [Qualification script](../scripts/check-permission.py) · [Stage 2 results](stage-2.md)

The standalone Claude ACP adapter 0.109.0 and its bundled Claude Code 2.1.328 initialized and created a session over stdio. Claude reported signed out. The ACP fixture verifies approval, denial, rejection of invalid or duplicate decisions, and continuation after reopening. The final Rust suite passed with `cargo test --locked`. Live cross-provider collaboration remains open. [Permission test](../tests/permissions.rs) · [ACP fixture](../tests/fixtures/permission-acp.py)

Two live Codex CLI 0.170.0 threads discovered each other and exchanged a review through the loopback `huggler` MCP server. The reviewer issued two identical `thread_send` calls. Both returned the same message ID, and reopening showed one reply from the reviewer with its original run ID and receipt. Reproduce with `cargo test --locked --test peers live_codex_peer_exchange -- --ignored`. The offline ACP tests also cover revocation, cross-project denial and restart with queued deliveries. [Peer tests](../tests/peers.rs)

Two live Codex CLI 0.170.0 sessions used app-server stdio and Huggler's loopback MCP endpoint for asynchronous delegation in a shared checkout. After the parent went idle, the child's result started one parent run. The parent repeated the finding, and reopening preserved the result, run IDs and event cursor. Reproduce with `cargo test --locked --test delegation live_codex_delegation -- --ignored`. The offline ACP cases cover passive reads, owner acknowledgement, cancellation, pending delivery on reopen, siblings and a grandchild. [Delegation tests](../tests/delegation.rs)

Live Codex CLI 0.170.0 over app-server stdio and a scripted ACP reviewer over stdio passed both cross-adapter cases through loopback MCP. The peer retry retained one reply and its sender and run. The delegated result woke its parent once and survived reopening. Run `cargo test --locked --test peers --test delegation live_codex_to_acp -- --ignored` with a signed-in Codex CLI. The offline `waiting_for_a_task` test covers completion, timeout, pending approval and revocation during `delegate_task(mode="wait")`. [Peer tests](../tests/peers.rs) · [Delegation tests](../tests/delegation.rs)

With the scripted ACP peer, a follow-up queued before settlement supplied the task's final result and woke the parent once. Cancelling the owned subtree interrupted active descendants and queued work, including work started after the original task settled. A retry with the same cancellation key after reopening preserved later work. Run `cargo test --locked --test delegation` for both regressions. [Delegation tests](../tests/delegation.rs)

Codex app-server 0.156.1 replayed the pinned three-level subagent tree over stdio. The Rust snapshot and event stream preserve each child's native parent thread and turn, owning application run and final answer across reopen. Scripted variants complete the root first, then finish the child, interrupt its exact turn, or close the provider while the child is running. The root keeps its answer in all three cases. Run `cargo test --locked --test native_children`. [Rust tests](../tests/native_children.rs) · [Upstream recording and assertions](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/subagent_v2_nested)

The installed Tauri app replayed the Codex 0.156.1 grandchild command approval over stdio. Its prompt identified `/root/relay/writer`. Selecting **Allow** sent the recorded decision and both children returned `Written.`. The Rust tests verify the owning run and native turn, reject a response addressed to another run, and recover both results on reopen. A pending approval survives foreground completion. If a grandchild asks after the foreground completes and the provider then exits, its request expires and the foreground keeps its result. Reproduce with `python3 scripts/check-permission.py recorded-codex` on macOS and `cargo test --locked --test native_children`. ACP native-child controls still need qualification. [Desktop check](../scripts/check-permission.py) · [Upstream recording and assertions](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/subagent_v2_nested_approval)

A scripted ACP peer read and answered a foreground question from the Codex 0.156.1 plan-mode recording through the loopback `pending_request` tool. Concurrent replies produced one recorded answer, retries returned `already_answered`, and reopening preserved the answering run. A caller in another project was denied, and the ACP permission stayed pending until the host answered it. The Rust host also answered the recorded question with agent tools disabled. Scripted variants cover option previews, provider withdrawal and process loss. Reproduce offline with `cargo test --locked --test questions`. [Rust tests](../tests/questions.rs) · [Upstream recording](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/plan_questions)

After the original delegation settled, the scripted ACP v1 peer used stdio and loopback MCP to start one owner follow-up and queue another on the same child. Each send returned its own task ID and attributed the message to the sending run. The first prompt returned ACP `cancelled`, the second returned its finding, and retrying both sends after reopening preserved all three results. When both follow-ups delegated again, each grandchild’s result woke the follow-up that requested it. Reproduce with `cargo test --locked --test delegation`. The schema upgrade check is `cargo test --locked --lib v7_tasks_keep_their_runs`. [Delegation tests](../tests/delegation.rs) · [Upstream follow-up implementation](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.ts#L10053-L10156)

A scripted ACP v1 parent called `thread_interrupt` over loopback MCP with the child thread ID, running follow-up ID and a reason. Huggler closed the managed stdio process for that run and recorded the requesting thread, run and reason. The queued review started and returned its finding. Retrying while the second review was running returned the first run’s terminal status. A caller in another project received `forbidden`. All three task results survived reopening. This qualifies Huggler’s process-stop fallback. The test runs offline with `cargo test --locked --test delegation agent_interrupt_stops_only_the_addressed_followup`. [Delegation tests](../tests/delegation.rs) · [Upstream interrupt assertions](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpToolkit.integration.test.ts#L509-L597)

A scripted ACP v1 caller used `create_threads` over loopback MCP to create two independent peers in its project. Concurrent calls and a retry after reopening returned the same ordered IDs. One peer read a marker from the shared checkout, and the other stayed idle until a later call supplied its first prompt. The host retained the creating run, the prompt sender and zero delegated tasks. The storage tests cover rollback on a conflicting entry and recovery of a committed prompt before worker startup. Reproduce offline with `cargo test --locked batch_`. [Peer test](../tests/peers.rs) · [Recovery tests](../src/store/creation.rs) · [Upstream retry assertions](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.test.ts#L3356-L3434)

A scripted ACP v1 caller used `delegate_task` over loopback MCP to edit a tracked file in both checkout modes. The default shared edit reached the caller. With `isolation: "worktree"`, the child started at the caller’s committed HEAD in a detached Git worktree under the runtime state directory. After reopening, a retry returned the same task, child and run even when the parent’s HEAD was unresolved. The child resumed with its edit intact. Shared descendants and independent peers used that checkout, and a nested isolated child started at its caller’s new commit. The recovery test reopens the database on both sides of Git creation, before provider startup. Reproduce offline with `cargo test --locked worktree_`. [Delegation test](../tests/delegation.rs) · [Recovery test](../src/worktree.rs) · [Upstream checkout selection](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.ts#L2875-L2903)

**Next action.** Characterize native-child visibility and control on the standalone Claude ACP route. Inspect the installed adapter’s ACP v1 output mapping for nested tool calls and child identifiers. Use representative stdio frames to test what a Rust host can observe and which child it can address. Record the adapter version, supported operations and any live checks that still require authentication. The source and fixture checks can run offline. [Huggler ACP adapter](../src/providers/claude_agent.rs) · [Upstream Claude SDK background-child case](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/claude_background_subagent_after_root) · [Stage 3 acceptance](#3-prove-agent-to-agent-operation-with-two-harnesses)

**Branch recommendation.** Commit and continue Stage 3 on this branch. Recorded Codex cases and scripted ACP cases can proceed with the current setup. Live collaboration with a second authenticated provider remains open.

### Maintaining the handoff

- On `continue`, compare the handoff with the current branch, working tree and linked evidence. Resume the first unfinished acceptance check. If the stage already passes, update the handoff and use the stage table to select the next eligible stage.
- At a stopping point, replace the status, next action and branch recommendation above. Link the evidence another developer can reproduce from a checkout. For a partial stage, identify the outstanding check, its expected result and any blocker so the next session can resume it.
- When a stage passes its acceptance checks, publish a Conductor artifact showing the verified outcome and the next stage’s first action and observable success condition. Include the artifact in the final reply. Keep the same next action in this tracked handoff so a fresh session can find it. After the final stage, record extraction as complete and use the artifact to point to the release evidence.
- End every extraction handoff with a branch recommendation and its reason. Recommend **merge** when the stage is independently verified and ready to land. Recommend **commit and continue** on the current branch when work forms a coherent checkpoint but stage checks remain. If a blocker prevents either action, say what must happen first. A merge requires the user’s instruction.

## Choose a stage

| Stage | Ready when | Evidence to keep |
| --- | --- | --- |
| [1. Capture the reference](#1-capture-the-upstream-reference) | A checkout contains the pinned source | [Codex recording, expected projections and focused test results](reference/codex-resume/README.md). |
| [2. Run one durable thread](#2-build-the-rust-core-and-run-one-durable-thread) | The reference case is reproducible | [Packaged-crate replay, receipt races, recovery and installed Tauri results](stage-2.md). |
| [3. Connect two harnesses](#3-prove-agent-to-agent-operation-with-two-harnesses) | One driver works through the public API | A peer reply with sender identity and a delegated completion that wakes its parent. |
| [4. Qualify all drivers](#4-qualify-every-provider-instance-and-capability) | Two harnesses share a conformance suite | Results per driver, provider version, instance configuration and supported operation. |
| [5. Complete runtime behavior](#5-complete-control-history-and-recovery) | Stage 2 works, with qualified drivers available for provider-specific cases | Results for history, commands, requests, stream recovery, process loss and migration. |
| [6. Assemble the full profile](#6-assemble-the-full-application-profile) | Agent tools and runtime controls work | An authorized second application completes a workflow with the host integrations enabled. |
| [7. Qualify a release](#7-validate-cargo-and-tauri-releases) | Begin Cargo packaging and Tauri install checks in stage 2 | Build and upgrade results for the crate and installed app, plus a comparison to the reviewed upstream SHA. |

## 1. Capture the upstream reference

**Prerequisite.** A separate checkout at the reference commit. Use its pinned Node 26.13.1 and pnpm 12.14.0 for the reference build. Give it a temporary state directory, provider homes and Git repository. Read the upstream testing guide and run the focused suites from the checkout. [Repository toolchain][src-toolchain] · [Test commands][src-agents] · [Testing strategy][src-strategy]

```sh
vp test run apps/server/src/orchestration-v2/testkit/CodexReplayFixtures.integration.test.ts
vp test run apps/server/src/orchestration-v2/Orchestrator.receipt-race.test.ts
vp test run apps/server/src/mcp/OrchestratorMcpToolkit.integration.test.ts
vp test run apps/server/src/orchestration-v2/testkit/OrchestratorReplayRecovery.integration.test.ts
```

1. Use the staged commands and expected outcomes in `OrchestratorScenario`, the real adapters wired by `ProviderReplayHarness`, and the versioned transcripts in the fixture directories. Carry over deterministic clock, IDs and workspace path substitution. Keep both inbound frames and expected outbound calls. [Scenario runner][src-1] · [Adapter harness][src-2] · [Scenario catalog][src-3] · [Transport utilities][src-4]
2. For each workflow as it enters scope, record the public entry point, required caller or host, source test or missing case, and an observable assertion. Save reference receipts, ordered lifecycle transitions, final conversations, relations between IDs, cursors, provider calls and relevant file changes. Normalize temporary paths, times and random identifiers consistently while preserving ordering and equalities. Keep representative upstream databases as fixtures for supported imports.
3. Compare the architecture notes to current registration and executable tests. Record differences by SHA. Keep provider versions alongside recorded transcripts and remove credentials or personal content before storing new captures. The recording scripts and tool-use eval cases cover different questions. A replay verifies runtime behavior. An eval checks whether a model can choose the right tool from its instructions. [Architecture notes][src-5] · [Recorders][src-6] · [Tool-use evals][src-7]

**Acceptance.** A second developer can repeat the reference runs, match a selected receipt to its projection and provider transcript, and identify which rows have only source evidence. Use the same scenario intent on both sides. Adapt inputs and assertions when the Huggler API or provider transport differs.

## 2. Build the Rust core and run one durable thread

**Depends on stage 1.** Make this the first crate milestone. Use Codex app-server JSON-RPC and the upstream restart fixture to verify opening, continuing and reopening a session.

1. Define Rust domain types, including distinct IDs, commands, events and typed errors. Define serializable DTOs for Tauri and versioned remote contracts where needed. Verify import/export compatibility for formats Huggler explicitly supports. [Upstream domain and wire shapes][src-8] · [Agent contracts][src-9]
2. Implement Huggler-owned SQLite migrations, command receipts, event publication, projections and durable effects. One runtime owns the state directory, provider registry and subscriptions. Use the upstream composition and transaction tests to identify ordering and failure cases. [Service composition][src-10] · [Persistence reference][src-11] · [Transaction tests][src-12]
3. Open the runtime with `Huggler::open`. Recover durable state before accepting commands, and release workers, subscriptions and managed processes on `close`. Expose project open, root-checkout launch, send, read, wait and subscribe. Drive the Rust Codex adapter with the recorded transport. [Startup][src-13] · [Launch tests][src-14] · [Management][src-15]
4. Package the crate and run the scenario from an independent Cargo consumer. Register the same flow in a Tauri backend and run it in an installed build with Codex available and Node absent at runtime. Start from a different working directory and quit and reopen the app. Exercise a second thread to check that state and event streams are scoped correctly. [First milestone](first-milestone.md)

**Acceptance.** The consumer observes ordered messages, the expected terminal state and a stable Huggler thread ID. The provider resumes its original session and the second prompt gets a new application run. Retrying a committed command returns its original receipt. Two different threads racing with one command ID produce one success and one typed conflict. Shutdown lets the process exit and the next instance recover the same state directory. [Race test][src-16] · [Restart scenario][src-17]

## 3. Prove agent-to-agent operation with two harnesses

**Depends on stage 2.** Add a second independently configured provider. Evaluate Claude through its standalone ACP adapter or Cursor through its CLI ACP endpoint. Drive a live provider permission prompt through Tauri first. Stream the request to the UI, respond through the Rust host and check that the same run resumes or denies the action as expected. Record the provider version, supported controls and required executables. Implement scoped agent tools on the `huggler` MCP server, including caller resolution, provenance, revocation, access checks and discovery. Run the collaboration cases through the provider-facing endpoint and check the results through the crate. [Crate conventions](sdk-proposal.md#naming) · [MCP service][src-18] · [Toolkit tests][src-19] · [Access races][src-20]

| Case | Arrange and act | Required observation |
| --- | --- | --- |
| Independent peers | Launch A on Codex and B on the second provider. A uses `thread_send` to request a review. Let A's first turn finish. B reads A's address from the received origin and sends back its findings. Repeat B's send with the same request key. | B receives the request with A as sender. A receives one reply with B as sender and can start a later run. Each send has a receipt and actual delivery mode. Reopening either thread preserves the exchange. |
| Owned delegation | A delegates an async review to B's provider instance. Let the parent go idle. Finish the child, observe it from a dashboard and allow completion delivery to the parent. Repeat with an explicit parent acknowledgement, a cancelled child and a grandchild. | Each task retains its parent, child address and result. A passive read leaves delivery intact. An eligible completion wakes the idle parent with the result once. An acknowledgement or cancellation has the upstream effect on later delivery. |
| Native child | Replay a harness-native subagent, including one that outlives the root turn. Inspect the execution tree and, where supported, interrupt the child. | The tree shows the native ancestry and correct owning run. The crate exposes the controls the adapter supports. Native child addressability follows the provider capability. |
| Caller constraints | Issue two caller-bound credentials, exercise an allowed send, then revoke one. Try an owned-task read as a different parent, a mode escalation, a user question and a provider approval. | The live caller owns the recorded action. Forbidden calls return the expected error and cause no write. A user question and a human permission decision reach their respective responders. |

Add local batch thread creation, shared checkout versus isolated worktree, queued child follow-up, status/timeout, concurrent completions and stop propagation to these cases. Run the upstream `review-round-uses-thread-send` eval when testing agent instructions. That case expects **another delegation** when the user requested a second delegated review round. A separate positive case should explicitly request an independent peer reply. [Completion tests][src-21] · [Native and delegated cancellation][src-22] · [Review eval inputs][src-23]

**Acceptance.** Capture the live approval, actual MCP requests and host-visible thread/task results. Characterize native-child behavior per provider. A two-agent demo can display who asked whom, which run produced the answer, who can stop the work and which request requires a human.

## 4. Qualify every provider instance and capability

**Depends on stage 3 for the common conformance suite.** Build the driver registry and qualify discovery, credentials, models, install/version probes, maintenance and usage reporting. Use upstream to identify behavior and integration gaps. For each driver keep a configuration boundary and a scoped session adapter. Verify continuation identity when using multiple instances of the same kind. Preserve restrictions on providers sharing a machine login or external server. [Registration and reader order][src-24] · [Driver contract][src-25] · [Session contract][src-26] · [Host implementation][src-27]

1. Use Codex JSON-RPC, ACP for compatible harnesses, OpenCode’s server API and Pi’s process RPC as starting points. Evaluate Muse’s MSP transport and provider tooling separately. Qualify what each transport exposes before considering an optional bridge for missing functions. Keep any bridge and its dependencies outside the base crate and installer. [Rust provider routes](upstream-orchestration.md#candidate-rust-provider-transports)
2. For each selected version and operating mode, test discovery, login state, session start, prompt, streamed output, settlement, interrupt, restart/resume, requests and MCP round-trip. Verify advertised fork, rollback, steering, model changes, compaction and native child operations. Exercise available history-based alternatives and record differences in the observable result.
3. Reuse raw upstream frames when the transport and version match. For other routes, capture new frames and keep equivalent end-to-end assertions. Qualify ACP flavors, managed and external OpenCode, Pi tool integration, background work and usage aggregation separately. [Replay registry][src-28] · [Antigravity adapter tests][src-29] · [Antigravity protocol tests][src-30] · [Usage aggregation tests][src-31]

**Acceptance.** Record driver, provider version, transport, OS, architecture and operating mode for each result. Track replay evidence, live smoke results, native features, portable fallbacks and unavailable operations. A missing executable, login or incompatible version produces actionable instance status while other configured instances stay usable.

## 5. Complete control, history and recovery

**Depends on stage 2.** Use the qualified adapters from stages 3–4 for provider-specific paths. Implement the remaining workflows in the capability inventory and test them through the crate or host transport. [Capability inventory](parity-inventory.md)

| Area | Tasks and observable acceptance |
| --- | --- |
| Queue, dispatch and stop | Exercise auto/queue/steer/restart delivery, editing and reordering queued work, fresh context, prepared-run failure/retry, interrupt, force stop, background waits, usage-limit reset and automatic continuation. Stop a thread with queued work and a live child. Confirm the queue stays held until resumed and stale completions cannot start the wrong run. [Queue finalization][src-32] · [Stop][src-33] · [Limit recovery][src-34] |
| Human input and attachments | Route approval/denial, questions, per-question attachments and dismissal to the owning session/node. Lose a live-only callback during restart and verify its projected outcome and retry behavior. Exercise upload preparation, claim, cleanup and duplicate send. [Request tests][src-35] · [Claims][src-36] |
| Model, context and files | Test live selection changes and changes that restart a session. Run native fork, prior-turn fork, portable fork, merge-back and cross-provider handoff. Check the stable prefix, attribution, history budget and the first outbound prompt after each transfer. Use temporary repositories for rollback with and without file restore, dirty state and shared-worktree ownership. [Switch][src-37] · [Fork][src-38] · [Merge-back][src-39] · [Restore safety][src-40] |
| Reads and transport | Cover the listed read, query and stream workflows using Rust APIs and Tauri IPC. For a remote host, expose equivalent authorized operations over a versioned transport. Exercise shell vs detail, bounded history paging, search, diffs, workflow/native child reads, command output and prompt suggestions. Disconnect, replay from a retained cursor, and reconnect with an expired cursor. Compare the rebuilt client state with a fresh authoritative snapshot. Run the upstream stream budgets against long histories and large output. [Stream budget][src-41] · [Bounded snapshots][src-42] · [Transport performance][src-43] |
| Crash and process loss | Terminate the runtime before commit, after commit/before claiming an effect, and after external I/O/before recording completion. Reopen the same file-backed database. Verify committed intent, outbox disposition by kind, state reconciliation, outstanding delegated completions and how uncertainty is reported. No second outbound prompt or approval may result from blindly replaying a process-bound row. Retried cleanup tolerates an already-removed resource. [Outbox recovery][src-44] · [Runtime recovery regressions][src-45] |
| Existing data | Migrate Huggler state from its previous releases. For supported T3 Code and provider imports, verify history and first continuation on a copy. Include Claude, Codex and Pi transcripts and the ACP Registry native import workflow in the inventory. Repeat, change or remove a source, and inspect history, provenance and continuation eligibility each time. [V1 cutover test][src-46] · [External import][src-47] · [Registry sessions][src-48] |

**Acceptance.** Each workflow has coverage for its observable outcome, fallback or current limitation. Older Huggler state and supported import formats still decode. Retention, coalescing and pagination produce a consistent client view across a disconnect. Scope denial and unsupported capabilities produce distinguishable errors.

## 6. Assemble the full application profile

**Depends on stages 3 and 5.** Keep integrations opt-in for small hosts. Implement each selected capability with its host, lifecycle and cleanup. Record prerequisites and test successful and unavailable or denied cases. A host or external service may supply some integrations.

| Integration | Acceptance scenario |
| --- | --- |
| Projects and workspaces | Launch in the root, an existing worktree and a newly allocated worktree. Fail a setup script, inspect preparation status, retry and release the run. Exercise clone, scratch promotion and cleanup with a sibling still using the checkout. [Launch][src-14] · [Setup][src-49] |
| Linked environments | Start two runtimes with independent state. Link with approval, send and delegate remotely, disconnect the follower, reconnect and complete the task. Preserve the remote address, sender and local parent result. Revoking a grant blocks subsequent writes. Reject unsupported remote worktree isolation. [Remote delegation][src-50] · [Durable remote task][src-51] |
| Guests and authenticated RPC | Redeem a share, inspect the guest view, submit a message, accept/dismiss it from the host and close the share. Verify another thread remains inaccessible. Exercise read-only and operating RPC identities on the same command. Qualify a relay-backed route separately if offered. [Guest service tests][src-52] · [RPC authorization][src-53] |
| Schedules and webhooks | With a controlled clock, exercise interval, fixed-time and one-shot schedules around a timezone boundary. Verify new-thread/continue/fresh-session behavior. Deliver a signed webhook twice, then an invalid signature or filtered event. Compare run history, deduplication and cleanup on thread deletion. [Scheduling and webhook tests][src-54] |
| SCM and issue trackers | Use the seven upstream SCM drivers to identify coverage for project bindings and repository workflows. Run per-driver supported operations against recorded service responses. Verify link/unlink, lists, CI/check reads, mutations and watch-triggered wakes. Repeat a stale sync and inspect its effect. Include the Linear integration and unavailable credentials. [SCM definitions][src-55] · [PR watch][src-56] · [Linear driver][src-57] |
| Secrets and policy | Request a secret, approve/deny through the host and inspect provider delivery, expiry and persisted/audited fields. Exercise configured secret references, including 1Password, and managed policy limits on a live agent. [Secret requests][src-58] · [1Password][src-59] · [Managed policy][src-60] |
| Terminal, preview, devices, HTML and MCP Apps | Stream terminal output through disconnect/close, open and close a thread-owned preview, navigate/capture it, select/open/capture/close a device, and render HTML under the host policy. Service an MCP App tool/resource and carry allowed model context to the next turn. Test cleanup and loss of host availability for each service. [Terminal manager][src-61] · [Browser hosts][src-62] · [Device tests][src-63] · [HTML renderer][src-64] · [MCP App requests][src-65] |

**Acceptance.** Account for each upstream tool by mapping its workflow to a Huggler tool or host API, or recording a limitation. Verify representative identities, scope and side effects. Exercise deferred discovery, user messaging, environment preferences and theme. Drive a complete workflow through a second host when qualifying remote operation.

## 7. Validate Cargo and Tauri releases

1. Run `cargo package` and inspect the crate contents, dependency resolution, embedded migrations, fixtures and license files. Build an independent consumer from the packaged source. Exercise the core with only Codex available and test optional integrations with their required dependencies. Set the Rust version requirement from verified builds.
2. Build and install Tauri artifacts for the desktop targets claimed. On a clean machine with a supported provider available, run the restart and approval flows without Node.js on PATH or bundled for Huggler. Verify app-data paths, paths containing spaces, shutdown, provider discoverability and missing-provider handling. Inspect optional bridge artifacts separately and record installed and download sizes. [Tauri bundling](https://v2.tauri.app/distribute/)
3. Preserve upstream notices for reused code or fixtures. Generate third-party notices from the actual distribution and verify redistribution terms for bundled binaries. Document externally installed providers, versions and account requirements. [Upstream MIT license][src-74] · [Upstream notice configuration][src-75]
4. Version the Rust API, persisted schema and any published IPC or RPC contracts. Upgrade an installed Tauri build over state created by the previous release, then verify history and continuation. Repeat affected scenarios after upstream updates. [Upstream watch workflow](upstream-watch-guide.md)

**Release evidence.** Retain the packaged crate contents, installer platform and size results, attribution and a passing scenario or explicit limitation for each inventory group. Track provider smoke results separately from replay results.

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
[src-74]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/LICENSE
[src-75]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/third-party-licenses.config.json
