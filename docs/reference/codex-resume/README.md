# Stage 1 reference · Resume one Codex conversation after restart

T3 Code at `338352dee2f43079482e054e8229a2ba055641d9` accepted two prompts on one application thread. Between prompts the test closed the orchestrator and opened a second runtime on the same SQLite file. Codex resumed the original native thread, completed a new turn and repeated its first answer in the second reply. This is the upstream comparison case for Huggler's first packaged consumer. [Pinned recovery test][recovery] · [Rust milestone](../../first-milestone.md)

## Reproduce the result

From the Huggler root, supply a local T3 Code checkout containing the pin and a **new** output directory whose parent exists. The runner uses Git, npm, a POSIX shell and the SQLite CLI with JSON support. Allow several GB for a disposable checkout and build dependencies, and registry access for installation.

```sh
scratch=$(mktemp -d)
scripts/replay-reference.sh /path/to/t3code "$scratch/stage1-run"
```

The runner clones and detaches at the pin, compares the stored transcript with upstream byte for byte, installs Node 26.13.1 and pnpm 12.14.0 locally, then installs the locked server dependencies. It runs all four suites from `apps/server` using Vite+ 1.4.0, a fresh test home and the environment variables listed in the script. The restart test creates a temporary Git repository with a committed `README.md`. The recording supplies both directions of the provider exchange. [Runner](../../../scripts/replay-reference.sh) · [Workspace fixture][workspace]

After the baseline passes, the runner applies a small observation patch to the disposable copy and repeats the Codex restart test. The patch reads both cursors and exports a consistent copy of the fixture database before teardown. The runner exits successfully when the following two comparisons match. The resulting database can be queried directly with `sqlite3 -readonly "$scratch/stage1-run/capture/restart.sqlite"`. [Capture patch](capture.patch)

| Output in `capture/` | Checked-in expectation |
| --- | --- |
| `trace.txt` | Receipt, event sequence, run lifecycle, identities, conversation order, checkpoint files and session state. [Query](trace.sql) · [Expected output](expected.txt) |
| `cursors.json` | Final shell snapshot sequence and number of transport entries consumed by each runtime. [Expected cursors](cursors.json) |

## Follow the second receipt

1. **Command.** Look up `command:fixture:provider_thread_resume:message-2:8478dfa5-4dc9-4a11-8643-f5bef8200b7d` in `orchestration_command_receipts`. Its status is `accepted` and its result sequence is **38**. The query joins on the command ID and thread stream in `orchestration_events`. Events 32–38 include `run.created` at 33, whose payload ID identifies projected run **2**.
2. **Provider binding.** Follow run 2 through its attempt to its provider thread and turn. Both runs share the application thread ID and the native Codex thread `01a0d5ef-62bd-7c20-861b-0528ff7d86bd`. The native turns differ. Counting from the NDJSON header as line 1, the first runtime requests `thread/start` at 7 and receives that thread ID at 10. The second requests `thread/resume` with the same ID at 47, receives it at 52, and starts the new turn at 58. The turn ID returned at 62 matches the second row of the identity query. [Recorded JSON-RPC](codex_transcript.ndjson) · [Identity rows](expected.txt)
3. **Conversation.** Line 89 contains the second final answer and 92 completes the turn. Run 2 reaches `completed` at event **58** after `starting → running → waiting`. Four projected messages, ordered by turn-item ordinal, preserve the first exchange and both lines of the second answer. The three ready checkpoints have empty file lists. The provider thread ends `idle`. The separate provider-session row ends `error` with a diagnostic about the replayed process exiting with code 0. Both application runs end `completed`. [Conversation and status rows](expected.txt)

The first shell snapshot covers event **31** after **40** replay entries. The second covers event **59** after all **92** entries. The recording's first line is metadata. The opaque history paging cursors returned by Codex on line 52 belong to its protocol. For comparison, use the event sequence, message ordinal and ID relationships. The harness seeds Effect random values, controls the test clock and substitutes the temporary workspace in outbound expectations. The database also contains host paths, checkpoint refs and timestamps from different clocks. The comparison excludes those values. [Cursor capture](cursors.json) · [Deterministic runtime][clock]

The checked-in transcript records `codex.app-server` **0.156.1** and model `gpt-6-luna`. Its SHA-256 is `1733ab62d74d9618ca83187d644a762152d772267d5327d42b94cc77c7566d72`. It contains the upstream fixture's synthetic prompts and replay identity. The included MIT license covers the copied upstream material. [Transcript source][transcript] · [License](LICENSE)

## What the four suites exercised

On 2026-11-20, **35 tests passed in four unmodified test files**, with zero skipped, on Darwin arm64 using Vitest 5.0.5. The end-to-end runner repeated all 35 tests in a fresh clone and matched the trace and cursor files from two earlier independent captures. These results qualify the listed upstream scenarios at this revision. The capability inventory provides source evidence for the remaining variants. [Inventory](../../parity-inventory.md)

| Inventory area | Entry point and caller | Observed scope |
| --- | --- | --- |
| Message intake, work graph, provider binding, conversation and checkpoints | Server scenario dispatches through `OrchestratorV2`, drives the Codex adapter and reads the resulting projection | **26 Codex fixture tests**. 25 scenarios cover basic and queued turns, policy and tool calls, subagents, rerouting, planning, steering, interruption, command output and rollback. A separate test checks provider transaction writes. Recordings use app-server 0.156.1, with 0.157.0 for command output streaming. [Codex suite][codex] · [Fixture registry][registry] |
| Run configuration and command ownership | Two threads concurrently dispatch `thread.model-selection.set` through the server service | **1 race test**. A capability stub synchronizes planning. One thread wins and the other receives `OrchestratorCommandIdConflictError`. The winner's retry returns the original sequences. A no-op retry returns the original empty event list. [Race suite][race] |
| Parent/child coordination and orchestrator MCP tools | A scoped in-process MCP caller delegates, sends, interrupts, waits, reads, requests task status and discovers capabilities | **4 MCP tests**. Distinct, identical, emoji and long child results exercise delivery and result reads. The handler uses a constructed Codex app-server 0.120.0 transcript and test host services. [MCP suite][mcp] |
| Provider binding and recovery after restart | A second server runtime opens the same SQLite state with startup recovery enabled | **4 recovery tests**. Native Codex resume, Cursor resume via `cursor-agent-sdk.local` format 1 (SDK 1.4.1), and two Claude native-subagent recovery cases via `claude-agent-sdk.query` 0.3.276, including continuation after cancellation. [Recovery suite][recovery] |

At this pin the general `provider_thread_resume` registry entry exercises a 31-minute idle interval with **Pi**. The Codex restart case lives in the recovery suite. Built-in registration lists **11 execution drivers and 8 usage readers**. The four runs exercise selected server and in-process tool paths. HTTP/RPC clients, installed apps, live provider/version compatibility, the other drivers and toolkits, and SQLite import into Huggler remain source-only or planned. Stage 2 will check continuation and retry through Huggler's own API and storage. [Fixture registry][registry] · [Driver registration][drivers] · [Stage 2](../../extraction-plan.md#2-build-the-rust-core-and-run-one-durable-thread)

[clock]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/DeterministicRuntime.ts
[codex]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/CodexReplayFixtures.integration.test.ts
[drivers]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/builtInDrivers.ts
[mcp]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpToolkit.integration.test.ts
[race]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.receipt-race.test.ts
[recovery]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/OrchestratorReplayRecovery.integration.test.ts
[registry]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/index.ts
[transcript]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/provider_thread_resume/codex_transcript.ndjson
[workspace]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/provider-testing/src/replayWorkspace.ts
