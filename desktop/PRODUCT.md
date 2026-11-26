# Product

<!-- impeccable:product-schema 1 -->

## Platform

macOS

Huggler Desktop is a native GPUI app. This record covers the desktop app.

## Users

Huggler Desktop is built for its maintainer’s own day-to-day development. The maintainer starts coding tasks, keeps track of active conversations, answers questions when an agent needs input, and reviews the resulting changes before shipping them.

## Product Purpose

Huggler Desktop is a personal agentic development environment. It brings together the steps the maintainer uses to direct coding agents and carry work from a prompt to reviewed changes and, when needed, a pull request. Success means being able to pick up a task, see where it stands, and take the next action without losing context.

## Positioning

The desktop app is built around the maintainer’s own workflow. Its scope is the small set of capabilities the maintainer needs and wants to focus on during real work. A task’s conversation, agent activity, and Git checkout belong to the same workspace.

## Operating Context

- Start a conversation in an existing Git checkout, a new worktree, or a scratch directory. Choose a configured provider and model for the task.
- Follow live progress, inspect command output, respond to questions and approvals, and queue follow-ups or steer a running turn where supported. Reopen saved conversations and see which tasks need attention.
- Inspect files and diffs, use a terminal in the checkout, and create or follow a GitHub pull request where configured. [Desktop workflow](../docs/desktop-coding.md)
- Use a configured SSH machine when a task needs a remote checkout. The documented remote workflow supports Codex conversations and reconnection. [Remote machines](../docs/remote-machines.md)

## Capabilities and Constraints

- The base desktop app runs without a Node.js dependency. Provider-specific runtime requirements belong to their adapters. [Contributor instructions](../AGENTS.md)
- Provider and host capabilities vary. Describe a workflow according to what the selected setup supports. [Capability inventory](../docs/parity-inventory.md)

## Product Principles

1. Keep the scope grounded in the maintainer’s recurring work. Add features when they serve a concrete need in that workflow.
2. Make it easy to see what is running, what needs a decision, and where to resume.
3. Keep agent work reviewable in the context of the checkout so the maintainer can inspect the result before shipping it.
