# Huggler

Huggler is an agent orchestration engine for running and coordinating coding agents in your own application. It’s built in Rust.

## Run the desktop app

On macOS, run this command from the repository root to build and open Huggler:

```sh
cargo run --locked --manifest-path desktop/Cargo.toml
```

Press **⌘Q** to quit.

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

- **Rust**. Embed the `huggler` crate in your application. [Rust quickstart](docs-app/content/docs/bindings/rust/quickstart.mdx)
- **TypeScript**. Use the `@huggler/sdk` preview in a Node.js application. [TypeScript quickstart](docs-app/content/docs/bindings/typescript/quickstart.mdx)

See which capabilities each language supports. [Feature comparison](docs-app/content/docs/bindings/index.mdx#feature-availability)
