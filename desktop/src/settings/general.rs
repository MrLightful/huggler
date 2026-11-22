use super::*;
use gpui_component::switch::Switch;

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SendShortcut {
    #[default]
    Enter,
    CommandEnter,
}

impl SendShortcut {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Enter => "Enter",
            Self::CommandEnter if cfg!(target_os = "macos") => "⌘Enter",
            Self::CommandEnter => "Ctrl+Enter",
        }
    }

    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FollowUp {
    #[default]
    Queue,
    Steer,
}

impl FollowUp {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn setting(
    title: &'static str,
    description: &'static str,
    input: impl IntoElement,
    cx: &App,
) -> Div {
    h_flex()
        .gap_4()
        .justify_between()
        .border_b_1()
        .border_color(cx.theme().border)
        .py_3()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(section_title(title))
                .child(hint(description, cx)),
        )
        .child(div().flex_shrink_0().child(input))
}

fn toggle(id: &'static str, title: &'static str, enabled: bool) -> Button {
    Button::new(id).ghost().tooltip(title).child(
        Switch::new((id, 0_u32))
            .label(if enabled { "On" } else { "Off" })
            .checked(enabled),
    )
}

impl Settings {
    pub(super) fn general(&self, cx: &Context<Self>) -> Div {
        let shortcuts = [SendShortcut::Enter, SendShortcut::CommandEnter].map(|shortcut| {
            let name = shortcut.label();
            control!(
                name,
                choice(name, name, self.prefs.send_shortcut == shortcut).on_click(cx.listener(
                    move |view, _, _, cx| {
                        let mut prefs = view.prefs.clone();
                        prefs.send_shortcut = shortcut;
                        view.persist(prefs, cx);
                    }
                ))
            )
        });
        let follow_ups =
            [(FollowUp::Queue, "Queue"), (FollowUp::Steer, "Steer")].map(|(mode, name)| {
                control!(
                    name,
                    choice(name, name, self.prefs.follow_up == mode).on_click(cx.listener(
                        move |view, _, _, cx| {
                            let mut prefs = view.prefs.clone();
                            prefs.follow_up = mode;
                            view.persist(prefs, cx);
                        }
                    ))
                )
            });
        v_flex()
            .child(setting(
                "Send message with",
                match self.prefs.send_shortcut {
                    SendShortcut::Enter => "Shift+Enter adds a new line.",
                    SendShortcut::CommandEnter => "Enter adds a new line.",
                },
                h_flex().gap_1().children(shortcuts),
                cx,
            ))
            .child(setting(
                "Follow-up behavior",
                "Steer sends text into a running Codex turn. Other follow-ups queue.",
                h_flex().gap_1().children(follow_ups),
                cx,
            ))
            .child(setting(
                "Agent notifications",
                "Show an alert when a conversation completes.",
                control!(
                    "Agent notifications",
                    toggle(
                        "agent-notifications",
                        "Agent notifications",
                        self.prefs.agent_notifications
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        let mut prefs = view.prefs.clone();
                        prefs.agent_notifications = !prefs.agent_notifications;
                        view.persist(prefs, cx);
                    }))
                ),
                cx,
            ))
            .child(setting(
                "Keep awake while agents run",
                if cfg!(target_os = "macos") {
                    "Prevent idle system sleep while Huggler is running agents."
                } else {
                    "Available on macOS."
                },
                control!(
                    "Keep awake while agents run",
                    toggle(
                        "keep-awake",
                        "Keep awake while agents run",
                        self.prefs.keep_awake
                    )
                    .disabled(!cfg!(target_os = "macos"))
                    .on_click(cx.listener(|view, _, _, cx| {
                        let mut prefs = view.prefs.clone();
                        prefs.keep_awake = !prefs.keep_awake;
                        view.persist(prefs, cx);
                    }))
                ),
                cx,
            ))
    }
}
