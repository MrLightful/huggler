# T3 Code capability inventory for Huggler

Use this inventory to identify the observable outcome and upstream regression for a workflow. The names, counts and wire shapes describe T3 Code at `338352dee2f43079482e054e8229a2ba055641d9`. The review cursor tracks later changes independently. [Upstream review record](upstream-review.json)

Check Huggler’s public interface and tests for current support. Record provider and host prerequisites, a passing scenario, and any fallback or limitation. [Verification guide](verification.md) · [0.0.1 coverage](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-7.md#capability-coverage-at-release)

| Surface | Baseline | Implementation to compare |
| --- | ---: | --- |
| Thread orchestration commands | 58 wire variants + 15 additional internal variants | [Command and event unions][src-1] · [Command policy][src-2] |
| Orchestration RPC and HTTP | 23 named RPC operations + 5 HTTP GETs | [RPC contracts][src-3] · [HTTP contracts][src-4] |
| Thread orchestration events | 48 domain event names | [Wire and storage shapes][src-1] · [Event sink][src-5] |
| Durable effects | 17 kinds with two recovery policies | [Outbox][src-6] |
| T3 MCP declarations | 92 tools across 15 toolkit directories | [Toolkits and access-checked handlers][src-7] |
| Harnesses | 11 built-in driver kinds, 8 aggregate usage readers | [Driver source map](upstream-watch-guide.md#locate-a-harness-change) · [Registration][src-8] |

## Commands

A host should normally use launch, send, stop and the other management operations. Low-level dispatch accepts a broader schema, including messages normally produced by server workers. Use the authenticated intake, scope checks, provenance normalization and existing command policy for any remote dispatch API. A thread agent gets the narrower MCP interface below. [Management service][src-9] · [Message intake][src-10] · [RPC authorization][src-11]

| Responsibility | Wire command discriminants | Behavior to preserve |
| --- | --- | --- |
| Thread lifecycle | `thread.create`, `thread.archive`, `thread.unarchive`, `thread.delete` | Create a durable address, move to/from archive, delete and clean up dependent resources. Cover conditional deletion and living forks. |
| Settlement and visibility | `thread.settle`, `thread.auto-settle`, `thread.unsettle`, `thread.snooze`, `thread.unsnooze`, `thread.auto-settle.set` | Manual/automatic settlement, policy toggle and timed snoozes affect the shell independently of run status. |
| Organization and read state | `thread.pin`, `thread.unpin`, `thread.tag.set`, `thread.pin.reorder`, `thread.active.reorder`, `thread.visit`, `thread.mark-unread`, `thread.metadata.update`, `thread.title.regeneration.complete` | Pins, tags, ordering, visit/unread watermarks, metadata and title regeneration. Ignore a stale title-generation completion. |
| Guests | `thread.guest-share.close`, `thread.guest-message.dismiss` | Close sharing or dismiss a pending guest message. Acceptance through message dispatch uses the stored guest text. |
| Repository links and watches | `thread.pull-request.link`, `thread.pull-request.unlink`, `thread.pull-request-link.sync`, `thread.issue.link`, `thread.issue.unlink`, `thread.pull-request.watch`, `thread.pull-request.sync` | Explicit and branch-derived PR links, issue links, watch enrollment and reconciled snapshots. Carry expected state to handle stale sync results. |
| Run configuration | `thread.runtime-mode.set`, `thread.interaction-mode.set`, `thread.model-selection.set`, `provider-session.detach`, `provider.switch` | Change execution/interaction policy or model and options, detach a specific native session, or switch providers with continuity policy. |
| Message intake and preparation | `message.dispatch`, `prepared-run.release`, `notification.delivery.accept`, `prepared-run.progress`, `prepared-run.fail`, `prepared-run.retry` | Accept a message or notification and drive a prepared run through worktree/setup to release, failure or retry. |
| Execution and queue controls | `thread.stop`, `subagent.stop`, `run.interrupt`, `queued-message.promote-to-steer`, `queue.resume`, `queued-run.start`, `queued-run.reorder`, `queued-run.cancel`, `queued-run.edit` | Stop a whole thread, stop a native child, interrupt a run, start/edit/reorder/cancel queued work or promote a queued message to steering. |
| Human requests | `runtime-request.respond`, `thread.user-input.dismiss` | Respond with a decision, feedback, answers or per-question attachments as supported. Dismiss an outstanding question. |
| History and context | `checkpoint.rollback`, `thread.fork`, `thread.merge_back` | Restore a checkpoint, fork a stable prefix or merge context into a related thread. File restore is separately requested. |
| Parent/child coordination | `delegated_task.request`, `delegated_task.wake-policy`, `delegated_task.completion-delivery.acknowledge`, `delegated_task.completion-delivery.dispose`, `thread.created.record` | Create a delegated child, set completion wake policy, acknowledge/dispose delivery or record the creation of an independent thread. Preserve the parent run and node. |

**Project lifecycle** crosses a different boundary. `projects.mutate` accepts `project.create`, `project.update` and `project.delete`. The service maps metadata updates to `project.meta.update`. Its events are `project.created`, `project.meta-updated` and `project.deleted`. Project events contribute to the application/shell stream. Include project defaults, root resolution and project mutation receipts in the extraction. [Project contract][src-12] · [Project service][src-13] · [Project commands][src-14]

### Variants that need separate scenarios

| Input or state | Cases that a single happy-path test would miss |
| --- | --- |
| Launch and dispatch | Workspace strategies `root`, `existing_worktree`, `worktree`. Empty launch and atomic thread-plus-first-run launch. Dispatch modes `defer_start`, `steer_active`, `restart_active`, `queue_after_active`, `start_immediately`. A high-level send takes `auto`, `queue`, `steer` or `restart` and returns the actual delivery mode and run ID. |
| Queue and continuation | Promote against a specific active run, edit just text or replace attachments, explicitly resume a held queue, handle manual/restart/usage-limit continuation and preserve fresh-context intent. Reordering before a null target moves to the end. |
| Commands in message text | The parser recognizes `/goal` show/set/clear/pause/resume subject to provider support and prompt shape. Preserve explicit `/compact`, automatic idle compaction and dispatch while compaction is active. An attachment can change whether text is a control message. |
| Human input | Request response capability is `live`, `message` or `not_resumable`. Cover approval, denial, feedback, question answers, attachments, dismissal, expiry and a callback lost on restart. A pending secret or environment link uses its dedicated human response flow. |
| Context and workspace | Fork/merge source points are `latest_stable`, `run` and `checkpoint`. Transfer kinds include fork, provider handoff, merge-back and subagent spawn/result. Persist `pending`, `resolved_native`, `resolved_portable`, `failed`, `consumed` and `superseded`. Test later delivery, supersession and replay. Independently exercise checkpoint rollback with/without file restoration. |
| Delegation | Match the parent thread, run and node. Wake policy is `always` or `settled_only`. Delivery states are `pending`, `claimed`, `acknowledged`, `delivered`, `disposed`. Check interruptions, queued child work, grandchildren, a later follow-up result and simultaneous completions. |

The authoritative payload fields, defaults and result types are in the contracts. Huggler can use its own Rust types and host payloads while preserving these distinct outcomes. [V2 contracts][src-1] · [Agent result contracts][src-15] · [Dispatch policy][src-2] · [Request delivery][src-16]

### Additional server-only commands

These 15 variants are admitted by the internal command union. A host integration invokes the relevant service, which checks its authority and state before producing the command. Preserve this authority boundary in the crate API and its transports.

| Issuer | Internal discriminants | Purpose |
| --- | --- | --- |
| Guest and project services | `thread.guest-share.open`, `thread.guest-share.redeem`, `thread.guest-message.submit`, `thread.project.adopt-folder` | Issue/redeem a share, submit a guest prompt, or adopt a folder after service checks. |
| PR watch reactor | `thread.pull-request-watch.checks-reopen`, `thread.pull-request-watch.sync` | Reopen checks or apply watch synchronization. |
| Linked environment follower | `delegated_task.remote.request`, `delegated_task.remote.complete`, `delegated_task.remote.stop` | Record remote request, terminal result and stop against the durable local task. |
| Failure and reconciliation | `checkpoint.rollback.fail`, `runtime-request.delivery.fail`, `thread.interrupt.unresponsive`, `thread.background-work.settle` | Record failed rollback/answer delivery, mark an unresponsive interrupt and settle background work. |
| Human-mediated integrations | `secret_request.record`, `link_request.record` | Persist a secret request or a link request so the authorized human can respond. |

## Read and control transports

The four control operations in the first row require orchestration-operate authority. The remaining entries use orchestration-read as their base scope. Payload checks can require an additional scope. Preserve payload codecs, errors, access checks, paging, stream cleanup and observably different protocols across all 23 operations. [Method names and payloads][src-1] · [RPC enforcement][src-11]

| Surface | RPC methods | Behavior |
| --- | --- | --- |
| Act | `orchestration.dispatchCommand`, `orchestration.stopWorkflow`, `orchestration.forceStopThread`, `orchestration.launchThread` | Operate scope. Dispatch a validated command, stop a workflow, force-stop a thread or launch with project/workspace preparation. |
| Workspace diffs | `orchestration.getTurnDiff`, `orchestration.getFullThreadDiff` | Read diffs for a turn or the whole thread. |
| Search | `orchestration.searchThreads`, `orchestration.searchThread`, `orchestration.searchThreadStream` | Search across threads, search within one thread, or consume the streaming search result. |
| Shell and relationships | `orchestration.getArchivedShellSnapshot`, `orchestration.getThreadFamily`, `orchestration.getThreadTitles`, `orchestration.getThreadLinkMetadata`, `orchestration.getWorktreeThreadIds`, `orchestration.getOlderSettledThreads` | Get archived/older settled rows, family, titles, link metadata and threads sharing a worktree. |
| Detailed inspection | `orchestration.getThreadProjection`, `orchestration.getWorkflowScript`, `orchestration.getNativeChildWork`, `orchestration.getTurnItem` | Read a projection or an individual turn item, inspect workflow script and provider-native child work. |
| Subscriptions | `orchestration.subscribeShell`, `orchestration.subscribeThread`, `orchestration.subscribeCommandOutput`, `orchestration.subscribePromptSuggestion` | Shell and conversation changes, live command output and prompt suggestions have different payloads and recovery semantics. |

| HTTP GET | Consumer contract |
| --- | --- |
| `/api/orchestration/shell` | Active project/thread snapshot with omission options and a resumability check when the client supplies a cursor. |
| `/api/orchestration/threads/:threadId` | Full wire projection and snapshot sequence. |
| `/api/orchestration/threads/:threadId/transcript` | Conversation transcript with explicit size/error handling. |
| `/api/orchestration/threads/:threadId/bounded` | Bounded initial thread window and paging metadata for a long conversation. |
| `/api/orchestration/threads/:threadId/history` | Earlier history using a server-issued cursor. Test invalid cursors, inherited history and page boundaries. |

All five reads require orchestration-read scope. The live shell, detailed thread stream and history pages must agree on what a client can safely resume or prepend. [HTTP routing][src-4] · [Handlers][src-17] · [Bounded transport tests][src-18]

**Adjacent routes needed by a T3-like application.** Projects, filesystem, assets/attachments, external agent sessions, VCS/worktree setup, Git/review, source control/PRs/issues, settings, provider authentication/install, ACP Registry management, provider catalogs/usage, peer links, guest sharing, secrets, schedules/webhook history, MCP Apps, terminal, preview and devices live outside this 23-method map. For example, `agentSessions.scan` and `agentSessions.import` ingest local transcripts, `server.importAcpRegistrySession` handles a registry session and `secrets.answerRequest` completes a secret request. Use the main RPC map and its per-call permissions when implementing the full host profile. [Main RPC map][src-3] · [Per-call scopes][src-11]

## Durable facts and effects

These 48 distinct event names belong to the thread domain union. A stored event, the in-memory domain value and a compact wire delta have different encodings. Persisted application sequences also cover project activity. Consumers should use the stream frames and cursors defined for their view. [Domain and JSON schemas][src-1] · [Wire projection][src-19] · [Application event envelope][src-20]

| Projection | Event types | Read-model responsibility |
| --- | --- | --- |
| Thread and visibility | `thread.created`, `thread.archived`, `thread.unarchived`, `thread.deleted`, `thread.settled`, `thread.unsettled`, `thread.snoozed`, `thread.unsnoozed`, `thread.pinned`, `thread.auto-settle-set`, `thread.unpinned` | Create, retire, archive, pin, snooze, settle or change automatic settlement. |
| Ordering, metadata, read state | `thread.visit-recorded`, `thread.pin-reordered`, `thread.active-reordered`, `thread.tags-updated`, `thread.visited`, `thread.marked-unread`, `thread.metadata-updated` | Project sidebar order, tags, metadata, visits and unread state. Keep both visit event shapes decodable. |
| External links and configuration | `thread.pull-request-link-updated`, `thread.pull-request-synced`, `thread.issue-synced`, `thread.runtime-mode-updated`, `thread.interaction-mode-updated`, `thread.model-selection-updated`, `thread.provider-switched`, `thread.guest-share-updated` | PR link updates, PR/issue sync, mode/model/provider changes and guest-sharing state. |
| Work graph | `run.created`, `run.updated`, `run.background-work-cancelled`, `run-attempt.created`, `run-attempt.updated`, `node.updated`, `subagent.updated` | Track run, attempt, node and child lifecycle, including background work cancellation. |
| Provider binding | `provider-session.detached`, `provider-thread.updated`, `provider-turn.updated`, `provider-session.attached`, `provider-session.updated` | Attach/update/detach sessions and correlate native thread/turn refs. |
| Conversation and decisions | `runtime-request.updated`, `message.updated`, `turn-item.updated`, `plan.updated` | Update requests, messages, activity items and plans. Display and answer pending decisions by request ID. |
| Checkpoints and transfers | `checkpoint-scope.created`, `checkpoint.captured`, `checkpoint.rollback-requested`, `context-handoff.updated`, `context-transfer.created`, `context-transfer.updated` | Persist checkpoint scope, capture and restore intent, portable handoff and transfer progress. |

| Process-loss policy | Effect kinds | Required recovery behavior |
| --- | --- | --- |
| Replay-safe (10) | `provider-runtime.continue`, `provider-session.detach`, `provider-thread.rollback`, `checkpoint.capture`, `scheduled-tasks.cleanup`, `terminal.cleanup`, `preview.cleanup`, `attachment.cleanup`, `thread-title.generate`, `delegated-tasks.stop` | Return running rows to pending. Complete, retry or reconcile by durable identity, including cleanup of resources already removed. |
| Process-bound (7) | `provider-session.apply-model-selection`, `provider-session.apply-thread-title`, `provider-turn.start`, `provider-turn.interrupt`, `provider-turn.steer`, `provider-turn.restart`, `runtime-request.respond` | Cancel pending and running rows tied to the lost process. Reconcile actual run, session and request state before subsequent work. |

Make the distinction visible in failure tests. A durable command receipt proves T3 accepted the command. The provider may have received it moments before a crash. Reopening the database and blindly resending every prompt or permission answer could perform an external action twice. [Effect classification and recovery SQL][src-6] · [Effect worker][src-21] · [Provider runtime reconciliation][src-22]

## Agent-facing tools

The table records exact upstream names for source lookup and regression comparison. Huggler’s corresponding built-in tools omit the `t3_` prefix, as in `thread_send`. Compare the observable result, authorization and side effects. The historical mapping records which workflows had agent tools, host APIs or coverage gaps at initial qualification. [Built-in Huggler tools](../docs-app/content/docs/reference/agent-tools.mdx) · [Historical mapping](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-6.md#upstream-tool-coverage)

MCP is the agent’s interface to orchestration. For deferred discovery, the direct set is `orchestrator_capabilities`, `delegate_task`, `task_status` and `tool_search`, subject to caller eligibility. Searching can expose additional matching tools within the session. Tool visibility, write access, link permissions, target mode limits, handler capabilities and available host services all participate in the effective surface. An outside read-only client, a live author thread and a linked agent can receive different results. [Exposure][src-23] · [Invocation context][src-24] · [Access declarations][src-25]

| Toolkit (declarations) | Exact names in T3 Code | Purpose and eligibility |
| --- | --- | --- |
| [orchestrator][src-26] (20) | `orchestrator_capabilities`, `delegate_task`, `task_status`, `task_cancel`, `t3_workflow_stop`, `t3_background_wait`, `schedule_task`, `list_scheduled_tasks`, `delete_scheduled_task`, `request_secret`, `create_threads`, `t3_thread_list`, `t3_thread_read`, `t3_thread_update`, `t3_thread_send`, `t3_thread_wait`, `t3_thread_interrupt`, `t3_environment_links`, `t3_environment_link`, `t3_environment_unlink` | Capability/catalog lookup, owned delegation, status/cancel, workflow control, background waits, schedules, secrets, independent threads and linked environments. Delegation, task status/cancel, secret requests and batch thread creation require a live thread caller. Targeted writes and environment changes have their own checks. |
| [thread][src-27] (7) | `t3_thread_organize`, `t3_queue`, `t3_pending_request`, `t3_thread_configure`, `t3_thread_transfer`, `t3_thread_search`, `run_scheduled_task_now` | Organize, inspect/change the queue, answer user questions, inspect/configure a model, fork/merge, search and trigger a schedule. Check access for each action of a multi-action tool. Model configuration preserves permission modes. |
| [project][src-28] (7) | `t3_project_list`, `t3_project_read`, `t3_project_create`, `t3_project_update`, `t3_project_delete`, `t3_project_clone`, `t3_thread_launch` | List/read projects, manage roots and clones, launch a thread. Project writes need environment authority. Launch enforces the caller’s mode ceiling and linked-origin policy. |
| [worktree][src-29] (3) | `t3_worktree_handoff`, `t3_worktree_status`, `t3_worktree_list` | Handoff and status require the calling thread’s identity. Listing accepts a target thread and returns branch refs and checkout paths. |
| [environment][src-30] (2) | `t3_environment_read`, `t3_environment_preferences_update` | Read environment data and change supported preferences. Updates require environment-write authority. |
| [attachment][src-31] (3) | `t3_attachment_prepare_upload`, `t3_attachment_discard`, `t3_thread_send_attachments` | Prepare/discard uploads and send a message with attachment claims. Check claim ownership and the target thread when sending. |
| [message][src-32] (1) | `send_to_user` | Deliver an interim message to the user from a live calling thread. |
| [issues][src-33] (8) | `list_issues`, `read_issue`, `create_issue`, `comment_on_issue`, `set_issue_state`, `link_issue`, `unlink_issue`, `list_thread_issues` | Look up or mutate issues through the project’s configured tracker and link them to threads. External writes require the corresponding capability and credentials. |
| [pullRequests][src-34] (18) | `link_pull_request`, `link_pull_requests`, `unlink_pull_request`, `unlink_pull_requests`, `list_thread_pull_requests`, `watch_pull_request`, `unwatch_pull_request`, `write_pull_request_guides`, `read_pull_request_guide`, `pull_request_read`, `pull_request_list`, `pull_request_create`, `pull_request_update`, `pull_request_respond`, `pull_request_review`, `pull_request_merge`, `ci_logs`, `ci_rerun` | Manage links and watches, review guides, PR data, comments/reviews, merging and CI. Enforce thread-target and repository authorization appropriate to the operation. |
| [preview][src-35] (12) | `preview_status`, `preview_open`, `preview_dialog`, `preview_navigate`, `preview_resize`, `preview_set_appearance`, `preview_snapshot`, `preview_act`, `preview_evaluate`, `preview_wait_for`, `preview_recording_start`, `preview_recording_stop` | Inspect and operate the caller’s preview, navigate, capture or record through the browser host. Requires thread context and a usable preview host. |
| [previewControls][src-36] (2) | `t3_preview_list`, `t3_preview_close` | List and close previews in the caller’s context. |
| [device][src-37] (4) | `device_list`, `device_open`, `device_screenshot`, `device_close` | List, open, capture and close through the device integration. Uses caller-bound reads/actions and device-host support. |
| [html][src-38] (2) | `html_preview`, `html_render` | Check rendering and display an inline page through the calling thread’s host. Preview and render can be exposed separately. |
| [theme][src-39] (2) | `t3_theme_read`, `t3_theme_set` | Read theme information or change an environment theme with the appropriate write authority. |
| [toolSearch][src-40] (1) | `tool_search` | Discover deferred tools for this provider session. Listing a tool preserves its existing invocation checks. |

**Three acceptance checks for Huggler callers.** A dashboard can inspect thread/child projections without acknowledging delivery. A live parent that calls `task_status` on its completed task can acknowledge its result. An agent can answer an eligible user question through `pending_request`. A human decision on provider permissions goes through an authorized responder. Exercise positive and denied cases through the real handlers. [Orchestrator handlers][src-41] · [Thread handlers][src-42] · [Access race tests][src-43]

[src-1]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/orchestrationV2.ts
[src-2]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/CommandPolicy.ts
[src-3]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/rpc.ts
[src-4]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/environmentHttp.ts
[src-5]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/EventSink.ts
[src-6]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/EffectOutbox.ts
[src-7]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits
[src-8]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/builtInDrivers.ts
[src-9]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ThreadManagementService.ts
[src-10]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ThreadMessageIntake.ts
[src-11]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/auth/RpcAuthorization.ts
[src-12]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/project.ts
[src-13]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/project/ProjectService.ts
[src-14]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ProjectCommands.ts
[src-15]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/orchestratorMcp.ts
[src-16]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/RuntimeRequestService.ts
[src-17]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/http.ts
[src-18]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/boundedSnapshotTransport.test.ts
[src-19]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/WireProjection.ts
[src-20]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/contracts/src/applicationEvent.ts
[src-21]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/EffectWorker.ts
[src-22]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/ProviderRuntimeRecoveryService.ts
[src-23]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/McpToolExposure.ts
[src-24]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/McpInvocationContext.ts
[src-25]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/McpToolAccess.ts
[src-26]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/orchestrator
[src-27]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/thread
[src-28]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/project
[src-29]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/worktree
[src-30]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/environment
[src-31]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/attachment
[src-32]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/message
[src-33]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/issues
[src-34]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/pullRequests
[src-35]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/preview
[src-36]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/previewControls
[src-37]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/device
[src-38]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/html
[src-39]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/theme
[src-40]: https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/toolSearch
[src-41]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/orchestrator/handlers.ts
[src-42]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/toolkits/thread/handlers.ts
[src-43]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/McpToolAccess.race.test.ts
