# Huggler for TypeScript

The `@huggler/sdk` preview embeds Huggler in a Node.js process. A CLI or backend can launch a conversation in a Git checkout, respond to a provider, and reopen the same thread after a restart. The application owns the state directory and provider configuration.

## Install the preview

Use Node.js 22.14 or newer in the 22 series, Node.js 24, or Node.js 26. This preview is distributed as local tarballs. Copy the SDK tarball and the native tarball for your platform into your project. Install both in the same command. For Apple Silicon macOS, run

```sh
npm install --ignore-scripts ./huggler-sdk-0.0.1-preview.0.tgz ./huggler-sdk-darwin-arm64-0.0.1-preview.0.tgz
```

On Linux x64 with glibc, use `./huggler-sdk-linux-x64-gnu-0.0.1-preview.0.tgz` as the second file. The SDK includes TypeScript declarations and supports ESM imports and CommonJS `require`. The tarballs contain prebuilt binaries. Install Git and the provider executable you intend to use, and authenticate that provider for the account running Node.

## Send a prompt

Run this example with a Git checkout at `./my-project` and an authenticated Codex CLI on `PATH`. Use stable IDs for each logical command so a host can recover its receipt after restarting.

```ts
import { Huggler, commandId, threadId } from "@huggler/sdk"

const host = await Huggler.open({ stateDir: "./.huggler" })
try {
  const project = await host.openProject("./my-project")
  const id = threadId("repo-walkthrough")
  await host.launch({
    commandId: commandId("launch-walkthrough"),
    threadId: id,
    projectId: project.id,
    providerInstanceId: "codex",
    name: "Repository walkthrough",
  })
  const receipt = await host.send({
    commandId: commandId("walkthrough-first-question"),
    threadId: id,
    text: "Where does this project handle authentication?",
  })
  const run = await host.wait(id, receipt.runId)
  if (run.status !== "completed") throw new Error(run.error ?? run.status)
  console.log((await host.readThread(id)).messages.at(-1)?.text)
} finally {
  await host.close()
}
```

Reopen the same state and send to the same thread with a fresh command ID for each follow-up. Repeating an identical command ID and payload returns the saved receipt. Reusing the ID for different intent raises a `HugglerError` with code `conflict`. One host owns a database at a time. Await `close()` before handing it to another Node or Rust process.

## Observe and control a run

- `subscribe(threadId)` returns a consistent snapshot and an async iterator of subsequent thread events. With `{ after: savedCursor }`, the iterator replays events after the saved cursor, including events already represented in the snapshot. Keep the cursor of the last event your application applied. If replay reports `cursor_expired`, subscribe again for a fresh snapshot.
- Await one `events.next()` at a time or use `for await`. Break from the loop, call `events.return()`, or pass an `AbortSignal` to release a pending read. Aborting `wait(threadId, runId, { signal })` detaches the observer. Use `stopThread({ commandId, threadId })` to interrupt work.
- A `permissionRequested` event includes the request ID, run ID and offered option IDs. Pass the owning thread, run and request IDs plus the chosen `option` to `respondToPermission`. For a Codex plan-mode send, set `mode: "plan"`. Answer a `questionRequested` event with `respondToQuestion`, mapping each question ID to a string or array of strings.
- Use `readHistory(threadId, { limit, query })` for bounded reads. Supply the returned `history.older` as `before` on the next page and keep the query consistent. On `history_changed`, restart with a fresh first page. `readMessage` returns the full text of a truncated preview. Persist cursors as strings. Sequence numbers, ordinals and token totals also use decimal strings.
- The built-in instances are `"codex"` and `"claudeAgent"`. For a custom instance, import `providerInstanceId`, create an ID such as `const work = providerInstanceId("work")`, supply `{ id: work, driver: "codex", executable: "codex" }` in the `instances` array, and pass `work` as the launch command’s `providerInstanceId`. Use `listProviderInstances` to inspect configuration, `probeProvider(instance, cwd)` for runtime capabilities and `readProviderUsage()` for reported token totals. The `storage` and `output` options configure paths, agent identity, instructions, labels and tool presentation. [Public types](../bindings/node/src/types.ts)
- Catch `HugglerError` and switch on `error.code`. For example, `conflict` includes both thread IDs in `detail`, and `invalid` covers malformed native arguments and inputs that cannot be serialized as JSON. Provider failures expose a summary. Conversation text and provider request payloads can contain user data.
- Replay includes the persisted events from other hosts, including peer creation and delegated-task delivery. The preview launches root-checkout threads and leaves the caller-scoped agent tool service disabled.

## Build and test from a checkout

Use Rust 1.94.1, a C toolchain, Git, Python 3.11+ and Node.js. From the repository root, run

```sh
cd bindings/node
npm ci --ignore-scripts
npm run build
npm test
```

For distributable packages, install the Rust license documentation and notice generator, then build on each target.

```sh
rustup component add rust-docs
cargo install --locked cargo-about --version 0.9.1
npm run pack:local
npm test -- packages/huggler-sdk-0.0.1-preview.0.tgz packages/huggler-sdk-darwin-arm64-0.0.1-preview.0.tgz
```

The pack command writes the SDK and host binary packages under `bindings/node/packages/`. Substitute the Linux tarball name on Linux. On macOS the build uses Rust's bundled LLVM linker. With Xcode 27, install an SDK compatible with that linker and point `SDKROOT` to it. The build script detects the Command Line Tools macOS 26 SDK when available.

## Verified workflows

The consumer installs both tarballs offline with install scripts disabled. It compiles ESM and CommonJS consumers against the installed SDK and checks native response samples against its TypeScript declarations. [Consumer test](../bindings/node/tests/consumer/consumer.mts) · [CI workflow](../.github/workflows/node-bindings.yml)

| Workflow | Harness and prerequisites | Observed outcome |
| --- | --- | --- |
| Open and configure | Git checkout, custom SQLite path, two named instances and a custom client version | A competing process gets `already_open`. The Codex handshake uses the configured version. An absolute path supplied as a project-relative worktree root is rejected. |
| Send and resume | Codex app-server 0.156.1 recording over JSON-RPC stdio, with the same core revision and configuration in each host | Node sends a prompt. Rust reopens the database and retrieves the original receipt on retry. A second Node process sends a follow-up in the same native session. The sibling thread stays isolated. |
| Replay and page | Recorded Codex conversation plus a disposable database seeded with a retention floor and large sequence numbers | Events arrive in order. Cursors above the JavaScript safe-integer limit round-trip. An expired event cursor and a stale history page produce distinct recovery errors. |
| Answer and stop | Recorded Codex 0.156.1 plan questions and a scripted ACP v1 permission peer, both over stdio | The owning run accepts a response. The SDK rejects stale or mismatched responses. Aborting a wait preserves the pending approval, and stopping the thread interrupts the run. The ACP peer reports session loading and supplies neither a CLI version nor an authentication verdict. |

The addon targets Node-API 8. The macOS build sets a deployment target of 14.0. The packed consumer passed on Node.js 22.14.0, 22.23.5, 24.0.0, 24.22.3, 26.0.0 and 26.13.1 on both macOS 27.0.1 arm64 and emulated Debian Bookworm x64 with glibc 2.36.
