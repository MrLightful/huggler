use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Rope, h_flex,
    highlighter::{HighlightTheme, Language, SyntaxHighlighter},
    scroll::{ScrollableElement, ScrollbarAxis},
};
use std::{
    ops::Range,
    path::Path,
    sync::{Arc, Mutex},
};

const CODE_SIZE: Rems = Rems(0.875);
const HEADER_SIZE: Rems = Rems(0.75);
const MARKER_WIDTH: Rems = Rems(1.25);

type Highlights = Vec<(Range<usize>, HighlightStyle)>;

pub(super) struct Diff {
    lines: Vec<Line>,
    widest: Mutex<Option<WidthMeasurement>>,
    number_width: Pixels,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Hunk,
    Context,
    Added,
    Removed,
    Metadata,
}

struct Line {
    kind: Kind,
    text: SharedString,
    before: Option<usize>,
    after: Option<usize>,
    highlights: [Highlights; 2],
}

struct WidthMeasurement {
    family: SharedString,
    rem_size: Pixels,
    row: usize,
}

impl Diff {
    pub(super) fn new(text: &str, path: &Path) -> Self {
        let mut lines = Vec::new();
        let mut hunk = None;
        let mut last_number = 0;
        for line in text.lines() {
            let (kind, text, before, after) = if let Some(next) = Hunk::parse(line) {
                hunk = Some(next);
                (Kind::Hunk, line, None, None)
            } else if let Some((kind, before, after)) =
                hunk.as_mut().and_then(|hunk| hunk.consume(line))
            {
                // Git can omit the context prefix on empty lines.
                (kind, line.get(1..).unwrap_or_default(), before, after)
            } else if line.is_empty()
                || ["diff --git ", "index ", "--- ", "+++ "]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            {
                continue;
            } else {
                (Kind::Metadata, line, None, None)
            };
            let text = text.replace('\t', "    ");
            last_number = last_number.max(before.unwrap_or(0)).max(after.unwrap_or(0));
            lines.push(Line {
                kind,
                text: text.into(),
                before,
                after,
                highlights: Default::default(),
            });
        }
        let language = Language::from_str(path.extension().and_then(|s| s.to_str()).unwrap_or(""));
        if language.name() != "text" {
            highlight_side(&mut lines, language, true);
            highlight_side(&mut lines, language, false);
        }
        Self {
            lines,
            widest: Mutex::default(),
            number_width: px(12. + 8. * last_number.to_string().len().max(3) as f32),
        }
    }

    fn widest_row(&self, window: &Window, cx: &App) -> usize {
        let family = &cx.theme().mono_font_family;
        let rem_size = window.rem_size();
        let mut cached = self.widest.lock().expect("diff width measurement");
        if let Some(width) = cached
            .as_ref()
            .filter(|w| &w.family == family && w.rem_size == rem_size)
        {
            return width.row;
        }
        let style = TextStyle {
            font_family: family.clone(),
            ..Default::default()
        };
        let gutter = self.number_width * 2. + MARKER_WIDTH.to_pixels(rem_size);
        let measure = |line: &Line| {
            let (font_size, inset) = if line.is_header() {
                (HEADER_SIZE, px(0.))
            } else {
                (CODE_SIZE, gutter)
            };
            inset
                + window
                    .text_system()
                    .layout_line(
                        &line.text,
                        font_size.to_pixels(rem_size),
                        &[style.to_run(line.text.len())],
                        None,
                    )
                    .width
        };

        // In a monospace font the longest printable ASCII row of each kind
        // bounds its peers. Measure other rows for fallback fonts and glyph widths.
        let mut ascii_candidates: [Option<usize>; 2] = [None, None];
        let mut widest = (0, px(0.));
        let mut consider_width = |index: usize, line: &Line| {
            let width = measure(line);
            if width > widest.1 {
                widest = (index, width);
            }
        };
        for (index, line) in self.lines.iter().enumerate() {
            if line.text.bytes().all(|byte| (b' '..=b'~').contains(&byte)) {
                let candidate = &mut ascii_candidates[usize::from(line.is_header())];
                if candidate.is_none_or(|old| self.lines[old].text.len() < line.text.len()) {
                    *candidate = Some(index);
                }
            } else {
                consider_width(index, line);
            }
        }
        for index in ascii_candidates.into_iter().flatten() {
            consider_width(index, &self.lines[index]);
        }
        *cached = Some(WidthMeasurement {
            family: family.clone(),
            rem_size,
            row: widest.0,
        });
        widest.0
    }

    pub(super) fn render(
        diff: &Arc<Self>,
        scroll: &UniformListScrollHandle,
        window: &Window,
        cx: &App,
    ) -> Div {
        let diff = diff.clone();
        let list = uniform_list("unified-diff", diff.lines.len(), {
            let diff = diff.clone();
            move |range, _, cx| {
                diff.lines[range]
                    .iter()
                    .map(|line| line.render(diff.number_width, cx))
                    .collect()
            }
        })
        .size_full()
        .with_width_from_item(Some(diff.widest_row(window, cx)))
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .track_scroll(scroll.clone())
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(CODE_SIZE);
        div()
            .relative()
            .size_full()
            .child(list)
            .scrollbar(scroll, ScrollbarAxis::Both)
    }
}

impl Line {
    fn is_header(&self) -> bool {
        matches!(self.kind, Kind::Metadata | Kind::Hunk)
    }

    fn render(&self, number_width: Pixels, cx: &App) -> Div {
        let theme = cx.theme();
        let (added, removed, _) = crate::changes::change_colors(cx);
        let muted = theme.muted_foreground.blend(theme.foreground.opacity(0.23));
        let row = h_flex().h(px(23.)).min_w_full();
        if self.is_header() {
            return row
                .bg(theme.muted)
                .border_y_1()
                .border_color(theme.border)
                .px_3()
                .text_size(HEADER_SIZE)
                .text_color(muted)
                .child(self.text.clone());
        }

        let (marker, color) = match self.kind {
            Kind::Added => ("+", Some(added)),
            Kind::Removed => ("-", Some(removed)),
            _ => ("", None),
        };
        let gutter = color.map_or(theme.muted, |color| color.opacity(0.19));
        let number = |value: Option<usize>| {
            div()
                .w(number_width)
                .flex_none()
                .pr_2()
                .text_right()
                .text_xs()
                .child(value.map(|n| n.to_string()).unwrap_or_default())
        };
        let _probe = format!("Diff line/{marker}{}", self.text);
        row.when_some(color, |row, color| row.bg(color.opacity(0.11)))
            .child(
                h_flex()
                    .h_full()
                    .flex_none()
                    .bg(gutter)
                    .text_color(color.unwrap_or(muted))
                    .child(number(self.before))
                    .child(number(self.after))
                    .child(
                        div()
                            .w(MARKER_WIDTH)
                            .flex_none()
                            .text_center()
                            .child(marker),
                    ),
            )
            .child(control!(
                _probe,
                div().px_3().flex_none().whitespace_nowrap().child(
                    StyledText::new(self.text.clone())
                        .with_highlights(self.highlights[usize::from(theme.is_dark())].clone())
                )
            ))
    }
}

struct Hunk {
    before: Range<usize>,
    after: Range<usize>,
}

impl Hunk {
    fn parse(line: &str) -> Option<Self> {
        let mut parts = line.strip_prefix("@@ ")?.split_ascii_whitespace();
        let before = hunk_range(parts.next()?.strip_prefix('-')?)?;
        let after = hunk_range(parts.next()?.strip_prefix('+')?)?;
        (parts.next()? == "@@").then_some(Self { before, after })
    }

    fn consume(&mut self, line: &str) -> Option<(Kind, Option<usize>, Option<usize>)> {
        match line.as_bytes().first().copied() {
            Some(b' ') | None if !self.before.is_empty() && !self.after.is_empty() => {
                Some((Kind::Context, self.before.next(), self.after.next()))
            }
            Some(b'+') if !self.after.is_empty() => Some((Kind::Added, None, self.after.next())),
            Some(b'-') if !self.before.is_empty() => {
                Some((Kind::Removed, self.before.next(), None))
            }
            _ => None,
        }
    }
}

fn hunk_range(part: &str) -> Option<Range<usize>> {
    let (start, count) = part.split_once(',').unwrap_or((part, "1"));
    let start = start.parse::<usize>().ok()?;
    let end = start.checked_add(count.parse().ok()?)?;
    Some(start..end)
}

fn highlight_side(lines: &mut [Line], language: Language, before: bool) {
    let belongs_to_side = |line: &Line| {
        if before {
            line.before.is_some()
        } else {
            line.after.is_some()
        }
    };
    let mut source = String::new();
    for line in lines.iter() {
        if belongs_to_side(line) {
            source.push_str(&line.text);
            source.push('\n');
        } else if line.kind == Kind::Hunk {
            source.push('\n');
        }
    }
    let mut highlighter = SyntaxHighlighter::new(language.name());
    highlighter.update(None, &Rope::from_str(&source));
    for (mode, theme) in [
        HighlightTheme::default_light(),
        HighlightTheme::default_dark(),
    ]
    .iter()
    .enumerate()
    {
        let styles = highlighter.styles(&(0..source.len()), theme);
        let mut offset = 0;
        for line in lines.iter_mut() {
            if belongs_to_side(line) {
                let end = offset + line.text.len();
                if !before || line.kind == Kind::Removed {
                    let first = styles.partition_point(|(range, _)| range.end <= offset);
                    line.highlights[mode] = styles[first..]
                        .iter()
                        .take_while(|(range, _)| range.start < end)
                        .filter_map(|(range, style)| {
                            let range =
                                range.start.max(offset) - offset..range.end.min(end) - offset;
                            (!range.is_empty()).then_some((range, *style))
                        })
                        .collect();
                }
                offset = end + 1;
            } else if line.kind == Kind::Hunk {
                offset += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Diff;
    use std::path::Path;

    #[test]
    fn unified_diff_keeps_line_numbers_and_non_text_changes() {
        let input = "diff --git a/file b/file\nold mode 100644\nnew mode 100755\n--- a/file\n+++ b/file\n@@ -8,2 +8,3 @@\n old\n-previous\n+new\n+added\n\\ No newline at end of file\n\nStaged changes\n\n@@ -0,0 +1 @@\n++++ content\n";
        let diff = Diff::new(input, Path::new("file.txt"));
        let rows: Vec<_> = diff
            .lines
            .iter()
            .map(|line| (line.text.as_ref(), line.before, line.after))
            .collect();
        assert_eq!(
            rows,
            [
                ("old mode 100644", None, None),
                ("new mode 100755", None, None),
                ("@@ -8,2 +8,3 @@", None, None),
                ("old", Some(8), Some(8)),
                ("previous", Some(9), None),
                ("new", None, Some(9)),
                ("added", None, Some(10)),
                ("\\ No newline at end of file", None, None),
                ("Staged changes", None, None),
                ("@@ -0,0 +1 @@", None, None),
                ("+++ content", None, Some(1)),
            ]
        );
    }

    #[test]
    fn unprefixed_empty_context_advances_both_line_numbers() {
        let diff = Diff::new(
            "@@ -1,4 +1,3 @@\n first\n-removed\n\n last\n",
            Path::new("file.txt"),
        );
        let rows: Vec<_> = diff
            .lines
            .iter()
            .skip(1)
            .map(|line| (line.text.as_ref(), line.before, line.after))
            .collect();
        assert_eq!(
            rows,
            [
                ("first", Some(1), Some(1)),
                ("removed", Some(2), None),
                ("", Some(3), Some(2)),
                ("last", Some(4), Some(3))
            ]
        );
    }
}
