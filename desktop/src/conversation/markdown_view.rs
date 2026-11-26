mod response_image;

use super::transcript::transcript_muted;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, h_flex,
    text::{TextView, TextViewStyle},
    v_flex,
};
use markdown::mdast::Node;
use std::{borrow::Cow, path::Path};

pub(crate) fn prose(
    id: String,
    source: &str,
    images: ResponseImages<'_>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    if !source.contains("![") {
        return Markdown::new(source).render(id, source, false, window, cx);
    }
    Markdown::parse(source.to_owned()).render_blocks(id, source, false, Some(images), window, cx)
}

#[derive(Clone, Copy)]
pub(crate) struct ResponseImages<'a> {
    pub(crate) run: &'a huggler::RunId,
    pub(crate) checkout: Option<&'a Path>,
    pub(crate) executor: &'a tokio::runtime::Handle,
    pub(crate) scroll: &'a ScrollHandle,
}

pub(super) struct Markdown {
    source: String,
    root: Option<markdown::mdast::Root>,
    definitions: String,
}

impl Markdown {
    pub(super) fn new(source: &str) -> Self {
        Self::parse(safe_markdown(source, false))
    }

    fn parse(source: String) -> Self {
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
        self.render_blocks(id, raw, document, None, window, cx)
    }

    fn render_blocks(
        &self,
        id: String,
        raw: &str,
        document: bool,
        images: Option<ResponseImages<'_>>,
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
            quote_depth: 0,
            document,
            images,
            root,
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
    quote_depth: usize,
    document: bool,
    images: Option<ResponseImages<'a>>,
    root: &'a markdown::mdast::Root,
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
                                let children = Self {
                                    in_list: true,
                                    ..*self
                                }
                                .children(
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
            Node::Paragraph(paragraph) if self.images.is_some() => self
                .response_paragraph(node, &paragraph.children, id, window, cx)
                .or_else(|| self.text_block(node, id, window, cx)),
            Node::Blockquote(quote) if self.images.is_some() && contains_image(node) => Some(
                Self {
                    quote_depth: self.quote_depth + 1,
                    ..*self
                }
                .children(&quote.children, id, window, cx)
                .pl_3()
                .border_l_2()
                .border_color(cx.theme().border),
            ),
            _ => self.text_block(node, id, window, cx),
        }
    }

    fn text_block(&self, node: &Node, id: &str, window: &mut Window, cx: &mut App) -> Option<Div> {
        let position = node.position()?;
        let source = node_source(node, self.source)?;
        Some(self.text(
            source,
            position.start.column.saturating_sub(1),
            matches!(node, Node::Heading(_)),
            id,
            window,
            cx,
        ))
    }

    fn text(
        &self,
        source: &str,
        prefix_width: usize,
        heading: bool,
        id: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let source = standalone_source(source, prefix_width, self.in_list, self.quote_depth);
        let source = if self.definitions.is_empty() {
            source.into_owned()
        } else {
            format!("{source}\n\n{}", self.definitions)
        };
        let source = if self.images.is_some() {
            safe_markdown(&source, true)
        } else {
            source
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
        div()
            .w_full()
            .min_w_0()
            .when(self.document && heading, |block| block.mt_3())
            .child(
                TextView::markdown(SharedString::from(id.to_owned()), source, window, cx)
                    .style(style)
                    .selectable(true),
            )
    }

    fn response_paragraph(
        &self,
        node: &Node,
        children: &[Node],
        id: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Div> {
        let images = self.images?;
        let position = node.position()?;
        let prefix = position.start.column.saturating_sub(1);
        let mut start = position.start.offset;
        let mut body = v_flex().w_full().min_w_0().gap_2();
        for (index, child) in children.iter().enumerate() {
            let Some(image) = inline_image(child, self.root) else {
                continue;
            };
            let span = child.position()?;
            let before = &self.source[start..span.start.offset];
            if !before.trim().is_empty() {
                body = body.child(self.text(
                    before,
                    prefix,
                    false,
                    &format!("{id}-text-{index}"),
                    window,
                    cx,
                ));
            }
            body = body.child(response_image::render(
                &format!("{id}-image-{index}"),
                image,
                images,
                window,
                cx,
            ));
            start = span.end.offset;
        }
        if start == position.start.offset {
            return None;
        }
        let after = &self.source[start..position.end.offset];
        if !after.trim().is_empty() {
            body = body.child(self.text(after, prefix, false, &format!("{id}-end"), window, cx));
        }
        Some(body)
    }

    fn children(&self, nodes: &[Node], id: &str, window: &mut Window, cx: &mut App) -> Div {
        v_flex().gap_1().children(
            nodes.iter().enumerate().filter_map(|(index, node)| {
                self.render(node, &format!("{id}-{index}"), window, cx)
            }),
        )
    }
}

struct InlineImage<'a> {
    destination: &'a str,
    alt: &'a str,
    link: Option<&'a str>,
}

fn definition_url<'a>(nodes: &'a [Node], identifier: &str) -> Option<&'a str> {
    nodes.iter().find_map(|node| match node {
        Node::Definition(definition) if definition.identifier == identifier => {
            Some(definition.url.as_str())
        }
        _ => definition_url(node.children()?, identifier),
    })
}

fn inline_image<'a>(node: &'a Node, root: &'a markdown::mdast::Root) -> Option<InlineImage<'a>> {
    match node {
        Node::Image(image) => Some(InlineImage {
            destination: &image.url,
            alt: &image.alt,
            link: None,
        }),
        Node::ImageReference(image) => Some(InlineImage {
            destination: definition_url(&root.children, &image.identifier)?,
            alt: &image.alt,
            link: None,
        }),
        Node::Link(_)
        | Node::LinkReference(_)
        | Node::Emphasis(_)
        | Node::Strong(_)
        | Node::Delete(_) => match node.children()?.as_slice() {
            [child] => {
                let mut image = inline_image(child, root)?;
                let link = match node {
                    Node::Link(link) => Some(link.url.as_str()),
                    Node::LinkReference(link) => definition_url(&root.children, &link.identifier),
                    _ => image.link,
                };
                image.link = link.filter(|url| web_url(url));
                Some(image)
            }
            _ => None,
        },
        _ => None,
    }
}

fn contains_image(node: &Node) -> bool {
    matches!(node, Node::Image(_) | Node::ImageReference(_))
        || node
            .children()
            .is_some_and(|children| children.iter().any(contains_image))
}

fn standalone_source(
    source: &str,
    prefix_width: usize,
    in_list: bool,
    quote_depth: usize,
) -> Cow<'_, str> {
    if (!in_list || prefix_width == 0) && quote_depth == 0 {
        return source.into();
    }
    let Some((first, rest)) = source.split_once('\n') else {
        return source.into();
    };
    // The first line starts at the block's marker. Continuation lines still
    // carry the list indentation in the original source.
    let mut standalone = String::with_capacity(source.len());
    standalone.push_str(first);
    standalone.push('\n');
    for line in rest.split_inclusive('\n') {
        let mut content = line;
        for _ in 0..quote_depth {
            let Some(quote) = content.trim_start_matches(' ').strip_prefix('>') else {
                break;
            };
            content = quote.strip_prefix(' ').unwrap_or(quote);
        }
        let prefix = content
            .bytes()
            .take(if in_list {
                prefix_width.saturating_sub(line.len() - content.len())
            } else {
                0
            })
            .take_while(|byte| *byte == b' ')
            .count();
        standalone.push_str(&content[prefix..]);
    }
    standalone.into()
}

fn node_source<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    let pos = node.position()?;
    source.get(pos.start.offset..pos.end.offset)
}

// GPUI's renderer can load images and open links. Keep raw HTML and image URLs
// inert and allow explicit navigation only to web URLs.
fn safe_markdown(source: &str, omit_unsafe_definitions: bool) -> String {
    fn visit(
        node: &Node,
        source: &str,
        omit_unsafe_definitions: bool,
        edits: &mut Vec<(std::ops::Range<usize>, String)>,
    ) {
        let replacement = match node {
            Node::Image(image) => Some(escape_markdown(&image.alt)),
            Node::ImageReference(image) => Some(escape_markdown(&image.alt)),
            Node::Html(_) => node
                .position()
                .map(|pos| escape_markdown(&source[pos.start.offset..pos.end.offset])),
            Node::Link(link) if !web_url(&link.url) => node
                .position()
                .map(|pos| escape_markdown(&source[pos.start.offset..pos.end.offset])),
            Node::Definition(link) if !web_url(&link.url) && omit_unsafe_definitions => {
                Some(String::new())
            }
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
                visit(child, source, omit_unsafe_definitions, edits);
            }
        }
    }
    let mut safe = source.to_owned();
    if source.contains(['[', '<'])
        && let Ok(tree) = markdown::to_mdast(source, &markdown::ParseOptions::gfm())
    {
        let mut edits = Vec::new();
        visit(&tree, source, omit_unsafe_definitions, &mut edits);
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
    use super::{node_source, standalone_source};

    #[test]
    fn standalone_list_blocks_keep_quotes_and_code_indentation() {
        let source = "1. > First line\n   > Second line\n\n   ```text\n     two spaces\n   ```";
        let tree = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).unwrap();
        let list = &tree.children().unwrap()[0];
        let item = &list.children().unwrap()[0];
        let blocks = item.children().unwrap();
        assert_eq!(
            standalone_source(node_source(&blocks[0], source).unwrap(), 3, true, 0),
            "> First line\n> Second line"
        );
        assert_eq!(
            standalone_source(node_source(&blocks[1], source).unwrap(), 3, true, 0),
            "```text\n  two spaces\n```"
        );
    }
}
