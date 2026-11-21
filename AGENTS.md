# Working on Huggler

Build `huggler` as a Rust library crate distributed through Cargo. The native desktop app uses Zed’s GPUI framework and embeds the crate directly. Keep the base app free of a Node.js runtime dependency and scope provider-specific runtime requirements to their adapters.

Use T3 Code as the reference for what Huggler should enable. Aim to cover all of its user-visible orchestration capabilities where feasible. Judge coverage by the workflows users can accomplish. Choose Huggler’s interfaces, storage schema, provider transports and implementation to fit the Rust crate and its hosts.

The local T3 Code reference checkout is `/Users/mrlightful/Projects/Personal/t3code`. Make changes in the current Huggler worktree. Keep the upstream checkout path out of crate code and runtime defaults.

Prefer recorded fixtures or the existing Codex subscription for testing. Before starting a local language-model server or running inference on the user’s computer, ask for explicit permission for that specific run and wait for approval. This includes tests and adapters that call an already-running local model.

## Continuing extraction

When the entire prompt is `continue`, resume extraction from the [stage handoff](docs/extraction-plan.md#stage-handoff). Follow its next action through the stage acceptance checks. At each handoff, update the plan and tell the user whether to merge or commit and continue. Publish the next-stage artifact when a stage is complete.

## Porting and reviewing upstream changes

- To locate the owner of a behavior, start with the [upstream component map](docs/upstream-watch-guide.md#find-the-implementation-by-behavior). Trace the implementation and its regression test at the revision under review. Use `git -C <checkout> show <sha>:<path>` to read a pinned file while keeping the local branch in place.
- To review newer commits, run `python3 scripts/review-upstream.py <checkout>` from the Huggler root. The [review record](docs/upstream-review.json) holds the last reviewed SHA and deferred items. Advance it after accounting for the range using the [review workflow](docs/upstream-watch-guide.md#adapt-and-merge-a-relevant-change).
- When implementing a capability, identify its expected outcome in the [capability inventory](docs/parity-inventory.md) and verify it through Huggler’s public interface. For provider-specific behavior, record the harness, version, transport and operation. Distinguish native behavior from fallbacks and unavailable operations.

## Crate and documentation conventions

- Follow the [crate naming conventions](docs/sdk-proposal.md#naming) for Rust modules, public APIs, agent tools and configuration. Preserve upstream spellings in source references and original license notices.
- Use the [extraction plan](docs/extraction-plan.md) for milestones and acceptance criteria. Check the implementation and its tests before describing a proposed capability as available.
- Keep design and engineering documentation under `docs/`. The maintainer owns the root `README.md`.
