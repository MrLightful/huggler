# Stage 7 qualification · Huggler 0.0.2

**Ready to merge.** The independent Cargo consumer resumed a recorded conversation from the packaged crate. On an Apple Silicon development Mac, the unzipped production app opened a fresh database and quit. An installed GPUI qualification build then continued a live Codex conversation created by 0.0.1. The maintainer accepted this Mac for the desktop check. A separate clean machine, older macOS versions and a quarantined Gatekeeper launch remain untested. These results cover local release artifacts. [Acceptance criteria](extraction-plan.md#7-validate-cargo-and-desktop-releases)

## What ships

Sizes are bytes. The unpacked column sums regular file lengths and excludes filesystem allocation. Cargo records the Git revision in the archive. The crate measurement below comes from clean commit `81a299e`.

| Artifact | Archive | Unpacked | Verified contents |
| --- | ---: | ---: | --- |
| `huggler-0.0.2.crate` | 222,322 | 1,064,173 | 140 files including Cargo.lock, Rust source, all 35 SQL migrations, tests, fixtures and licenses. [Complete file list](reference/stage-7/huggler-0.0.2.contents.txt) |
| `huggler-0.0.2-macos-arm64.zip` | 10,509,329 | 28,778,277 | Six files in Huggler.app. The executable links 18 libraries and frameworks under macOS system paths. [Packaging and launch check](../scripts/check-desktop-release.py) |
| `huggler-0.0.2-pi.zip` | 1,752 | 3,074 | `huggler.mjs` and LICENSE. The check extracted and compared the bytes of both entries. The Pi adapter accepts the installed extension’s absolute path. [Pi adapter qualification](stage-4.md#controls-and-fallbacks) |

The production bundle contains the executable (27,331,760), Info.plist (641), CodeResources (2,936), Huggler’s LICENSE.txt (1,077), Rust-library.html (425,661) and Third-party notices.txt (1,016,202). The app embeds the default Rust library. The optional `application` feature is available to other Cargo hosts. The app and the crate ship independently of Node, external provider executables and the Pi extension. [Desktop manifest](../desktop/Cargo.toml) · [Crate manifest](../Cargo.toml)

Huggler’s own code and Pi extension carry MIT terms. The crate preserves T3 Code’s MIT notice with the Codex resume, questions, subagent and provider recordings. For the desktop bundle, cargo-about used the locked macOS target and selected features. The notices list 538 registry packages including build dependencies, with source archive URLs, selected license texts and notices found in the package trees. When a package omits a text, the generator identifies SPDX standard text where used and adds version-specific upstream supplements. It includes source links for MPL components, the SQLite public-domain attribution and the Rust toolchain’s standard library notice. The installer signs after adding resources. [Huggler license](../LICENSE) · [Original fixture notice](reference/codex-resume/LICENSE) · [Notice generator](../scripts/desktop_notices.py) · [Supplement sources](../desktop/upstream-notices.toml)

## Compatibility and installed results

The Cargo API and desktop version are 0.0.2, macOS bundle build 2. The crate declares Rust 1.94.1, the compiler used for both builds. The persisted SQLite version is 35 in both desktop installations. The upgrade exercises preservation across app replacement at that schema. Earlier database fixtures exercise migration separately. Hosts can mount the authenticated JSON endpoints at `/v1/peer` and `/v1/guest`. Agents negotiate MCP during initialization on the streamable HTTP endpoint at `/mcp`. GPUI calls Rust directly. [Migration coverage](stage-5.md#existing-data) · [Peer interface](../src/remote.rs) · [Guest interface](../src/guest.rs) · [Agent endpoint](../src/mcp.rs)

| Run | Evidence |
| --- | --- |
| Production startup | On macOS 27.0.1 (26A434), arm64, the checker extracted the ad-hoc signed ZIP into a path with spaces, verified its signature with `codesign --verify --strict`, and launched the installed executable from `/` in a GUI session. With a fresh home and only system directories on PATH, it created `~/Library/Application Support/dev.huggler.stage2/huggler.sqlite3` at the default location. AppKit Quit exited successfully. The Mach-O deployment target is 11.0. Compatibility at that floor, Developer ID signing and notarization are unqualified. |
| Upgrade with a live provider | With the existing signed-in Codex CLI 0.170.0 over app-server stdio, launch 1 created a conversation on the Stage 6 app (0.0.1, build 1). Launch 2 replaced the bundle at the same path, displayed the prior answer and resumed the same application and native thread with a distinct native turn. Launch 3 created an isolated sibling. Launch 4 restored the selected sibling and switched back to the two-turn history. Each launch exited. The scripted GPUI build explicitly used the fresh home’s expected Application Support path. PATH contained the native Codex executable and system tools. [Upgrade runner](../scripts/check-desktop.py) |
| Permissions | The live cloud Codex 0.170.0 run recorded the denial, interrupted the requesting run and left the proposed file absent. Separately, the Codex app-server 0.156.1 recording showed the native grandchild `/root/relay/writer` in the window. Approving once completed both children with `Written.`. [Permission checks](../scripts/check-permission.py) |
| Provider discovery | In the installed scripted window, selecting a missing Cursor executable produced a diagnostic with its path. A registry replay peer reported ACP v1 and two models, with login unverified. [Discovery check](../scripts/check-provider-desktop.py) |

macOS emitted the diagnostic `sandbox_extension_issue_file_to_process failed` for the temporary app paths. The production process created the database and exited successfully, and all four scripted upgrade launches completed.

Only the live Codex rows above used a model account in Stage 7. Replays run offline after build. Other harnesses need their own installed executable or configured server and whatever login the selected provider requires. Earlier clean-home probes of Cursor and Antigravity reached a login challenge. Muse’s installed smoke used its deterministic echo provider. The per-driver record names versions, transports, operations, native capabilities and fallbacks for all eleven kinds, including the two OpenCode generations. [Driver results](stage-4.md#driver-results) · [Earlier live Claude and Codex runs](stage-3.md)

## Capability coverage at release

The six groups below follow the original T3 Code inventory. Its counts still refer to `338352dee2`. Each result describes what a caller can exercise in Huggler. [Baseline inventory](parity-inventory.md)

| Inventory group | Passing consumer scenario | Limits |
| --- | --- | --- |
| Thread commands | The packaged consumer launches, sends, reopens and retries the original command ID. Queue tests edit and reorder held messages. A workflow stop interrupts descendants while an independent peer continues. [Consumer assertions](../tests/support/resume.rs) · [Control results](stage-5.md#queue-dispatch-and-stop) | Setup retry and resubmission are host operations. Thread organization and standalone workflow scripts remain unavailable. |
| RPC and HTTP | An independent HTTP client retried a peer send and read its history. Read-only grants rejected writes. A guest could submit to the shared thread and the host could accept or dismiss it. [Remote and guest checks](stage-6.md#acceptance-results) | The host mounts the router. Linked reads use direct grants or a host-managed tunnel. A hosted relay and remote cancellation remain unavailable. |
| Thread events | A cursor replay reconstructed the same conversation as a fresh read after reopening. An expired cursor required a new snapshot. [History and transport](stage-5.md#reads-and-transport) | Search covers one thread. Huggler uses its own event sequence and storage format. |
| Durable effects | A process killed around provider I/O recovered the committed receipt and emitted one outbound prompt in total. A recovered delegated result woke the idle parent once. [Crash and process loss](stage-5.md#crash-and-process-loss) | Ambiguous provider delivery is reported to the host. The upgrade tests Huggler-owned state. Direct T3 database import is unsupported. |
| Agent tools | Scoped callers sent peer replies, delegated and consumed results. The host applied a three-tool policy for a live Codex secret-reference request. [Collaboration](stage-3.md#cross-harness-collaboration) · [92-tool mapping](stage-6.md#upstream-tool-coverage) | The mapping identifies trusted-host APIs and unavailable workflows, including several SCM mutations and preview operations. |
| Providers and usage | The suite replayed instance discovery, approval, streamed text and native continuation for capable adapters. Token totals came from reporting runs. The separate Pi extension called a scoped tool. [Driver results](stage-4.md#driver-results) | Native child control is qualified for Codex and Claude Agent. Huggler does not provide upstream’s eight aggregate usage readers. |

## Reproduce

Build on a Mac with Rust/Cargo 1.94.1, Xcode, Git, Python 3.12+ and a GUI session. This run used Xcode 27.2 and Python 3.15.1. Fetch dependencies once before the frozen license scan. The production check writes both ZIPs to the output directory after the unpacked app exits.

```sh
rustup component add rust-docs
cargo install cargo-about --version 0.9.1 --locked
cargo fetch --locked --manifest-path desktop/Cargo.toml
python3 scripts/check-package.py
diff -u docs/reference/stage-7/huggler-0.0.2.contents.txt target/package/huggler-0.0.2.contents.txt
python3 scripts/check-desktop-release.py target/distribution
```

The independent consumer builds in a temporary directory from extracted source. Its Cargo.lock starts with the packaged lockfile. The check verifies each resolved registry package’s name, version, source and checksum against that lock. A fixture supplies the only Codex peer used by the consumer. [Package check](../scripts/check-package.py)

For the upgrade, build the prior qualification app from the Stage 6 merge. Run these commands from the current checkout and keep the previous build and the qualification bundle in the temporary directory. The live upgrade and denial require a signed-in native Codex CLI on PATH or at `HUGGLER_CODEX`. The allow and discovery checks use fixtures.

```sh
previous=$(mktemp -d)
git archive 6c013a1fd52406cded27af0627c153be056d0940 | tar -x -C "$previous"
python3 - "$previous" <<'PY'
from pathlib import Path
import runpy, sys
root = Path(sys.argv[1])
runpy.run_path(str(root / "scripts/install-desktop.py"))["install"](root / "Huggler.app", qualification=True)
PY
python3 scripts/check-desktop.py --previous-app "$previous/Huggler.app"
python3 scripts/check-permission.py codex
python3 scripts/check-permission.py recorded-codex
python3 scripts/install-desktop.py --qualification "$previous/Discovery.app"
python3 scripts/check-provider-desktop.py "$previous/Discovery.app/Contents/MacOS/huggler-desktop"
```

The default suite passed 71 tests with 15 ignored, and the all-features suite passed 78 with 16 ignored. The optional Pi and Chromium cases passed separately. Node 22.22.2 was available to the Pi check. Formatting and strict Clippy passed for the library and the desktop qualification build.

```sh
cargo test --locked
cargo test --locked --all-features
cargo test --locked --test provider_instances pi_extension_calls_scoped_tool -- --ignored --exact
HUGGLER_TEST_CHROME='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  cargo test --locked --all-features --test host_services chromium_capture -- --ignored --exact
cargo fmt --all -- --check
cargo fmt --manifest-path desktop/Cargo.toml -- --check
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo clippy --locked --manifest-path desktop/Cargo.toml --features qualification --all-targets -- -D warnings
```

## Upstream comparison

The release review covered all 31 commits after the inventory pin `338352dee2f43079482e054e8229a2ba055641d9` through `22dcb0af5a836ec10a38674e219c123ba67a49de`. The implementation and regression assertions were compared at the candidate. The Huggler suites above reran the related Rust workflows. The upstream review record retains the full SHA and a concrete acceptance check for each deferred change. [Upstream diff](https://github.com/pingdotgg/t3code/compare/338352dee2f43079482e054e8229a2ba055641d9...22dcb0af5a836ec10a38674e219c123ba67a49de) · [Outstanding checks](upstream-review.json)

| Upstream behavior and commits | Release disposition |
| --- | --- |
| ACP commandless activity labels (`9294a980`) and delayed command discovery after load (`6d6ca609`) | **Deferred.** The shared ACP tests cover model discovery, permission, streaming and load. Huggler lacks the command activity timeline and native compaction in those regressions. |
| Antigravity transcript tail alignment (`ad4d0f06`) and retry after mid-turn 404 (`f68d36d8`), Pi terminal child status (`c53ac237`), Claude plugin-directory skill discovery (`a1b9e1cc`) | **Deferred.** Antigravity’s vendor transcript and retry policy, Pi’s native child snapshots and the Claude composer skill catalog need their own provider cases. The release exercised the existing ACP and Pi protocols. |
| Linked read relay (`1cb3a8c7`) and pairing from a read-only thread (`d7f6b3ef`) | **Deferred.** Direct peer reads, denied writes and revocation passed on separate Huggler runtimes. Routing through another environment and redeeming full-access pairing from that view are outstanding. |
| Terminal PR watch enrollment (`b076d8c8`), merge with abbreviated head SHA (`5e59fc67`) and fork-owned review guides (`d984b713`) | **Deferred.** The host feeds ordered PR observations and the watch test verifies deduplication and stale revision handling. Remote enrollment checks, PR merge and review guides are outside the implemented client. |
| Cursor SDK worker isolation (`29af131f`) | **Host-only** for the upstream SDK worker. Huggler already launches Cursor through an external ACP process. The scripted adapter exercised streaming, interruption and per-instance capability checks. The installed CLI’s live coverage stops at the login challenge. |
| Disabled providers in text generation (`6f5293fb`), PR tool wording (`b985057d`), startup auto-pull (`f2ec853f`) | **Host-only.** These changes target upstream’s separate generator selection, its PR agent toolkit and the startup Git service. Huggler samples suggestions on an eligible Codex run, exposes repository operations through host APIs and leaves automatic pulls to the host. |
| Link opening, navigation, environment views, preview, updater, deregistration, theme and usage banners (`0702c6fd`, `7c6e889d`, `ccabed08`, `39252676`, `27135704`, `a1acd7aa`, `21d7feaf`, `8d37736c`) | **Host-only.** These change web, mobile and Electron behavior. The release target is the native GPUI app. |
| Log writes, speech, token analytics, managed tunnels and Connect status, eval credentials and proxy selection, launch videos (`90c028c6`, `d20e6c29`, `46b62626`, `13e940a4`, `921a54c2`, `6788be77`, `0fa2b92f`, `22dcb0af`) | **Host-only.** These changes affect upstream services, evaluation or presentation outside the distributed artifacts. |

To repeat the comparison, set `upstream_repo` to a T3 Code checkout containing both revisions. Normal maintenance starts from the updated review marker. [Review workflow](upstream-watch-guide.md#adapt-and-merge-a-relevant-change)

```sh
python3 scripts/review-upstream.py "$upstream_repo" --all \
  --base 338352dee2f43079482e054e8229a2ba055641d9 --to 22dcb0af5a836ec10a38674e219c123ba67a49de
```
