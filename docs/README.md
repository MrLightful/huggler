# Huggler engineering documentation

The [documentation site](../docs-app/content/docs/index.mdx) covers how to use the Rust crate and TypeScript SDK. Build the Rust API reference from a checkout with `cargo doc --locked --all-features --no-deps`.

## Maintainer references

| Task | Reference |
| --- | --- |
| Review or port a T3 Code change | [Review workflow and upstream component map](upstream-watch-guide.md), [review cursor and deferred work](upstream-review.json) |
| Check coverage for an upstream workflow | [T3 Code capability inventory](parity-inventory.md) |
| Reproduce the desktop coding workflow | [Planning through pull request](desktop-coding.md) |
| Run regression checks or qualify a release | [Verification and release checks](verification.md) |
| Build and verify TypeScript packages | [Local packaging and consumer checks](typescript-sdk.md#build-and-test-from-a-checkout) |

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
