# TypeScript SDK packaging and qualification

The `@huggler/sdk` preview embeds the Rust runtime in Node.js through a Node-API addon. See the documentation site for [installation and first run](../docs-app/content/docs/bindings/typescript/quickstart.mdx) and the [TypeScript API](../docs-app/content/docs/bindings/typescript/api.mdx).

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
cargo install --locked cargo-about --version 0.9.1 --features cli
npm run pack:local
npm test -- packages/huggler-sdk-0.0.1-preview.0.tgz packages/huggler-sdk-darwin-arm64-0.0.1-preview.0.tgz
```

The pack command writes the SDK and host binary packages under `bindings/node/packages/`. Substitute the Linux tarball name on Linux. On macOS the build uses Rust's bundled LLVM linker. With Xcode 27, install an SDK compatible with that linker and point `SDKROOT` to it. The build script detects the Command Line Tools macOS 26 SDK when available.

## Registry release

The preview is distributed as local tarballs. For a registry release, confirm access to the `@huggler` npm scope and set up trusted publishing. Publish the exact-version platform packages before the SDK, then repeat the consumer checks from a registry install before promoting the preview tag. [Package manifests](../bindings/node/npm) · [Consumer check](../bindings/node/scripts/test-consumer.mjs)

## Verified workflows

The consumer installs both tarballs offline with install scripts disabled. It compiles ESM and CommonJS consumers against the installed SDK and checks native response samples against its TypeScript declarations. Successful **Node bindings** runs attach the tested SDK and native tarballs for each platform to the workflow run. [Consumer test](../bindings/node/tests/consumer/consumer.mts) · [CI workflow](../.github/workflows/node-bindings.yml)

| Workflow | Harness and prerequisites | Observed outcome |
| --- | --- | --- |
| Open and configure | Git checkout, custom SQLite path, two named instances and a custom client version | A competing process gets `already_open`. The Codex handshake uses the configured version. An absolute path supplied as a project-relative worktree root is rejected. |
| Send and resume | Codex app-server 0.156.1 recording over JSON-RPC stdio, with the same core revision and configuration in each host | Node sends a prompt. Rust reopens the database and retrieves the original receipt on retry. A second Node process sends a follow-up in the same native session. The sibling thread stays isolated. |
| Replay and page | Recorded Codex conversation plus a disposable database seeded with a retention floor and large sequence numbers | Events arrive in order. Cursors above the JavaScript safe-integer limit round-trip. An expired event cursor and a stale history page produce distinct recovery errors. |
| Answer and stop | Recorded Codex 0.156.1 plan questions and a scripted ACP v1 permission peer, both over stdio | The owning run accepts a response. The SDK rejects stale or mismatched responses. Aborting a wait preserves the pending approval, and stopping the thread interrupts the run. The ACP peer reports session loading and supplies neither a CLI version nor an authentication verdict. |

The addon targets Node-API 8. The macOS build sets a deployment target of 14.0. The packed consumer passed on Node.js 22.14.0, 22.23.5, 24.0.0, 24.22.3, 26.0.0 and 26.13.1 on both macOS 27.0.1 arm64 and emulated Debian Bookworm x64 with glibc 2.36.
