use std::{ops::Range, rc::Rc};

use gpui::{AnyElement, App, Pixels, Point, ShapedLine, SharedString, Window, point, px};
use smallvec::SmallVec;

type RenderInline = dyn Fn(Range<usize>, Pixels, Pixels, bool) -> AnyElement;

/// A native inline element that replaces matching text visually while preserving
/// the backing text for the input handler, selection, clipboard and undo.
pub struct InputInlineElement {
    marker: SharedString,
    pub(super) width: Pixels,
    render: Rc<RenderInline>,
}

impl InputInlineElement {
    pub fn new(
        marker: impl Into<SharedString>,
        width: Pixels,
        render: impl Fn(Range<usize>, Pixels, Pixels, bool) -> AnyElement + 'static,
    ) -> Self {
        Self {
            marker: marker.into(),
            width,
            render: Rc::new(render),
        }
    }

    pub(super) fn render(
        &self,
        range: Range<usize>,
        width: Pixels,
        height: Pixels,
        selected: bool,
    ) -> AnyElement {
        (self.render)(range, width, height, selected)
    }
}

pub(super) fn matches(text: &str, elements: &[InputInlineElement]) -> Vec<(Range<usize>, usize)> {
    let mut matches = Vec::new();
    for (index, element) in elements.iter().enumerate() {
        if !element.marker.is_empty() {
            matches.extend(
                text.match_indices(element.marker.as_str())
                    .map(|(start, marker)| (start..start + marker.len(), index)),
            );
        }
    }
    matches.sort_unstable_by_key(|(range, _)| range.start);
    let mut end = 0;
    matches.retain(|(range, _)| {
        if range.start < end {
            return false;
        }
        end = range.end;
        true
    });
    matches
}

pub(super) fn same_geometry(a: &[InputInlineElement], b: &[InputInlineElement]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| a.marker == b.marker && a.width == b.width)
}

pub(super) enum RowFragment {
    Text(ShapedLine),
    Element {
        index: usize,
        width: Pixels,
        len: usize,
    },
}

struct RowPart {
    start: usize,
    x: Pixels,
    fragment: RowFragment,
}

pub(crate) struct VisualRow {
    pub(super) text: SharedString,
    pub(super) width: Pixels,
    parts: SmallVec<[RowPart; 1]>,
}

impl From<ShapedLine> for VisualRow {
    fn from(line: ShapedLine) -> Self {
        Self::new(line.text.clone(), [RowFragment::Text(line)])
    }
}

impl VisualRow {
    pub(super) fn new(
        text: SharedString,
        fragments: impl IntoIterator<Item = RowFragment>,
    ) -> Self {
        let mut width = px(0.);
        let mut offset = 0;
        let parts = fragments
            .into_iter()
            .map(|fragment| {
                let part = RowPart {
                    start: offset,
                    x: width,
                    fragment,
                };
                match &part.fragment {
                    RowFragment::Text(line) => {
                        offset += line.len();
                        width += line.width;
                    }
                    RowFragment::Element {
                        width: advance,
                        len,
                        ..
                    } => {
                        offset += len;
                        width += *advance;
                    }
                }
                part
            })
            .collect();
        Self { text, width, parts }
    }

    pub(super) fn len(&self) -> usize {
        self.text.len()
    }

    pub(super) fn x_for_index(&self, index: usize) -> Pixels {
        if index >= self.len() {
            return self.width;
        }
        self.parts
            .iter()
            .rev()
            .find(|part| part.start <= index)
            .map_or(px(0.), |part| {
                part.x
                    + match &part.fragment {
                        RowFragment::Text(line) => line.x_for_index(index - part.start),
                        RowFragment::Element { width, .. } => {
                            if index == part.start {
                                px(0.)
                            } else {
                                *width
                            }
                        }
                    }
            })
    }

    pub(super) fn closest_index_for_x(&self, x: Pixels) -> usize {
        if x >= self.width {
            return self.len();
        }
        self.parts
            .iter()
            .rev()
            .find(|part| part.x <= x)
            .map_or(0, |part| {
                part.start
                    + match &part.fragment {
                        RowFragment::Text(line) => line.closest_index_for_x(x - part.x),
                        RowFragment::Element { width, len, .. } => {
                            if x - part.x < *width / 2. {
                                0
                            } else {
                                *len
                            }
                        }
                    }
            })
    }

    pub(super) fn index_for_x(&self, x: Pixels) -> Option<usize> {
        if x >= self.width {
            return None;
        }
        self.parts
            .iter()
            .rev()
            .find(|part| part.x <= x)
            .and_then(|part| match &part.fragment {
                RowFragment::Text(line) => {
                    line.index_for_x(x - part.x).map(|index| part.start + index)
                }
                RowFragment::Element { .. } => Some(self.closest_index_for_x(x)),
            })
    }

    pub(super) fn elements(
        &self,
    ) -> impl Iterator<Item = (Pixels, Pixels, usize, Range<usize>)> + '_ {
        self.parts.iter().filter_map(|part| match part.fragment {
            RowFragment::Element { width, index, len } => {
                Some((part.x, width, index, part.start..part.start + len))
            }
            RowFragment::Text(_) => None,
        })
    }

    pub(super) fn paint(
        &self,
        origin: Point<Pixels>,
        height: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        for part in &self.parts {
            if let RowFragment::Text(line) = &part.fragment {
                _ = line.paint(origin + point(part.x, px(0.)), height, window, cx);
            }
        }
    }
}
