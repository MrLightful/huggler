# Working on Huggler

Huggler is a Cargo library embedded directly in the GPUI desktop app. Keep the base app free of a Node.js runtime dependency and scope provider-specific runtime requirements to their adapters.

Prefer recorded fixtures or the existing Codex subscription for testing. Before starting a local language-model server or running local inference, including through an already-running server, get explicit approval for that specific run.

- **Desktop QA.** Track the PID of every Huggler instance you launch for testing, including previews and screenshot sessions. When its check ends, even on failure, quit that instance by PID and verify it exited.
- **Capability changes.** Before adding or porting a workflow, use the [consumer and orchestration boundary](knowledge/README.md#consumer-and-orchestration-boundary) to decide who owns each behavior. Aim for workflow parity with T3 Code where feasible. Use the [capability inventory](knowledge/parity-inventory.md) and verify outcomes through the owning layer’s public interface. Choose APIs, storage and transports that fit Huggler’s hosts. For provider-specific behavior, record the harness, version, transport, operation and whether the result is native, a fallback or unavailable.
- **Upstream ports and reviews.** Follow the [review guide](knowledge/upstream-watch-guide.md) to trace the implementation and regression at the revision under review. Use `/Users/mrlightful/Projects/Personal/t3code` as a reference checkout and keep its path out of code and runtime defaults. The [review record](knowledge/upstream-review.json) tracks deferred work and the last reviewed SHA.
- **Naming and docs.** Follow the [contributor conventions](knowledge/README.md#naming) and preserve upstream license notices. Keep engineering docs under `knowledge/`. The maintainer owns the root `README.md`.
