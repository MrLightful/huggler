mod emulator;
mod grid;
mod pty;

use anyhow::Result;
use emulator::*;
use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;
use grid::{Decoration, Grid, Palette};
use portable_pty::PtySize;
use pty::Pty;
use smol::{Timer, channel, future};
use std::{
    io::{self, Read, Write},
    ops::Range,
    path::Path,
    time::Duration,
};

pub(super) struct TerminalView {
    focus: FocusHandle,
    emulator: Emulator,
    grid: Option<Grid>,
    size: PtySize,
    palette: Palette,
    session: Option<Session>,
    status: Option<String>,
    composition: String,
    composition_selection: Range<usize>,
    scroll_remainder: f32,
    selecting: bool,
    mouse_button: Option<MouseButton>,
}

pub(super) struct StatusChanged;
impl EventEmitter<StatusChanged> for TerminalView {}

struct Session {
    input: channel::Sender<Vec<u8>>,
    _reader: Task<()>,
    _writer: Task<()>,
    pty: Pty,
}

impl TerminalView {
    pub(super) fn start(
        root: &Path,
        command: Option<&str>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<Self>> {
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };
        let palette = Palette::new(cx);
        let mut emulator = Emulator::new(size.cols, size.rows)?;
        palette.apply(&mut emulator);

        let (pty, io) = Pty::spawn(root, size, command)?;
        Ok(cx.new(move |cx: &mut Context<Self>| {
            let (input, writes) = channel::bounded::<Vec<u8>>(128);
            let writer_io = io.clone();
            let writer = cx.spawn(async move |view, cx| {
                while let Ok(bytes) = writes.recv().await {
                    let mut offset = 0;
                    while offset < bytes.len() {
                        match writer_io
                            .write_with(|mut file| file.write(&bytes[offset..]))
                            .await
                        {
                            Ok(n) if n > 0 => offset += n,
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                            outcome => {
                                let message = outcome.err().map_or_else(
                                    || "PTY write closed".into(),
                                    |error| error.to_string(),
                                );
                                let _ = view.update(cx, |view, cx| {
                                    view.end(format!("Terminal write failed. {message}"), cx)
                                });
                                return;
                            }
                        }
                    }
                }
            });
            let reader = cx.spawn(async move |view, cx| {
                let mut buffer = [0; 32768];
                // A shell can exit while a background job keeps the slave open.
                // Check its status independently of PTY EOF.
                let mut check_exit = Timer::interval(Duration::from_millis(200));
                let mut eof = false;
                loop {
                    let read = future::race(
                        async {
                            if eof {
                                future::pending().await
                            } else {
                                Some(io.read_with(|mut file| file.read(&mut buffer)).await)
                            }
                        },
                        async {
                            (&mut check_exit).await;
                            None
                        },
                    )
                    .await;
                    let alive = view
                        .update(cx, |view, cx| match read {
                            Some(Ok(n)) if n > 0 => {
                                view.emulator.feed(&buffer[..n]);
                                view.send_replies(cx);
                                cx.notify();
                                true
                            }
                            Some(Err(ref error)) if error.kind() == io::ErrorKind::Interrupted => {
                                true
                            }
                            Some(Err(ref error)) if error.raw_os_error() != Some(libc::EIO) => {
                                view.end(format!("Terminal read failed. {error}"), cx);
                                false
                            }
                            Some(_) => {
                                eof = true;
                                true
                            }
                            None => {
                                let exited =
                                    view.session.as_mut().map(|session| session.pty.try_wait());
                                match exited {
                                    Some(Ok(Some(status))) => {
                                        // The exit check can win the race against buffered output.
                                        if !eof && let Ok(n) = io.get_ref().read(&mut buffer) {
                                            view.emulator.feed(&buffer[..n]);
                                        }
                                        let message = match status.signal() {
                                            Some(signal) => format!("Shell terminated by {signal}"),
                                            None if status.success() => "Shell exited".into(),
                                            None => format!(
                                                "Shell exited with code {}",
                                                status.exit_code()
                                            ),
                                        };
                                        view.end(message, cx)
                                    }
                                    Some(Err(error)) => view
                                        .end(format!("Could not read shell status. {error}"), cx),
                                    _ if eof => view.end("Shell closed".into(), cx),
                                    _ => return view.session.is_some(),
                                }
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !alive {
                        break;
                    }
                    // Yield to window input when a process continuously fills its PTY.
                    future::yield_now().await;
                }
            });
            let focus = cx.focus_handle().tab_stop(true);
            cx.on_focus(&focus, window, |view, _, cx| {
                view.send(view.emulator.focus(true).to_vec(), cx);
                cx.notify();
            })
            .detach();
            cx.on_blur(&focus, window, |view, _, cx| {
                view.composition.clear();
                view.selecting = false;
                view.mouse_button = None;
                view.emulator.finish_selection();
                view.send(view.emulator.focus(false).to_vec(), cx);
                cx.notify();
            })
            .detach();
            Self {
                focus,
                emulator,
                grid: None,
                size,
                palette,
                session: Some(Session {
                    input,
                    _reader: reader,
                    _writer: writer,
                    pty,
                }),
                status: None,
                composition: String::new(),
                composition_selection: 0..0,
                scroll_remainder: 0.,
                selecting: false,
                mouse_button: None,
            }
        }))
    }

    pub(super) fn focus(&self, window: &mut Window) {
        self.focus.focus(window);
    }

    pub(super) fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    pub(super) fn is_running(&self) -> bool {
        self.session.is_some()
    }

    pub(super) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub(super) fn stop(&mut self) {
        self.session = None;
    }

    fn end(&mut self, message: String, cx: &mut Context<Self>) {
        self.stop();
        self.status = Some(message);
        cx.emit(StatusChanged);
        cx.notify();
    }

    fn send(&mut self, data: Vec<u8>, cx: &mut Context<Self>) {
        if data.is_empty() {
            return;
        }
        if let Some(session) = &self.session
            && let Err(error) = session.input.try_send(data)
        {
            if error.is_full() {
                self.status =
                    Some("Terminal input is busy. Paste or type again when it responds.".into());
                cx.emit(StatusChanged);
                cx.notify();
            } else {
                self.end("Terminal input closed".into(), cx);
            }
        }
    }

    fn send_replies(&mut self, cx: &mut Context<Self>) {
        let replies = self.emulator.take_replies();
        self.send(replies, cx);
    }

    fn input(&mut self, data: Vec<u8>, cx: &mut Context<Self>) {
        if !data.is_empty() && self.is_running() {
            if self.status.take().is_some() {
                cx.emit(StatusChanged);
            }
            self.emulator.clear_selection();
            self.emulator.scroll_to_bottom();
            self.send(data, cx);
            cx.notify();
        }
    }

    fn prepare(&mut self, grid: Grid, palette: Palette, cx: &mut Context<Self>) -> Option<Screen> {
        self.grid = Some(grid.clone());
        if self.palette != palette {
            palette.apply(&mut self.emulator);
            self.palette = palette;
        }
        let size = grid.pty_size();
        if self.size != size {
            let (width, height) = (size.pixel_width / size.cols, size.pixel_height / size.rows);
            let result = self
                .emulator
                .resize(size.cols, size.rows, width, height)
                .and_then(|()| {
                    self.session
                        .as_ref()
                        .map_or(Ok(()), |session| session.pty.resize(size))
                });
            if let Err(error) = result {
                self.end(format!("Terminal resize failed. {error}"), cx);
            }
            self.size = size;
            self.send_replies(cx);
        }
        self.emulator.screen()
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        if stroke.modifiers.platform {
            let handled = match stroke.key.as_str() {
                "a" => {
                    self.emulator.select_all();
                    cx.notify();
                    true
                }
                "c" => {
                    if let Some(text) = self.emulator.selected_text() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                    true
                }
                "v" => {
                    if self.is_running()
                        && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                    {
                        self.paste(text, window, cx);
                    }
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
            }
            return;
        }
        if !self.is_running() || !self.composition.is_empty() {
            return;
        }
        let data = self.emulator.key(stroke, event.is_held, false);
        if !data.is_empty() {
            self.input(data, cx);
            cx.stop_propagation();
        }
    }

    fn paste(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(data) = self.emulator.paste(&text, false) {
            self.input(data, cx);
            return;
        }
        let response = window.prompt(
            PromptLevel::Warning,
            "Paste into the terminal?",
            Some("This text contains line breaks that may run commands in the shell."),
            &["Cancel", "Paste"],
            cx,
        );
        cx.spawn(async move |view, cx| {
            if response.await == Ok(1) {
                let _ = view.update(cx, |view, cx| {
                    if let Some(data) = view.emulator.paste(&text, true) {
                        view.input(data, cx);
                    }
                });
            }
        })
        .detach();
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(grid) = &self.grid else { return };
        self.scroll_remainder -= event.delta.pixel_delta(grid.cell.height).y / grid.cell.height;
        let lines = self.scroll_remainder as isize;
        self.scroll_remainder -= lines as f32;
        if lines == 0 {
            return;
        }
        if self.is_running() && self.emulator.mouse_tracking() && !event.modifiers.shift {
            let button = if lines < 0 {
                GHOSTTY_MOUSE_BUTTON_FOUR
            } else {
                GHOSTTY_MOUSE_BUTTON_FIVE
            };
            for _ in 0..lines.unsigned_abs().min(48) {
                self.report_mouse(
                    GHOSTTY_MOUSE_ACTION_PRESS,
                    Some(button),
                    event.position,
                    event.modifiers,
                    cx,
                );
            }
        } else if self.is_running() && self.emulator.alternate_scroll() && !event.modifiers.shift {
            let key = if lines < 0 { "up" } else { "down" };
            let data = self.emulator.key(
                &Keystroke {
                    key: key.into(),
                    ..Default::default()
                },
                false,
                false,
            );
            self.send(data.repeat(lines.unsigned_abs().min(48)), cx);
        } else {
            self.emulator.scroll(lines);
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn report_mouse(
        &mut self,
        action: u32,
        button: Option<u32>,
        point: Point<Pixels>,
        mods: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if let Some(grid) = &self.grid {
            let data =
                self.emulator
                    .mouse(action, button, mods, grid.mouse_position(point), self.size);
            self.send(data, cx);
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window);
        if self.is_running() && self.emulator.mouse_tracking() && !event.modifiers.shift {
            self.mouse_button = Some(event.button);
            self.report_mouse(
                GHOSTTY_MOUSE_ACTION_PRESS,
                mouse_button(event.button),
                event.position,
                event.modifiers,
                cx,
            );
        } else if event.button == MouseButton::Left
            && let Some(grid) = &self.grid
        {
            self.emulator
                .start_selection(grid.grid_point(event.position));
            self.selecting = true;
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.selecting
            && let Some(grid) = &self.grid
        {
            self.emulator
                .extend_selection(grid.grid_point(event.position));
            cx.notify();
        } else if self.is_running() && self.emulator.mouse_tracking() && !event.modifiers.shift {
            self.report_mouse(
                GHOSTTY_MOUSE_ACTION_MOTION,
                self.mouse_button.and_then(mouse_button),
                event.position,
                event.modifiers,
                cx,
            );
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if event.button == MouseButton::Left && self.selecting {
            self.selecting = false;
            self.emulator.finish_selection();
        }
        if self.mouse_button == Some(event.button) {
            self.mouse_button = None;
            self.report_mouse(
                GHOSTTY_MOUSE_ACTION_RELEASE,
                mouse_button(event.button),
                event.position,
                event.modifiers,
                cx,
            );
        }
    }
}

fn mouse_button(button: MouseButton) -> Option<u32> {
    match button {
        MouseButton::Left => Some(GHOSTTY_MOUSE_BUTTON_LEFT),
        MouseButton::Right => Some(GHOSTTY_MOUSE_BUTTON_RIGHT),
        MouseButton::Middle => Some(GHOSTTY_MOUSE_BUTTON_MIDDLE),
        _ => None,
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let prepare_view = view.clone();
        let focus = self.focus.clone();
        let focused = focus.is_focused(window);
        let running = self.is_running();
        let composition = self.composition.clone();
        let selection = cx.theme().sidebar_accent;
        let palette = Palette::new(cx);
        let family = cx.theme().mono_font_family.clone();
        let content = canvas(
            move |bounds, window, cx| {
                #[cfg(feature = "qualification")]
                crate::qualification::observe_bounds("Terminal grid".into(), bounds, cx);
                let grid = Grid::new(bounds, family, window);
                let screen =
                    prepare_view.update(cx, |view, cx| view.prepare(grid.clone(), palette, cx));
                (grid, screen)
            },
            move |bounds, (grid, screen), window, cx| {
                if focused && running {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, view), cx);
                }
                if let Some(screen) = screen {
                    grid.paint(
                        bounds,
                        screen,
                        Decoration {
                            selection,
                            focused,
                            running,
                            composition: &composition,
                        },
                        window,
                        cx,
                    );
                }
            },
        )
        .size_full();

        let mut surface = div()
            .id("terminal-grid")
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .occlude()
            .cursor_text()
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(|view, event: &KeyUpEvent, _, cx| {
                if !event.keystroke.modifiers.platform && view.is_running() {
                    let data = view.emulator.key(&event.keystroke, false, true);
                    view.send(data, cx);
                }
            }))
            .on_scroll_wheel(cx.listener(|view, event, _, cx| view.wheel(event, cx)))
            .on_mouse_move(cx.listener(|view, event, _, cx| view.mouse_move(event, cx)))
            .child(content);
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            surface = surface
                .on_mouse_down(button, cx.listener(Self::mouse_down))
                .on_mouse_up(
                    button,
                    cx.listener(|view, event, _, cx| view.mouse_up(event, cx)),
                )
                .on_mouse_up_out(
                    button,
                    cx.listener(|view, event, _, cx| view.mouse_up(event, cx)),
                );
        }
        surface
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let units: Vec<_> = self.composition.encode_utf16().collect();
        units.get(range).map(String::from_utf16_lossy)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.composition_selection.clone(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.composition.is_empty()).then(|| 0..self.composition.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composition.clear();
        self.composition_selection = 0..0;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.unmark_text(window, cx);
        self.input(text.as_bytes().to_vec(), cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        text.clone_into(&mut self.composition);
        let end = text.encode_utf16().count();
        self.composition_selection = selection.unwrap_or(end..end);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let grid = self.grid.as_ref()?;
        Some(
            self.emulator
                .screen()
                .and_then(|screen| screen.cursor)
                .map_or(bounds, |cursor| {
                    Bounds::new(grid.origin(cursor.x, cursor.y), grid.cell)
                }),
        )
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
