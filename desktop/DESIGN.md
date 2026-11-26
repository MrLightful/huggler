---
name: Huggler Desktop
description: Quiet, compact, and direct native workspace for coding with agents.
colors:
  light-primary: "#171717"
  light-on-primary: "#fafafa"
  light-primary-hover: "#262626"
  light-primary-active: "#0a0a0a"
  dark-primary: "#fafafa"
  dark-on-primary: "#171717"
  dark-primary-hover: "#e5e5e5"
  dark-primary-active: "#d4d4d4"
  light-workspace: "#f9f9f9"
  light-sidebar: "#f4f4f4"
  light-sidebar-selection: "#dfdfdf"
  light-muted-surface: "#efefef"
  light-secondary-hover: "#eaeaea"
  light-popover: "#fafafa"
  light-divider: "#dfdfdf"
  light-text: "#0a0a0a"
  light-muted-text: "#606060"
  light-input-border: "#e5e5e5"
  light-focus: "#0a0a0a"
  dark-workspace: "#171615"
  dark-sidebar: "#201e1c"
  dark-sidebar-selection: "#302d2a"
  dark-muted-surface: "#2c2926"
  dark-secondary-hover: "#3a3632"
  dark-popover: "#252220"
  dark-divider: "#34312e"
  dark-text: "#fafafa"
  dark-muted-text: "#aaa49e"
  dark-input-border: "#262626"
  dark-focus: "#d4d4d4"
  light-attention: "#9a470c"
  dark-attention: "#f6aa61"
  light-ready: "#177243"
  dark-ready: "#63ce91"
  light-working: "#285eaa"
  dark-working: "#7fb3ff"
  light-settled: "#5d6576"
  dark-settled: "#aaa7bc"
typography:
  headline:
    fontFamily: "DM Sans"
    fontSize: "1.25rem"
    fontWeight: 600
  title:
    fontFamily: "DM Sans"
    fontSize: "1.125rem"
    fontWeight: 600
  section-title:
    fontFamily: "DM Sans"
    fontSize: "0.875rem"
    fontWeight: 600
  body:
    fontFamily: "DM Sans"
    fontSize: "0.875rem"
    lineHeight: 1.55
  label:
    fontFamily: "DM Sans"
    fontSize: "0.8125rem"
    fontWeight: 500
  sidebar-meta:
    fontFamily: "DM Sans"
    fontSize: "0.75rem"
  command-output:
    fontFamily: "Menlo"
    fontSize: "0.75rem"
    lineHeight: "18px"
rounded:
  attachment: "5px"
  control: "6px"
  group: "0.5rem"
  composer: "0.75rem"
  circular-action: "999px"
spacing:
  field-gap: "0.25rem"
  toolbar-gap: "0.5rem"
  composer-inset: "0.75rem"
  content-inset: "1rem"
  settings-inset: "1.25rem"
components:
  send-light:
    backgroundColor: "{colors.light-primary}"
    textColor: "{colors.light-on-primary}"
    rounded: "{rounded.circular-action}"
    size: "2rem"
  send-dark:
    backgroundColor: "{colors.dark-primary}"
    textColor: "{colors.dark-on-primary}"
    rounded: "{rounded.circular-action}"
    size: "2rem"
  button-ghost:
    backgroundColor: "transparent"
    rounded: "{rounded.control}"
    height: "1.5rem"
  settings-input:
    rounded: "{rounded.control}"
    height: "2rem"
  conversation-row:
    rounded: "{rounded.control}"
  attachment-chip:
    rounded: "{rounded.attachment}"
    height: "1.25rem"
  settings-group:
    rounded: "{rounded.group}"
  composer:
    rounded: "{rounded.composer}"
    padding: "{spacing.composer-inset}"
---

# Design System: Huggler Desktop

## Overview

**Creative North Star: "Quiet Workbench"**

Huggler Desktop is quiet, compact, and direct. The conversation occupies the center, with task navigation on the left and files, changes, and traces on the right. The app uses subtle shifts in surface tone and compact controls to keep attention on the current task.

Meaningful state gets a stronger signal. A primary action has a high-contrast fill, an active row has a tonal highlight, and an attention item pairs a colored icon with a label and a reason.

**Key Characteristics**

- Neutral surfaces with warmer dark tones
- Compact DM Sans controls and readable transcript columns
- Status color paired with text or icon shape

## Colors

### Primary

The inherited GPUI Component primary is near-black on light and near-white on dark. Use it for the circular send action and selected settings choices. Hover and pressed states move along the same neutral scale.

### Neutral

The light palette uses a near-white workspace and a slightly darker sidebar. Dark mode moves from warm charcoal in the workspace to progressively lighter sidebar, selection, and popover fills. Thin borders divide regions and outline grouped controls. Muted text carries hints and metadata. The transcript blends its muted role toward the foreground for stronger legibility.

### Status

Attention is amber, Ready is green, Working is blue, and Settled is slate. The conversation sidebar pairs each color with a distinct icon and a named group. An attention row adds a specific reason beneath its title.

Palette overrides live in [ui.rs](src/ui.rs), status colors in [sidebar.rs](src/conversation/sidebar.rs), and inherited primary, text, and focus roles in the GPUI Component theme pinned by [Cargo.toml](Cargo.toml).

## Typography

Bundled DM Sans is the interface and prose family in both appearances. The starter prompt is the largest recurring workspace heading (20 at the default scale), followed by shared headings (18), section titles and transcript prose (14), form labels and conversation rows (13), and sidebar metadata (12). Headings and selected conversation titles use semibold. Most prose stays regular.

Assistant Markdown uses a line-height multiplier of 1.55. Markdown file previews use 1.65 with more space between blocks. Command output uses Menlo on macOS, at 12 with an 18-pixel line height. [Font setup](src/fonts.rs) · [Shared type](src/ui.rs) · [Markdown](src/conversation/markdown_view.rs) · [Transcript](src/conversation/transcript.rs)

## Layout

The window opens at 1120 × 740 with a 640 × 440 minimum. The sidebars start from preferred widths of 288 and 336 and can be resized. Below 900 logical pixels, the right sidebar defaults closed and overlays the workspace when opened. In wider windows it shares the row with the main pane.

The launch form and composer column cap at 736 pixels, assistant responses at 700, and settings at 720. Outgoing messages align right and cap at 88% of the content width or 620 pixels, whichever is smaller. Toolbar controls wrap as space tightens. Compact gaps group related controls, with larger insets around the composer and settings content. [Workspace](src/conversation.rs) · [Sidebar sizing](src/conversation/sidebar.rs) · [Transcript](src/conversation/transcript.rs) · [Settings](src/settings.rs) · [Window](src/main.rs)

## Elevation & Depth

The workspace and docked sidebars rely on tonal separation and one-pixel rules. Settings groups use a shared border and internal dividers. Standard bordered fields carry the library's small control shadow. Popovers use a stronger shadow to sit above the workspace, and the floating scroll-to-end button uses a small shadow.

The sidebar toggle animates over 180 ms with a cubic ease-out. On macOS it switches immediately when Reduce Motion is enabled. [Workspace](src/conversation.rs) · [Sidebar motion](src/conversation/sidebar.rs)

## Shapes

Buttons and settings inputs use the 6-pixel theme radius. Attachment chips use 5 pixels. Settings groups clip their rows inside an 8-pixel corner at the default scale. The composer and outgoing message bubble use a 12-pixel corner. The send, stop, and scroll-to-end controls use circular silhouettes. [Composer and actions](src/conversation.rs) · [Attachments](src/conversation/attachments.rs) · [Settings](src/settings.rs)

## Components

| Component | Treatment |
| --- | --- |
| Send and toolbar buttons | Send is a 32-pixel circle with a 16-pixel arrow and the primary fill. Secondary toolbar actions often use compact ghost buttons with a tonal hover or pressed fill. Library buttons supply the focus treatment. |
| Settings fields | A medium-weight label and tight gap lead into a bordered, rounded GPUI input. The library changes the border on focus and gives disabled inputs the muted surface. |
| Conversation navigation | Collapsible groups have compact headers with an icon, count, and chevron. A selected row uses semibold text and a filled background. A long title truncates, with the full title and status available in its tooltip. The Settle action appears on hover or keyboard focus. |
| Attachment chips | Small outlined controls pair a file-type icon with a bounded filename. Draft chips let the user remove a file. Sent chips open attachment details. |
| Settings groups | One border wraps related rows. A leading icon, title, supporting text, and a trailing chevron identify a destination. Dividers follow the interior rows. |
| Composer | A multiline input sits inside a softly tinted, bordered container. Attachments sit between the editor and its controls. Provider and model selection stay at the lower left, with attachment and send actions at the right. |
| Transcript | Outgoing messages use a tinted bubble. Assistant replies use the workspace surface and render as Markdown. Expandable activity sits alongside the conversation flow, and command output uses a muted, scrollable monospace panel. |

## Do's and Don'ts

- Do reuse the theme roles so new desktop surfaces follow System, Light, and Dark.
- Do pair task status color with a recognizable icon or a readable label.
- Do keep the conversation usable as sidebars resize, collapse, or overlay it.
- Do preserve built-in focus handling when composing native controls.
