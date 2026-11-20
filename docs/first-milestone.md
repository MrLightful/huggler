# Run one durable conversation with Huggler

The first Rust consumer should send a prompt, close Huggler after the answer, reopen the same SQLite state and continue on the original thread. Repeat the flow through the Rust backend of an installed Tauri app. The recorded Codex transport makes the expected provider calls repeatable. A live installed-app run separately verifies provider discovery and desktop lifecycle. [Stages 1 and 2](extraction-plan.md#choose-a-stage)

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

The last two lines form a single assistant answer. The resumed request carries the provider thread ID returned by the first start. The application’s durable thread ID is a separate value to check at the crate boundary. [Recorded transport][recording] · [Thread and run identities](upstream-orchestration.md#which-identity-or-status-to-expose)

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
| 1. Reuse the reference case | Use the two prompts, recorded JSON-RPC frames, deterministic IDs and clock, and temporary Git workspace in a Rust replay test. Drive the Codex adapter through its transport interface. | The adapter consumes the expected outbound calls and delivers both recorded answers. Keep the fixture revision and protocol version with the recording. [Upstream replay harness][harness] |
| 2. Open persistent state | Add Huggler-owned SQLite migrations, command receipts, events and effects. Open one runtime per state directory and implement shutdown. | The command-ID race and winner retry match the reference behavior. A second runtime can read the first conversation. [Runtime reference][wiring] · [Stage 2](extraction-plan.md#2-build-the-rust-core-and-run-one-durable-thread) |
| 3. Exercise the packaged crate | Run `cargo package`. Build an independent Cargo consumer from the packaged source. Use `Huggler::open` to launch, send, wait and subscribe. | After turn one, close and reopen the same directory. Read the first answer, then send a second prompt. The original Huggler thread ID survives, the second prompt gets a new run ID, and the adapter resumes the original Codex session. A second Huggler thread receives its own updates. [Proposed Rust API](sdk-proposal.md#public-api-behavior) |
| 4. Install and reopen the desktop app | Embed the crate in Tauri and pass commands and events through the Rust backend. Install the app on a machine with a supported, authenticated Codex executable. Run with Node.js absent from PATH and the app bundle. | Send a live prompt, quit, reopen and continue from the UI. Check that state lands in app data and the previous app instance releases its resources. Record the installer target, provider version and result. [Tauri integration](sdk-proposal.md#embed-in-tauri) · [Release checks](extraction-plan.md#7-validate-cargo-and-tauri-releases) |

Next, drive a live approval from Claude or Cursor through the Tauri UI. Then use that second provider to verify peer reply and owned delegation. [Cross-harness scenarios](extraction-plan.md#3-prove-agent-to-agent-operation-with-two-harnesses)

[recording]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/provider_thread_resume/codex_transcript.ndjson
[input]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/provider_thread_resume/input.ts
[recovery]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/OrchestratorReplayRecovery.integration.test.ts
[race]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.receipt-race.test.ts
[wiring]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/runtimeLayer.ts
[harness]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/ProviderReplayHarness.ts
[toolchain]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/package.json
[setup]: https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/AGENTS.md
