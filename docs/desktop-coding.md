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

1. **Start a conversation.** Choose **Create**, select your checkout with the folder button, type your task and press **Enter** or click the up arrow. Huggler uses the first line as the conversation name. To work in a fresh scratch directory, choose **or start without a project**. With no checkout selected, you can send a message directly. Open the provider and model picker beside the send button to choose a provider. With a checkout selected, you can also search its models. Use **Huggler → Create with Setup…** (⌘⇧N) to set a name, check the selected provider, or create a linked worktree on a new branch. Leave the branch empty to use your checkout and its uncommitted edits. In an existing conversation, use the same picker to change the model for your next turn.
2. **Send a message.** Type your task and press **Enter** to send. Use **Shift+Enter** to add a line. In either composer, click **+** to include up to eight UTF-8 text files (64 KiB each). Click a filename to remove it before sending. In history, filenames appear below your message. Click one to read the saved text, then click it again to collapse it. You can send files on their own or queue them with a follow-up. The provider receives the filenames and the contents captured when you selected them. The built-in Codex instance can edit the checkout during implementation. You can change **Model** beside **Message** before sending or queuing your next turn. Huggler remembers the selection for this conversation. Read the streaming reply, open command output and respond to questions or permission requests in **Conversation**.
3. **Manage ongoing work.** While the agent works, **Enter** queues your follow-up and the round stop button replaces the send arrow. **Stop thread** interrupts the current work and pauses queued messages. Use **Resume queue** to deliver them. Choose **Show conversations** in the header to switch to another task. A partially typed answer stays with its question while the app is open. Reopening the app restores the selected conversation and its history.
4. **Review changed files.** The right sidebar lists paths relative to the selected conversation’s checkout alongside their Git status. Toggle the sidebar from the conversation header. It starts collapsed in narrow windows. Hover over a truncated path to see it in full. The list refreshes when you select a conversation or a turn finishes. Choose the refresh icon beside **Changes** to pick up external edits.
5. **Open the checkout or a pull request.** Use the folder menu beside **Create PR** to open this conversation’s checkout in Finder, Terminal or VS Code. To open GitHub’s pull request form, push your commits to a tracked GitHub.com branch, then click **Create PR**. Review the base branch and submit on GitHub.

If a conversation's checkout was moved or deleted, Huggler shows its saved path. Restore the directory at that path to continue the same conversation. To start a conversation in the moved checkout or a different one, select **Choose another checkout**. The original conversation stays in the sidebar with its history.

## Read the transcript

Your prompts appear on the right. Assistant replies support Markdown. Hover over a message to see its local time, elapsed turn time, and copy button. Hover over the time for the full date. Older and imported messages may have missing timing information. Use **Show full message** to expand a long reply. Hover over a mark to the left of the transcript to preview a prompt and the start of its reply. Click the mark to jump to that turn. Use **Older** and **Newest** to browse history, or click the down arrow above **Message** to return to the end.

While a response is running, the summary shows counts, the latest intermediate message, and the latest tool call. The elapsed time updates below them. Expand the summary to inspect the activity. Intermediate messages appear inside it without hover footers. When the reply arrives, the activity collapses above the final message. Command rows open the latest available output, including after you reopen a conversation. Completed turns show the activity counts even when earlier records have been omitted from the expanded list. Provider-reported context compaction is marked in the activity. Expand a filename to read the saved attachment inline.

| Harness and transport | Transcript coverage |
| --- | --- |
| Codex app-server over stdio, scripted protocol fixture | Native command, file change, MCP and dynamic tool, search, agent, commentary and compaction records when reported. Older command logs use generic labels where only output was saved. |
| ACP v1 or v2 over stdio | Native tool names and statuses when the adapter reports `tool_call` and `tool_call_update`. |
| Other transports and imported history | Displays the messages, saved attachments, and command output already available in history. |

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

The check drives the GPUI window with scripted Codex app-server peers over stdio. It exercises questions and approvals, file edits, switching between checkouts, and pausing and resuming a queued message. It also checks mixed transcript activity around a compaction boundary and opens saved command output after relaunch. [Desktop check](../scripts/check-desktop-coding.py) · [Coding fixture](../tests/fixtures/desktop-coding-peer.py) · [Transcript fixture](../tests/fixtures/desktop-transcript-peer.py) · [Live checks](verification.md#macos-desktop-checks)
