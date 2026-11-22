# Python SDK packaging and qualification

The Python package embeds Huggler with Cargo's `application` feature enabled. It exposes all public `Huggler` methods and uses the same action, event and persistence contracts as Rust. Python hosts choose the timer, HTTP server and external credentials, and supply host callbacks for preview and device integrations. [Python documentation](../docs-app/content/docs/bindings/python/index.mdx) · [Consumer and orchestration boundary](README.md#consumer-and-orchestration-boundary)

## Build and test from a checkout

Use Rust 1.94.1, a C toolchain, Git and CPython 3.11 or later. The release builder uses the locked Cargo graph to include the dependency notices. Run these commands from the repository root.

```sh
python3 -m venv .context/python
.context/python/bin/python -m pip install 'maturin==1.16.0'
rustup component add rust-docs
cargo install --locked cargo-about --version 0.9.1 --features cli
.context/python/bin/python bindings/python/scripts/build-wheel.py
.context/python/bin/python -m pip install --no-deps bindings/python/dist/*.whl
.context/python/bin/python bindings/python/tests/consumer.py -v
```

Build the Linux x86-64 release wheel on Debian Bookworm with `--compatibility manylinux_2_36`. The Apple Silicon wheel targets macOS 14. The builder uses Rust's bundled LLVM linker on Apple Silicon. Set `SDKROOT` to a compatible SDK if the default Xcode SDK is newer than that linker supports. The builder detects the Command Line Tools macOS 26 SDK when installed.

Successful **Python bindings** workflow runs attach the installed and tested wheels. [CI workflow](../.github/workflows/python-bindings.yml)

## Verified workflows

| Workflow | Harness and transport | Consumer check |
| --- | --- | --- |
| Continue a session, replay events, and answer a planning question | Recorded Codex app-server 0.156.1 over JSON-RPC stdio | The second turn resumes the original native session. Retries return the original receipt, expired cursors report recovery errors, and a plan run accepts the answer through the native question flow. |
| Handle a permission and stop a run | Scripted ACP v1 over stdio, with no reported CLI version or authentication verdict | Cancelling a Python wait preserves the pending permission. Explicit interruption stops the run. |
| Deliver webhooks, attach files, and accept a guest submission | Scripted Codex app-server over stdio and the native guest router through ASGI | Signed deliveries deduplicate, attachment bytes round-trip, and a host text callback formats an approved guest message. |
| Approve and consume a secret | Scripted ACP v1 over stdio with scoped agent tools over loopback HTTP | A Python host approves a request and consumes its reference once. |
| Use repository and service integrations | Local Git checkout, a scripted loopback GitHub GraphQL response, native terminal host and Python callbacks | The original pull request snapshot guards the merge. Managed checkouts, resource cleanup, terminal output and MCP App permissions work through the installed package. |

The [consumer suite](../bindings/python/tests/consumer.py) also checks native peer authorization through ASGI, Python integer cursors above `2**53` and async iterator cancellation. The provider runs use recordings and scripted peers.

The initial 10-test suite passed against release wheels on macOS 27.0.1 arm64 with CPython 3.13.18 and 3.15.2, and on emulated Debian Bookworm x86-64 with CPython 3.11.2 and glibc 2.36.

The expanded 13-test suite adds regressions for cancellation during event delivery, implicit shutdown during a Python callback, and ASGI routing and body limits. It passed against the rebuilt macOS release wheel on both Python versions and an installed Linux development wheel on the same Debian setup.
