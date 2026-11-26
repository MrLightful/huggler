use super::super::{history_at_bottom, transcript::transcript_muted};
use anyhow::{Result, ensure};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use mermaid_rs_renderer::{LayoutConfig, Theme, compute_layout, parse_mermaid_strict, render_svg};
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg,
};
use std::sync::{Arc, LazyLock};

const MAX_SOURCE_BYTES: usize = 16 * 1024;
const MAX_DIAGRAM_PIXELS: f32 = 2048.;

pub(super) fn is_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("mermaid")
}

// The Markdown parser also recognizes fences which implicitly close at EOF.
// Keep those as source until a closing marker arrives.
pub(super) fn is_closed(source: &str) -> bool {
    let Some((opening, rest)) = source.split_once('\n') else {
        return false;
    };
    let opening = opening.trim_start();
    let Some(marker @ ('`' | '~')) = opening.chars().next() else {
        return false;
    };
    let count = opening.chars().take_while(|ch| *ch == marker).count();
    let closing = rest
        .lines()
        .last()
        .unwrap_or_default()
        .trim_start_matches([' ', '\t', '>'])
        .trim_end();
    count >= 3 && closing.len() >= count && closing.chars().all(|ch| ch == marker)
}

struct Preview {
    diagram: Option<Result<Arc<RenderImage>>>,
    show_source: bool,
}

pub(super) fn render(
    id: &str,
    source: &str,
    code_block: impl FnOnce(&mut Window, &mut App) -> Option<Div>,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let dark = cx.theme().mode.is_dark();
    let source = SharedString::from(source.to_owned());
    let key = SharedString::from(format!("{id}-mermaid-{}", gpui::hash(&(&source, dark))));
    let preview = window.use_keyed_state(key, cx, |_, cx| {
        let source = source.clone();
        let scroll = scroll.clone();
        let task = cx
            .background_executor()
            .spawn(async move { rasterize(&source, dark) });
        cx.spawn(async move |preview, cx| {
            let diagram = task.await;
            let _ = preview.update(cx, |preview: &mut Preview, cx| {
                if history_at_bottom(&scroll) {
                    scroll.scroll_to_bottom();
                }
                preview.diagram = Some(diagram);
                cx.notify();
            });
        })
        .detach();
        Preview {
            diagram: None,
            show_source: false,
        }
    });
    let state = preview.read(cx);
    let diagram = state
        .diagram
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned();
    let show_source = state.show_source;
    let status = match &state.diagram {
        None => "Rendering diagram…",
        Some(Err(_)) => "Diagram unavailable",
        Some(Ok(_)) => "Mermaid",
    };
    let label = if show_source {
        "Show diagram"
    } else {
        "Show source"
    };
    let body = match diagram.as_ref().filter(|_| !show_source) {
        Some(image) => {
            let size = image.size(0);
            let width = size.width.0 as f32 / 2.;
            let height = size.height.0 as f32 / 2.;
            let mut frame = div()
                .relative()
                .w_full()
                .max_w(px(width.min(480. * width / height)));
            frame.style().aspect_ratio = Some(width / height);
            Some(
                control!(
                    "Mermaid diagram",
                    frame.child(img(image.clone()).absolute().size_full())
                )
                .w_full(),
            )
        }
        None => code_block(window, cx),
    };
    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .justify_between()
                .child(control!(
                    status,
                    div()
                        .text_xs()
                        .text_color(transcript_muted(cx))
                        .child(status)
                ))
                .child(
                    h_flex()
                        .gap_1()
                        .when(diagram.is_some(), |row| {
                            row.child(control!(
                                label,
                                Button::new(SharedString::from(format!("{id}-mermaid-toggle")))
                                    .ghost()
                                    .small()
                                    .label(label)
                                    .on_click(move |_, _, cx| {
                                        preview.update(cx, |state, cx| {
                                            state.show_source = !state.show_source;
                                            cx.notify();
                                        });
                                    })
                            ))
                        })
                        .child(control!(
                            "Copy Mermaid source",
                            Button::new(SharedString::from(format!("{id}-mermaid-copy")))
                                .ghost()
                                .small()
                                .label("Copy source")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        source.to_string(),
                                    ));
                                })
                        )),
                ),
        )
        .children(body)
}

fn rasterize(source: &str, dark: bool) -> Result<Arc<RenderImage>> {
    ensure!(
        source.len() <= MAX_SOURCE_BYTES,
        "Mermaid source is too large"
    );
    let parsed = parse_mermaid_strict(source)?;
    ensure!(
        parsed.graph.nodes.len() <= 200 && parsed.graph.edges.len() <= 300,
        "Mermaid graph is too large"
    );
    let theme = if dark { Theme::dark() } else { Theme::modern() };
    let layout_config = LayoutConfig::default();
    let layout = compute_layout(&parsed.graph, &theme, &layout_config);
    let svg = render_svg(&layout, &theme, &layout_config);

    static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        Arc::new(fonts)
    });
    let options = usvg::Options {
        fontdb: FONTS.clone(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(svg.as_bytes(), &options)?;
    let size = tree.size();
    let scale = (MAX_DIAGRAM_PIXELS / size.width().max(size.height())).min(2.);
    let width = (size.width() * scale).ceil() as u32;
    let height = (size.height() * scale).ceil() as u32;
    let mut pixmap =
        Pixmap::new(width, height).ok_or_else(|| anyhow::anyhow!("Invalid diagram size"))?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let pixels = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.blue(), color.green(), color.red(), color.alpha()]
        })
        .collect();
    let buffer = image::RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| anyhow::anyhow!("Invalid diagram pixels"))?;
    Ok(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::{MAX_DIAGRAM_PIXELS, is_closed, rasterize};

    #[test]
    fn recognizes_closed_fences() {
        assert!(is_closed("```mermaid\nflowchart TD\nA --> B\n```"));
        assert!(is_closed(
            "~~~~mermaid\n> sequenceDiagram\n> A->>B: Hi\n> ~~~~"
        ));
        assert!(!is_closed("````mermaid\nA --> B\n```"));
        assert!(!is_closed("```mermaid\nA --> B"));
    }

    #[test]
    fn rasterizes_diagrams_and_rejects_invalid_source() {
        for source in [
            "flowchart TD\nA[Start] --> B[Done]",
            "sequenceDiagram\nAlice->>Bob: Hello",
        ] {
            let image = rasterize(source, false).unwrap();
            assert!(
                image
                    .as_bytes(0)
                    .unwrap()
                    .chunks_exact(4)
                    .any(|pixel| pixel[3] > 0)
            );
            assert!(image.size(0).width.0 <= MAX_DIAGRAM_PIXELS as i32);
        }
        assert!(rasterize("unknown diagram", true).is_err());
    }
}
