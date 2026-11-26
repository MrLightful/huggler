# Changelog

## Unreleased

### Changed

- Managed worktrees use short dictionary names by default. Hosts can supply a custom dictionary or request an exact directory name. [#74](https://github.com/MrLightful/huggler/pull/74)

### Removed

- Huggler Desktop moved out of this repository. This repository ships the Rust SDK and its bindings. [#71](https://github.com/MrLightful/huggler/pull/71)

## [0.1.0](https://github.com/MrLightful/huggler/compare/v0.0.1...v0.1.0) (2026-11-26)


### Features

* add opt-in conversation title generation ([39d9551](https://github.com/MrLightful/huggler/commit/39d9551576ac941a44da2693ba79cc064b8c46b1))
* add opt-in conversation title generation ([f75b065](https://github.com/MrLightful/huggler/commit/f75b0653677c4e364a724326e175f6245873cba7))


### Bug Fixes

* **bindings:** restore license notice generation ([3079135](https://github.com/MrLightful/huggler/commit/30791359e9df3dbc02992e58fa469d5d31fed9eb))

## [0.0.1](https://github.com/MrLightful/huggler/releases/tag/v0.0.1) (2026-11-26)

### Added

- Initial crates.io release of the Huggler Rust SDK for in-process coding-agent orchestration.
- Durable conversations with named provider instances, history, restart recovery, and runtime controls.
- Host-configurable storage, Git worktree management, GitHub pull request operations, and attachment support.
- Optional application integrations and the `hugglerd` machine service.
