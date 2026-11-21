# Work on a coding task in the desktop app

On macOS, install Git, Xcode, Rust/Cargo 1.94.1 or later and a signed-in Codex CLI. From the Huggler repository, launch the native app with the following command.

```sh
cargo run --locked --manifest-path desktop/Cargo.toml --target-dir target/desktop
```

To install a bundle, use the release build prerequisites and choose an unused destination. [Release build prerequisites](verification.md#package-the-crate-and-desktop-app)

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
2. **Send a message.** Type your task and press **Enter** to send. Use **Shift+Enter** to add a line and **⌘L** to focus the message field. In either composer, click **+** or drop files into the composer to attach up to eight files totaling 50 MiB, including up to 20 MiB of images. Click a draft filename to remove it before sending. In history, expand a filename to preview text, images, or extracted PDF text when available. Use **Save as…** to save the original file. You can send files on their own or queue them with a follow-up. The provider receives the filenames and the contents or local file paths supported by that provider. The built-in Codex instance can edit the checkout during implementation. You can change **Model** beside **Message** before sending or queuing your next turn. Huggler remembers the selection for this conversation. Read the streaming reply, open command output and respond to questions or permission requests in **Conversation**.
3. **Manage ongoing work.** While the agent works, **Enter** queues your follow-up and the round stop button replaces the send arrow. **Stop thread** interrupts the current work and pauses queued messages. Use **Resume queue** to deliver them. Choose **Show conversations** in the header to switch to another task. A partially typed answer stays with its question while the app is open. Reopening the app restores the selected conversation and its history.
4. **Review changed files.** The **Changes** tab in the right sidebar defaults to **All changes**, which compares the selected checkout with the branch’s merge base. Choose **Uncommitted files** for staged, unstaged, and untracked files. The line totals follow the selection. Click a file row or its chevron to expand or collapse the diff. Hover over the file name to see its full path. Hover over a row with eligible uncommitted edits to show **Revert file**. Confirming a revert restores the path to its state at the current commit and removes files added since then. Resolve conflicts and submodule changes in Git. The list refreshes when you open the sidebar, select a conversation, finish a turn, or revert a file.
5. **Open the checkout or a pull request.** Use the folder menu beside **Create PR** to open this conversation’s checkout in Finder, Terminal or VS Code. To open GitHub’s pull request form, push your commits to a tracked GitHub.com branch, then click **Create PR**. Review the base branch and submit on GitHub.

If a conversation's checkout was moved or deleted, Huggler shows its saved path. Restore the directory at that path to continue the same conversation. To start a conversation in the moved checkout or a different one, select **Choose another checkout**. The original conversation stays in the sidebar with its history.

## Read the transcript

Your prompts appear on the right. Assistant replies support Markdown. Hover over a message to see its local time, elapsed turn time, and copy button. Hover over the time for the full date. Older and imported messages may have missing timing information. Use **Show full message** to expand a long reply. Hover over a mark to the left of the transcript to preview a prompt and the start of its reply. Click the mark to jump to that turn. Use **Older** and **Newest** to browse history, or click the down arrow above **Message** to return to the end.

While a response is running, the summary shows counts, the latest intermediate message, and the latest tool call. The elapsed time updates below them. Expand the summary to inspect the activity. Intermediate messages appear inside it without hover footers. When the reply arrives, the activity collapses above the final message. Command rows open the latest available output, including after you reopen a conversation. Completed turns show the activity counts even when earlier records have been omitted from the expanded list. Provider-reported context compaction is marked in the activity.

| Harness and transport | Transcript coverage |
| --- | --- |
| Codex app-server over stdio, scripted protocol fixture | Native command, file change, MCP and dynamic tool, search, agent, commentary and compaction records when reported. Older command logs use generic labels where only output was saved. |
| ACP v1 or v2 over stdio | Native tool names and statuses when the adapter reports `tool_call` and `tool_call_update`. |
| Other transports and imported history | Displays the messages, saved attachments, and command output already available in history. |

## Mention a file or skill

In **First message** or **Message**, type `@` and part of a filename or path to find a file in the conversation's checkout. Choose a result to insert a relative file reference. With a Codex provider selected, type `/` and part of a skill name to find enabled skills. Choosing a skill inserts its `$skill-name` mention. When skills share a name, the suggestions show their paths and the selected mention includes its path.

Use **↑** and **↓** to move through the suggestions, **Enter** or **Tab** to insert the highlighted result, and **Esc** to close the menu. You can also click a result. Continue editing your draft or send it with the up arrow.

## Find a conversation by status

Use the left sidebar button in the workspace header to hide or show conversations independently of Changes. The sidebar updates even while you work in another conversation. Needs attention and Ready sit at the top. Working and Settled sit together at the bottom. Click a section heading to collapse or expand it. Each section shows its count and lists the most recently active conversations first.

- **Needs attention** shows conversations that need an answer or approval, have a paused queue, or ended in an error. The reason appears below the conversation name.
- **Ready** shows new conversations and conversations whose work completed or was stopped.
- **Working** shows conversations with queued or active work, including running subagents. By default, only your open conversation appears in this group. A request for your input moves the conversation into **Needs attention** automatically.
- **Settled** holds conversations you have marked as finished. Click the checkmark beside an idle conversation to settle it. Expand **Settled** to browse its history. Click the undo arrow to unsettle it, or send a message to reopen it automatically. Settlement persists across restarts.

## Desktop data directory

Desktop uses `~/.huggler` by default. `userdata/` holds `huggler.sqlite3`, preferences, and the saved conversation selection. Managed Git checkouts go in `worktrees/`, and conversations started without a project go in `scratch/`.

Set `HUGGLER_HOME` before launch to use the same layout under a different directory. The existing `HUGGLER_STATE_DIR` override takes precedence and uses its original flat layout, with `huggler.sqlite3`, `worktrees/`, and `scratch/` directly under the specified path.

If `~/.huggler/userdata/huggler.sqlite3` is absent and an existing database is found at `~/Library/Application Support/dev.huggler.stage2/huggler.sqlite3`, Desktop continues to use the old directory in place. Saved conversations can still open their original checkout paths. If the home directory cannot be resolved, Desktop uses the previous platform data directory when available.

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
