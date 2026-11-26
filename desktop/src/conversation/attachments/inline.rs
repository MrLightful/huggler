use super::*;
use gpui_component::input::RopeExt;
use std::ops::Range;

const REFERENCE_START: &str = "⟦Attachment ";

// The caption stores a readable reference to the attachment's ordinal. The
// same caption is used in Backlog, in history, and in the provider prompt.
pub(super) fn reference(position: usize, name: &str) -> String {
    let name = name.replace(['\r', '\n', '\t', '⟦', '⟧'], " ");
    format!("{REFERENCE_START}{}: {name}⟧", position + 1)
}

pub(super) fn references<'a>(
    text: &str,
    files: impl IntoIterator<Item = (usize, &'a str)>,
) -> Vec<(Range<usize>, usize)> {
    let mut matches = Vec::new();
    for (position, name) in files {
        let marker = reference(position, name);
        matches.extend(
            text.match_indices(&marker)
                .map(|(start, _)| (start..start + marker.len(), position)),
        );
    }
    matches.sort_unstable_by_key(|(range, _)| range.start);
    matches
}

#[derive(Clone, Copy)]
pub(in crate::conversation) enum Direction {
    Backward,
    Forward,
}

impl Direction {
    pub(super) fn touches(self, marker: &Range<usize>, cursor: usize) -> bool {
        match self {
            Self::Backward => marker.start < cursor && cursor <= marker.end,
            Self::Forward => marker.contains(&cursor),
        }
    }
}

pub(super) fn expand_selection(
    mut selection: Range<usize>,
    references: &[(Range<usize>, usize)],
) -> Range<usize> {
    if selection.is_empty() {
        if let Some((marker, _)) = references
            .iter()
            .find(|(marker, _)| marker.start < selection.start && selection.start < marker.end)
        {
            return marker.end..marker.end;
        }
    } else {
        for (marker, _) in references {
            if marker.start < selection.end && selection.start < marker.end {
                selection.start = selection.start.min(marker.start);
                selection.end = selection.end.max(marker.end);
            }
        }
    }
    selection
}

pub(super) fn selection_range(
    input: &mut InputState,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> Option<Range<usize>> {
    let range = input.selected_text_range(false, window, cx)?.range;
    Some(
        input.text().offset_utf16_to_offset(range.start)
            ..input.text().offset_utf16_to_offset(range.end),
    )
}

pub(super) struct Insertion {
    text: SharedString,
    range: Range<usize>,
    cursor: usize,
}

impl Insertion {
    pub(super) fn capture(input: &Entity<InputState>, window: &mut Window, cx: &mut App) -> Self {
        input.update(cx, |input, cx| Self {
            text: input.value(),
            range: selection_range(input, window, cx).unwrap_or_default(),
            cursor: input.cursor(),
        })
    }

    pub(super) fn insert(
        self,
        input: &mut InputState,
        markers: &str,
        references: &[(Range<usize>, usize)],
        focus: bool,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let text = input.value();
        let range = expand_selection(rebase_range(&self.text, &text, self.range), references);
        let cursor = input.cursor();
        let keep_cursor = (text != self.text || cursor != self.cursor) && cursor != range.start;
        let next = if keep_cursor && cursor >= range.end {
            cursor - range.len() + markers.len()
        } else if keep_cursor && cursor < range.start {
            cursor
        } else {
            range.start + markers.len()
        };
        replace_range(input, range, markers, window, cx);
        if focus {
            input.set_cursor_position(input.text().offset_to_position(next), window, cx);
        }
    }
}

fn rebase_range(old: &str, new: &str, range: Range<usize>) -> Range<usize> {
    if old == new {
        return range;
    }
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    if range.end <= prefix {
        return range;
    }
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    if range.start >= old.len() - suffix {
        let start = new.len() - (old.len() - range.start);
        return start..start + range.len();
    }
    // The original selection was edited while the file was loading.
    prefix..prefix
}

pub(super) fn replace_range(
    input: &mut InputState,
    range: Range<usize>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<InputState>,
) {
    let range = input.text().offset_to_offset_utf16(range.start)
        ..input.text().offset_to_offset_utf16(range.end);
    input.replace_text_as_edit(range, text, window, cx);
}
