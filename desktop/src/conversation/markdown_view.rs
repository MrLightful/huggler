use super::transcript::transcript_muted;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, h_flex,
    text::{TextView, TextViewStyle},
    v_flex,
};
use markdown::mdast::Node;
use std::borrow::Cow;

pub(super) fn prose(id: String, source: &str, window: &mut Window, cx: &mut App) -> Div {
    Markdown::new(source).render(id, source, false, window, cx)
}

pub(super) struct Markdown {
    source: String,
    root: Option<markdown::mdast::Root>,
    definitions: String,
}

impl Markdown {
    pub(super) fn new(source: &str) -> Self {
        let source = safe_markdown(source);
        let root = match markdown::to_mdast(&source, &markdown::ParseOptions::gfm()) {
            Ok(Node::Root(root)) => Some(root),
            _ => None,
        };
        let definitions = root
            .iter()
            .flat_map(|root| &root.children)
            .filter(|node| matches!(node, Node::Definition(_)))
            .filter_map(|node| node_source(node, &source))
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            source,
            root,
            definitions,
        }
    }

    pub(super) fn render(
        &self,
        id: String,
        raw: &str,
        document: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let mut body = v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .text_sm()
            .line_height(relative(1.55))
            .when(document, |body| body.gap_3().line_height(relative(1.65)));
        let Some(root) = &self.root else {
            return body.child(raw.to_owned());
        };
        let blocks = MarkdownBlock {
            source: &self.source,
            definitions: &self.definitions,
            in_list: false,
            document,
        };
        for (index, node) in root.children.iter().enumerate() {
            body = body.children(blocks.render(node, &format!("{id}-{index}"), window, cx));
        }
        body
    }
}

struct MarkdownBlock<'a> {
    source: &'a str,
    definitions: &'a str,
    in_list: bool,
    document: bool,
}

impl MarkdownBlock<'_> {
    fn render(&self, node: &Node, id: &str, window: &mut Window, cx: &mut App) -> Option<Div> {
        match node {
            // GPUI Component's intrinsic list layout can under-measure wrapped
            // rows. Give each marker and its contents separate flex columns.
            Node::List(list) => Some(
                v_flex()
                    .gap_1()
                    .children(
                        list.children
                            .iter()
                            .enumerate()
                            .filter_map(|(index, node)| {
                                let Node::ListItem(item) = node else {
                                    return None;
                                };
                                let marker = match list.start {
                                    Some(start) => {
                                        format!("{}.", u64::from(start) + index as u64)
                                    }
                                    _ => "•".into(),
                                };
                                let children = self.children(
                                    &item.children,
                                    &format!("{id}-{index}"),
                                    window,
                                    cx,
                                );
                                Some(
                                    h_flex()
                                        .items_start()
                                        .gap_2()
                                        .child(
                                            div()
                                                .min_w(px(14.))
                                                .text_right()
                                                .text_color(transcript_muted(cx))
                                                .children(item.checked.is_none().then_some(marker))
                                                .children(item.checked.map(|checked| {
                                                    div()
                                                        .mt_1()
                                                        .size_3()
                                                        .border_1()
                                                        .rounded_sm()
                                                        .border_color(cx.theme().border)
                                                        .when(checked, |div| {
                                                            div.child(
                                                                Icon::new(IconName::Check).size_3(),
                                                            )
                                                        })
                                                })),
                                        )
                                        .child(children.flex_1().min_w_0()),
                                )
                            }),
                    ),
            ),
            Node::Definition(_) => None,
            _ => {
                let source = standalone_source(node, self.source, self.in_list)?;
                let source = if self.definitions.is_empty() {
                    source.into_owned()
                } else {
                    format!("{source}\n\n{}", self.definitions)
                };
                let style = TextViewStyle {
                    highlight_theme: cx.theme().highlight_theme.clone(),
                    is_dark: cx.theme().mode.is_dark(),
                    ..Default::default()
                }
                .heading_font_size({
                    let document = self.document;
                    move |level, _| {
                        px(match (document, level) {
                            (true, 1) => 28.,
                            (true, 2) => 21.,
                            (true, 3) => 17.,
                            (_, 1) => 20.,
                            (_, 2) => 18.,
                            _ => 15.,
                        })
                    }
                });
                Some(
                    div()
                        .w_full()
                        .min_w_0()
                        .when(self.document && matches!(node, Node::Heading(_)), |block| {
                            block.mt_3()
                        })
                        .child(
                            TextView::markdown(
                                SharedString::from(id.to_owned()),
                                source,
                                window,
                                cx,
                            )
                            .style(style)
                            .selectable(true),
                        ),
                )
            }
        }
    }

    fn children(&self, nodes: &[Node], id: &str, window: &mut Window, cx: &mut App) -> Div {
        let block = Self {
            in_list: true,
            ..*self
        };
        v_flex().gap_1().children(
            nodes.iter().enumerate().filter_map(|(index, node)| {
                block.render(node, &format!("{id}-{index}"), window, cx)
            }),
        )
    }
}

fn standalone_source<'a>(node: &Node, source: &'a str, in_list: bool) -> Option<Cow<'a, str>> {
    let prefix_width = node.position()?.start.column.saturating_sub(1);
    let source = node_source(node, source)?;
    if !in_list || prefix_width == 0 {
        return Some(source.into());
    }
    let Some((first, rest)) = source.split_once('\n') else {
        return Some(source.into());
    };
    // The first line starts at the block's marker. Continuation lines still
    // carry the list indentation in the original source.
    let mut standalone = String::with_capacity(source.len());
    standalone.push_str(first);
    standalone.push('\n');
    for line in rest.split_inclusive('\n') {
        let prefix = line
            .bytes()
            .take(prefix_width)
            .take_while(|byte| *byte == b' ')
            .count();
        standalone.push_str(&line[prefix..]);
    }
    Some(standalone.into())
}

fn node_source<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    let pos = node.position()?;
    source.get(pos.start.offset..pos.end.offset)
}

// GPUI's renderer can load images and open links. Keep raw HTML and image URLs
// inert and allow explicit navigation only to web URLs.
fn safe_markdown(source: &str) -> String {
    fn visit(node: &Node, source: &str, edits: &mut Vec<(std::ops::Range<usize>, String)>) {
        let replacement = match node {
            Node::Image(image) => Some(escape_markdown(&image.alt)),
            Node::ImageReference(image) => Some(escape_markdown(&image.alt)),
            Node::Html(_) => node
                .position()
                .map(|pos| escape_markdown(&source[pos.start.offset..pos.end.offset])),
            Node::Link(link) if !web_url(&link.url) => node
                .position()
                .map(|pos| escape_markdown(&source[pos.start.offset..pos.end.offset])),
            Node::Definition(link) if !web_url(&link.url) => node
                .position()
                .map(|pos| escape_markdown(&source[pos.start.offset..pos.end.offset])),
            _ => None,
        };
        if let Some(replacement) = replacement
            && let Some(pos) = node.position()
        {
            edits.push((pos.start.offset..pos.end.offset, replacement));
        } else if let Some(children) = node.children() {
            for child in children {
                visit(child, source, edits);
            }
        }
    }
    let mut safe = source.to_owned();
    if source.contains(['[', '<'])
        && let Ok(tree) = markdown::to_mdast(source, &markdown::ParseOptions::gfm())
    {
        let mut edits = Vec::new();
        visit(&tree, source, &mut edits);
        for (span, text) in edits.into_iter().rev() {
            safe.replace_range(span, &text);
        }
    }
    safe
}

fn escape_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn web_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

#[cfg(test)]
mod tests {
    use super::standalone_source;

    #[test]
    fn standalone_list_blocks_keep_quotes_and_code_indentation() {
        let source = "1. > First line\n   > Second line\n\n   ```text\n     two spaces\n   ```";
        let tree = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).unwrap();
        let list = &tree.children().unwrap()[0];
        let item = &list.children().unwrap()[0];
        let blocks = item.children().unwrap();
        assert_eq!(
            standalone_source(&blocks[0], source, true).unwrap(),
            "> First line\n> Second line"
        );
        assert_eq!(
            standalone_source(&blocks[1], source, true).unwrap(),
            "```text\n  two spaces\n```"
        );
    }
}
