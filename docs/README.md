# Huggler engineering documentation

The [documentation site](../docs-app/content/docs/index.mdx) covers how to use the Rust crate and the TypeScript and Python SDKs. Build the Rust API reference from a checkout with `cargo doc --locked --all-features --no-deps`.

## Maintainer references

| Task | Reference |
| --- | --- |
| Review or port a T3 Code change | [Review workflow and upstream component map](upstream-watch-guide.md), [review cursor and deferred work](upstream-review.json) |
| Check coverage for an upstream workflow | [T3 Code capability inventory](parity-inventory.md) |
| Set up a remote machine | [Remote machines over SSH](remote-machines.md) |
| Reproduce the desktop coding workflow | [Planning through pull request](https://github.com/MrLightful/bear/blob/main/docs/desktop-coding.md) |
| Run regression checks or qualify a release | [Verification and release checks](verification.md) |
| Build and verify TypeScript packages | [Local packaging and consumer checks](typescript-sdk.md#build-and-test-from-a-checkout) |
| Build and verify Python wheels | [Local packaging and consumer checks](python-sdk.md#build-and-test-from-a-checkout) |

## Consumer and orchestration boundary

Start by trying to implement the workflow in the consumer using Huggler’s existing commands, queries and events. Ask whether two hosts could reasonably choose different behavior for the same conversation. If either choice leaves execution and shared state correct, let each consumer decide. This includes preferences saved across restarts.

Before extending the crate, name a concrete failure a consumer cannot prevent or a piece of execution state only Huggler can report. For example, a host can choose when to request an operation, but only the orchestrator can validate it against work arriving concurrently from another host. Expose the smallest fact or operation that closes that gap. When porting from T3 Code, apply this test even if upstream puts the whole workflow in its server.

**Conversation settlement** illustrates the split. A queued turn can arrive after the desktop renders an enabled Settle button. Huggler checks eligibility when it applies the command, clears pending usage-limit recovery on settlement, and reopens the conversation when it accepts a new turn. The consumer decides when to request settlement, how to group settled conversations, and whether to remember a collapsed sidebar section. A host that only wants to hide a row can save that preference locally and let recovery proceed. [Settlement and message intake](../src/store.rs) · [Desktop sidebar](https://github.com/MrLightful/bear/blob/main/desktop/src/conversation/sidebar.rs)

## Naming

- Use `snake_case` for Rust modules and methods and distinct Rust types for distinct IDs. Preserve stable serialized driver keys such as `claudeAgent` alongside Rust module names such as `claude_agent`.
- Use `huggler` as the default MCP server name and `snake_case` keys such as `thread_send` and `delegate_task` for built-in tools. Keep canonical keys stable across discovery, dispatch and policy, including when a host changes the advertised names.
- Prefix Huggler-owned environment variables with `HUGGLER_`. Preserve vendor spellings in provider protocols, readers of historical data and source references.

## Historical qualification

PR #9 (`ef2ffe7`) completed the initial extraction. The original reports in that revision record the exact harness versions, observed results and limits.

| Scope | Evidence |
| --- | --- |
| Codex restart and durable receipts | [Upstream capture and replay](reference/codex-resume/README.md), [Cargo and desktop qualification](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-2.md) |
| Collaboration and provider compatibility | [Cross-harness runs and native children](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-3.md), [provider versions and transports](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-4.md) |
| Runtime and application behavior | [Control, history and recovery](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-5.md), [host integrations and upstream tool mapping](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-6.md) |
| Cargo and macOS release | [Release evidence and upstream comparison](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-7.md), [archive manifest](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/reference/stage-7/huggler-0.0.1.contents.txt) |
| T3 Code architecture at the baseline revision | [Server, adapters and host services](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/upstream-orchestration.md) |
| Node.js preview | [Packed consumer results](typescript-sdk.md#verified-workflows) |
