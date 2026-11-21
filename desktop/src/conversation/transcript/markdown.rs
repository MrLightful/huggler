use super::transcript_muted;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, h_flex,
    text::{TextView, TextViewStyle},
    v_flex,
};
use markdown::mdast::Node;

pub(super) fn prose(id: String, source: &str, window: &mut Window, cx: &mut App) -> Div {
    let safe = safe_markdown(source);
    let mut body = v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .text_sm()
        .line_height(relative(1.55));
    let Ok(Node::Root(root)) = markdown::to_mdast(&safe, &markdown::ParseOptions::gfm()) else {
        return body.child(source.to_owned());
    };
    let definitions = root
        .children
        .iter()
        .filter_map(|node| match node {
            Node::Definition(_) => node_source(node, &safe),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    for (index, node) in root.children.iter().enumerate() {
        body = body.children(
            MarkdownBlock {
                source: &safe,
                definitions: &definitions,
            }
            .render(node, &format!("{id}-{index}"), window, cx),
        );
    }
    body
}

struct MarkdownBlock<'a> {
    source: &'a str,
    definitions: &'a str,
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
            Node::Blockquote(quote) => Some(
                self.children(&quote.children, id, window, cx)
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .pl_3()
                    .text_color(transcript_muted(cx)),
            ),
            Node::Definition(_) => None,
            _ => {
                let source = node_source(node, self.source)?;
                let source = if self.definitions.is_empty() {
                    source.to_owned()
                } else {
                    format!("{source}\n\n{}", self.definitions)
                };
                let style = TextViewStyle {
                    highlight_theme: cx.theme().highlight_theme.clone(),
                    is_dark: cx.theme().mode.is_dark(),
                    ..Default::default()
                }
                .heading_font_size(|level, _| {
                    px(match level {
                        1 => 20.,
                        2 => 18.,
                        _ => 15.,
                    })
                });
                Some(
                    div().w_full().min_w_0().child(
                        TextView::markdown(SharedString::from(id.to_owned()), source, window, cx)
                            .style(style)
                            .selectable(true),
                    ),
                )
            }
        }
    }

    fn children(&self, nodes: &[Node], id: &str, window: &mut Window, cx: &mut App) -> Div {
        v_flex().gap_1().children(
            nodes.iter().enumerate().filter_map(|(index, node)| {
                self.render(node, &format!("{id}-{index}"), window, cx)
            }),
        )
    }
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
