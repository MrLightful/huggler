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

## Verify language packages

Run the installed consumer for the binding you changed. The packaging guides cover prerequisites, distribution artifacts, and fixture coverage. [Python release wheels](python-sdk.md#build-and-test-from-a-checkout) · [TypeScript tarballs](typescript-sdk.md#build-and-test-from-a-checkout)

## Publish the Rust crate

The [Publish crate workflow](../.github/workflows/publish-crate.yml) runs on pushed
`v*` tags in `MrLightful/huggler`. It requires the tag to equal `v` followed by the
root Cargo.toml version, runs formatting, Clippy, default and all-feature tests,
and checks the packaged independent consumer before publishing to crates.io.
The existing Release candidates workflow only uploads artifacts.

### One-time setup

Create a GitHub Actions environment named `crates-io` in the repository settings.
Restrict its deployment tags to `v*` and configure any desired required reviewers.

For a new crate, crates.io requires an API token for the initial publication.
Sign in to crates.io, verify your email and create a short-lived token allowing
publication of the new `huggler` crate. Save it as the environment secret
`CRATES_IO_BOOTSTRAP_TOKEN`. The workflow uses this secret when present.

After the first successful publication, open the crate's Settings → Trusted
Publishing on crates.io and add this GitHub publisher:

| Field | Value |
| --- | --- |
| Repository owner | `MrLightful` |
| Repository name | `huggler` |
| Workflow filename | `publish-crate.yml` |
| Environment | `crates-io` |

Remove the bootstrap secret and revoke its API token after configuring the
publisher. Subsequent releases use a temporary OIDC token, obtained only after
verification. See the [crates.io Trusted Publishing documentation](https://crates.io/docs/trusted-publishing).
For an already published crate you own, configure this publisher directly and
omit the bootstrap secret.

### Release a version

The initial release uses the existing `0.0.1` version. For subsequent releases,
pass the next version to the version script from the repository root:

```sh
python3 scripts/version-crate.py 0.0.2
```

The script updates the root Cargo manifest and the local Huggler entry in all
four Cargo lockfiles. Run it without an argument to check that they agree.
Commit and merge version changes into `master`. From the reviewed release commit,
push the tag matching the crate version:

```sh
git tag -a v0.0.1 -m "Huggler 0.0.1"
git push origin v0.0.1
```

Monitor **Actions → Publish crate**, then confirm the version on crates.io.
For a transient failure before publication, rerun the failed workflow jobs.
Published versions cannot be overwritten; fixes require a new version and tag.
The workflow does not create a GitHub Release or publish the desktop or language
bindings.

## macOS desktop checks

Use Xcode and a macOS GUI session. Build the qualification binary once for the [desktop coding checks](desktop-coding.md#reproduce-the-desktop-check) and the history check.

```sh
cargo fmt --manifest-path desktop/Cargo.toml -- --check
cargo clippy --locked --manifest-path desktop/Cargo.toml --features qualification --all-targets -- -D warnings
cargo build --locked --manifest-path desktop/Cargo.toml --features qualification --target-dir target/desktop
python3 scripts/check-desktop-history.py target/desktop/debug/huggler-desktop
```

The response image check uses a scripted Codex app-server and a local HTTP fixture. It verifies previews while streaming and after reopening history, fallback for missing and out-of-checkout files, and user-initiated web loading.

```sh
python3 scripts/check-desktop-images.py target/desktop/debug/huggler-desktop
```

The Mermaid check verifies the open-fence source view, rendering after the fence closes, the source toggle and clipboard, malformed-diagram fallback, and saved history at a narrow width.

```sh
python3 scripts/check-desktop-mermaid.py target/desktop/debug/huggler-desktop
```

The merged-PR settlement check drives the desktop with a local GitHub fixture. It verifies selected and background conversations, the persisted General setting, closed PRs, and edits or branch switches during a status request. It tracks each app PID and verifies exit.

```sh
python3 scripts/check-desktop-pull-requests.py target/desktop/debug/huggler-desktop --settlement-only
```

### Capture a desktop screenshot baseline

Keep the macOS session unlocked and grant Screen Recording access to the process
hosting the check. Run the suite with the qualification binary built above.

```sh
python3 scripts/check-desktop-screenshots.py target/desktop/debug/huggler-desktop .context/desktop-baseline
```

The suite drives the real window with an isolated checkout, preferences, and a
recorded provider fixture. It captures the starter and conversation setup, every
Settings page, project, branch and model pickers, the conversation and attention
states, file tree and changes panel, document and diff views, terminal, MCP status,
and image and PDF attachments. It also saves standalone component crops. Use
`--appearance light` for a separate light-mode baseline.

After a change, rebuild the qualification binary and compare a new capture with
the baseline.

```sh
python3 scripts/check-desktop-screenshots.py target/desktop/debug/huggler-desktop \
  .context/desktop-current --baseline .context/desktop-baseline
```

Use an empty output directory each time. The comparison reports added, removed
and changed images and exits with status 1 if any differ. Compare captures using
the same macOS version, display scale, time zone and appearance. The qualification
build fixes transcript timestamps and durations during capture and hides the
blinking input caret. The comparison uses exact decoded pixel values. Use
`--check-only` in place of the output directory to exercise the scene checkpoints
and validate component bounds without Screen Recording access.

To add a scene, put a named `shot` in the script after the actions that open it
and supply a visible control as the readiness check. Pass `component=` with an
observed control name to save just its bounds. Register a new control with
`control!` or use `observe_bounds` for custom-painted components. See [the screenshot suite](../scripts/check-desktop-screenshots.py).

### Run provider and installation checks

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
cargo install --locked cargo-about --version 0.9.1 --features cli
cargo fetch --locked --manifest-path desktop/Cargo.toml
python3 scripts/check-desktop-release.py target/distribution
```

The checker creates the app and Pi extension ZIPs, verifies the bundle signature and launches the extracted app with a fresh home and a system-only PATH. The 0.0.1 check ran on an Apple Silicon development Mac. Qualification on a separate clean Mac, the macOS deployment floor and Gatekeeper assessment of a notarized download remain open. [Production checker](../scripts/check-desktop-release.py) · [0.0.1 qualification](https://github.com/MrLightful/huggler/blob/ef2ffe78541842fe3b13d7468e8fd330b0e674d4/docs/stage-7.md)

Run **Release candidates** from the GitHub Actions tab to download the verified crate and archive inventory, along with the ad-hoc signed macOS app and Pi extension ZIPs. The macOS job uses `--package-only`. Run the command above in a macOS GUI session to also verify launch, database creation, and quit. [Release candidates workflow](../.github/workflows/release-candidates.yml)
