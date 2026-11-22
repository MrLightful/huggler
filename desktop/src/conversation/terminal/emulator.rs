use anyhow::{Result, ensure};
use gpui::{Keystroke, Modifiers};
use std::{cell::RefCell, ffi::c_void, mem::size_of, ptr, slice};

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/ghostty.rs"));
}
use ffi::*;

pub(super) use ffi::{
    GHOSTTY_CURSOR_STYLE_BAR, GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW, GHOSTTY_CURSOR_STYLE_UNDERLINE,
    GHOSTTY_MOUSE_ACTION_MOTION, GHOSTTY_MOUSE_ACTION_PRESS, GHOSTTY_MOUSE_ACTION_RELEASE,
    GHOSTTY_MOUSE_BUTTON_FIVE, GHOSTTY_MOUSE_BUTTON_FOUR, GHOSTTY_MOUSE_BUTTON_LEFT,
    GHOSTTY_MOUSE_BUTTON_MIDDLE, GHOSTTY_MOUSE_BUTTON_RIGHT, GhosttyColorRgb as Color,
};

#[derive(Clone, Copy, Default)]
pub(super) struct CellStyle {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

pub(super) struct Screen {
    pub cells: Vec<Cell>,
    pub foreground: Color,
    pub background: Color,
    pub cursor: Option<Cursor>,
}

pub(super) struct Cell {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub wide: bool,
    pub selected: bool,
    pub style: CellStyle,
}

pub(super) struct Cursor {
    pub x: u16,
    pub y: u16,
    pub style: GhosttyCursorStyle,
    pub color: Color,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct GridPoint {
    pub x: u16,
    pub y: u16,
}

// Ghostty handles stay on the GPUI thread. Destroy dependent handles before
// the terminal and keep the callback's userdata at a stable address.
pub(super) struct Emulator {
    terminal: GhosttyTerminal,
    state: GhosttyRenderState,
    rows: GhosttyRenderStateRowIterator,
    cells: GhosttyRenderStateRowCells,
    key_encoder: GhosttyKeyEncoder,
    key_event: GhosttyKeyEvent,
    mouse_encoder: GhosttyMouseEncoder,
    mouse_event: GhosttyMouseEvent,
    selection_start: GhosttyTrackedGridRef,
    replies: Box<RefCell<Vec<u8>>>,
}

impl Emulator {
    pub(super) fn new(cols: u16, rows: u16) -> Result<Self> {
        let mut this = Self {
            terminal: ptr::null_mut(),
            state: ptr::null_mut(),
            rows: ptr::null_mut(),
            cells: ptr::null_mut(),
            key_encoder: ptr::null_mut(),
            key_event: ptr::null_mut(),
            mouse_encoder: ptr::null_mut(),
            mouse_event: ptr::null_mut(),
            selection_start: ptr::null_mut(),
            replies: Box::default(),
        };
        let allocator = ptr::null();
        unsafe {
            for result in [
                ghostty_terminal_new(allocator, &mut this.terminal, cols, rows),
                ghostty_render_state_new(allocator, &mut this.state),
                ghostty_render_state_row_iterator_new(allocator, &mut this.rows),
                ghostty_render_state_row_cells_new(allocator, &mut this.cells),
                ghostty_key_encoder_new(allocator, &mut this.key_encoder),
                ghostty_key_event_new(allocator, &mut this.key_event),
                ghostty_mouse_encoder_new(allocator, &mut this.mouse_encoder),
                ghostty_mouse_event_new(allocator, &mut this.mouse_event),
            ] {
                ensure!(result == GHOSTTY_SUCCESS, "Initialize Ghostty ({result})");
            }
            ghostty_terminal_set(
                this.terminal,
                GHOSTTY_TERMINAL_OPT_USERDATA,
                ptr::from_ref(this.replies.as_ref()).cast(),
            );
            ghostty_terminal_set(
                this.terminal,
                GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                write_reply as *const c_void,
            );
        }
        this.set(
            GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
            &(4 * 1024 * 1024_usize),
        );
        this.set(GHOSTTY_TERMINAL_OPT_RESIZE_PULL_SCROLLBACK, &true);
        // The view draws text cells. Disable the image protocol, including its
        // file-backed media, because this surface has no image renderer.
        this.set(GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT, &0_u64);
        this.set(GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_FILE, &false);
        this.set(GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_TEMP_FILE, &false);
        this.set(GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_SHARED_MEM, &false);
        Ok(this)
    }

    fn set<T>(&mut self, option: GhosttyTerminalOption, value: &T) {
        // Each caller supplies the payload type specified for its option.
        unsafe { ghostty_terminal_set(self.terminal, option, ptr::from_ref(value).cast()) };
    }

    pub(super) fn colors(&mut self, foreground: Color, background: Color, palette: [Color; 256]) {
        self.set(GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, &foreground);
        self.set(GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, &background);
        self.set(GHOSTTY_TERMINAL_OPT_COLOR_PALETTE, &palette);
    }

    pub(super) fn feed(&mut self, data: &[u8]) {
        unsafe { ghostty_terminal_vt_write(self.terminal, data.as_ptr(), data.len()) };
    }

    pub(super) fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut *self.replies.borrow_mut())
    }

    pub(super) fn resize(&mut self, cols: u16, rows: u16, width: u16, height: u16) -> Result<()> {
        let result = unsafe {
            ghostty_terminal_resize(
                self.terminal,
                cols,
                rows,
                u32::from(width),
                u32::from(height),
            )
        };
        ensure!(result == GHOSTTY_SUCCESS, "Resize Ghostty ({result})");
        Ok(())
    }

    pub(super) fn scroll(&mut self, lines: isize) {
        self.scroll_viewport(
            GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA,
            GhosttyTerminalScrollViewportValue { delta: lines },
        );
    }

    pub(super) fn scroll_to_bottom(&mut self) {
        self.scroll_viewport(
            GHOSTTY_TERMINAL_SCROLL_VIEWPORT_BOTTOM,
            GhosttyTerminalScrollViewportValue::default(),
        );
    }

    fn scroll_viewport(
        &mut self,
        tag: GhosttyTerminalScrollViewportTag,
        value: GhosttyTerminalScrollViewportValue,
    ) {
        unsafe {
            ghostty_terminal_scroll_viewport(
                self.terminal,
                GhosttyTerminalScrollViewport { tag, value },
            )
        };
    }

    fn mode(&self, dec_mode: u16) -> bool {
        let mut mode = GhosttyTerminalModeConfig {
            mode: dec_mode,
            value: false,
        };
        let result = unsafe {
            ghostty_terminal_get(
                self.terminal,
                GHOSTTY_TERMINAL_DATA_MODE,
                ptr::from_mut(&mut mode).cast(),
            )
        };
        result == GHOSTTY_SUCCESS && mode.value
    }

    pub(super) fn alternate_scroll(&self) -> bool {
        let mut screen = GHOSTTY_TERMINAL_SCREEN_PRIMARY;
        unsafe {
            ghostty_terminal_get(
                self.terminal,
                GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN,
                ptr::from_mut(&mut screen).cast(),
            )
        };
        screen == GHOSTTY_TERMINAL_SCREEN_ALTERNATE && self.mode(1007)
    }

    pub(super) fn focus(&self, focused: bool) -> &'static [u8] {
        if !self.mode(1004) {
            return b"";
        }
        if focused { b"\x1b[I" } else { b"\x1b[O" }
    }

    pub(super) fn key(&mut self, stroke: &Keystroke, held: bool, released: bool) -> Vec<u8> {
        let key = key_code(&stroke.key);
        let text = stroke.key_char.as_deref().unwrap_or_default();
        if key == GHOSTTY_KEY_UNIDENTIFIED && text.is_empty() {
            return Vec::new();
        }
        let action = if released {
            GHOSTTY_KEY_ACTION_RELEASE
        } else if held {
            GHOSTTY_KEY_ACTION_REPEAT
        } else {
            GHOSTTY_KEY_ACTION_PRESS
        };
        let mut encoded = vec![0; text.len().saturating_add(64)];
        let mut len = 0;
        let result = unsafe {
            ghostty_key_encoder_set_from_terminal(self.key_encoder, self.terminal);
            ghostty_key_event_set_action(self.key_event, action);
            ghostty_key_event_set_key(self.key_event, key);
            ghostty_key_event_set_mods(self.key_event, modifiers(stroke.modifiers));
            let unshifted = stroke
                .key
                .chars()
                .next()
                .filter(|_| stroke.key.chars().count() == 1);
            ghostty_key_event_set_unshifted_codepoint(
                self.key_event,
                unshifted.map_or(0, u32::from),
            );
            ghostty_key_event_set_utf8(
                self.key_event,
                GhosttyString {
                    ptr: text.as_ptr(),
                    len: text.len(),
                },
            );
            ghostty_key_encoder_encode(
                self.key_encoder,
                self.key_event,
                encoded.as_mut_ptr(),
                encoded.len(),
                &mut len,
            )
        };
        encoded.truncate(if result == GHOSTTY_SUCCESS { len } else { 0 });
        encoded
    }

    pub(super) fn paste(&mut self, text: &str, confirmed: bool) -> Option<Vec<u8>> {
        if !confirmed
            && !self.mode(2004)
            && !unsafe { ghostty_paste_is_safe(text.as_ptr().cast(), text.len()) }
        {
            return None;
        }
        let mut text = text.as_bytes().to_vec();
        let mut encoded = vec![0; text.len().saturating_add(16)];
        let mut len = 0;
        let result = unsafe {
            ghostty_paste_encode(
                text.as_mut_ptr().cast(),
                text.len(),
                self.mode(2004),
                encoded.as_mut_ptr(),
                encoded.len(),
                &mut len,
            )
        };
        encoded.truncate(if result == GHOSTTY_SUCCESS { len } else { 0 });
        Some(encoded)
    }

    pub(super) fn mouse_tracking(&self) -> bool {
        let mut enabled = false;
        unsafe {
            ghostty_terminal_get(
                self.terminal,
                GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING,
                ptr::from_mut(&mut enabled).cast(),
            );
        }
        enabled
    }

    pub(super) fn mouse(
        &mut self,
        action: GhosttyMouseAction,
        button: Option<GhosttyMouseButton>,
        mods: Modifiers,
        position: (f32, f32),
        size: portable_pty::PtySize,
    ) -> Vec<u8> {
        let dimensions = GhosttyMouseEncoderSize {
            size: size_of::<GhosttyMouseEncoderSize>(),
            screen_width: u32::from(size.pixel_width),
            screen_height: u32::from(size.pixel_height),
            cell_width: u32::from(size.pixel_width / size.cols),
            cell_height: u32::from(size.pixel_height / size.rows),
            ..Default::default()
        };
        let mut data = [0; 64];
        let mut len = 0;
        let result = unsafe {
            ghostty_mouse_encoder_set_from_terminal(self.mouse_encoder, self.terminal);
            ghostty_mouse_encoder_set(
                self.mouse_encoder,
                GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
                ptr::from_ref(&dimensions).cast(),
            );
            ghostty_mouse_encoder_set(
                self.mouse_encoder,
                GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
                ptr::from_ref(&button.is_some()).cast(),
            );
            ghostty_mouse_encoder_set(
                self.mouse_encoder,
                GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL,
                ptr::from_ref(&true).cast(),
            );
            ghostty_mouse_event_set_position(
                self.mouse_event,
                GhosttyMousePosition {
                    x: position.0,
                    y: position.1,
                },
            );
            ghostty_mouse_event_set_action(self.mouse_event, action);
            ghostty_mouse_event_set_mods(self.mouse_event, modifiers(mods));
            match button {
                Some(button) => ghostty_mouse_event_set_button(self.mouse_event, button),
                None => ghostty_mouse_event_clear_button(self.mouse_event),
            };
            ghostty_mouse_encoder_encode(
                self.mouse_encoder,
                self.mouse_event,
                data.as_mut_ptr(),
                data.len(),
                &mut len,
            )
        };
        if result == GHOSTTY_SUCCESS {
            data[..len].to_vec()
        } else {
            Vec::new()
        }
    }

    pub(super) fn clear_selection(&mut self) {
        self.finish_selection();
        unsafe { ghostty_terminal_set(self.terminal, GHOSTTY_TERMINAL_OPT_SELECTION, ptr::null()) };
    }

    pub(super) fn start_selection(&mut self, point: GridPoint) {
        self.clear_selection();
        unsafe {
            ghostty_terminal_grid_ref_track(
                self.terminal,
                viewport_point(point),
                &mut self.selection_start,
            )
        };
    }

    pub(super) fn extend_selection(&mut self, point: GridPoint) {
        if self.selection_start.is_null() {
            return;
        }
        let mut selection = GhosttySelection::default();
        unsafe {
            if ghostty_tracked_grid_ref_snapshot(self.selection_start, &mut selection.start)
                == GHOSTTY_SUCCESS
                && ghostty_terminal_grid_ref(
                    self.terminal,
                    viewport_point(point),
                    &mut selection.end,
                ) == GHOSTTY_SUCCESS
            {
                self.set(GHOSTTY_TERMINAL_OPT_SELECTION, &selection);
            }
        }
    }

    pub(super) fn finish_selection(&mut self) {
        if !self.selection_start.is_null() {
            unsafe { ghostty_tracked_grid_ref_free(self.selection_start) };
            self.selection_start = ptr::null_mut();
        }
    }

    pub(super) fn select_all(&mut self) {
        let mut selection = GhosttySelection::default();
        if unsafe { ghostty_terminal_select_all(self.terminal, &mut selection) } == GHOSTTY_SUCCESS
        {
            self.set(GHOSTTY_TERMINAL_OPT_SELECTION, &selection);
        }
    }

    pub(super) fn selected_text(&self) -> Option<String> {
        let mut selection = GhosttySelection::default();
        let mut formatter = ptr::null_mut();
        unsafe {
            if ghostty_terminal_get(
                self.terminal,
                GHOSTTY_TERMINAL_DATA_SELECTION,
                ptr::from_mut(&mut selection).cast(),
            ) != GHOSTTY_SUCCESS
                || ghostty_formatter_terminal_new(ptr::null(), &mut formatter, self.terminal)
                    != GHOSTTY_SUCCESS
            {
                return None;
            }
            ghostty_formatter_set(
                formatter,
                GHOSTTY_FORMATTER_OPT_SELECTION,
                ptr::from_ref(&selection).cast(),
            );
            ghostty_formatter_set(
                formatter,
                GHOSTTY_FORMATTER_OPT_UNWRAP,
                ptr::from_ref(&true).cast(),
            );
            ghostty_formatter_set(
                formatter,
                GHOSTTY_FORMATTER_OPT_TRIM,
                ptr::from_ref(&true).cast(),
            );
            let mut len = 0;
            ghostty_formatter_format_buf(formatter, ptr::null_mut(), 0, &mut len);
            let mut text = vec![0; len];
            let result =
                ghostty_formatter_format_buf(formatter, text.as_mut_ptr(), text.len(), &mut len);
            ghostty_formatter_free(formatter);
            (result == GHOSTTY_SUCCESS).then(|| {
                text.truncate(len);
                String::from_utf8_lossy(&text).into_owned()
            })
        }
    }

    pub(super) fn screen(&mut self) -> Option<Screen> {
        let mut colors = GhosttyRenderStateColors {
            size: size_of::<GhosttyRenderStateColors>(),
            ..Default::default()
        };
        let mut cursor = GhosttyRenderStateCursor {
            size: size_of::<GhosttyRenderStateCursor>(),
            ..Default::default()
        };
        unsafe {
            if ghostty_render_state_update(self.state, self.terminal) != GHOSTTY_SUCCESS
                || ghostty_render_state_get(
                    self.state,
                    GHOSTTY_RENDER_STATE_DATA_COLORS,
                    ptr::from_mut(&mut colors).cast(),
                ) != GHOSTTY_SUCCESS
                || ghostty_render_state_get(
                    self.state,
                    GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                    ptr::from_mut(&mut self.rows).cast(),
                ) != GHOSTTY_SUCCESS
            {
                return None;
            }
            ghostty_render_state_get(
                self.state,
                GHOSTTY_RENDER_STATE_DATA_CURSOR,
                ptr::from_mut(&mut cursor).cast(),
            );
        }
        let cursor = (cursor.visible && cursor.viewport_has_value).then_some(Cursor {
            x: cursor
                .viewport_x
                .saturating_sub(u16::from(cursor.wide_tail)),
            y: cursor.viewport_y,
            style: cursor.visual_style,
            color: if colors.cursor_has_value {
                colors.cursor
            } else {
                colors.foreground
            },
        });
        let mut cells = Vec::new();
        let mut y = 0;
        let mut text_buffer = vec![0; 64];
        while unsafe { ghostty_render_state_row_iterator_next(self.rows) } {
            unsafe {
                ghostty_render_state_row_get(
                    self.rows,
                    GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                    ptr::from_mut(&mut self.cells).cast(),
                )
            };
            let mut x = 0;
            while unsafe { ghostty_render_state_row_cells_next(self.cells) } {
                if let Some(cell) = self.cell(x, y, &mut text_buffer) {
                    cells.push(cell);
                }
                x += 1;
            }
            y += 1;
        }
        Some(Screen {
            cells,
            foreground: colors.foreground,
            background: colors.background,
            cursor,
        })
    }

    fn cell_data<T>(&self, key: GhosttyRenderStateRowCellsData, value: &mut T) -> bool {
        let result = unsafe {
            ghostty_render_state_row_cells_get(self.cells, key, ptr::from_mut(value).cast())
        };
        result == GHOSTTY_SUCCESS
    }

    fn cell(&self, x: u16, y: u16, text_buffer: &mut Vec<u8>) -> Option<Cell> {
        let mut raw = 0;
        let mut width = GHOSTTY_CELL_WIDE_NARROW;
        let mut selected = false;
        self.cell_data(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW, &mut raw);
        self.cell_data(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_SELECTED, &mut selected);
        unsafe {
            ghostty_cell_get(
                raw,
                GHOSTTY_CELL_DATA_WIDE,
                ptr::from_mut(&mut width).cast(),
            )
        };
        let mut text = GhosttyBuffer {
            ptr: text_buffer.as_mut_ptr(),
            cap: text_buffer.len(),
            len: 0,
        };
        if !self.cell_data(
            GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
            &mut text,
        ) && text.len > text.cap
        {
            text_buffer.resize(text.len, 0);
            text.ptr = text_buffer.as_mut_ptr();
            text.cap = text_buffer.len();
            if !self.cell_data(
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
                &mut text,
            ) {
                text.len = 0;
            }
        }
        let mut foreground = Color::default();
        let mut background = Color::default();
        let foreground = self
            .cell_data(
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
                &mut foreground,
            )
            .then_some(foreground);
        let background = self
            .cell_data(
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
                &mut background,
            )
            .then_some(background);
        let mut style = GhosttyStyle::default();
        self.cell_data(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE, &mut style);
        if text.len == 0 && !selected && background.is_none() && style.flags == 0 {
            return None;
        }
        // A wide cell's tail carries independent selection/background state.
        // Only its head paints the grapheme.
        let wide = width == GHOSTTY_CELL_WIDE_WIDE;
        let styled = |mask: u16| style.flags & mask != 0;
        Some(Cell {
            x,
            y,
            wide,
            selected,
            text: String::from_utf8_lossy(&text_buffer[..text.len.min(text_buffer.len())])
                .into_owned(),
            style: CellStyle {
                foreground,
                background,
                bold: styled(1 << 0),
                italic: styled(1 << 1),
                faint: styled(1 << 2),
                inverse: styled(1 << 4),
                invisible: styled(1 << 5),
                strikethrough: styled(1 << 6),
                underline: styled(0x7 << 8),
            },
        })
    }
}

impl Drop for Emulator {
    fn drop(&mut self) {
        self.finish_selection();
        unsafe {
            ghostty_mouse_event_free(self.mouse_event);
            ghostty_mouse_encoder_free(self.mouse_encoder);
            ghostty_key_event_free(self.key_event);
            ghostty_key_encoder_free(self.key_encoder);
            ghostty_render_state_row_cells_free(self.cells);
            ghostty_render_state_row_iterator_free(self.rows);
            ghostty_render_state_free(self.state);
            ghostty_terminal_free(self.terminal);
        }
    }
}

unsafe extern "C" fn write_reply(
    _: GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    if len != 0 {
        // Ghostty calls synchronously with the boxed userdata and borrowed bytes.
        unsafe { &*userdata.cast::<RefCell<Vec<u8>>>() }
            .borrow_mut()
            .extend_from_slice(unsafe { slice::from_raw_parts(data, len) });
    }
}

fn viewport_point(point: GridPoint) -> GhosttyPoint {
    GhosttyPoint {
        tag: GHOSTTY_POINT_TAG_VIEWPORT,
        value: GhosttyPointValue {
            coordinate: GhosttyPointCoordinate {
                x: point.x,
                y: u32::from(point.y),
            },
        },
    }
}

fn modifiers(mods: Modifiers) -> GhosttyMods {
    let mut encoded = 0;
    for (pressed, flag) in [
        (mods.shift, GHOSTTY_MODS_SHIFT),
        (mods.control, GHOSTTY_MODS_CTRL),
        (mods.alt, GHOSTTY_MODS_ALT),
        (mods.platform, GHOSTTY_MODS_SUPER),
    ] {
        if pressed {
            encoded |= flag as GhosttyMods;
        }
    }
    encoded
}

fn key_code(key: &str) -> GhosttyKey {
    match key.as_bytes() {
        [letter @ b'a'..=b'z'] => GHOSTTY_KEY_A + GhosttyKey::from(letter - b'a'),
        [digit @ b'0'..=b'9'] => GHOSTTY_KEY_DIGIT_0 + GhosttyKey::from(digit - b'0'),
        _ => match key {
            "enter" => GHOSTTY_KEY_ENTER,
            "tab" => GHOSTTY_KEY_TAB,
            "space" => GHOSTTY_KEY_SPACE,
            "backspace" => GHOSTTY_KEY_BACKSPACE,
            "delete" => GHOSTTY_KEY_DELETE,
            "insert" => GHOSTTY_KEY_INSERT,
            "escape" => GHOSTTY_KEY_ESCAPE,
            "up" => GHOSTTY_KEY_ARROW_UP,
            "down" => GHOSTTY_KEY_ARROW_DOWN,
            "left" => GHOSTTY_KEY_ARROW_LEFT,
            "right" => GHOSTTY_KEY_ARROW_RIGHT,
            "home" => GHOSTTY_KEY_HOME,
            "end" => GHOSTTY_KEY_END,
            "pageup" => GHOSTTY_KEY_PAGE_UP,
            "pagedown" => GHOSTTY_KEY_PAGE_DOWN,
            "`" => GHOSTTY_KEY_BACKQUOTE,
            "\\" => GHOSTTY_KEY_BACKSLASH,
            "[" => GHOSTTY_KEY_BRACKET_LEFT,
            "]" => GHOSTTY_KEY_BRACKET_RIGHT,
            "," => GHOSTTY_KEY_COMMA,
            "=" => GHOSTTY_KEY_EQUAL,
            "-" => GHOSTTY_KEY_MINUS,
            "." => GHOSTTY_KEY_PERIOD,
            "'" => GHOSTTY_KEY_QUOTE,
            ";" => GHOSTTY_KEY_SEMICOLON,
            "/" => GHOSTTY_KEY_SLASH,
            _ => key
                .strip_prefix('f')
                .and_then(|number| number.parse::<GhosttyKey>().ok())
                .filter(|number| (1..=25).contains(number))
                .map_or(GHOSTTY_KEY_UNIDENTIFIED, |number| {
                    GHOSTTY_KEY_F1 + number - 1
                }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_utf8_and_vt_attributes_after_split_reads() {
        let mut emulator = Emulator::new(16, 4).unwrap();
        emulator.feed(b"\x1b[1;38;2;20;120;220m\xe7");
        emulator.feed(b"\x95\x8c\x1b[0m!");
        let screen = emulator.screen().unwrap();
        let mut graphemes = screen.cells.iter().filter(|cell| !cell.text.is_empty());
        let wide = graphemes.next().unwrap();
        let normal = graphemes.next().unwrap();
        assert!(graphemes.next().is_none());
        assert_eq!(wide.text, "界");
        assert!(wide.wide && wide.style.bold);
        let foreground = wide.style.foreground.unwrap();
        assert_eq!((foreground.r, foreground.g, foreground.b), (20, 120, 220));
        assert_eq!(normal.text, "!");
        assert_eq!(normal.x, 2);
        assert!(!normal.style.bold);
    }
}
