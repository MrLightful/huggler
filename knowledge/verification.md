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

The [Release crate workflow](../.github/workflows/publish-crate.yml) runs manually
on `master`. It reads the version from the root Cargo manifest, verifies the
crate and an independent consumer, and publishes to crates.io. It then creates
the matching `v<version>` tag and a GitHub Release with generated notes on the
published commit.

### One-time setup

In the GitHub Actions environment named `crates-io`, select **Selected branches
and tags** and allow the `master` branch. Remove any older `v*` tag rule.

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
three Cargo lockfiles. Run it without an argument to check that they agree.
Commit and merge version changes into `master`. In GitHub, open **Actions →
Release crate → Run workflow**, select `master`, and start the run. Confirm the
version on crates.io and the matching tag and release on GitHub.

If the GitHub release job fails after the crate publishes, use **Re-run failed
jobs** on the same run. That job can reuse a matching tag from a partial attempt.
Published versions cannot be overwritten. Fixes to a published crate require a
new version.

## Package the crate and Pi extension

The Cargo check requires Python 3.12 or later. It verifies that migrations, fixtures and license files survive packaging, then builds an independent Rust consumer of the archive and replays a Codex restart. [Package check](../scripts/check-package.py)

```sh
python3 scripts/check-package.py
```

Run **Release candidates** in GitHub Actions to download the verified crate, its archive inventory, and the Pi extension ZIP. [Release candidates workflow](../.github/workflows/release-candidates.yml)

[Desktop qualification and packaging](https://github.com/MrLightful/bear/blob/main/docs/verification.md)
