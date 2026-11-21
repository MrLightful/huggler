# Verify Huggler changes

## Rust replay checks

Run from the Huggler root with the Rust version in `Cargo.toml`, Git and Python 3 on PATH. These commands use fixtures and temporary checkouts.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo test --locked
cargo test --locked --all-features
```

| Workflow | Test target or source |
| --- | --- |
| Codex resume, receipts, history cursors and process loss | [`resume`](../tests/resume.rs), [`runtime`](../tests/runtime.rs), [commit boundaries and migrations](../src/store/crash_tests.rs) |
| Peers, delegation and native children | [`peers`](../tests/peers.rs), [`delegation`](../tests/delegation.rs), [`native_children`](../tests/native_children.rs), [`acp_native_children`](../tests/acp_native_children.rs) |
| Driver discovery, streaming and continuation | [`provider_instances`](../tests/provider_instances.rs), [provider fixtures](../tests/fixtures/providers/) |
| Permissions, questions, forks, attachments and imports | [`permissions`](../tests/permissions.rs), [`questions`](../tests/questions.rs), [`context`](../tests/context.rs) |
| Remote and guest access, application integrations and host services | [`application_profile`](../tests/application_profile.rs), [`host_services`](../tests/host_services.rs) with `--features application` |

The optional Pi extension fixture needs Node on PATH. The optional display test needs a Chromium executable at `HUGGLER_TEST_CHROME`.

```sh
cargo test --locked --test provider_instances pi_extension_calls_scoped_tool -- --ignored --exact
cargo test --locked --features application --test host_services chromium_capture -- --ignored --exact
```

## macOS desktop checks

Use Xcode and a macOS GUI session. Build the qualification binary once for the [desktop coding checks](desktop-coding.md#reproduce-the-desktop-check) and the history check.

```sh
cargo fmt --manifest-path desktop/Cargo.toml -- --check
cargo clippy --locked --manifest-path desktop/Cargo.toml --features qualification --all-targets -- -D warnings
cargo build --locked --manifest-path desktop/Cargo.toml --features qualification --target-dir target/desktop
python3 scripts/check-desktop-history.py target/desktop/debug/huggler-desktop
```

The peer and recorded permission checks build and install their own temporary qualification bundles. Set up the release prerequisites below before running them.

```sh
python3 scripts/check-desktop-peers.py
python3 scripts/check-permission.py recorded-codex
```

For live checks, use an authenticated native Codex CLI on PATH or at `HUGGLER_CODEX`. The restart check verifies continuation after quit and reopen. To verify an upgrade, pass `--previous-app` with a signed qualification bundle with the same bundle identifier and a lower build number. The script works on a temporary installation.

```sh
python3 scripts/check-desktop.py
python3 scripts/check-permission.py codex
```

## Package the crate and desktop app

The Cargo check requires Python 3.12 or later. It verifies that migrations, fixtures and license files survive packaging, then builds an independent Rust consumer of the archive and replays a Codex restart. [Package check](../scripts/check-package.py)

```sh
python3 scripts/check-package.py
```

On macOS, install the Rust license documentation and notice generator, fetch the locked desktop dependencies, then run the production packaging check in a GUI session.

```sh
rustup component add rust-docs
cargo install --locked cargo-about --version 0.9.1
cargo fetch --locked --manifest-path desktop/Cargo.toml
python3 scripts/check-desktop-release.py target/distribution
```

The checker creates the app and Pi extension ZIPs, verifies the bundle signature and launches the extracted app with a fresh home and a system-only PATH. The 0.0.1 check ran on an Apple Silicon development Mac. Qualification on a separate clean Mac, the macOS deployment floor and Gatekeeper assessment of a notarized download remain open. [Production checker](../scripts/check-desktop-release.py) · [0.0.1 qualification](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-7.md)
