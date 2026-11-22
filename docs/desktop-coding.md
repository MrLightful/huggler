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

Use **Settings → Providers → Add provider** for Cursor, Grok Build, fx, Kiro, Antigravity, OpenCode, Pi, and Muse Code. Huggler finds installed CLIs at startup. If it cannot find one, open the provider's installation guide or save the full path under **Custom executable**. Restart Huggler after installing a CLI or saving its path, then choose the provider from the composer.

For a separate provider instance or custom launch arguments, set `HUGGLER_PROVIDERS` to the path of a JSON file before launch. This file registers a separate Cursor instance.

```json
[
  {"id":"cursor-work","driver":"cursor","executable":"agent","args":["acp"]}
]
```

Use unique instance IDs. A JSON entry with the same ID as one of the eight presets replaces that preset. `HUGGLER_CODEX` and `HUGGLER_CLAUDE_ACP` override the executables for `codex` and `claudeAgent`. Use JSON to register other ACP agents or connect an existing OpenCode server. [Provider configuration](../docs-app/content/docs/guides/configure-providers.mdx)

1. **Start a conversation.** Choose **Create**, open **Choose project**, and search your saved checkouts by name or path. Use **Add project** to choose and save a local Git checkout. Saved projects are available after restarting Huggler. Use **Settings → Projects** to add, rename, or remove entries. Removing an entry leaves its checkout and existing conversations in place. Choose **Start without a project** for a fresh scratch directory.

   **New worktree** starts a linked worktree from the repository's default branch. Huggler uses the cached remote default when available, then looks for `main`, `master`, the checked-out commit, or another available branch. Open **From …** to search local and remote-tracking branches for a different base. Choose **Current checkout** to use the selected folder. Pick a local branch to switch to it when the conversation opens. If Git detects a conflict with local files, the draft stays open so you can change your selection. Repositories without a starting commit use the current checkout.

   Type your task and use the send shortcut or click the up arrow. Huggler derives a short title from the first message. For example, “Please fix the parser.” becomes **Fix the parser**, with branch `huggler/fix-the-parser` and a checkout folder beginning with `fix-the-parser-`. If the branch name is taken, Huggler tries a numbered suffix. The setup view shows the base selection, checkout creation, and conversation opening. Use the model picker at the lower left of the composer to choose a provider and model. **Huggler → Create with Setup…** (⌘⇧N) also lets you set the conversation title or the exact branch name.
2. **Send a message.** Press **Enter** to send by default. Use **Shift+Enter** to add a line and **⌘L** to focus the message field. Change the send shortcut under **Settings → General**. In either composer, click **+** or drop files into the composer to attach up to eight files totaling 50 MiB, including up to 20 MiB of images. Click a draft filename to remove it before sending. In history, expand a filename to preview text, images, or extracted PDF text when available. Use **Save as…** to save the original file. You can send files on their own or queue them with a follow-up. The provider receives the filenames and the contents or local file paths supported by that provider. The built-in Codex instance can edit the checkout during implementation. You can change **Model** beside **Message** before sending or queuing your next turn. Huggler remembers the selection for this conversation. Read the streaming reply, open command output and respond to questions or permission requests in **Conversation**.
3. **Manage ongoing work.** While the agent works, the send shortcut queues your follow-up by default. Select **Steer** under **Settings → General** to send a text follow-up into a Codex turn that can accept it. **Stop thread** interrupts the current work and pauses queued messages. Use **Resume queue** to deliver them. Choose **Show conversations** in the header to switch to another task. A partially typed answer stays with its question while the app is open. Reopening the app restores the selected conversation and its history.
4. **Review changed files.** The **Changes** tab in the right sidebar defaults to **All changes**, which compares the selected checkout with the branch’s merge base. Choose **Uncommitted files** for staged, unstaged, and untracked files. The line totals follow the selection. Hover over the file name to see its full path. Use the row’s chevron to open **Open in**. Finder reveals the file, Terminal opens its folder, and VS Code opens the file. For deleted files the menu opens the nearest existing folder. Hover over a row with eligible uncommitted edits to show **Revert file**. Confirming a revert restores the path to its state at the current commit and removes files added since then. Resolve conflicts and submodule changes in Git. The list refreshes when you open the sidebar, select a conversation, finish a turn, or revert a file.

   Click a filename in **Changes** to open its diff in the main area. Click a file in **All files** to preview its contents. Tabs stay open for the conversation while Huggler is running. **Chat** stays pinned at the left. Close a preview with its close button or press **⌘W** while viewing it. Click the sidebar file again to reload its preview. Markdown opens as rendered prose in **Preview**. Diffs open in **Unified** with before and after line numbers, and **Patch** shows the Git output. Choose **Raw** in a file or diff tab to edit the working-tree text. **Save** or **⌘S** writes the file and refreshes its previews. Open tabs for the same path share one draft within the conversation. If the file changes on disk before you save, copy your edits or reload the working-tree version. The inline editor supports regular UTF-8 text files up to 1 MiB. Use the copy icon in the preview toolbar to copy the file contents, image, or diff. When the preview reaches its size limit, the icon copies the displayed excerpt. The revert icon in a diff uses the same confirmation and checkout checks as the sidebar.

   Hover over an eligible file row and click **Approve** to move it into the **Approved** filter. Approvals apply to the file diff you marked and are saved for the conversation. If that diff changes, the file reappears for review when the sidebar refreshes. Open **Approved** to see approved files or choose **Undo approval**. Approval changes only the sidebar's review state.

   Open **File actions** beside the filter to **Approve all** eligible files or **Undo all approvals** in the **Approved** filter. **Revert all** applies to files in the active filter with working-tree changes that can be reverted. The confirmation shows how many files will be affected. If a file changes while the confirmation is open, the revert stops so you can review the new changes.
5. **Work with a pull request.** Use the **Open in** folder menu to open the checkout in Finder, Terminal or VS Code. Sign in with GitHub CLI (`gh auth login`) for PR status and creation. **Create PR** includes staged changes, unstaged edits, deletions, and new files Git tracks by default. Codex reads the pending diff and recent commit subjects to write one commit message. Huggler commits the changes, pushes the selected branch using Git credentials, configures its upstream when needed, and uses the latest commit’s subject and body for the PR. An unpublished branch uses `origin` or the sole remote. The PR targets the remote repository’s default branch unless `branch.<name>.gh-merge-base` selects another base in that repository.

   **Resolve** and **Fix** queue a task in the current conversation and preserve the draft in the composer. For a PR, the task asks the agent to verify the head, resolve conflicts or address check and review failures, run relevant checks, and push the verified fix. For local conflicts, **Resolve** asks the agent to continue the Git operation already in progress. **Merge**, **Auto-merge**, and **Queue merge** require confirmation. The shared repository client uses the GitHub account signed in to the CLI and rechecks the PR before sending the update. The confirmation shows the target branch and the selected method. The desktop picks the first available method from merge commit, squash, and rebase. For merge queues, GitHub chooses the method. **View PR**, **View draft**, and labels such as **Merged** open the corresponding request on GitHub. If more than one PR is open for the branch, **View PR** lists the targets.

   **Create PR** appears on a GitHub topic branch with uncommitted changes or commits beyond its base, including commits you have already pushed. Commit message generation uses the conversation’s Codex instance and model when selected, or the built-in Codex instance and its default model for other providers. Automatic creation uses one GitHub repository for both branches. A separate Git push remote requires you to publish the branch and open the PR through your fork workflow. The header refreshes on selection, after a completed turn or an editor save, when the window regains focus, and every 20 seconds while a conversation is selected. If GitHub status is unavailable, the compare form is available for a synced branch with a clean working tree and a cached default branch. Hover over the button for the status error.

If a conversation's checkout was moved or deleted, Huggler shows its saved path. Restore the directory at that path to continue the same conversation. To start a conversation in the moved checkout or a different one, select **Choose another checkout**. The original conversation stays in the sidebar with its history.

## Run a project action

Expand **Terminal** in the right sidebar. Click **+** beside the tabs to open another shell. Open the menu beside the play button and choose **Configure** to add or edit named shell commands. You can also manage them under **Settings → Projects → Actions**. The first action is the default. Use **Make default** to move another action onto the play button.

Click play or choose an action from the menu to run its command immediately in a new terminal tab in the conversation’s checkout. A new worktree uses the actions from the saved project you selected. Other linked worktrees can use the main checkout’s actions.

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

## Check MCP servers

Click **MCP server status** beside the attachment and send buttons in **First message** or **Message**. The popover shows the provider's tool counts, authentication requirements and connection errors. Use **Refresh MCP status** to check again.

On macOS, in the MCP status popover, click a Codex server marked **Needs auth** to run `codex mcp login` for that server immediately. Huggler uses the selected provider instance’s executable, arguments, and environment. Login opens in the conversation’s terminal, or a separate terminal window before a conversation is open. Refresh MCP status after completing authentication.

| Harness and transport | Status coverage |
| --- | --- |
| Codex CLI 0.170.0, app-server over stdio | Native `mcpServerStatus/list` using a temporary session to check the servers configured for the checkout. |
| Other harnesses | Status is unavailable and the popover shows an explanation. |

Each refresh checks the checkout in a fresh Codex session. Servers that Huggler injects into individual runs use their own sessions. The desktop owns the popover and refresh controls. [T3 Code Codex adapter](https://github.com/pingdotgg/t3code/blob/338352dee2f43079482e054e8229a2ba055641d9/apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts#L7865-L7884) · [macOS provider check](../scripts/check-provider-desktop.py)

## Find a conversation by status

The conversation sidebar is visible when Huggler opens. Use the left sidebar button in the workspace header to hide or show conversations independently of Changes. The sidebar updates even while you work in another conversation. Needs attention and Ready sit at the top. Working and Settled sit together at the bottom. Click a section heading to collapse or expand it. Each section shows its count and lists the most recently active conversations first.

- **Needs attention** shows conversations that need an answer or approval, have a paused queue, or ended in an error. The reason appears below the conversation name.
- **Ready** shows new conversations and conversations whose work completed or was stopped.
- **Working** shows conversations with queued or active work, including running subagents. By default, only your open conversation appears in this group. A request for your input moves the conversation into **Needs attention** automatically.
- **Settled** holds conversations you have marked as finished. Hover over an idle conversation and click its checkmark to settle it. Expand **Settled** to browse its history. Hover over a settled conversation and click its undo arrow to unsettle it. Sending a message also reopens the conversation. Settlement persists across restarts. By default, Huggler also checks idle conversations every minute and settles them when their branch’s pull request has merged. The checkout must be clean and its current commit must match the merged PR’s head; reused branches with new work stay open. Turn off **Auto-settle merged PRs** in **Settings → General** to disable this behavior. Active or queued work prevents settlement.

## Desktop data directory

Desktop uses `~/.huggler` by default. `userdata/` holds `huggler.sqlite3`, preferences, and the saved conversation selection. Managed Git checkouts go in `worktrees/`, and conversations started without a project go in `scratch/`.

Set `HUGGLER_HOME` before launch to use the same layout under a different directory. The existing `HUGGLER_STATE_DIR` override takes precedence and uses its original flat layout, with `huggler.sqlite3`, `worktrees/`, and `scratch/` directly under the specified path.

If `~/.huggler/userdata/huggler.sqlite3` is absent and an existing database is found at `~/Library/Application Support/dev.huggler.stage2/huggler.sqlite3`, Desktop continues to use the old directory in place. Saved conversations can still open their original checkout paths. If the home directory cannot be resolved, Desktop uses the previous platform data directory when available.

## Desktop settings

Choose **Settings** in the sidebar or press **⌘,**. On Linux and Windows, use Ctrl in place of ⌘ for desktop shortcuts.

**General** controls how messages are sent. Select **⌘Enter** on macOS or **Ctrl+Enter** on Linux and Windows to use Enter for a new line. **Queue** holds follow-ups until the agent is ready. **Steer** sends text into eligible running Codex turns and queues other follow-ups. Turn on **Agent notifications** for completion alerts. On macOS, **Keep awake while agents run** prevents idle system sleep while agents work.

**Providers** shows detected installations and configured servers. Select a card to see that provider’s settings below, or choose **Add provider** to set up another one. Use **Install** where available, or open **Custom executable** to set the path to an existing installation. Restart Huggler after saving a custom path. Huggler uses the CLI’s credentials. Follow the provider’s sign-in guide if needed, then choose **Refresh**.

The provider heading shows the setup state, detected executable version, and available actions. **Refresh** checks the installed version, sign-in status, and available models. Huggler offers **Update** when its bundled release is newer than the installed copy it manages. **Reset to defaults** restores model visibility and order and clears executable and launch overrides saved in Huggler. Restart Huggler to apply reset executable or launch settings.

New conversations start with Codex selected. Use the composer’s provider and model picker to choose another provider before starting a conversation. Under **Appearance**, select **System**, **Light** or **Dark** to change the color theme immediately. Use the back arrow to retrace your steps to the conversation or launch form, including any draft. When you open settings from the model picker or terminal, the arrow takes you straight back to the workspace. **⌘N** opens the new conversation form.

Under **Storage**, use **Clean up now** to remove unused worktrees that have no local changes or new commits. Turn on **Automatic cleanup** to check immediately, at startup, and hourly.

Select a provider and use **Models** to show or hide models in the conversation picker. Models appear in descending order by name until you reorder them with the arrows. Preferences apply to that provider instance across checkouts. The first option follows the provider’s default. It shows the model name when available and **Automatic** when the model is unknown. Model discovery uses the current checkout when available.

Use **Effort** or **Fast mode** for Codex models that support those options. **Use default** restores the checkout’s configured reasoning effort or the model’s default effort. Existing conversations remember their selections, and queued messages keep the settings selected when they were queued. Use the gear beside the model search field to manage provider setup.

Under **Environment**, add variables such as a custom API endpoint for that provider. Use **Save variables**, or leave the provider’s settings to save valid edits automatically. Restart Huggler to apply them. Huggler masks values in the editor and saves them in the local preferences file with user-only permissions. They override inherited variables for that provider’s subprocesses. A config directory set in Advanced settings takes precedence over a matching environment variable. Configure the environment of any separately managed server where it runs.

Open **Advanced settings** to set a preset's executable path. Codex and Claude Agent also expose a config directory and launch arguments. Save and restart Huggler to apply runtime changes. Clear the executable field to return to automatic discovery. `HUGGLER_CODEX` and `HUGGLER_CLAUDE_ACP` take precedence over the corresponding saved paths. Instances defined in `HUGGLER_PROVIDERS` show their launch configuration in Advanced settings. Edit the JSON file to change those settings. Desktop stores preferences in `preferences.json` in the app’s state directory.

## Reproduce the desktop check

From the repository root, run these commands in a macOS GUI session with Python 3.12 or later.

```sh
cargo build --locked --manifest-path desktop/Cargo.toml --features qualification --target-dir target/desktop
python3 scripts/check-provider-desktop.py target/desktop/debug/huggler-desktop
python3 scripts/check-desktop-coding.py target/desktop/debug/huggler-desktop
python3 scripts/check-desktop-pull-requests.py target/desktop/debug/huggler-desktop
python3 scripts/check-desktop-settings.py target/desktop/debug/huggler-desktop
```

The check drives the GPUI window with scripted Codex app-server peers over stdio. It exercises questions and approvals, file edits, switching between checkouts, and pausing and resuming a queued message. It also checks mixed transcript activity around a compaction boundary and opens saved command output after relaunch. [Desktop check](../scripts/check-desktop-coding.py) · [Coding fixture](../tests/fixtures/desktop-coding-peer.py) · [Transcript fixture](../tests/fixtures/desktop-transcript-peer.py) · [Live checks](verification.md#macos-desktop-checks)
