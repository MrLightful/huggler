# Remote machines over SSH

Huggler Desktop can open a Git checkout on a Unix machine running `hugglerd`. The remote service runs Codex and stores conversation history in its own SQLite database. The desktop reconnects to the same environment and refreshes conversation snapshots after a connection loss. Disconnecting or closing the desktop leaves the remote service and its active runs going.

## Prepare the remote host

Install Git and Codex on the remote host and sign in to Codex as the user who will run the service. That account needs access to the checkout and a working OpenSSH key or agent identity from the desktop. Enable SSH Unix socket forwarding for the account.

Build and install the native daemon for that machine using a Huggler source checkout and the Rust toolchain specified in `Cargo.toml`.

```sh
cargo install --locked --path . --features machine --bin hugglerd
```

Make `hugglerd` and `codex` available on the remote account’s PATH. In particular, `ssh <alias> hugglerd socket-path` must resolve `hugglerd` in a non-interactive shell. Run `hugglerd serve` under a user process supervisor or in a persistent terminal session. Start with the default state directory at `~/.huggler/daemon`. To use a custom directory, set `HUGGLER_MACHINE_STATE_DIR` to the same absolute path for the service and the SSH command. The state directory is private to the remote account. To choose a different Codex executable, set `HUGGLER_CODEX` on the service.

Run `ssh <alias>` once from a local terminal to verify the host key and confirm that authentication works. For a custom port use `ssh -p <port> <user>@<host>`. Desktop reuses OpenSSH configuration and requires a trusted host key and non-interactive authentication.

## Open a remote conversation

1. Open **Settings → Machines → Add SSH machine**.
2. Enter an alias or `user@host`, an optional SSH port, and the absolute path to an existing remote Git checkout.
3. Choose **Add machine**, then name and open a conversation. The machine and checkout appear in the sidebar.
4. Send a message. The remote conversation supports Codex questions, approvals, stop, and queue resume. If the connection drops during a command, **Retry request** uses the original command identity for conversation creation, sending, stopping, and queue resume.

Use **Show full message** to expand a long reply beyond its history preview.

**Disconnect** closes the SSH connection. **Forget** removes the saved entry from the desktop. Stop `hugglerd` through the remote supervisor or terminal to stop the service itself. The first release uses the remote Codex default model and requires a checkout and daemon prepared on the host. Remote folder browsing, file editing, interactive terminals, and Git review are future work.

## Verify the runtime boundary

`cargo test --locked --features machine --test machine` runs a scripted Codex app-server peer over provider stdio. It checks project scoping, environment identity, command receipt reuse, cursor-based refresh, full message retrieval, and a result written in the selected checkout.

Desktop qualification used the macOS GPUI client, OpenSSH and `hugglerd` 0.0.1 on Debian Linux. The scripted [Codex app-server fixture](../tests/fixtures/machine-codex.py) stood in for a live Codex CLI. After the SSH forward was terminated during a turn, the fixture completed the turn on Linux. The window reconnected and displayed the result, then reopened the saved conversation after relaunch. Quitting the desktop closed its SSH helper while the Linux daemon continued running.
