# Stage 6 qualification · Application integrations

An independent Python client sent a message through Huggler’s authenticated HTTP interface, retried it with the same receipt and read the completed conversation. Two Rust hosts also recovered a delegated task after a lost reply and a restart. The parent received its result once. [Operating client](../tests/fixtures/peer-client.py) · [End-to-end assertions](../tests/application_profile.rs) · [Stage acceptance criteria](extraction-plan.md#6-assemble-the-full-application-profile)

Small hosts can use the default crate. The embedding host mounts `remote_router()` to serve peer and guest requests, enables `agent_tools` to start the agent endpoint, and sets `AgentPolicy` to allow secret requests or defer discovery. The `application` Cargo feature adds workspace preparation, schedules, repository clients and the thread-owned service registry. Each service requires an installed host adapter. [Feature definition](../Cargo.toml) · [Runtime options](../src/runtime.rs) · [Service host contract](../src/services.rs)

## Acceptance results

| Integration | Verified outcome | Prerequisites and limits |
| --- | --- | --- |
| Projects and workspaces | Scratch promotion preserved the project ID and file contents. A scripted provider wrote its transcript in a local clone, an existing Git worktree and a managed checkout. A failed setup blocked sends only in its checkout. A delegated thread continued while its parent checkout had a failed setup. An active child blocked setup through a second project ID for its worktree. After setup succeeded, the host retried the blocked send with its original ID. Release refused an occupied checkout, then a dirty checkout, and removed it once clean. Closing the runtime reaped an active setup process and preserved its failure status across reopen. [Workspace scenarios](../tests/application_profile.rs) · [Isolated checkout regression](../tests/delegation.rs) | Git and host-selected setup executables. The host retries setup and resubmits the send. Only managed checkouts can be released. [Workspace API](../src/workspace.rs) |
| Linked environments | Two independent state directories preserved the remote address and original sender. After the receiver committed a delegation and its response was lost, a retry with the same intent recovered the original child and run. A changed task conflicted after the lost receipt. A definitive rejection during checkout preparation let the sender reuse the key with a corrected task. The follower reopened, used a replacement grant and delivered one attributed completion to the parent. Read-only writes, sibling reads and revoked writes were denied. [Remote scenario](../tests/application_profile.rs) | The host supplies grants and polls `follow_remote`. `Peer::loopback` connects locally or to the local end of a host-managed secure tunnel. The crate has no relay or remote cancellation. An isolated remote worktree request returns `Unsupported`. [Peer contract](../src/remote.rs) |
| Guests and authenticated RPC | The independent operating client sent a message, retried it, polled its result and read its history through `/v1/peer`. The same write under a read grant was denied. On `/v1/guest`, an invitation redeemed once. Three submissions yielded two inbox entries. Accepting one and dismissing the other produced one run. A sibling view and attempts after closure were denied. [Operating client](../tests/fixtures/peer-client.py) · [Host and guest assertions](../tests/application_profile.rs) | The embedding host mounts the router and moderates stored submissions. The guest sees messages in the shared thread. [Guest contract](../src/guest.rs) |
| Schedules and webhooks | A one-shot fired once across reopen. A late interval tick coalesced missed occurrences and continued the original session. A fixed-time schedule started a fresh session and skipped the nonexistent 02:30 in Europe/Oslo on March 29, 2026. A signed matching webhook launched a new thread. Duplicate delivery returned the original occurrence, including after destination deletion. Filtered events and invalid signatures started zero runs. Deleting a destination woke its subscription and its `wait_for_change` caller with `NotFound`. Deleting the schedule owner cleared the occurrence history. [Controlled clock and webhook assertions](../tests/application_profile.rs) | The host calls `tick_schedule` with Unix seconds or supplies the configured signing key, signature and exact request bytes to `deliver_webhook`. [Trigger and destination API](../src/automation.rs) |
| SCM and issue trackers | Sixteen recorded HTTP requests covered the first PR page on all seven SCMs, GitHub check runs and conversation comments on GitHub, GitLab, Bitbucket and Forgejo. Linear listed a team’s issues, accepted a comment on one and rejected a write after lookup placed another issue in a different team. The client rejected missing or expired credentials. Linking checked project ownership. After reopen, unchanged and stale watch updates produced zero wakes. One newer change produced one run. Unlinking removed its watch. [Recorded responses](../tests/fixtures/repository-services.json) · [Request and watch assertions](../tests/application_profile.rs) | The host binds a project to a service origin and repository, supplies credentials and feeds ordered watch revisions. Azure DevOps, GitCafe and Cursor Origin support listing. GitHub alone supports check reads. Pagination, inline PR replies, Linear issue creation and state changes are unavailable. Qualification used a local HTTP fixture for all eight services. [Client operations](../src/repository.rs) |
| Secrets and policy | A scripted Claude Agent ACP subprocess used its own run credential to discover eligible tools, send an interim notice and request a secret. A live Codex app-server agent reported the three configured Huggler tools, requested a synthetic secret and returned the approved opaque reference. The host redeemed it. The ACP fixture also verified single redemption and rejection from another project. Denial and expiry reached the caller. Reopened state retained the three outcomes and dark-theme/reduced-motion preferences. A scan of state files found zero copies of the fixture secret. Allowlisted tools remained callable with search disabled. [Agent fixture](../tests/fixtures/peer-acp.py) · [Policy and persistence assertions](../tests/application_profile.rs) | The trusted host redeems raw bytes. The 1Password path used an `op read --no-newline` stand-in. Provider-side injection of raw bytes, a live vault and GPUI preference rendering are unqualified. [Host secret and preference API](../src/application.rs) |
| Terminal, preview, device, HTML and MCP Apps | A Unix subprocess wrote to stdout and stderr. A reconnecting reader received the next bytes using its cursor. A 70,000-byte write retained the last 65,536 bytes and an older cursor expired. A display host navigated two allowed pages, opened a virtual device and rendered approved HTML. Deleting a thread reaped its terminal PID and released its MCP App connection while sibling resources stayed available. Runtime shutdown released the sibling resources. Display hosts also exercised sibling denial, close and host loss. An app resource read and tool call returned data to the host. After a provider startup failure and reopen, only the owner’s next submitted prompt carried the selected JSON. [Host service tests](../tests/host_services.rs) | Terminal output shares one pipe. PTY input and resize are unavailable. The display fixture implements the URL, device and HTML policies. Headless Chromium optionally renders those documents to PNG. Physical devices and interactive browser controls are unqualified. MCP Apps require a host-supplied initialized connection and explicit approval of up to 16 KiB of JSON. [Terminal adapter](../src/services/terminal.rs) · [MCP App adapter](../src/services/mcp_app.rs) |

## Reproduce

Stage 6 passed on macOS 27.0.1 arm64 with Rust 1.94.1. Run from the Huggler root with Git and Python 3 available. The replay suite uses a scripted Codex app-server and a scripted Claude Agent ACP v1 peer, both over stdio. The ACP peer’s MCP client, `peer-fixture` version `1`, calls Huggler over HTTP. The separate Stage 6 live Codex run is listed below. Stage 4 records the earlier provider-driver qualification. [Codex fixture](../tests/fixtures/context-provider.py) · [ACP and MCP fixture](../tests/fixtures/peer-acp.py) · [Driver qualification](stage-4.md#driver-results)

| Command | Result |
| --- | --- |
| `cargo test --locked` | 71 passed, 15 ignored |
| `cargo test --locked --all-features` | 78 passed, 16 ignored. The passing tests include 8 application-profile and 3 host-service cases. |
| Pi extension fixture under Node 22.22.2 | 1 passed in a separate run |
| Headless Chrome 157.0.8103.80 | 1 passed in a separate run. Captures were PNGs and navigating to the second document changed the captured bytes. |
| Authenticated Codex CLI 0.170.0, app-server over stdio and Huggler MCP over HTTP | 1 passed in a separate run. The agent reported the three allowed tools and returned the approved reference. The trusted host redeemed the synthetic bytes. |

The two Cargo totals describe overlapping configurations. Three ignored entries are subprocess entry points exercised by crash tests. The optional commands below select the Pi extension, Chrome and live Codex cases individually. The Codex case requires an authenticated Codex CLI and uses a synthetic value. For a focused replay, run `cargo test --locked --features application --test application_profile --test host_services`.

```sh
cargo fmt --all -- --check
cargo fmt --manifest-path desktop/Cargo.toml -- --check
cargo test --locked
cargo test --locked --all-features
cargo clippy --locked --all-features --all-targets -- -D warnings
CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR="$PWD/target/desktop" \
  cargo clippy --locked --manifest-path desktop/Cargo.toml \
    --features qualification --all-targets -- -D warnings
cargo test --locked --all-features --test provider_instances \
  pi_extension_calls_scoped_tool -- --ignored --exact
HUGGLER_TEST_CHROME='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  cargo test --locked --all-features --test host_services chromium_capture -- --ignored --exact
cargo test --locked --all-features --test application_profile \
  live_codex_managed_secret -- --ignored --exact
```

Formatting and Clippy passed for the crate and desktop. Desktop Clippy built the GPUI qualification target against its default crate configuration. The Stage 6 service scenarios exercised the Rust API and the independent HTTP client. Installed desktop release and upgrade checks belong to Stage 7. [Release criteria](extraction-plan.md#7-validate-cargo-and-desktop-releases)

## Upstream tool coverage

The first column accounts for the 92 declarations in the pinned T3 Code inventory. **MCP** identifies a tool callable by an eligible Huggler agent. Rust method names refer to trusted host entry points on `Huggler` unless another type is named. A host workflow can cover part of a declaration. [Upstream inventory](parity-inventory.md#agent-facing-tools) · [MCP handlers](../src/mcp.rs) · [Host methods](../src/runtime.rs) · [Dispatch actions](../src/contracts.rs)

### Orchestration, threads and schedules

| Upstream tool | Huggler workflow or limit |
| --- | --- |
| `orchestrator_capabilities`, `tool_search` | **MCP** `orchestrator_capabilities` lists eligible names. `tool_search` exposes matching schemas to the calling run. |
| `delegate_task`, `task_status`, `task_cancel` | **MCP** tools of the same names delegate, observe or acknowledge a result, and stop owned local child work. Hosts use `delegate_remote` and `follow_remote` for linked environments. |
| `t3_workflow_stop`, `t3_background_wait` | `stop_thread` stops a local thread subtree. **MCP** `delegate_task` with `mode="wait"` waits for a local delegation. Generic workflow stop and background waits across arbitrary jobs are unavailable. |
| `schedule_task`, `list_scheduled_tasks`, `delete_scheduled_task`, `run_scheduled_task_now` | `create_schedule`, `schedules` and `delete_schedule` manage triggers. `tick_schedule` runs a due occurrence. Early forced execution is unavailable. |
| `request_secret`, `send_to_user` | **MCP** `request_secret` polls for a host decision and an opaque reference. `send_to_user` delivers an interim notice in the caller’s thread. |
| `create_threads`, `t3_thread_list`, `t3_thread_send`, `t3_thread_interrupt` | **MCP** `create_threads`, `thread_list`, `thread_send` and `thread_interrupt` operate within the caller’s project and authority. |
| `t3_thread_read`, `t3_thread_wait`, `t3_thread_search` | `read` returns the thread, `detail` pages or searches one thread, `wait` waits for a run and `wait_for_change` waits past a cursor. Cross-thread text search is unavailable. |
| `t3_thread_update`, `t3_thread_organize` | General metadata edits, archive, tags, pinning and ordering are unavailable. |
| `t3_environment_links`, `t3_environment_link`, `t3_environment_unlink` | The host supplies the sender and target addresses to `approve_link` and revokes a grant with `revoke_link`. Link enumeration is unavailable. |
| `t3_queue` | `read` exposes queued runs. Dispatch `Enqueue`, `EditQueued`, `MoveQueued`, `CancelQueued` and `ResumeQueue`. Promoting queued work to steering is unavailable. |
| `t3_pending_request` | **MCP** `pending_request` lists, reads and answers eligible user questions. Provider permission decisions go through the host. |
| `t3_thread_configure`, `t3_thread_transfer` | `SetModel` configures future submissions. `Fork` and `MergeBack` transfer context at supported boundaries. [Provider and context limits](stage-5.md#model-context-and-files) |

### Projects, worktrees, environment and attachments

| Upstream tool | Huggler workflow or limit |
| --- | --- |
| `t3_project_list`, `t3_project_read`, `t3_project_create` | `open_project` registers or resolves a known root and returns its `Project`. `create_scratch` creates a project directory and `promote_scratch` initializes Git there. General project listing and metadata lookup are unavailable. |
| `t3_project_update`, `t3_project_delete` | General project metadata updates and deletion are unavailable. |
| `t3_project_clone`, `t3_thread_launch` | `clone_project` clones to a new directory. `launch` or dispatch of `Launch`, `LaunchProvider` or `LaunchInstance` starts a thread. |
| `t3_worktree_status`, `t3_worktree_list`, `t3_worktree_handoff` | `workspace_setup_status` reports preparation. `allocated_workspaces` lists managed checkouts for a repository. General branch-status inspection and thread handoff between checkouts are unavailable. |
| `t3_environment_read`, `t3_environment_preferences_update`, `t3_theme_read`, `t3_theme_set` | `address` identifies a thread’s environment. `preferences` and `set_preferences` read and persist theme and reduced motion. General environment inspection and a theme catalog are unavailable. |
| `t3_attachment_prepare_upload`, `t3_attachment_discard`, `t3_thread_send_attachments` | `prepare_upload`, `discard_upload` and `SendAttachments` support thread-owned text uploads. [Attachment limits](stage-5.md#human-input-and-attachments) |

### Issues, pull requests and CI

| Upstream tool | Huggler workflow or limit |
| --- | --- |
| `list_issues`, `read_issue` | `RepositoryClient::list` reads the first Linear issue page and returns identifier, title and state. Individual issue detail is unavailable. |
| `create_issue`, `comment_on_issue`, `set_issue_state` | `RepositoryClient::comment_on_issue` resolves the issue, checks its current team against the project binding and posts a comment by service ID. Issue creation and state changes are unavailable. |
| `link_issue`, `unlink_issue`, `list_thread_issues` | `link_repository_item`, `unlink_repository_item` and `repository_links` manage thread links across the project’s bindings. The host selects links for the issue tracker. |
| `link_pull_request`, `link_pull_requests`, `unlink_pull_request`, `unlink_pull_requests`, `list_thread_pull_requests` | The same three link methods manage PRs. A host repeats single-item operations for a batch. |
| `watch_pull_request`, `unwatch_pull_request` | `watch_repository_item` and `unwatch_repository_item` manage a linked item’s watch. The host supplies a revision and state to `sync_repository_watch`. |
| `pull_request_list`, `pull_request_read` | `RepositoryClient::list` reads the first PR page with number, title and state for each SCM driver. Individual PR detail is unavailable. |
| `pull_request_respond` | `RepositoryClient::comment` posts a conversation comment on GitHub, GitLab, Bitbucket and Forgejo. Inline replies and comments on the other three SCMs are unavailable. |
| `pull_request_create`, `pull_request_update`, `pull_request_review`, `pull_request_merge` | PR creation, edits, formal review submission and merge are unavailable. |
| `write_pull_request_guides`, `read_pull_request_guide` | PR guide storage and retrieval are unavailable. |
| `ci_logs`, `ci_rerun` | `RepositoryClient::checks` reads GitHub check runs for a commit. Job log output and reruns are unavailable. |

### Preview, device and HTML hosts

| Upstream tool | Huggler workflow or limit |
| --- | --- |
| `preview_status`, `t3_preview_list`, `preview_open`, `t3_preview_close` | `service_handles` lists the thread’s open previews. `open_service(Open::Preview)` and `close_service` manage their lifetime. Detailed browser state is unavailable. |
| `preview_navigate`, `preview_snapshot` | `service_call` with `Navigate` or `Capture` navigates and returns host-defined capture bytes. The display fixtures return HTML or PNG. Accessibility-tree snapshots are unavailable. |
| `preview_dialog`, `preview_resize`, `preview_set_appearance`, `preview_act`, `preview_evaluate`, `preview_wait_for`, `preview_recording_start`, `preview_recording_stop` | The service interface has no dialog, resize, appearance, browser input, script evaluation, element wait or recording operations. |
| `device_list`, `device_open`, `device_screenshot`, `device_close` | `devices`, `open_service(Open::Device)`, `service_call` with `Capture`, and `close_service` use the installed device adapter. Tested with a virtual display. |
| `html_preview`, `html_render` | `open_service(Open::Html)` and `service_call` with `Capture` present and render a document according to host policy. |
