# Stage 4 qualification · Provider instances

On macOS 27.0.1 arm64 with Cargo 1.94.1, a cloud Codex turn completed while a second instance reported a missing executable. Installed Muse reopened its native session and completed a second echo turn. A debug GPUI qualification build displayed the missing path, then discovered two models on a working ACP fixture. The packaged crate passed native Codex resume, receipt retry and sibling-thread isolation from an independent Cargo consumer. [Instance scenarios](../tests/provider_instances.rs) · [Native window check](../scripts/check-provider-desktop.py) · [Package check](../scripts/check-package.py)

## Configure and maintain an instance

The Rust host supplies extra entries in `HugglerOptions.instances` before opening the runtime. GPUI reads the same list from the JSON file named by `HUGGLER_PROVIDERS` at startup. The built-in IDs are `codex` and `claudeAgent`. Their executable overrides in GPUI are `HUGGLER_CODEX` and `HUGGLER_CLAUDE_ACP`. Give every additional instance a unique ID and specify its executable as a name on PATH or an absolute path. For example, the following file adds Cursor's CLI ACP route and an externally managed OpenCode server. [Registry and validation](../src/providers/registry.rs) · [Desktop configuration](../desktop/src/main.rs)

```json
[
  {"id":"cursor-work","driver":"cursor","executable":"agent","args":["acp"]},
  {"id":"open-dev","driver":"opencode","executable":"opencode","serverUrl":"http://127.0.0.1:4096","passwordEnv":"MY_OPENCODE_PASSWORD"}
]
```

- **Discovery and maintenance.** Enumerate `provider_instances()`, then call `probe_provider(id, checkout)`, or select an instance in GPUI and choose **Check provider**. The probe returns the transport, version when available, model IDs, native resume and agent-tool capabilities. It sends no prompt. ACP may create an empty native session to obtain its model catalog. Install or update the executable with the provider's installer, sign in with its CLI under the intended home, then probe again. A missing executable, an explicit login challenge, an incompatible handshake and an OpenCode access denial have distinct errors. Only a positive Codex account read confirms login. An empty catalog is a valid result. [Probe implementation](../src/providers/probe.rs) · [Error types](../src/contracts.rs)
- **Homes and credentials.** An absolute `home` selects `CODEX_HOME`, `CLAUDE_CONFIG_DIR` or `PI_CODING_AGENT_DIR` for the respective driver. Other drivers use their provider's own configuration and login. In particular, two fx instance names can share a machine login. External OpenCode accepts numeric loopback HTTP origins and can read its password from the environment variable named by `passwordEnv`.
- **Launch and continuation.** Dispatch `Action::LaunchInstance` with the selected ID. Read `Thread.provider_instance` to identify the binding. The MCP tools `create_threads` and `delegate_task` accept `target.providerInstanceId` and otherwise inherit the caller's instance. Choosing another instance starts with that instance's default model. Native continuation is scoped to the instance and its configuration. Restore the original executable, arguments, home and server configuration to continue a bound thread. A version-12 database migrates its threads to their original driver IDs and preserves native IDs, receipts, events and thread order. [Migration](../migrations/013.sql) · [Migration assertions](../src/store.rs)
- **Output and usage.** In a thread subscription, `Change::OutputDelta` carries text with its Huggler run ID. Read the final message and terminal run from the thread. `provider_usage()` groups reported input/output tokens and the count of reporting runs by instance, including previously configured IDs. A run without a report contributes no row. Historical provider-account usage, cost and quota are outside this aggregate.

## Driver results

The platform above applies to all Huggler checks below. Each replay peer validates outbound calls and supplies recorded inbound data or scripted responses. For MCP-enabled fixtures the peer invokes Huggler over its real scoped HTTP endpoint. The token columns are observations across two fixture turns, including one after reopen. A dash means the scenario established no total. [Conformance suite](../tests/provider_instances.rs) · [Fixture peers and frames](../tests/fixtures/providers/) · [Codex resume assertions](../tests/support/resume.rs)

| Driver | Version, transport and operating mode | What the check observed | Input / output tokens |
| --- | --- | --- | ---: |
| Codex | 0.156.1 recorded app-server stdio, 0.170.0 live cloud CLI | Native resume and receipt retry in the packaged consumer. Live account read, ten models and a completed turn alongside the missing ACP instance. Stage 3 covers MCP and approvals. | 42,897 / 50 |
| Claude Agent | Standalone ACP 0.109.0, bundled Claude Code 2.1.328, ACP v1 stdio | Stage 3 live permission denial, peer reply and delegated completion. The Rust regression suite covers permission ownership and child sessions after the root settles. | — |
| fx | 0.0.13, recorded ACP v1 stdio | Selected model through a config option, denied a permission, streamed the answer and loaded the same session. | 39,080 / 18 |
| Grok | 1.0.41, recorded ACP v1 stdio | Config-option model selection, permission response, streamed text, resume and root interruption. | 44,458 / 68 |
| Kiro | 2.32.0, recorded ACP **v2** stdio | Empty catalog, successful default-model turn, permission response, streaming and resume. | — |
| ACP Registry | Agent-reported version `1`, recorded ACP v1 stdio | Model discovery, permission, streaming, scoped MCP and resume. A limited peer omitted HTTP MCP and rejected continuation when it lacked session loading. | — |
| Cursor | CLI `2026.11.19-f005e9d`, `agent acp`, ACP v1 stdio | With an empty home the CLI advertised session loading and HTTP MCP, then required login on session creation. A scripted peer exercised prompt, permission, stream, resume, MCP and interrupt. | — |
| Antigravity | CLI `1.5.13`, `agy acp`, ACP v1 stdio. Upstream server `agy_acp_server_1.1.1`, recorded ACP v2 stdio | The installed CLI advertised session loading, omitted HTTP MCP and required login on session creation. The v2 recording advertised loading and HTTP MCP. Scripted turns exercised opaque permission IDs, stream, resume and interrupt on both versions, and scoped MCP on v2. Vendor-specific question cards and children remain unverified. | — |
| Pi | 1.0.0, recorded process RPC with two private homes | Both instances resumed the same native ID independently. Changed home, mismatched file and mismatched session ID were rejected. Optional extension called a scoped Huggler tool. | 3,572 / 56 per instance |
| Muse | 1.4.3 recorded MSP v1 stdio, installed CLI 1.4.23-R8977.1 with MSP server 1.4.23 | Replay covered early events, streamed text, usage and session MCP. Installed deterministic echo reported an empty catalog and answered twice across reopen with one session ID. | 64,700 / 48 in replay |
| OpenCode 1.x | 1.14.39, fixture HTTP API, managed and external | Reopened the same session, read final assistant text from the prompt response and aborted an active prompt. Managed process exited and external server stayed up. | 1,142 / 18 |
| OpenCode 2.x | 2.0.18, fixture HTTP and SSE, managed and external | Streamed text and resumed. Covered replayed events, a disconnect, an interactive form, refused admission, a joined prompt and cancellation of queued and active work. | 17,402 / 226 |

**Evidence provenance.** The upstream pin is `338352dee2f43079482e054e8229a2ba055641d9`. The ACP, Pi and Muse peers reuse selected protocol frames. The OpenCode peers translate recorded SDK responses and events into HTTP/SSE exchanges. [Upstream simple transcripts](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/simple) · [OpenCode 2.x transcript](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/opencode2_simple) · [Stage 3 live and native-child evidence](stage-3.md)

The Cursor and Antigravity CLI captures contain their actual `initialize` responses and the `-32000` login challenges from `session/new`. The Antigravity v2 handshake comes from the pinned upstream recording. Successful sessions and turns for these two providers use scripted peers based on registry frames. [Cursor CLI capture](../tests/fixtures/providers/cursor-cli-startup.json) · [Antigravity CLI capture](../tests/fixtures/providers/antigravity-cli-startup.json) · [Upstream v2 handshake](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/packages/effect-acp/test/fixtures/antigravity-initialize.json) · [Antigravity permission protocol test](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/provider/acp/AntigravityProtocol.test.ts)

### Controls and fallbacks

- ACP uses the negotiated session-loading and HTTP MCP capabilities. A saved session that cannot load produces an error. Huggler sends the host's selected permission option ID back to the requesting session. Native child tracking and interruption are qualified for Claude Agent and Codex only. A root interrupt on managed stdio stops its process.
- OpenCode 1.x provides final text at settlement. OpenCode 2.x provides live text deltas. Managed mode starts a dedicated server for the run and can install its scoped MCP connection. External mode skips MCP registration and requires one writer per native session. An external server keeps its process and credentials across Huggler shutdown. New sessions use deny-by-default tool permissions, with a Huggler tool exception for managed runs that enable agent tools.
- OpenCode 2.x assigns each submitted prompt a client-generated inbox ID. Before Huggler observes delivery it requests cancellation by that ID. After observing delivery it can interrupt the session. On an external 2.0.18 server, a disconnect between actual and observed delivery can leave the prompt running because the server ignores deletion of an already-delivered item. [Admission and delivery handling](../src/providers/opencode/events.rs) · [Cleanup requests](../src/providers/opencode.rs)
- Pi takes an absolute `piExtension` path to the separately installed `adapters/pi/huggler.mjs` when agent tools are enabled. The extension registers tools with Pi and runs in the provider's JavaScript runtime. The Cargo archive and desktop installer ship independently of it. Pi cancels interactive extension dialogs. Muse fails a turn that requests unsupported interactive input. [Pi extension](../adapters/pi/huggler.mjs)
- Model selection applies to future submissions. Stage 5 added fixture coverage for steering a bound Codex turn, restarting it and forking on the original instance at a verified native turn. Portable transfer can cross instances and providers. Checkpoint forks can restore files in a separate worktree. Native compaction and in-place rollback remain unavailable. [Stage 5 controls and history](stage-5.md)

## Reproduce

From the repository root, the default suite uses fixtures and temporary Git checkouts. The separate Pi extension check requires Node on PATH. The desktop commands require a macOS GUI session.

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --test provider_instances pi_extension_calls_scoped_tool -- --ignored
python3 scripts/check-package.py
cargo fmt --manifest-path desktop/Cargo.toml -- --check
cargo clippy --locked --manifest-path desktop/Cargo.toml --features qualification --all-targets -- -D warnings
cargo build --locked --manifest-path desktop/Cargo.toml --features qualification --target-dir desktop/target
python3 scripts/check-provider-desktop.py desktop/target/debug/huggler-desktop
```

The ordinary Rust suite passed 48 tests with 12 opt-in tests skipped. The Pi extension check, the three installed-provider checks below, strict Clippy in both crates, the Cargo consumer and the desktop discovery check also passed. Cargo reported the existing missing-license metadata warning. The desktop evidence for this stage comes from the debug qualification binary.

```sh
# Signed-in Codex cloud subscription, one short prompt.
cargo test --locked --test provider_instances cloud_codex_with_missing_acp -- --ignored --nocapture
# Installed Muse CLI with its deterministic echo provider and a temporary home.
cargo test --locked --test provider_instances installed_muse_echo -- --ignored --nocapture
# Cursor and Antigravity discovery in fresh homes. Defaults to agent and agy on PATH.
# Set HUGGLER_TEST_CURSOR and HUGGLER_TEST_ANTIGRAVITY to select specific executables.
cargo test --locked --test provider_instances installed_acp_login_challenges -- --ignored
```
