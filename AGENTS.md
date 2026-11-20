# Working on Huggler

The local T3 Code checkout on the maintainer’s Mac is `/Users/mrlightful/Projects/Personal/t3code`. Use it as the reference for orchestration behavior. Make Huggler changes in the current Huggler worktree. Keep the checkout path out of package code and runtime defaults.

## Porting and reviewing upstream changes

- To locate the owner of a behavior, start with the [upstream component map](docs/upstream-watch-guide.md#find-the-implementation-by-behavior). Trace the implementation and its regression test at the revision under review. Use `git -C <checkout> show <sha>:<path>` to read a pinned file while keeping the local branch in place.
- To review newer commits, run `python3 scripts/review-upstream.py <checkout>` from the Huggler root. The [review record](docs/upstream-review.json) holds the last reviewed SHA and deferred items. Advance it after accounting for the range using the [review workflow](docs/upstream-watch-guide.md#adapt-and-merge-a-relevant-change).
- When porting a capability, identify its expected outcome in the [parity inventory](docs/parity-inventory.md) and verify it through Huggler’s public interface. For provider-specific behavior, record which harness and operation the evidence covers.

## Package and documentation conventions

- Use `huggler` for the npm package. Follow the [SDK naming conventions](docs/sdk-proposal.md#naming) for files, exports, agent tools and configuration. Preserve upstream spellings in source references and original license notices.
- Use the [extraction plan](docs/extraction-plan.md) for milestones and acceptance criteria. Check the implementation and its tests before describing a proposed capability as available.
- Keep design and engineering documentation under `docs/`. The maintainer owns the root `README.md`.
