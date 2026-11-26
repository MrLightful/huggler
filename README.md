# Huggler

Huggler is a Rust library for running and coordinating coding agents in your application. It runs agents in Git working trees and stores conversation state in SQLite, so your app can stream replies, handle approvals, and reopen conversations after a restart.

## Get started

Add the published [`huggler` crate](https://crates.io/crates/huggler) to your Rust project:

```sh
cargo add huggler
```

Requires Rust 1.94.1 or later. Follow the [Rust quickstart](docs-app/content/docs/bindings/rust/quickstart.mdx#run-one-turn) to launch a conversation and recover its reply after reopening the database.

Huggler runs inside your application. Your host provides the interface and decides when to send prompts, display questions, and request approvals. Install and authenticate the coding agent you want to use; runtime requirements belong to the selected provider adapter. The Rust library itself does not require Node.js.

## Capabilities

| Capability | What you can build with it |
| --- | --- |
| Agent and model selection | Let users start a conversation with an agent such as Codex or Claude Code and choose from the models it offers. |
| Live progress | Show an agent’s reply as it arrives and let users inspect Codex command output from the same conversation. |
| Planning | Let users ask Codex for a plan and review it before asking the agent to edit files. |
| Follow-ups and interruptions | Queue a follow-up while an agent is busy, give it a new direction mid-task, or stop the current run. |
| Questions and approvals | Show an agent’s questions and approval requests in your app, then pass the user’s answer or decision back to the agent. |
| Agent collaboration | Let agents exchange messages or hand off specific work. For example, an implementation agent can delegate a review and receive the findings. |
| Conversation recovery | Reopen saved conversations when your app restarts and let users search earlier messages. |
| Reusable context | Import an existing conversation or start a separate thread from a completed turn to explore another approach. |
| Guest access | Invite someone to view a conversation and review their submitted messages before adding them to the thread. |
| Scheduled and event-driven runs | Start or continue agent work from your app’s scheduler or a webhook, such as running a daily review. |

## Available languages

| Language | Distribution | Guide |
| --- | --- | --- |
| Rust | Published `huggler` crate on crates.io | [Rust quickstart](docs-app/content/docs/bindings/rust/quickstart.mdx) · [API reference](https://docs.rs/huggler) |
| TypeScript | `@huggler/sdk` preview, installed from locally built tarballs | [TypeScript quickstart](docs-app/content/docs/bindings/typescript/quickstart.mdx) |
| Python | Native wheels from CI artifacts or built from a checkout | [Python quickstart](docs-app/content/docs/bindings/python/quickstart.mdx) |

Bindings expose different parts of the runtime. Check the [feature comparison](docs-app/content/docs/bindings/index.mdx#feature-availability) before choosing one. Rust hosts can enable the optional `application` feature for workspace management, schedules, repository bindings, and host services.

## Providers

Huggler integrates with Codex, Claude Agent, Cursor, Grok, FX, Kiro, Antigravity, OpenCode, Pi, Muse, and agents from the ACP registry. Available operations depend on the adapter and installed agent version. See the [provider reference](docs-app/content/docs/reference/providers.mdx) for transports, tested versions, and limitations.

## Documentation

- [User documentation](docs-app/content/docs/index.mdx): runtime concepts and guides for building a host.
- [Engineering documentation](docs/README.md): architecture boundaries, verification, bindings, and upstream review.

## License

[MIT](LICENSE).
