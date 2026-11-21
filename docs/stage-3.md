# Stage 3 qualification · Two harnesses

## Desktop permission

The installed GPUI app on macOS arm64 opened a live permission prompt on each harness. With Codex CLI 0.170.0 over app-server stdio, selecting **Deny and stop turn** interrupted the requesting run. With standalone Claude Agent ACP 0.109.0 and its bundled Claude Code 2.1.328 over ACP v1 stdio, selecting the one-time rejection denied the write and the same run completed. Each check observed one persisted permission decision on that run and an absent `approval-check.txt`. The Claude harness used Qwen3.5-4B running locally. [Desktop check](../scripts/check-permission.py)

## Cross-harness collaboration

On macOS arm64, Codex CLI 0.170.0 used app-server stdio and Claude Code 2.1.328 used standalone Claude Agent ACP 0.109.0 over stdio. Both harnesses called the same Huggler MCP endpoint on loopback. The Claude harness used a local Qwen3.5-4B model. The default Anthropic configuration on this machine returned `Authentication required`.

**Independent peers.** Codex run `901e1234` called `thread_list`, then `thread_send` to the reviewer with key `review-1`. The reviewer received message `9ce6d400` and read the author's address from its origin. Reviewer run `4fb37c1e` made the following call twice. Both receipts identified message `a0bb7eef`. The first delivery started run `caf1364a` on the idle author. After reopening, the author had one attributed reply and the reviewer had one run. Neither thread owned a delegated task. IDs here are abbreviated. The cross-harness check specified the tool and its inputs. The independent tool-choice check used Codex on both sides. [Peer assertions](../tests/peers.rs)

```json
{"name":"thread_send","arguments":{"threadId":"author","message":"2+2=4","clientRequestId":"reply-1"}}
```

**Owned delegation.** Codex run `891c86a0` called `delegate_task` with request key `review` and selected the `claudeAgent` instance. The author finished before the host released the child's one-time `thread_list` permission. Child thread `425bdae2`, run `3d71b4ed`, returned `2+2=4`. The task retained the requesting run and result run, marked delivery complete, and woke the author once in run `217c6506`. The author repeated the finding. After reopening, the Rust host read the same tasks, messages, runs and event cursor. The parent owns cancellation, and the host resolves provider permissions. [Delegation assertions](../tests/delegation.rs)

## Run the live checks

Start with a signed-in Codex CLI, an active macOS GUI session and the standalone ACP distribution. Keep its bundled `claude` executable beside `claude-agent-acp`. The checks find the adapter on PATH or accept an absolute executable path in `HUGGLER_CLAUDE_ACP`. For local inference, point that variable at the wrapper below and run the commands sequentially so each check has the model server to itself. The peer and delegation checks each create a temporary Git checkout and request one-time approval for the reviewer's Huggler tools. [Standalone ACP release](https://github.com/agentclientprotocol/claude-agent-acp/releases/tag/v0.109.0) · [Harness configuration](../tests/support/reviewer.rs)

```sh
python3 scripts/check-permission.py codex
python3 scripts/check-permission.py claudeAgent
cargo test --locked --no-fail-fast --test peers --test delegation live_codex_to_claude -- --ignored --nocapture
```

### Local inference configuration

The tested Apple Silicon setup used llama.cpp build 12600 and Qwen3.5-4B-Q4_K_M.gguf (SHA-256 `00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4`). The server, model and wrapper are qualification dependencies. All three run outside the Rust library and desktop app. [llama.cpp build](https://github.com/ggml-org/llama.cpp/releases/tag/b12600) · [Model revision](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/tree/e87f176479d0855a907a41277aca2f8ee7a09523)

1. In a private directory, generate a loopback credential with `(umask 077 && openssl rand -hex 32 > local.key)`. Start the server with the command below, initially omitting `--chat-template-file claude.jinja`. The server exposes the model's embedded Jinja template in the `chat_template` field of its authenticated `GET /props` response.
2. Save the embedded template as `claude.jinja`. In the branch that handles a system message after the first, replace `raise_exception('System message must be at the beginning.')` with `'<|im_start|>system\n' + content + '<|im_end|>\n'`. Keep the backslash escapes literal. Claude Code sends multiple system blocks. This change preserves their order and content. Restart the server with the template selected.
3. Save the wrapper below next to the key and make it executable. Set the final `exec` path to the downloaded standalone adapter. Export the wrapper's absolute path as `HUGGLER_CLAUDE_ACP` before running the checks. Its model settings and Claude configuration directory are scoped to that process.

```sh
llama-server -m Qwen3.5-4B-Q4_K_M.gguf --alias qwen-local \
  --host 127.0.0.1 --port 18238 -c 32768 -np 1 --jinja --reasoning off \
  --chat-template-file claude.jinja --api-key-file local.key
```

```sh
#!/bin/sh
set -eu
dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
export ANTHROPIC_BASE_URL=http://127.0.0.1:18238
ANTHROPIC_AUTH_TOKEN=$(cat "$dir/local.key")
export ANTHROPIC_AUTH_TOKEN
unset ANTHROPIC_API_KEY CLAUDE_CODE_OAUTH_TOKEN
export ANTHROPIC_MODEL=qwen-local ANTHROPIC_DEFAULT_OPUS_MODEL=qwen-local
export ANTHROPIC_DEFAULT_SONNET_MODEL=qwen-local ANTHROPIC_DEFAULT_HAIKU_MODEL=qwen-local
export CLAUDE_CONFIG_DIR="$dir/claude-config" CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
export ENABLE_TOOL_SEARCH=false
exec /absolute/path/to/claude-agent-acp "$@"
```

## Build and lifecycle checks

The Rust suite passed all 41 offline tests. The packaged `huggler` 0.0.1 archive built and ran in an independent consumer. Cargo reported the existing missing-license warning. The Rust crate and GPUI host passed strict Clippy and the desktop release built on macOS arm64. [Package check](../scripts/check-package.py) · [Build commands](stage-2.md#reproduce-the-checks)

On Unix, the runtime stops helpers in the managed provider process group on host close, executor teardown or provider protocol failure. Each regression test checks that a forked helper releases its file lock. [Host lifecycle tests](../tests/runtime.rs)

The GPUI host enables agent tools and watches for newly created conversations. In the offline ACP exchange, the author created a reviewer, the reviewer appeared in the sidebar, and both conversations showed the sending peer’s name beside its message. Run `python3 scripts/check-desktop-peers.py` on macOS to repeat the exchange. [Desktop peer check](../scripts/check-desktop-peers.py)

## Codex live runs and offline coverage

Two live Codex CLI 0.170.0 threads discovered each other and exchanged a review through the loopback `huggler` MCP server. The reviewer issued two identical `thread_send` calls. Both returned the same message ID, and reopening showed one reply from the reviewer with its original run ID and receipt. Reproduce with `cargo test --locked --test peers live_codex_peer_exchange -- --ignored`. The offline ACP tests also cover revocation, cross-project denial and restart with queued deliveries. [Peer tests](../tests/peers.rs)

Two live Codex CLI 0.170.0 sessions used app-server stdio and Huggler's loopback MCP endpoint for asynchronous delegation in a shared checkout. After the parent went idle, the child's result started one parent run. The parent repeated the finding, and reopening preserved the result, run IDs and event cursor. Reproduce with `cargo test --locked --test delegation live_codex_delegation -- --ignored`. The offline ACP cases cover passive reads, owner acknowledgement, cancellation, pending delivery on reopen, siblings and a grandchild. [Delegation tests](../tests/delegation.rs)

Live Codex CLI 0.170.0 over app-server stdio and a scripted ACP reviewer over stdio passed both cross-adapter cases through loopback MCP. The peer retry retained one reply and its sender and run. The delegated result woke its parent once and survived reopening. Run `cargo test --locked --test peers --test delegation live_codex_to_acp -- --ignored` with a signed-in Codex CLI. The offline `waiting_for_a_task` test covers completion, timeout, pending approval and revocation during `delegate_task(mode="wait")`. [Peer tests](../tests/peers.rs) · [Delegation tests](../tests/delegation.rs)

With the scripted ACP peer, a follow-up queued before settlement supplied the task's final result and woke the parent once. Cancelling the owned subtree interrupted active descendants and queued work, including work started after the original task settled. A retry with the same cancellation key after reopening preserved later work. Run `cargo test --locked --test delegation` for both regressions. [Delegation tests](../tests/delegation.rs)

Codex app-server 0.156.1 replayed the pinned three-level subagent tree over stdio. The Rust snapshot and event stream preserve each child's native parent thread and turn, owning application run and final answer across reopen. Scripted variants complete the root first, then finish the child, interrupt its exact turn, or close the provider while the child is running. The root keeps its answer in all three cases. Run `cargo test --locked --test native_children`. [Rust tests](../tests/native_children.rs) · [Upstream recording and assertions](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/subagent_v2_nested)

The installed GPUI app replayed the Codex 0.156.1 grandchild command approval over stdio. Its prompt identified `/root/relay/writer`. Selecting **Allow** sent the recorded decision and both children returned `Written.`. The Rust tests verify the owning run and native turn, reject a response addressed to another run, and recover both results on reopen. A pending approval survives foreground completion. If a grandchild asks after the foreground completes and the provider then exits, its request expires and the foreground keeps its result. Reproduce with `python3 scripts/check-permission.py recorded-codex` on macOS and `cargo test --locked --test native_children`. [Desktop check](../scripts/check-permission.py) · [Upstream recording and assertions](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/subagent_v2_nested_approval)

A scripted ACP peer read and answered a foreground question from the Codex 0.156.1 plan-mode recording through the loopback `pending_request` tool. Concurrent replies produced one recorded answer, retries returned `already_answered`, and reopening preserved the answering run. A caller in another project was denied, and the ACP permission stayed pending until the host answered it. The Rust host also answered the recorded question with agent tools disabled. Rust consumers can read question IDs and option previews from `UserInputQuestion` and `UserInputOption`. Both types preserve extra provider fields. Scripted variants cover previews, provider withdrawal and process loss. Reproduce offline with `cargo test --locked --test questions`. [Rust tests](../tests/questions.rs) · [Upstream recording](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/plan_questions)

After the original delegation settled, the scripted ACP v1 peer used stdio and loopback MCP to start one owner follow-up and queue another on the same child. Each send returned its own task ID and attributed the message to the sending run. The first prompt returned ACP `cancelled`, the second returned its finding, and retrying both sends after reopening preserved all three results. When both follow-ups delegated again, each grandchild’s result woke the follow-up that requested it. Reproduce with `cargo test --locked --test delegation`. The schema upgrade check is `cargo test --locked --lib v7_tasks_keep_their_runs`. [Delegation tests](../tests/delegation.rs) · [Upstream follow-up implementation](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Orchestrator.ts#L10053-L10156)

A scripted ACP v1 parent called `thread_interrupt` over loopback MCP with the child thread ID, running follow-up ID and a reason. Huggler closed the managed stdio process for that run and recorded the requesting thread, run and reason. The queued review started and returned its finding. Retrying while the second review was running returned the first run’s terminal status. A caller in another project received `forbidden`. All three task results survived reopening. This qualifies Huggler’s process-stop fallback. The test runs offline with `cargo test --locked --test delegation agent_interrupt_stops_only_the_addressed_followup`. [Delegation tests](../tests/delegation.rs) · [Upstream interrupt assertions](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpToolkit.integration.test.ts#L509-L597)

A scripted ACP v1 caller used `create_threads` over loopback MCP to create two independent peers in its project. Concurrent calls and a retry after reopening returned the same ordered IDs. One peer read a marker from the shared checkout, and the other stayed idle until a later call supplied its first prompt. The host retained the creating run, the prompt sender and zero delegated tasks. The storage tests cover rollback on a conflicting entry and recovery of a committed peer message before a worker claims it, including a follow-up promoted from the queue. Reproduce offline with `cargo test --locked batch_`. [Peer test](../tests/peers.rs) · [Recovery tests](../src/store/creation.rs) · [Upstream retry assertions](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.test.ts#L3356-L3434)

A scripted ACP v1 caller used `delegate_task` over loopback MCP to edit a tracked file in both checkout modes. The default shared edit reached the caller. With `isolation: "worktree"`, the child started at the caller’s committed HEAD in a detached Git worktree under the runtime state directory. After reopening, a retry returned the same task, child and run even when the parent’s HEAD was unresolved. The child resumed with its edit intact. Shared descendants and independent peers used that checkout, and a nested isolated child started at its caller’s new commit. The recovery test reopens the database on both sides of Git creation, before provider startup. Reproduce offline with `cargo test --locked worktree_`. [Delegation test](../tests/delegation.rs) · [Recovery test](../src/worktree.rs) · [Upstream checkout selection](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/mcp/OrchestratorMcpService.ts#L2875-L2903)

On macOS arm64, a scripted ACP v1 adapter sent representative stdio frames for standalone Claude Agent ACP 0.109.0 with bundled Claude Code 2.1.328. The Rust host tracked a child and grandchild by session ID and owning run, without native turn IDs. After the foreground returned its answer, the host answered a grandchild permission and sent `session/cancel` to that session when it advertised cancellation. The grandchild reported interruption and its parent returned a report. Other cases cover withdrawn and expired permissions, provider loss and resumed work after reopening. Huggler exposes the latest child state and result. Detailed child transcripts, child prompts and automatic parent wakes are outside this check. Reproduce offline with `cargo test --locked --test acp_native_children`. [Rust tests](../tests/acp_native_children.rs) · [Stdio fixture](../tests/fixtures/native-children-acp.py) · [Adapter protocol](https://github.com/agentclientprotocol/claude-agent-acp/blob/9f1ca6eb9c65c2f5d1bbf48335af42947ddb74d6/docs/subagent-sessions.md) · [T3 background-child case](https://github.com/pingdotgg/t3code/tree/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/testkit/fixtures/claude_background_subagent_after_root)

Codex CLI 0.170.0 passed the pinned review-round tool-choice case through app-server stdio and Huggler’s loopback MCP server. The adaptation uses the locally configured Codex model and a temporary Git commit that fixes both defects. Asked to run round two, the author called `delegate_task` again. After reopening, the Rust host found distinct tasks, child threads and runs. The new review brief carried forward the lost v4 usage and the shared `usage.json` filename. In the independent-peer case, the reviewer chose `thread_send` when asked to reply to its sender. The first send started a run on the idle author. The retry returned the same receipt, and reopening showed one attributed reply and zero delegated tasks. [Delegation test](../tests/delegation.rs) · [Peer test](../tests/peers.rs) · [Upstream eval](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/evals/mcp-tool-use/cases/orchestration.ndjson)

Reproduce both checks with a signed-in Codex CLI.

```sh
cargo test --locked --test delegation live_codex_delegated_review_rounds -- --ignored
cargo test --locked --test peers live_codex_peer_exchange -- --ignored
```
