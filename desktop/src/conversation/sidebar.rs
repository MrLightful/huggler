use super::*;
use gpui_component::Sizable;
use huggler::RunStatus;

pub(super) const WIDTH: f32 = 184.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Attention,
    Ready,
    Working,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Self::Attention => "Needs attention",
            Self::Ready => "Ready",
            Self::Working => "Working",
        }
    }
}

fn activity(thread: &ConversationSummary) -> (Section, &'static str) {
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
        let mut list = div().id("threads").flex_1().min_h_0().overflow_y_scroll();
        for section in [Section::Attention, Section::Ready, Section::Working] {
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
            let expanded = section != Section::Working || self.show_working;
            let selected = self
                .current
                .as_ref()
                .filter(|_| !self.show_settings)
                .map(|t| &t.id);
            let can_fold =
                section == Section::Working && rows.iter().any(|(t, _)| Some(&t.id) != selected);
            let header = h_flex()
                .px_1()
                .gap_2()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{title} ({count})")),
                )
                .when(can_fold, |header| {
                    let action = if expanded { "Hide" } else { "Show" };
                    header.child(control!(
                        format!("{action} working"),
                        Button::new("toggle-working")
                            .small()
                            .compact()
                            .ghost()
                            .label(action)
                            .tooltip(format!("{action} working conversations"))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_working = !view.show_working;
                                cx.notify();
                            }))
                    ))
                });
            let group = v_flex()
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
            .w(px(WIDTH - 43.))
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
