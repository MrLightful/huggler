# Take a coding task from plan to pull request

On macOS, install Git, Xcode, Rust/Cargo 1.94.1 or later and a signed-in Codex CLI. From the Huggler repository, launch the native app with the following command.

```sh
cargo run --locked --manifest-path desktop/Cargo.toml --target-dir target/desktop
```

1. **Open a checkout.** Choose **New conversation**, browse to your Git checkout, select a provider and enter a conversation name. **Model** lets you search the models that provider lists for the checkout. Leave **Provider default** selected to use its configuration. **Check provider** reports the adapter's version and login status when available. Leave the branch field empty to work in this checkout, including its uncommitted edits. To work independently, enter a new branch name. Huggler creates a linked worktree at the checkout's current commit and displays its path in the conversation header.
2. **Plan and implement.** With Codex selected, enter your task and choose **Send as plan** to request a read-only planning turn. When the plan is ready, use **Send** to ask Codex to implement it. The built-in Codex instance can edit the checkout during implementation. You can change **Model** beside **Message** before sending or queuing your next turn. Huggler remembers the selection for this conversation. Read the streaming reply, open command output and respond to questions or permission requests in **Conversation**.
3. **Manage ongoing work.** Use **Queue message** to schedule a follow-up while the agent works. **Stop thread** interrupts the current work and pauses queued messages. Use **Resume queue** to deliver them. Switch conversations from the sidebar to work on another task. A partially typed answer stays with its question while the app is open. Reopening the app restores the selected conversation and its history.
4. **Review and test.** Open **Changes** and select a file to inspect its diff. **Request revision** starts an editable follow-up about that file. In **Checks**, enter a shell command such as `cargo test` and choose **Run check**. It runs in the conversation's checkout. **Discuss check** adds the command, result and recent output to your draft. Send it to the agent, then rerun the check after the fix. Drafts and check output stay with their conversation while the app is open.
5. **Commit.** In **Changes**, select and stage the files you want. If a file has both staged and unstaged edits, select its entry in each group to review the two versions. Enter a message and choose **Commit staged** to commit the current index. During a merge, select each file under **Conflicts**, resolve it in the checkout and choose **Stage resolution**. Once every conflict is resolved, use **Commit merge**. This also works when the resolution keeps the current branch’s contents.
6. **Publish.** In **Publish**, choose a configured remote, check the destination and select **Push branch**. Git pushes to the same branch name and sets its upstream. Use **Open pull request form** for a synced branch with a supported github.com remote. Review the base, title and changes in your browser and submit on GitHub. For other hosts, open the pushed branch in the host's own review interface.

If a conversation's checkout was moved or deleted, Huggler shows its saved path. Restore the directory at that path to continue the same conversation. To start a conversation in the moved checkout or a different one, select **Choose another checkout**. The original conversation stays in the sidebar with its history.

## Find a conversation by status

The sidebar updates even while you work in another conversation. Each section shows its count and lists the most recently active conversations first.

- **Needs attention** shows conversations that need an answer or approval, have a paused queue, or ended in an error. The reason appears below the conversation name.
- **Ready** shows new conversations and conversations whose work completed or was stopped.
- **Working** shows conversations with queued or active work, including running subagents. Your open conversation stays visible. Choose **Show** to see the others or **Hide** to fold them away. A request for your input moves the conversation into **Needs attention** automatically.

## Desktop settings

Choose **Settings** in the sidebar or press **⌘,**. Select **System**, **Light** or **Dark** to change the color theme immediately. Choose a default coding provider for new conversations. Each existing conversation keeps its assigned provider. **Done** returns to the tab or launch form you were using, including text you had already entered. **⌘N** opens the new conversation form.

If the app cannot find Codex, run `command -v codex` in the shell where Codex works. Enter the resulting absolute path in **Codex executable**, choose **Save path**, then quit and reopen Huggler. Clear the field to use `codex` from the app’s search path. When `HUGGLER_CODEX` is set, Settings displays the environment override. Preferences are stored in `preferences.json` in the app’s state directory.

## Reproduce the desktop check

From the repository root, run these commands in a macOS GUI session with Python 3.12 or later.

```sh
cargo build --locked --manifest-path desktop/Cargo.toml --features qualification --target-dir target/desktop
python3 scripts/check-provider-desktop.py target/desktop/debug/huggler-desktop
python3 scripts/check-desktop-coding.py target/desktop/debug/huggler-desktop
python3 scripts/check-desktop-settings.py target/desktop/debug/huggler-desktop
```

The check drives the GPUI window with a scripted Codex app-server peer over stdio. It exercises a planning turn, a question and approval, a real file edit, a failing shell check, a correction and a passing check. It also checks the staged commit against the Git index, restarts the app, and pushes an isolated branch to a temporary bare repository. For versioned live provider evidence, see the earlier macOS qualification. [Desktop check](../scripts/check-desktop-coding.py) · [Codex fixture](../tests/fixtures/desktop-coding-peer.py) · [Live Codex qualification](stage-2.md)
