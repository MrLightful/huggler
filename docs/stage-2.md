# Stage 2 qualification · Resume a Codex conversation

The packaged `huggler` library and the installed desktop app both continued a conversation after reopening SQLite. The second prompt started a new application run and a new Codex turn on the original native thread. A sibling thread in the same Git checkout kept its own conversation and events. [Stage 2 acceptance](extraction-plan.md#2-build-the-rust-core-and-run-one-durable-thread)

## Reproduce the checks

Run from the Huggler root with Rust/Cargo 1.94.1 or later, Git and Python 3.12 or later. The replay uses a POSIX executable fixture and runs without a Codex login. The live check requires a logged-in, native Codex executable, macOS with Xcode and an active GUI session. Set `HUGGLER_CODEX` to the absolute executable path if Codex is outside the launcher's search path.

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --manifest-path desktop/Cargo.toml -- -D warnings
python3 scripts/check-package.py
python3 scripts/check-desktop.py
```

The package check runs `cargo package`, verifies the archive, extracts it and builds a separate Cargo binary against that extracted source. The binary runs the same public API assertions as the integration test. The verified 0.1.0 archive contained 20 files, including the migration, test peer, recorded frames and upstream MIT notice. Cargo warned that the Stage 2 manifest lacked a license. Stage 7 adopted MIT for Huggler. [Release terms](stage-7.md#what-ships) · [Package check](../scripts/check-package.py) · [Fixture license](reference/codex-resume/LICENSE)

## What the replay verifies

| Observation | Assertion |
| --- | --- |
| First send | The receipt identifies the user message, application run and committed event sequences. Codex receives `thread/start` then `turn/start`. The first reply matches the recorded answer exactly. |
| Reopen and retry | The second runtime recovers the same Huggler thread and first answer. Repeating the first command with its original payload returns the identical receipt. The next provider process starts only on the second send. |
| Native continuation | Codex receives `thread/resume` with the native ID returned on the first start. The second `turn/start` carries only the new prompt. Both native turn IDs match the recording. Run ordinals are 1 and 2. |
| Projections and events | Each run emits `starting → running → waiting → completed`. Each receipt's sequences identify the accepted run and user message in its thread stream. The reopened stream continues after the saved cursor. The four messages appear in user/assistant/user/assistant order. The last answer repeats the first marker and adds the second on a new line. |
| Sibling isolation | A second Huggler thread gets its own native binding. Its subscription receives its own run and messages. Sending there leaves the first thread's cursor at its previous value. |
| Racing commands | Two threads concurrently select a model with one command ID. Only the winner commits. The loser receives `Error::Conflict` identifying both threads. Retrying the winner returns its original event sequences, including the empty list for a no-op selection. |

The peer reads the checked-in Codex app-server 0.156.1 transcript (`gpt-6-luna`) and compares every expected outbound frame. The process boundaries are replay cursors 40 and 92. It substitutes the temporary workspace, maps unique request IDs, identifies the Huggler client, omits the upstream MCP Apps capability and checks Huggler's read-only turn policy. The sibling case substitutes a second native thread ID. Caller IDs are fixed for replay. Generated Huggler IDs are compared by identity, and SQLite sequences establish order without a test clock. Huggler uses its own event numbering and schema. [Public scenario](../tests/support/resume.rs) · [Replay peer](../tests/fixtures/replay-codex.py) · [Expected identities and answers](reference/codex-resume/expected.txt) · [Reference cursors](reference/codex-resume/cursors.json)

## Runtime and recovery

`Huggler::open(HugglerOptions::new(state_dir))` acquires the directory lock, migrates SQLite and reconciles outstanding effects before returning. Use a Tokio runtime with time and I/O enabled. Call `open_project` on a Git checkout, then `launch` with caller-supplied command and thread IDs. Use `send` for an idle thread, `wait` with the receipt's run ID for settlement, and `read` for the conversation. `subscribe(thread, None)` returns a snapshot and later events. Passing a saved cursor replays events strictly after that sequence. [Public API](../src/runtime.rs) · [Commands, results and errors](../src/contracts.rs)

The send transaction commits the receipt, user message, run, events and pending effect together. One worker claims the effect and launches a managed `codex app-server` over stdio. Reopening marks an unfinished pending or claimed turn failed and preserves its receipt, so retries cannot issue a duplicate prompt. Once Codex completes, the assistant answer and waiting transition commit together. Recovery can finish that run exactly once, even across repeated opens. Explicit `close().await` stops intake, wakes observers, joins workers, reaps the managed child and releases the state lock. Tests cover a competing open, close during an active turn and abrupt host exit after Codex accepted a turn. [Persistence](../src/store.rs) · [Lifecycle tests](../tests/runtime.rs)

Stage 2 supports one configured Codex instance, root-checkout launches and idle sends. An active turn produces `Error::Busy` on a second send. A model selection applies to subsequent turns. Human permission decisions and multi-provider routing begin in Stage 3. [Codex adapter](../src/providers/codex.rs)

## Original Stage 2 desktop result

The live check ran on Apple Silicon, macOS 27.0.1, Xcode 27.2, Rust/Cargo 1.94.1, Tauri 2.12.6 and Codex CLI 0.170.0 using the locally configured model. It installed an ad-hoc signed release bundle in `~/Applications` and ran the installed executable from `/` with a Node-free PATH. It also inspected bundle files and executable linkage for Node. A qualification-only script drove the normal WebView controls, read the rendered answers and clicked Quit. Between launches the checker verified the process exit, native bindings, completed runs and empty effect table in app data.

| App launch | Result observed through the UI |
| --- | --- |
| 1 | Created a conversation, sent the first fixture prompt and received the exact first marker. |
| 2 | Reopened with the first answer visible. Codex returned the first and second markers on separate lines. SQLite retained the same application/native thread pair and two distinct native turns. |
| 3 | Restored the two-turn conversation, opened a sibling and observed one answer there. SQLite held three completed runs across two native threads. |

## Running the GPUI host

The native GPUI app opens the existing database at `~/Library/Application Support/dev.huggler.stage2/huggler.sqlite3`. It saves the selected thread alongside the database. On the first GPUI launch it opens the oldest saved conversation, or the new conversation form if the database is empty. All existing threads are available in the sidebar. The desktop package and its lockfile are separate from the published Rust library. [Host](../desktop/src/main.rs) · [Conversation view](../desktop/src/conversation.rs)

On macOS with Rust, Cargo and Xcode installed, run the app from a terminal with the provider executable on PATH. Set `HUGGLER_CODEX` to an absolute path if needed. GPUI compiles its Metal shaders at runtime.

```sh
cargo run --locked --release --manifest-path desktop/Cargo.toml --target-dir target/desktop
```

For the current installer, prepare cargo-about, the Rust library notices and the locked dependencies, then choose an unused destination. The second command launches the installed executable with your shell environment. [Build prerequisites](stage-7.md#reproduce)

```sh
python3 scripts/install-desktop.py "$HOME/Applications/Huggler.app"
"$HOME/Applications/Huggler.app/Contents/MacOS/huggler-desktop"
```

The installed checks use temporary bundles, checkouts and state directories. Run the recorded permission check offline after building. The other two commands require a signed-in Codex CLI. [Installer](../scripts/install-desktop.py) · [Resume check](../scripts/check-desktop.py) · [Permission checks](../scripts/check-permission.py)

```sh
python3 scripts/check-permission.py recorded-codex
python3 scripts/check-desktop.py
python3 scripts/check-permission.py codex
```

On macOS arm64, the installed GPUI 0.2.2 app completed three live turns with Codex CLI 0.170.0 over app-server stdio. After restarting, the second turn recalled the first answer on the same native thread. A sibling conversation had its own native thread and history. The fourth launch restored that sibling, and switching back displayed the original two-turn conversation.

In the live permission check, selecting **Deny and stop turn** interrupted the requesting run and left the file absent. In the Codex 0.156.1 recording, the app displayed the requesting grandchild’s path, `/root/relay/writer`. Selecting **Allow** let both children finish with `Written.`. Desktop verification covers macOS arm64.
