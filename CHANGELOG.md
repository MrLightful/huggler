# Changelog

## Unreleased

### Changed

- Managed worktrees use short dictionary names by default. Hosts can supply a custom dictionary or request an exact directory name. [#74](https://github.com/MrLightful/huggler/pull/74)

### Removed

- Huggler Desktop moved out of this repository. This repository ships the Rust SDK and its bindings. [#71](https://github.com/MrLightful/huggler/pull/71)

## [0.0.1](https://github.com/MrLightful/huggler/releases/tag/v0.0.1) (2026-11-26)

### Added

- Initial crates.io release of the Huggler Rust SDK for in-process coding-agent orchestration.
- Durable conversations with named provider instances, history, restart recovery, and runtime controls.
- Host-configurable storage, Git worktree management, GitHub pull request operations, and attachment support.
- Optional application integrations and the `hugglerd` machine service.
