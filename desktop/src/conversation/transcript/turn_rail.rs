use super::*;
use ::markdown::{ParseOptions, mdast::Node, to_mdast};
use std::cell::Cell;

const PREVIEW_WIDTH: Pixels = px(278.);
const MARKER_WIDTH: Pixels = px(7.);
const MARKER_TARGET: Size<Pixels> = size(px(22.), px(10.));

pub(super) struct TurnPreview {
    id: huggler::MessageId,
    question: SharedString,
    response: Option<SharedString>,
}

impl TurnPreview {
    pub(super) fn new(
        messages: &[HistoryItem],
        activity: Option<&RunActivity>,
        streaming: Option<&str>,
    ) -> Self {
        let question = messages.iter().find(|item| item.message.role == Role::User);
        let response = messages
            .iter()
            .filter(|item| item.message.role == Role::Assistant)
            .find_map(|item| response_excerpt(&item.message.text))
            .or_else(|| {
                activity?.items.iter().find_map(|item| {
                    if item.kind == ActivityKind::Commentary {
                        response_excerpt(&item.title)
                    } else {
                        None
                    }
                })
            })
            .or_else(|| response_excerpt(streaming?));
        let question = question.and_then(|item| match &item.files {
            Some(files) if files.caption.trim().is_empty() => excerpt(&files.names.join(", "), 140),
            Some(files) => excerpt(&files.caption, 140),
            None => excerpt(&item.message.text, 140),
        });
        let (question, response) = match question {
            Some(question) => (question, response),
            None => (response.unwrap_or_else(|| "Conversation turn".into()), None),
        };
        Self {
            id: messages[0].message.id.clone(),
            question,
            response,
        }
    }
}

fn excerpt(text: &str, limit: usize) -> Option<SharedString> {
    let mut text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((end, _)) = text.char_indices().nth(limit) {
        text.truncate(end);
        text = format!("{}…", text.trim_end());
    }
    (!text.is_empty()).then(|| text.into())
}

fn response_excerpt(text: &str) -> Option<SharedString> {
    let prefix = &text[..text.floor_char_boundary(text.len().min(2048))];
    let parsed = to_mdast(prefix, &ParseOptions::gfm());
    let plain = match &parsed {
        Ok(root) => block_text(root),
        Err(_) => prefix.to_owned(),
    };
    excerpt(&plain, 240)
}

fn block_text(node: &Node) -> String {
    match node {
        Node::Root(_)
        | Node::List(_)
        | Node::ListItem(_)
        | Node::Blockquote(_)
        | Node::Table(_)
        | Node::TableRow(_) => node
            .children()
            .into_iter()
            .flatten()
            .map(block_text)
            .collect::<Vec<_>>()
            .join(" "),
        _ => node.to_string(),
    }
}

#[derive(Default)]
struct RailPosition {
    scroll: ScrollHandle,
    active: Cell<Option<(usize, Pixels)>>,
}

pub(super) fn render(
    previews: Vec<TurnPreview>,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let last = previews.len() - 1;
    let position = window.use_keyed_state(
        SharedString::from(format!("turn-rail-{}", previews[0].id)),
        cx,
        |_, _| RailPosition::default(),
    );
    let rail_scroll = position.read(cx).scroll.clone();
    // The transcript lays out first, so the rail can follow its clamped offset.
    let sync = canvas(
        {
            let position = position.clone();
            let history = scroll.clone();
            move |bounds, _, cx| {
                let rail = position.read(cx);
                let selected = if history_at_bottom(&history) {
                    last
                } else {
                    history.top_item().min(last)
                };
                let active = Some((selected, bounds.size.height));
                if rail.active.replace(active) != active {
                    if selected == last {
                        rail.scroll.scroll_to_bottom();
                    } else if selected == 0 {
                        rail.scroll.set_offset(Point::default());
                    } else {
                        rail.scroll.scroll_to_item(selected);
                    }
                }
            }
        },
        |_, (), _, _| {},
    )
    .absolute()
    .size_full();

    v_flex()
        .absolute()
        .left_1()
        .top_3()
        .bottom_3()
        .justify_center()
        .child(sync)
        .child(
            v_flex()
                .id("turn-rail")
                .track_scroll(&rail_scroll)
                .min_h_0()
                .overflow_y_scroll()
                .children(previews.into_iter().enumerate().map(|(index, preview)| {
                    let position = position.clone();
                    let jump = scroll.clone();
                    let quiet = cx.theme().muted_foreground.opacity(0.55);
                    let active = cx.theme().foreground;
                    let marker = canvas(
                        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                        move |bounds, hitbox, window, cx| {
                            let selected = position
                                .read(cx)
                                .active
                                .get()
                                .is_some_and(|(selected, _)| selected == index);
                            let hovered = hitbox.is_hovered(window);
                            let width = if hovered {
                                MARKER_WIDTH * 2.
                            } else {
                                MARKER_WIDTH
                            };
                            let origin =
                                point(bounds.left() + px(3.), bounds.center().y - px(0.75));
                            let marker = Bounds::new(origin, size(width, px(1.5)));
                            window.paint_quad(fill(
                                marker,
                                if selected || hovered { active } else { quiet },
                            ));
                            #[cfg(all(feature = "qualification", target_os = "macos"))]
                            if window.content_mask().bounds.contains(&marker.center()) {
                                if selected {
                                    crate::qualification::observe_bounds(
                                        format!("active-turn/{}", index + 1).into(),
                                        marker,
                                        cx,
                                    );
                                }
                                if marker.size.width > MARKER_WIDTH {
                                    crate::qualification::observe_bounds(
                                        format!("wide-turn/{}", index + 1).into(),
                                        marker,
                                        cx,
                                    );
                                }
                            }
                        },
                    )
                    .w(MARKER_TARGET.width)
                    .h(MARKER_TARGET.height);
                    let TurnPreview {
                        id,
                        question,
                        response,
                    } = preview;
                    control!(
                        format!("turn/{}", index + 1),
                        div()
                            .id(SharedString::from(format!("turn-preview-{id}")))
                            .tooltip(move |window, cx| {
                                let question = question.clone();
                                let response = response.clone();
                                Tooltip::element(move |window, cx| {
                                    preview_card(&question, response.as_ref(), window, cx)
                                })
                                .rounded_lg()
                                .p_3()
                                .build(window, cx)
                            })
                            .child(
                                Button::new(SharedString::from(id.0))
                                    .ghost()
                                    .w(MARKER_TARGET.width)
                                    .h(MARKER_TARGET.height)
                                    .p_0()
                                    .border_0()
                                    .child(marker)
                                    .on_click(move |_, window, _| {
                                        jump.scroll_to_top_of_item(index);
                                        window.refresh();
                                    }),
                            )
                    )
                })),
        )
}

fn preview_card(
    question: &SharedString,
    response: Option<&SharedString>,
    window: &mut Window,
    cx: &App,
) -> Div {
    let title = fit_preview(question, 2, FontWeight::MEDIUM, window, cx);
    v_flex()
        .w(PREVIEW_WIDTH)
        .gap_1()
        .child(control!(
            format!("turn-question/{question}"),
            div()
                .font_weight(FontWeight::MEDIUM)
                .line_clamp(2)
                .child(title)
        ))
        .when_some(response, |card, response| {
            let text = fit_preview(response, 3, FontWeight::NORMAL, window, cx);
            card.child(control!(
                format!("turn-response/{response}"),
                div()
                    .text_color(cx.theme().muted_foreground)
                    .line_clamp(3)
                    .child(text)
            ))
        })
}

fn fit_preview(
    text: &SharedString,
    lines: usize,
    weight: FontWeight,
    window: &Window,
    cx: &App,
) -> SharedString {
    let mut font = font(cx.theme().font_family.clone());
    font.weight = weight;
    let font_size = rems(0.875).to_pixels(window.rem_size());
    let mut wrapper = cx.text_system().line_wrapper(font, font_size);
    // Reserve space for an ellipsis before cutting at a wrap boundary.
    let fragments = [LineFragment::text(text.as_ref())];
    match wrapper
        .wrap_line(&fragments, PREVIEW_WIDTH - font_size)
        .nth(lines - 1)
    {
        Some(boundary) => format!("{}…", text[..boundary.ix].trim_end()).into(),
        None => text.clone(),
    }
}
