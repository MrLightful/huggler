# Run one durable conversation with Huggler

The first consumer should be able to send a prompt, close the orchestrator after the answer, reopen the same state and continue on the original thread. This exercises the package boundary, the SQLite store, command handling and a real provider adapter in one scenario. A recorded Codex transport makes the expected provider calls repeatable. This guide covers the first case in [stages 1 and 2](extraction-plan.md#choose-a-stage).

## Inspect the expected provider calls

Run this from a T3 Code clone containing the pinned commit. It reads the transcript directly from Git with Python 3. You can keep your current branch checked out.

```sh
python3 - <<'PY'
import json
import subprocess

revision = "338352dee2f43079482e054e8229a2ba055641d9"
fixture = "apps/server/src/orchestration-v2/testkit/fixtures/provider_thread_resume/codex_transcript.ndjson"
transcript = subprocess.check_output(["git", "show", f"{revision}:{fixture}"], text=True)
started_id = None
for line in transcript.splitlines():
    record = json.loads(line)
    frame = record.get("frame", {})
    method = frame.get("method")
    if record["type"] == "emit_inbound" and record.get("label") == "thread/start":
        started_id = frame["result"]["thread"]["id"]
    if record["type"] == "expect_outbound" and method in ("thread/start", "thread/resume", "turn/start"):
        print(method)
        if method == "thread/resume":
            assert started_id and frame["params"]["threadId"] == started_id
    if method == "item/completed":
        item = frame["params"]["item"]
        if item["type"] == "agentMessage":
            print(item["text"])
PY
```

Expected output from the Codex app-server 0.156.1 recording:

```text
thread/start
turn/start
provider thread resume fixture first turn complete
thread/resume
turn/start
provider thread resume fixture first turn complete
provider thread resume fixture second turn complete
```

The last two lines form a single assistant answer. The resumed request carries the provider thread ID returned by the first start. The application’s durable thread ID is a separate value to check at the SDK boundary. [Recorded transport][recording] · [Thread and run identities](upstream-orchestration.md#which-identity-or-status-to-expose)

## Establish the runtime reference

Use a T3 Code checkout at the same revision with its development dependencies installed. Upstream pins Node 26.13.1 and pnpm 12.14.0 and uses `vp i` for setup. Run both tests from the checkout root. [Toolchain][toolchain] · [Upstream setup and test commands][setup]

```sh
vp test run apps/server/src/orchestration-v2/testkit/OrchestratorReplayRecovery.integration.test.ts \
  -t 'resumes a provider-native Codex thread after recreating the orchestrator runtime'
vp test run apps/server/src/orchestration-v2/Orchestrator.receipt-race.test.ts \
  -t 'a command id reused concurrently for two threads succeeds for only one'
```

| Test | What to compare in the extraction |
| --- | --- |
| [Recovery across two runtimes][recovery] | The upstream test splits after the first idle point, creates a second runtime over the same file-backed database and enables startup recovery. The projection ends with one provider thread, run ordinals 1 and 2 both completed, and four messages in user/assistant/user/assistant order. Both final-answer markers survive. |
| [Concurrent command IDs][race] | Two threads submit the same command ID. Only the winner’s model changes. The loser gets `OrchestratorCommandIdConflictError` identifying both threads. A retry on the winner returns the original event sequences. A no-op command also retains its receipt and replays an empty event list. |

Use the recovery **test** as the reference for reopening SQLite. The general fixture input exercises resume by advancing one runtime’s clock by 31 minutes. Both paths can produce a provider resume call. Only the recovery test constructs the second orchestrator. [Fixture scenario input][input] · [Recovery test][recovery]

## Build the first consumer in four changes

| Change | Implement | Demonstrate before moving on |
| --- | --- | --- |
| 1. Port the reference case | Bring the two fixture prompts, recorded frames, deterministic IDs and clock, and temporary Git workspace into the package testkit. Drive the production Codex adapter through the replay transport. | The harness consumes the expected outbound calls and delivers both recorded answers. Keep the fixture revision and protocol version with the recording. [Replay harness][harness] |
| 2. Open persistent state | Extract contracts, migrations, command dispatch, receipts, event projections and the effect worker. Compose one runtime per state directory and implement scoped shutdown. | The command-ID race and winner retry match the reference. Closing then reopening the same database preserves the first conversation. [Runtime wiring][wiring] · [Stage 2 dependencies](extraction-plan.md#2-extract-contracts-and-run-one-durable-thread) |
| 3. Expose the conversation | Wire project open, root-workspace launch, send, wait, status, conversation history and subscription to the runtime. Preserve a returned thread ID across opens and correlate each wait with its own run. | After turn one, the consumer reads the first answer, closes the runtime and reads the answer after reopening. The next send returns a new run on the original thread. The final history, statuses and provider calls match the reference above. [Proposed interfaces](sdk-proposal.md#interface-contracts-for-the-sdk) |
| 4. Exercise the installed build | Pack the ESM build and declarations, install the tarball in a clean application outside the monorepo, and run change 3 with `openHuggler` imported from `huggler`. Include the SQLite worker and the replay assets used by the consumer. | The consumer type-checks and exits after shutdown. A second thread in the same runtime receives only its own conversation updates. Keep the package revision, tarball file list and consumer test result beside the upstream test results. [Distribution checks](extraction-plan.md#7-validate-the-npm-distribution-and-future-upgrades) |

When this passes, reuse the installed-consumer setup for Codex and Claude. The next stage needs two distinct observations. An independent reviewer can send a reply to the author’s thread. A delegated reviewer can finish an owned task and wake its idle parent. [Cross-harness scenarios](extraction-plan.md#3-prove-agent-to-agent-operation-with-two-harnesses)

[recording]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/provider_thread_resume/codex_transcript.ndjson
[input]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/provider_thread_resume/input.ts
[recovery]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/OrchestratorReplayRecovery.integration.test.ts
[race]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.receipt-race.test.ts
[wiring]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/runtimeLayer.ts
[harness]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/ProviderReplayHarness.ts
[toolchain]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/package.json
[setup]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/AGENTS.md
