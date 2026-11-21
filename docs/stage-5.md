# Stage 5 qualification · Control, history and recovery

Stage 5 passed on macOS 27.0.1 arm64 with Rust 1.94.1. The Rust suite passed 67 tests with 14 marked ignored. Three of those are subprocess entry points invoked by passing crash tests. The optional Pi extension fixture passed separately. An independent Cargo consumer replayed native Codex resume, retried its receipt and verified sibling isolation. Cargo warned that the manifest has no license or license-file. [Acceptance criteria](extraction-plan.md#5-complete-control-history-and-recovery) · [Package check](../scripts/check-package.py)

## Queue, dispatch and stop

`Send` returns `Busy` for outstanding work. `SendAuto` steers a bound, running Codex turn, queues behind other outstanding work or starts an idle thread. Retrying the command returns its original receipt. Explicit `Steer` names the expected run. The host can read its pending, accepted, rejected or uncertain outcome. Only acknowledged text enters conversation history. `Restart` checks the active run and gives its replacement priority over older queued work. Opt-in Codex quota recovery schedules one eligible continuation after a reported reset. A stop or newer message cancels that continuation. [Dispatch and provider-frame assertions](../tests/context.rs)

A host can edit, reorder or cancel queued messages. Stopping a parent interrupted its child and grandchild, preserved a completed review and let an independent peer continue. Held messages survived reopen and ran in the edited order after the host resumed each thread. Background task waits cover completion, timeout, approval and revocation. After a missing managed checkout was repaired, a new submission succeeded there. Promoting an existing queued message to steering, retrying preparation on the same run, manual release, setup scripts and standalone workflow control are unavailable. [Queue, stop and worktree scenarios](../tests/delegation.rs) · [Stale completion tests](../src/store/interrupts.rs) · [Native-child stop](../tests/acp_native_children.rs)

## Human input and attachments

Permissions and questions are scoped to the owning thread, run and request. The installed GPUI app approved a recorded Codex grandchild’s request and both children finished. A question can carry a separate upload with each answer. Dismissing a Codex question sends an empty answer map and closes the request. After process loss, the host observes both callbacks closing and a retry sends zero answers. [Permission tests](../tests/permissions.rs) · [Question tests](../tests/questions.rs) · [Callback and attachment scenarios](../tests/context.rs)

`prepare_upload` stages UTF-8 `text/plain` for one thread. A message can claim up to eight files of at most 64 KiB each. A failed multi-file claim leaves the valid files available. Repeating a committed send returns the original receipt, and discarding an unused upload twice succeeds. Binary attachments are unsupported. [Upload storage](../src/store/uploads.rs) · [Outbound prompt and answer assertions](../tests/context.rs)

## Model, context and files

`SetModel` selects the model for future submissions. The scripted Codex restart reused the native session and sent the replacement on the selected model. A newly launched thread started a fresh native session. For a native fork, the source instance must match. Huggler checks the returned parent and the last inherited turn with `thread/fork` and `thread/turns/list` before sending the fork’s first prompt. A boundary mismatch fails the run before that prompt. A portable fork can cross instances or providers. Its first prompt carries attributed history through the selected completed turn, capped at 16 KiB of serialized context and 2 KiB of text per message. A merge-back sends the result to the original parent. Native session compaction and in-place rollback are unavailable. [Fork, first-prompt and merge assertions](../tests/context.rs)

Capture a checkpoint on the latest completed turn of an idle thread with a clean checkout. A history-only fork leaves shared files intact. A checkpoint fork also restores the captured revision in a new detached worktree. The fixture modified a shared tracked file after capture and verified both its dirty contents in the source and the earlier contents in the destination. `checkpoint_diff` compares two captured revisions on the same thread with a 1 MiB output limit. [Checkpoint scenarios](../tests/context.rs) · [Git operations](../src/worktree.rs)

## Reads and transport

`threads` returns summaries. `detail` returns current run and native-child information with a page of 1–100 messages. Search matches literal text within one thread with ASCII case folding. Each preview is at most 4 KiB, and `read_message` expands it. A retained subscription cursor replays up to 64 changes and 256 KiB per fetch, allowing one oversized event to make progress. An expired cursor returns `CursorExpired`. Rebuilding from fresh pages after reopen produced the same messages as applying the retained change to the earlier snapshot. [Public history and replay scenario](../tests/runtime.rs)

Codex command output has a separate 32 KiB tail per item with a revision for waiting and a final value readable after reopen. Optional Codex sampling exposes an eligible follow-up as `Run.prompt_suggestion`. A late result from a superseded turn is dropped. The release GPUI window paged and searched 1,000 messages, expanded a full match, submitted a suggestion and preserved a typed draft. Thread archive and organization, cross-thread search, workflow scripts and authenticated remote RPC are outside the local read surface. [Output and suggestion tests](../tests/context.rs) · [GPUI window check](../scripts/check-desktop-history.py)

## Crash and process loss

The file-backed SQLite tests exit on either side of the command commit. An interrupted commit leaves no receipt, message or run. A committed host send lost before provider claim reopens as failed and unsent, with its original receipt. After provider I/O, the recovered run reports uncertain delivery and the fixture counts exactly one outbound prompt across restart and retry. A committed delegated result can wake the idle parent once. Repeating upload cleanup also succeeds after the resource is gone. [Commit boundaries](../src/store/crash_tests.rs) · [External delivery count](../tests/runtime.rs) · [Delegated completion recovery](../tests/delegation.rs) · [Callback expiry and cleanup](../tests/context.rs)

## Existing data

File-backed fixtures upgrade schema versions 13–23 to 24 and read the saved message and receipt. Historical v2, v7 and v12 cases cover event decoding, task receipts and native provider bindings. Claude, Codex and Pi file imports copy visible history from up to 8 MiB and 32,768 records. The first continuation includes the copied context even after source deletion. ACP Registry imports record provenance and require the original native session to load. The fixture removed the session, observed a failed continuation with zero prompts, restored it and continued on its original ID. Repeat imports return matching provenance, and changed sources are rejected. Direct T3 database import is unsupported. [Migration fixtures](../src/store/crash_tests.rs) · [Historical upgrades](../src/store.rs) · [Import and continuation scenarios](../tests/context.rs)

## Reproduce

Run from the Huggler root. Stage 5 used scripted providers and recorded frames. Earlier qualification identified Codex 0.156.1 on recorded app-server stdio and 0.170.0 on a live cloud turn, standalone Claude Agent ACP 0.109.0 on ACP v1 stdio, Pi 1.0.0 on process RPC and an ACP Registry peer reporting version `1`. The new steering, fork, quota, output and suggestion cases establish the host behavior against scripted Codex responses. [Stage 4 driver evidence](stage-4.md#driver-results) · [Stage 5 protocol fixture](../tests/fixtures/context-provider.py)

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked --test provider_instances pi_extension_calls_scoped_tool -- --ignored
python3 scripts/check-package.py
```

The Pi extension fixture needs Node. For the desktop checks, use a macOS GUI session. The first two Python commands build and install an ad-hoc signed release qualification app in a temporary directory. The last two drive the resulting release binary.

```sh
cargo fmt --manifest-path desktop/Cargo.toml -- --check
cargo clippy --locked --manifest-path desktop/Cargo.toml --features qualification --all-targets -- -D warnings
python3 scripts/check-desktop-peers.py
python3 scripts/check-permission.py recorded-codex
python3 scripts/check-desktop-history.py target/desktop/release/huggler-desktop
python3 scripts/check-provider-desktop.py target/desktop/release/huggler-desktop
```

In the pinned T3 Code checkout at `338352dee2f43079482e054e8229a2ba055641d9`, run the following from `apps/server` with the workspace dependencies installed. The three upstream suites passed all 20 tests under Node 26.13.1, pnpm 12.14.0 and Vitest 5.0.5. [Pinned suite references](extraction-plan.md#5-complete-control-history-and-recovery)

```sh
vp test run --maxWorkers 1 \
  src/orchestration-v2/LiveStreamBudget.test.ts \
  src/orchestration-v2/boundedSnapshotTransport.test.ts \
  src/orchestration-v2/ThreadTransportPerformance.test.ts
```
