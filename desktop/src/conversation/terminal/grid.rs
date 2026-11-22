use super::emulator::*;
use gpui::*;
use gpui_component::ActiveTheme;
use portable_pty::PtySize;

const FONT_SIZE: Pixels = px(12.);
const LINE_HEIGHT: Pixels = px(17.);
const PADDING: Pixels = px(8.);

#[derive(Clone, PartialEq)]
pub(super) struct Palette {
    foreground: Rgba,
    background: Rgba,
    dark: bool,
}

impl Palette {
    pub(super) fn new(cx: &App) -> Self {
        Self {
            foreground: cx.theme().foreground.into(),
            background: cx.theme().background.into(),
            dark: cx.theme().is_dark(),
        }
    }

    pub(super) fn apply(&self, emulator: &mut Emulator) {
        let ansi = if self.dark {
            [
                0x332f2c, 0xf08080, 0x81d99a, 0xe1ba70, 0x8bb7f0, 0xd3a0ed, 0x78cccf, 0xd9d4ce,
                0x847d75, 0xffa2a2, 0xa7edb7, 0xf6d39a, 0xa9cafa, 0xe3baf5, 0x9fe1e3, 0xf7f2ed,
            ]
        } else {
            [
                0x242321, 0xa72538, 0x216d3b, 0x795a16, 0x265da6, 0x81399f, 0x12696f, 0x66615b,
                0x78726c, 0xb52b3d, 0x2a763f, 0x805c10, 0x2b62ad, 0x8b429f, 0x197378, 0x746e68,
            ]
        };
        let mut palette = [Color::default(); 256];
        for (entry, value) in palette.iter_mut().zip(ansi) {
            *entry = Color {
                r: (value >> 16) as u8,
                g: (value >> 8) as u8,
                b: value as u8,
            };
        }
        for (index, entry) in palette[16..232].iter_mut().enumerate() {
            let levels = [0, 95, 135, 175, 215, 255];
            *entry = Color {
                r: levels[index / 36],
                g: levels[index / 6 % 6],
                b: levels[index % 6],
            };
        }
        for (index, entry) in palette[232..].iter_mut().enumerate() {
            let gray = 8 + 10 * index as u8;
            *entry = Color {
                r: gray,
                g: gray,
                b: gray,
            };
        }
        emulator.colors(
            terminal_color(self.foreground),
            terminal_color(self.background),
            palette,
        );
    }
}

fn terminal_color(color: Rgba) -> Color {
    Color {
        r: (color.r * 255.).round() as u8,
        g: (color.g * 255.).round() as u8,
        b: (color.b * 255.).round() as u8,
    }
}

fn color(value: Color) -> Hsla {
    rgb(u32::from_be_bytes([0, value.r, value.g, value.b])).into()
}

#[derive(Clone)]
pub(super) struct Grid {
    origin: Point<Pixels>,
    pub cell: Size<Pixels>,
    font: Font,
    cols: u16,
    rows: u16,
    pixel_cell_width: u16,
    pixel_cell_height: u16,
}

impl Grid {
    pub(super) fn new(bounds: Bounds<Pixels>, family: SharedString, window: &Window) -> Self {
        let font = gpui::font(family);
        let text = window.text_system();
        let id = text.resolve_font(&font);
        let width = text
            .advance(id, FONT_SIZE, 'M')
            .map_or(px(8.), |size| size.width);
        let scale = window.scale_factor();
        let pixel_cell_width = (f32::from(width) * scale).round().max(1.) as u16;
        let pixel_cell_height = (f32::from(LINE_HEIGHT) * scale).round().max(1.) as u16;
        let cell = size(
            px(f32::from(pixel_cell_width) / scale),
            px(f32::from(pixel_cell_height) / scale),
        );
        Self {
            origin: bounds.origin + point(PADDING, PADDING),
            cell,
            font,
            cols: ((bounds.size.width - PADDING * 2.) / cell.width).max(1.) as u16,
            rows: ((bounds.size.height - PADDING * 2.) / cell.height).max(1.) as u16,
            pixel_cell_width,
            pixel_cell_height,
        }
    }

    pub(super) fn pty_size(&self) -> PtySize {
        PtySize {
            cols: self.cols,
            rows: self.rows,
            pixel_width: self.pixel_cell_width.saturating_mul(self.cols),
            pixel_height: self.pixel_cell_height.saturating_mul(self.rows),
        }
    }

    pub(super) fn origin(&self, col: u16, row: u16) -> Point<Pixels> {
        self.origin
            + point(
                self.cell.width * f32::from(col),
                self.cell.height * f32::from(row),
            )
    }

    pub(super) fn grid_point(&self, point: Point<Pixels>) -> GridPoint {
        let p = point - self.origin;
        GridPoint {
            x: (p.x / self.cell.width).clamp(0., f32::from(self.cols - 1)) as u16,
            y: (p.y / self.cell.height).clamp(0., f32::from(self.rows - 1)) as u16,
        }
    }

    pub(super) fn mouse_position(&self, point: Point<Pixels>) -> (f32, f32) {
        let p = point - self.origin;
        (
            (p.x / self.cell.width * f32::from(self.pixel_cell_width))
                .clamp(0., f32::from(self.pty_size().pixel_width) - 1.),
            (p.y / self.cell.height * f32::from(self.pixel_cell_height))
                .clamp(0., f32::from(self.pty_size().pixel_height) - 1.),
        )
    }

    pub(super) fn paint(
        &self,
        bounds: Bounds<Pixels>,
        screen: Screen,
        decoration: Decoration<'_>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let fg = color(screen.foreground);
        let bg = color(screen.background);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, bg));
            for cell in &screen.cells {
                let (_, background) = cell_colors(cell, fg, bg, decoration.selection);
                if let Some(background) = background {
                    window.paint_quad(fill(
                        Bounds::new(self.origin(cell.x, cell.y), self.cell),
                        background,
                    ));
                }
            }

            let cursor = screen.cursor.filter(|_| decoration.running);
            let mut block_cursor = None;
            if let Some(cursor) = &cursor {
                let origin = self.origin(cursor.x, cursor.y);
                let color = color(cursor.color);
                let mut cursor_bounds = Bounds::new(origin, self.cell);
                let wide = screen
                    .cells
                    .iter()
                    .any(|cell| cell.x == cursor.x && cell.y == cursor.y && cell.wide);
                if wide {
                    cursor_bounds.size.width *= 2.;
                }
                let cursor_quad = match cursor.style {
                    _ if !decoration.focused
                        || cursor.style == GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW =>
                    {
                        outline(cursor_bounds, color, BorderStyle::Solid)
                    }
                    GHOSTTY_CURSOR_STYLE_BAR => {
                        cursor_bounds.size.width = px(1.5);
                        fill(cursor_bounds, color)
                    }
                    GHOSTTY_CURSOR_STYLE_UNDERLINE => {
                        cursor_bounds.origin.y += self.cell.height - px(1.5);
                        cursor_bounds.size.height = px(1.5);
                        fill(cursor_bounds, color)
                    }
                    _ => {
                        block_cursor = Some((cursor.x, cursor.y));
                        fill(cursor_bounds, color)
                    }
                };
                window.paint_quad(cursor_quad);
            }

            for cell in &screen.cells {
                if cell.text.is_empty() || cell.style.invisible {
                    continue;
                }
                let (mut foreground, _) = cell_colors(cell, fg, bg, decoration.selection);
                if block_cursor == Some((cell.x, cell.y)) {
                    foreground = bg;
                }
                let mut font = self.font.clone();
                if cell.style.bold {
                    font.weight = FontWeight::BOLD;
                }
                if cell.style.italic {
                    font.style = FontStyle::Italic;
                }
                let run = TextRun {
                    len: cell.text.len(),
                    font,
                    color: foreground,
                    background_color: None,
                    underline: cell.style.underline.then_some(UnderlineStyle {
                        thickness: px(1.),
                        ..Default::default()
                    }),
                    strikethrough: cell.style.strikethrough.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        ..Default::default()
                    }),
                };
                let advance = if cell.wide {
                    self.cell.width * 2.
                } else {
                    self.cell.width
                };
                let line = window.text_system().shape_line(
                    cell.text.clone().into(),
                    FONT_SIZE,
                    &[run],
                    Some(advance),
                );
                let _ = line.paint(self.origin(cell.x, cell.y), self.cell.height, window, cx);
            }

            if let Some(cursor) = cursor
                && !decoration.composition.is_empty()
            {
                let text = decoration.composition.replace(['\r', '\n'], " ");
                let run = TextRun {
                    len: text.len(),
                    font: self.font.clone(),
                    color: fg,
                    background_color: None,
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        ..Default::default()
                    }),
                    strikethrough: None,
                };
                let origin = self.origin(cursor.x, cursor.y);
                let line = window
                    .text_system()
                    .shape_line(text.into(), FONT_SIZE, &[run], None);
                window.paint_quad(fill(
                    Bounds::new(origin, size(line.width, self.cell.height)),
                    bg,
                ));
                let _ = line.paint(origin, self.cell.height, window, cx);
            }
        });
    }
}

pub(super) struct Decoration<'a> {
    pub selection: Hsla,
    pub focused: bool,
    pub running: bool,
    pub composition: &'a str,
}

fn cell_colors(cell: &Cell, fg: Hsla, bg: Hsla, selection: Hsla) -> (Hsla, Option<Hsla>) {
    let mut foreground = cell.style.foreground.map_or(fg, color);
    let mut background = cell.style.background.map(color);
    if cell.style.inverse {
        (foreground, background) = (background.unwrap_or(bg), Some(foreground));
    }
    if cell.selected {
        (foreground, background) = (fg, Some(selection));
    }
    if cell.style.faint {
        foreground.a *= 0.65;
    }
    (foreground, background)
}
