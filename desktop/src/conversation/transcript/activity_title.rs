use gpui::*;

const FADE_WIDTH: Pixels = px(32.);

pub(super) struct ActivityTitle(pub(super) SharedString);

impl IntoElement for ActivityTitle {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ActivityTitle {
    type RequestLayoutState = ShapedLine;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ShapedLine) {
        let style = window.text_style();
        let rem = window.rem_size();
        let font_size = style.font_size.to_pixels(rem);
        let line = window.text_system().shape_line(
            self.0.clone(),
            font_size,
            &[style.to_run(self.0.len())],
            None,
        );
        let width = line.width.ceil();
        let height = style.line_height.to_pixels(font_size.into(), rem).ceil();
        // Report no minimum text width so the surrounding button can shrink past
        // the intrinsic width of this single line while keeping its icons visible.
        let layout =
            window.request_measured_layout(Style::default(), move |known, available, _, _| {
                let width = known.width.unwrap_or(match available.width {
                    AvailableSpace::MinContent => px(0.),
                    AvailableSpace::MaxContent => width,
                    AvailableSpace::Definite(available) => width.min(available),
                });
                size(width, known.height.unwrap_or(height))
            });
        (layout, line)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut ShapedLine,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        line: &mut ShapedLine,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let width = bounds.size.width;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if line.width <= width {
                let _ = line.paint(bounds.origin, bounds.size.height, window, cx);
                return;
            }

            let color = window.text_style().color;
            let fade_width = width.min(FADE_WIDTH);
            let baseline = bounds.origin
                + point(
                    px(0.),
                    (bounds.size.height + line.ascent - line.descent) / 2.,
                );
            // Fade the glyphs themselves so the label blends into any button background.
            for run in &line.runs {
                for glyph in run
                    .glyphs
                    .iter()
                    .take_while(|glyph| glyph.position.x < width)
                {
                    let origin = point(baseline.x + glyph.position.x, baseline.y);
                    let opacity = ((width - glyph.position.x) / fade_width).min(1.);
                    if glyph.is_emoji {
                        let _ = window.paint_emoji(origin, run.font_id, glyph.id, line.font_size);
                    } else {
                        let _ = window.paint_glyph(
                            origin,
                            run.font_id,
                            glyph.id,
                            line.font_size,
                            color.opacity(opacity),
                        );
                    }
                }
            }
        });
    }
}
