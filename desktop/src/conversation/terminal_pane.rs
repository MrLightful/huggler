use super::{
    document_tabs::tab_surface,
    terminal::{StatusChanged, TerminalView},
    terminal_actions::StagedAction,
    *,
};
use gpui_component::button::ButtonCustomVariant;

const DEFAULT_HEIGHT: f32 = 236.;
const MIN_HEIGHT: f32 = 100.;
const RESIZE_HANDLE: f32 = 5.;
const HEADER_HEIGHT: f32 = 32.;

#[derive(Default)]
pub(super) struct TerminalPane {
    tabs: Vec<TerminalTab>,
    active: Option<usize>,
    next_id: usize,
    scroll: ScrollHandle,
    open: bool,
    height: Option<f32>,
    pub(super) main_worktree: Option<PathBuf>,
    pub(super) staged_action: Option<StagedAction>,
}

struct TerminalTab {
    id: usize,
    shell: Result<Entity<TerminalView>, String>,
}

pub(super) enum TerminalLaunch<'a> {
    Shell,
    Restart(usize),
    Action(&'a str),
}

#[derive(Clone)]
struct ResizeTerminal(ThreadId);

impl TerminalPane {
    pub(super) fn release(&mut self, cx: &mut App) {
        for tab in self.tabs.drain(..) {
            if let Ok(terminal) = tab.shell {
                terminal.update(cx, |terminal, _| terminal.stop());
            }
        }
        self.active = None;
    }

    pub(super) fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.staged_action
            .as_ref()
            .is_some_and(|action| action.is_focused(window, cx))
            || self
                .active_tab()
                .and_then(|tab| tab.shell.as_ref().ok())
                .is_some_and(|terminal| terminal.read(cx).is_focused(window))
    }

    fn active_tab(&self) -> Option<&TerminalTab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    pub(super) fn focus_active(&self, window: &mut Window, cx: &App) {
        if let Some(terminal) = self.active_tab().and_then(|tab| tab.shell.as_ref().ok()) {
            terminal.read(cx).focus(window);
        }
    }

    fn index_of(&self, id: usize) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn select(&mut self, index: usize) {
        self.active = Some(index);
        self.scroll.scroll_to_item(index);
    }
}

impl Desktop {
    pub(super) fn open_terminal(
        &mut self,
        thread: &ThreadId,
        launch: TerminalLaunch<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspaces.get_mut(thread).expect("terminal workspace");
        let pane = &mut workspace.terminal;
        let replace = if let TerminalLaunch::Restart(id) = launch {
            let Some(index) = pane.index_of(id) else {
                return;
            };
            if pane.tabs[index]
                .shell
                .as_ref()
                .is_ok_and(|terminal| terminal.read(cx).is_running())
            {
                return;
            }
            Some(index)
        } else {
            None
        };
        let command = match launch {
            TerminalLaunch::Action(command) => Some(command),
            _ => None,
        };
        let shell = TerminalView::start(&workspace.root, command, window, cx)
            .map_err(|error| format!("{error:#}"));
        if let Ok(terminal) = &shell {
            cx.subscribe(terminal, |_, _, _: &StatusChanged, cx| cx.notify())
                .detach();
        }
        let index = if let Some(index) = replace {
            pane.tabs[index].shell = shell;
            index
        } else {
            pane.next_id += 1;
            pane.tabs.push(TerminalTab {
                id: pane.next_id,
                shell,
            });
            pane.tabs.len() - 1
        };
        pane.select(index);
        if replace.is_none() {
            pane.scroll.scroll_to_item(pane.tabs.len());
        }
        pane.focus_active(window, cx);
        cx.notify();
    }

    fn close_terminal(
        &mut self,
        thread: &ThreadId,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = &mut self
            .workspaces
            .get_mut(thread)
            .expect("terminal workspace")
            .terminal;
        let Some(index) = pane.index_of(id) else {
            return;
        };
        if let Ok(terminal) = pane.tabs.remove(index).shell {
            terminal.update(cx, |terminal, _| terminal.stop());
        }
        match pane.active {
            Some(active) if active == index => {
                if pane.tabs.is_empty() {
                    pane.active = None;
                    self.focus_workspace(window, cx);
                } else {
                    pane.select(index.min(pane.tabs.len() - 1));
                    pane.focus_active(window, cx);
                }
            }
            Some(active) if active > index => pane.select(active - 1),
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn terminal_pane(
        &self,
        thread: &ConversationDetail,
        max_height: f32,
        cx: &Context<Self>,
    ) -> Div {
        let thread = &thread.id;
        let pane = &self.workspaces[thread].terminal;
        let open = pane.open;
        let height = pane
            .height
            .unwrap_or(DEFAULT_HEIGHT)
            .clamp(MIN_HEIGHT, max_height);
        let toggle = if open {
            "Collapse terminal"
        } else {
            "Expand terminal"
        };
        let toggle_thread = thread.clone();
        let new_thread = thread.clone();
        let header = h_flex()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .px_2()
            .gap_1()
            .child(control!(
                toggle,
                Button::new("toggle-terminal")
                    .ghost()
                    .small()
                    .compact()
                    .flex_none()
                    .icon(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .when(!open, |button| button.label("Terminal"))
                    .tooltip(toggle)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        let pane = &mut view
                            .workspaces
                            .get_mut(&toggle_thread)
                            .expect("terminal workspace")
                            .terminal;
                        pane.open = !open;
                        if open {
                            if pane.is_focused(window, cx) {
                                view.focus_workspace(window, cx);
                            }
                        } else if let Some(action) = &pane.staged_action {
                            action.focus(window, cx);
                        } else if pane.next_id == 0 {
                            view.open_terminal(&toggle_thread, TerminalLaunch::Shell, window, cx);
                        } else if let Some(index) = pane.active {
                            pane.scroll.scroll_to_item(index);
                            pane.focus_active(window, cx);
                        }
                        cx.notify();
                    }))
            ))
            .when(open, |header| {
                header
                    .child(
                        h_flex()
                            .id("terminal-tabs")
                            .flex_1()
                            .min_w_0()
                            .h(px(HEADER_HEIGHT))
                            .gap_0p5()
                            .overflow_x_scroll()
                            .track_scroll(&pane.scroll)
                            .children(pane.tabs.iter().enumerate().map(|(index, tab)| {
                                tab.selector(thread, pane.active == Some(index), cx)
                            }))
                            .child(
                                control!(
                                    "New terminal",
                                    Button::new("new-terminal")
                                        .ghost()
                                        .small()
                                        .compact()
                                        .icon(IconName::Plus)
                                        .tooltip("New terminal")
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.open_terminal(
                                                &new_thread,
                                                TerminalLaunch::Shell,
                                                window,
                                                cx,
                                            )
                                        }))
                                )
                                .flex_shrink_0(),
                            ),
                    )
                    .children(self.terminal_action_picker(thread, cx))
            });
        let drag = ResizeTerminal(thread.clone());
        v_flex()
            .w_full()
            .flex_shrink_0()
            .child(if open {
                control!(
                    "Resize terminal",
                    div()
                        .id("resize-terminal")
                        .h(px(RESIZE_HANDLE))
                        .w_full()
                        .flex_shrink_0()
                        .cursor_row_resize()
                        .occlude()
                        .border_b_1()
                        .border_color(cx.theme().sidebar_border)
                        .hover(|style| style.bg(cx.theme().sidebar_accent))
                        .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyView))
                )
            } else {
                div().h_px().w_full().bg(cx.theme().sidebar_border)
            })
            .child(header)
            .when(open, |container| {
                let contents = pane.active_tab().map_or_else(
                    || {
                        control!(
                            "No terminals open",
                            hint("No terminals open", cx).px_3().py_2()
                        )
                    },
                    |tab| tab.contents(thread, cx),
                );
                container.child(
                    v_flex()
                        .w_full()
                        .h(px(height - RESIZE_HANDLE - HEADER_HEIGHT))
                        .min_h_0()
                        .overflow_hidden()
                        .bg(cx.theme().background)
                        .children(
                            pane.staged_action
                                .as_ref()
                                .map(|action| action.render(thread, cx)),
                        )
                        .child(contents.flex_1().min_h_0()),
                )
            })
            .on_drag_move(cx.listener(
                move |view, event: &DragMoveEvent<ResizeTerminal>, window, cx| {
                    let height = f32::from(window.viewport_size().height - event.event.position.y);
                    if let Some(workspace) = view.workspaces.get_mut(&event.drag(cx).0) {
                        workspace.terminal.height = Some(height.clamp(MIN_HEIGHT, max_height));
                        cx.notify();
                    }
                },
            ))
    }
}

impl TerminalTab {
    fn selector(&self, thread: &ThreadId, active: bool, cx: &Context<Desktop>) -> impl IntoElement {
        let id = self.id;
        let name = format!("Terminal {id}");
        let _key = format!("Terminal tab/{name}");
        let close_label = format!("Close {name}");
        let close_thread = thread.clone();
        let select_thread = thread.clone();
        let theme = cx.theme();
        let label = if active {
            theme.foreground
        } else {
            theme.muted_foreground
        };

        tab_surface(active, cx)
            .id(("terminal-tab", id))
            .pr_0p5()
            .child(control!(
                _key,
                Button::new("select")
                    .custom(ButtonCustomVariant::new(cx).foreground(label))
                    .small()
                    .h_7()
                    .pl_2()
                    .pr_0p5()
                    .whitespace_nowrap()
                    .label(name)
                    .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                        let pane = &mut view
                            .workspaces
                            .get_mut(&select_thread)
                            .expect("terminal workspace")
                            .terminal;
                        if let Some(index) = pane.index_of(id) {
                            pane.select(index);
                            if !event.is_keyboard() {
                                pane.focus_active(window, cx);
                            }
                            cx.notify();
                        }
                    }))
            ))
            .child(control!(
                close_label.clone(),
                Button::new("close")
                    .ghost()
                    .size(px(24.))
                    .p_0()
                    .child(
                        Icon::new(IconName::Close)
                            .with_size(px(12.))
                            .text_color(theme.muted_foreground)
                    )
                    .tooltip(close_label)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.close_terminal(&close_thread, id, window, cx)
                    }))
            ))
    }

    fn contents(&self, thread: &ThreadId, cx: &Context<Desktop>) -> Div {
        let terminal = self.shell.as_ref().ok();
        let running = terminal.is_some_and(|terminal| terminal.read(cx).is_running());
        let status = self
            .shell
            .as_ref()
            .err()
            .map(String::as_str)
            .or_else(|| terminal.and_then(|terminal| terminal.read(cx).status()));
        let thread = thread.clone();
        let id = self.id;
        v_flex()
            .flex_1()
            .min_h_0()
            .when(status.is_some() || !running, |body| {
                body.child(
                    h_flex()
                        .flex_shrink_0()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .when_some(status, |bar, status| {
                            bar.child(control!(
                                status.to_owned(),
                                hint(status.to_owned(), cx).flex_1().min_w_0()
                            ))
                        })
                        .when(!running, |bar| {
                            bar.child(control!(
                                "Restart terminal",
                                Button::new("restart-terminal")
                                    .ghost()
                                    .small()
                                    .compact()
                                    .flex_none()
                                    .label("Restart")
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.open_terminal(
                                            &thread,
                                            TerminalLaunch::Restart(id),
                                            window,
                                            cx,
                                        );
                                    }))
                            ))
                        }),
                )
            })
            .when_some(terminal, |body, terminal| {
                body.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .child(terminal.clone()),
                )
            })
    }
}
