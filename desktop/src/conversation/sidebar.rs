use super::*;
use huggler::RunStatus;

use std::time::Instant;

#[derive(Default)]
pub(super) struct SidebarVisibility {
    open: Option<bool>,
    transition: Option<(Instant, f32)>,
}

impl SidebarVisibility {
    pub(super) fn is_open(&self, default: bool) -> bool {
        self.open.unwrap_or(default)
    }

    fn progress(&self, open: bool) -> f32 {
        let target = if open { 1.0 } else { 0.0 };
        let Some((start, from)) = self.transition else {
            return target;
        };
        let elapsed = (start.elapsed().as_secs_f32() / 0.18).min(1.0);
        target + (from - target) * (1.0 - elapsed).powi(3)
    }

    pub(super) fn toggle(&mut self, was_open: bool) {
        let from = self.progress(was_open);
        self.open = Some(!was_open);
        self.transition = Some((Instant::now(), from));
        #[cfg(target_os = "macos")]
        if objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            self.transition = None;
        }
    }

    pub(super) fn fraction(&mut self, default: bool, window: &mut Window) -> f32 {
        let open = self.is_open(default);
        let fraction = self.progress(open);
        if fraction == if open { 1.0 } else { 0.0 } {
            self.transition = None;
        } else {
            window.request_animation_frame();
        }
        fraction
    }
}

pub(super) const WIDTH: f32 = 184.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Attention,
    Ready,
    Working,
    Settled,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Self::Attention => "Needs attention",
            Self::Ready => "Ready",
            Self::Working => "Working",
            Self::Settled => "Settled",
        }
    }
}

fn activity(thread: &ConversationSummary) -> (Section, &'static str) {
    if thread.settled {
        return (Section::Settled, "Settled");
    }
    if thread.needs_approval {
        return (Section::Attention, "Approval requested");
    }
    if thread.needs_answer {
        return (Section::Attention, "Answer requested");
    }
    match thread.status {
        Some(RunStatus::Failed) => (Section::Attention, "Run failed"),
        Some(RunStatus::Queued) if thread.queue_held => (Section::Attention, "Queue paused"),
        Some(RunStatus::Queued) => (Section::Working, "Queued"),
        Some(RunStatus::Starting) => (Section::Working, "Starting"),
        Some(RunStatus::Running) => (Section::Working, "Running"),
        Some(RunStatus::Waiting) => (Section::Working, "Finishing"),
        _ if thread.active_children => (Section::Working, "Subagent working"),
        Some(RunStatus::Interrupted) => (Section::Ready, "Stopped"),
        Some(RunStatus::Completed) => (Section::Ready, "Completed"),
        None => (Section::Ready, "New conversation"),
    }
}

impl Desktop {
    pub(super) fn conversation_sidebar(&self, cx: &Context<Self>) -> Stateful<Div> {
        let mut list = v_flex()
            .id("threads")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for section in [
            Section::Attention,
            Section::Ready,
            Section::Working,
            Section::Settled,
        ] {
            // Keep the space above both lower groups even when Working is empty.
            if section == Section::Working {
                list = list.child(div().flex_grow());
            }
            let rows: Vec<_> = self
                .threads
                .iter()
                .filter_map(|thread| {
                    let (group, status) = activity(thread);
                    (section == group).then_some((thread, status))
                })
                .collect();
            if rows.is_empty() {
                continue;
            }

            let title = section.label();
            let count = rows.len();
            let expanded = match section {
                Section::Working => self.show_working,
                Section::Settled => self.show_settled,
                _ => true,
            };
            let selected = self
                .current
                .as_ref()
                .filter(|_| !self.show_settings)
                .map(|t| &t.id);
            let can_fold = matches!(section, Section::Working | Section::Settled)
                && rows.iter().any(|(t, _)| Some(&t.id) != selected);
            let header = h_flex()
                .px_1()
                .gap_2()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{title} ({count})")),
                )
                .when(can_fold, |header| {
                    let action = if expanded { "Hide" } else { "Show" };
                    header.child(control!(
                        format!("{action} {}", title.to_lowercase()),
                        Button::new(SharedString::from(format!("toggle-{title}")))
                            .small()
                            .compact()
                            .ghost()
                            .label(action)
                            .tooltip(format!("{action} {} conversations", title.to_lowercase()))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                match section {
                                    Section::Settled => view.show_settled = !expanded,
                                    Section::Working => view.show_working = !expanded,
                                    _ => {}
                                }
                                cx.notify();
                            }))
                    ))
                });
            let group = v_flex()
                .flex_shrink_0()
                .pb_3()
                .gap_1()
                .child(control!(format!("section/{title}/{count}"), header))
                .children(
                    rows.into_iter()
                        .filter(|(thread, _)| expanded || Some(&thread.id) == selected)
                        .map(|(thread, status)| self.sidebar_row(thread, section, status, cx)),
                );
            list = list.child(group);
        }
        list
    }

    fn sidebar_row(
        &self,
        thread: &ConversationSummary,
        section: Section,
        status: &'static str,
        cx: &Context<Self>,
    ) -> Div {
        let id = thread.id.clone();
        let selected = !self.show_settings && self.current.as_ref().is_some_and(|t| t.id == id);
        let attention = section == Section::Attention;
        // Button measures its custom label at its intrinsic width. Bound it to the
        // space inside the sidebar and row padding so long names get an ellipsis.
        let content = v_flex()
            .w(px(WIDTH - 76.))
            .items_start()
            .child(div().w_full().truncate().child(thread.name.clone()))
            .when(attention, |row| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            });
        let settled = thread.settled;
        let action = if settled { "Unsettle" } else { "Settle" };
        let enabled = settled
            || (!thread.needs_approval
                && !thread.needs_answer
                && !thread.active_children
                && thread.status.is_none_or(RunStatus::terminal));
        let target = id.clone();
        let settlement = control!(
            format!("{action}/{}", thread.name),
            Button::new(SharedString::from(format!("settlement-{}", thread.id.0)))
                .ghost()
                .small()
                .compact()
                .icon(if settled {
                    IconName::Undo2
                } else {
                    IconName::CircleCheck
                })
                .tooltip(format!("{action} conversation"))
                .disabled(!enabled)
                .on_click(cx.listener(move |view, _, _, cx| {
                    cx.stop_propagation();
                    let result = view
                        .runtime
                        .dispatch(Command {
                            id: CommandId(Uuid::new_v4().to_string()),
                            thread_id: target.clone(),
                            action: Action::SetSettled { settled: !settled },
                        })
                        .map(|_| ());
                    view.report(result, cx);
                }))
        );
        control!(
            format!("thread/{}", thread.name),
            Button::new(SharedString::from(id.0.clone()))
                .w_full()
                .compact()
                .justify_start()
                .when(attention, |button| button.h_12())
                .child(control!(
                    format!("{}/{}/{}", section.label(), thread.name, status),
                    content
                ))
                .child(settlement)
                .tooltip(format!("{} · {status}", thread.name))
                .ghost()
                .selected(selected)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.show_settings = false;
                    let result = view.select(id.clone(), window, cx);
                    view.report(result, cx);
                })),
        )
    }
}
