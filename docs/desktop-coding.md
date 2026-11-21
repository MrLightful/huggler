# Work on a coding task in the desktop app

On macOS, install Git, Xcode, Rust/Cargo 1.94.1 or later and a signed-in Codex CLI. From the Huggler repository, launch the native app with the following command.

```sh
cargo run --locked --manifest-path desktop/Cargo.toml --target-dir target/desktop
```

To install a bundle, use the release build prerequisites and choose an unused destination. Set `HUGGLER_STATE_DIR` before launch to use a custom state directory. [Release build prerequisites](verification.md#package-the-crate-and-desktop-app)

```sh
python3 scripts/install-desktop.py "$HOME/Applications/Huggler.app"
"$HOME/Applications/Huggler.app/Contents/MacOS/huggler-desktop"
```

To add provider instances, set `HUGGLER_PROVIDERS` to the path of a JSON file before launch. For example, this file registers a separate Cursor instance.

```json
[
  {"id":"cursor-work","driver":"cursor","executable":"agent","args":["acp"]}
]
```

Use unique instance IDs. `HUGGLER_CODEX` and `HUGGLER_CLAUDE_ACP` override the executables for the built-in instances. [Provider configuration](../docs-app/content/docs/guides/configure-providers.mdx)

1. **Start a conversation.** Choose **Create**, select your checkout with the folder button, type your task and click the up arrow or press **⌘Enter**. Huggler uses the first line as the conversation name. To work in a fresh scratch directory, choose **or start without a project**. With no checkout selected, you can send a message directly. Open the provider and model picker beside the send button to choose a provider. With a checkout selected, you can also search its models. Use **Huggler → Create with Setup…** (⌘⇧N) to set a name, check the selected provider, or create a linked worktree on a new branch. Leave the branch empty to use your checkout and its uncommitted edits. In an existing conversation, use the same picker to change the model for your next turn.
2. **Send a message.** Enter your task and click the up arrow beside **+**. In either composer, click **+** to include up to eight UTF-8 text files (64 KiB each). Click a filename to remove it before sending. In history, filenames appear below your message. Click one to read the saved text, then click it again to collapse it. You can send files on their own or queue them with a follow-up. The provider receives the filenames and the contents captured when you selected them. The built-in Codex instance can edit the checkout during implementation. You can change **Model** beside **Message** before sending or queuing your next turn. Huggler remembers the selection for this conversation. Read the streaming reply, open command output and respond to questions or permission requests in **Conversation**.
3. **Manage ongoing work.** While the agent works, the same up arrow queues your follow-up. **Stop thread** interrupts the current work and pauses queued messages. Use **Resume queue** to deliver them. Switch conversations from the sidebar to work on another task. A partially typed answer stays with its question while the app is open. Reopening the app restores the selected conversation and its history.
4. **Review changed files.** The right sidebar lists paths relative to the selected conversation’s checkout alongside their Git status. Toggle the sidebar from the conversation header. It starts collapsed in narrow windows. Hover over a truncated path to see it in full. The list refreshes when you select a conversation or a turn finishes. Choose the refresh icon beside **Changes** to pick up external edits.
5. **Open the checkout or a pull request.** Use the folder menu beside **Create PR** to open this conversation’s checkout in Finder, Terminal or VS Code. To open GitHub’s pull request form, push your commits to a tracked GitHub.com branch, then click **Create PR**. Review the base branch and submit on GitHub.

If a conversation's checkout was moved or deleted, Huggler shows its saved path. Restore the directory at that path to continue the same conversation. To start a conversation in the moved checkout or a different one, select **Choose another checkout**. The original conversation stays in the sidebar with its history.

## Find a conversation by status

Use the left sidebar button in the workspace header to hide or show conversations independently of Changes. The sidebar updates even while you work in another conversation. Needs attention and Ready sit at the top. Working and Settled sit together at the bottom. Click a section heading to collapse or expand it. Each section shows its count and lists the most recently active conversations first.

- **Needs attention** shows conversations that need an answer or approval, have a paused queue, or ended in an error. The reason appears below the conversation name.
- **Ready** shows new conversations and conversations whose work completed or was stopped.
- **Working** shows conversations with queued or active work, including running subagents. By default, only your open conversation appears in this group. A request for your input moves the conversation into **Needs attention** automatically.
- **Settled** holds conversations you have marked as finished. Click the checkmark beside an idle conversation to settle it. Expand **Settled** to browse its history. Click the undo arrow to unsettle it, or send a message to reopen it automatically. Settlement persists across restarts.

## Desktop settings

Choose **Settings** in the sidebar or press **⌘,**. Select **System**, **Light** or **Dark** to change the color theme immediately. Choose a default coding provider for new conversations. Each existing conversation keeps its assigned provider. **Done** returns to the conversation or launch form you were using, including text you had already entered. **⌘N** opens the new conversation form.

If the app cannot find Codex, run `command -v codex` in the shell where Codex works. Enter the resulting absolute path in **Codex executable**, choose **Save path**, then quit and reopen Huggler. Clear the field to use `codex` from the app’s search path. When `HUGGLER_CODEX` is set, Settings displays the environment override. Preferences are stored in `preferences.json` in the app’s state directory.

## Reproduce the desktop check

From the repository root, run these commands in a macOS GUI session with Python 3.12 or later.

```sh
cargo build --locked --manifest-path desktop/Cargo.toml --features qualification --target-dir target/desktop
python3 scripts/check-provider-desktop.py target/desktop/debug/huggler-desktop
python3 scripts/check-desktop-coding.py target/desktop/debug/huggler-desktop
python3 scripts/check-desktop-settings.py target/desktop/debug/huggler-desktop
```

The check drives the GPUI window with a scripted Codex app-server peer over stdio. It exercises questions and approvals, file edits, switching between checkouts, and pausing and resuming a queued message. [Desktop check](../scripts/check-desktop-coding.py) · [Codex fixture](../tests/fixtures/desktop-coding-peer.py) · [Live checks](verification.md#macos-desktop-checks)
