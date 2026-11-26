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

[Release Please](../.github/workflows/release-please.yml) maintains a release
pull request on `main` with the version bump, all three Cargo lockfile updates
and the [changelog](../CHANGELOG.md). Merging that pull request creates the
GitHub release and its `v<version>` tag. The
[Release crate workflow](../.github/workflows/publish-crate.yml) verifies and
publishes that tagged commit to crates.io when run manually.

### One-time setup

Release Please uses the built-in `GITHUB_TOKEN`. Enable **Allow GitHub Actions
to create and approve pull requests** in the repository's Actions settings.
On a generated release pull request, select **Approve workflows to run** to
start CI.

In the GitHub Actions environment named `crates-io`, select **Selected branches
and tags** and allow the `main` branch.

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

Use Conventional Commit subjects for releasable work, such as `feat:` for
features and `fix:` for fixes. Use `feat!:` or a `BREAKING CHANGE:` footer for
breaking changes. While the crate is below 1.0, breaking changes bump the minor
version. Release Please opens its first pull request once it finds a releasable
commit after the existing `v0.0.1` tag.

1. Review the Release Please pull request before merging. Check the version and
   the generated changelog. Fold any entries under **Unreleased** into the
   versioned entry and keep the notes in the pull request description in sync
   with the changelog, since Release Please uses that description for GitHub.
2. Merge the pull request and wait for the Release Please run on `main`. Check
   that the release notes and tag appear on GitHub.
3. Open **Actions → Release crate → Run workflow**, select `main`, and enter
   the GitHub release tag. Confirm that the same version appears on crates.io.

If publishing fails before crates.io accepts the upload, rerun the failed jobs
or dispatch the same tag again. Check crates.io before retrying if the upload
result is unclear. Published versions require a new release for corrections.

## Package the crate and Pi extension

The Cargo check requires Python 3.12 or later. It verifies that migrations, fixtures and license files survive packaging, then builds an independent Rust consumer of the archive and replays a Codex restart. [Package check](../scripts/check-package.py)

```sh
python3 scripts/check-package.py
```

Run **Release candidates** in GitHub Actions to download the verified crate, its archive inventory, and the Pi extension ZIP. [Release candidates workflow](../.github/workflows/release-candidates.yml)

[Desktop qualification and packaging](https://github.com/MrLightful/bear/blob/main/docs/verification.md)
