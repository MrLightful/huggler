use crate::FollowUp;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use huggler::{
    Huggler, ServiceHandleId, ThreadId,
    services::{Action, Open, Output},
};
use std::{sync::Arc, time::Duration};

pub struct Checks {
    runtime: Arc<Huggler>,
    thread: ThreadId,
    command: Entity<InputState>,
    last_command: String,
    output: String,
    status: String,
    handle: Option<ServiceHandleId>,
    poll: Option<Task<()>>,
}
impl EventEmitter<FollowUp> for Checks {}

impl Checks {
    pub fn new(
        runtime: Arc<Huggler>,
        thread: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            runtime,
            thread,
            command: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Command to run in this checkout")
            }),
            last_command: String::new(),
            output: String::new(),
            status: String::new(),
            handle: None,
            poll: None,
        }
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        if self.handle.is_some() {
            return;
        }
        let command = self.command.read(cx).value().to_string();
        if command.trim().is_empty() {
            self.status = "Enter a command.".into();
            cx.notify();
            return;
        }
        self.last_command = command.clone();
        self.output.clear();
        let request = Open::Terminal {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), command],
        };
        match self.runtime.open_service(&self.thread, request) {
            Err(error) => self.status = error.to_string(),
            Ok(handle) => {
                self.status = "Running".into();
                self.handle = Some(handle.clone());
                self.poll = Some(cx.spawn(async move |view, cx| {
                    let mut cursor = None;
                    let mut tail = Vec::new();
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                        let active = view.update(cx, |view, cx| {
                            if view.handle.as_ref() != Some(&handle) {
                                return false;
                            }
                            let previous = cursor;
                            match view.read_output(&handle, &mut cursor, &mut tail) {
                                Ok(None) => (),
                                Ok(Some(status)) => view.finish(status),
                                Err(err) => view.finish(err.to_string()),
                            }
                            if cursor != previous || view.handle.is_none() {
                                cx.notify();
                            }
                            view.handle.is_some()
                        });
                        if !matches!(active, Ok(true)) {
                            break;
                        }
                    }
                }));
            }
        }
        cx.notify();
    }

    fn read_output(
        &mut self,
        handle: &ServiceHandleId,
        cursor: &mut Option<u64>,
        tail: &mut Vec<u8>,
    ) -> huggler::Result<Option<String>> {
        let read = |after| {
            self.runtime
                .service_call(&self.thread, handle, Action::Output { after })
        };
        let result = match read(*cursor) {
            Err(huggler::Error::CursorExpired) => {
                tail.clear();
                read(None)?
            }
            result => result?,
        };
        let Output::Terminal { next, data, eof } = result else {
            return Err(huggler::Error::Integration(
                "Missing terminal output".into(),
            ));
        };
        if *cursor != Some(next) {
            tail.extend_from_slice(&data);
            tail.drain(..tail.len().saturating_sub(64 * 1024));
            self.output = String::from_utf8_lossy(tail).into_owned();
            if next > tail.len() as u64 {
                self.output.insert_str(0, "… earlier output omitted\n");
            }
            *cursor = Some(next);
        }
        if !eof {
            return Ok(None);
        }
        match self
            .runtime
            .service_call(&self.thread, handle, Action::ExitStatus)?
        {
            Output::ExitStatus(Some(status)) if status.success() => {
                Ok(Some("Passed · exit 0".into()))
            }
            Output::ExitStatus(Some(status)) => Ok(Some(match status.code() {
                Some(code) => format!("Failed · exit {code}"),
                None => format!("Failed · {status}"),
            })),
            Output::ExitStatus(None) => Ok(None),
            _ => Err(huggler::Error::Integration(
                "Missing terminal exit status".into(),
            )),
        }
    }

    fn finish(&mut self, status: String) {
        self.status = status;
        if let Some(id) = self.handle.take()
            && let Err(error) = self.runtime.close_service(&self.thread, &id)
        {
            self.status = error.to_string();
        }
    }
}

impl Render for Checks {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.handle.is_some();
        let run = Button::new("run-check")
            .primary()
            .label("Run check")
            .disabled(running)
            .on_click(cx.listener(|view, _, _, cx| view.run(cx)));
        let controls = h_flex()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(control!("Check command", Input::new(&self.command))),
            )
            .child(control!("Run check", run))
            .when(running, |row| {
                let stop = Button::new("stop-check")
                    .label("Stop")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.finish("Stopped".into());
                        view.poll = None;
                        cx.notify();
                    }));
                row.child(control!("Stop check", stop))
            });
        let footer = h_flex()
            .gap_2()
            .justify_between()
            .child(control!(
                format!("check/{}", self.status),
                self.status.clone()
            ))
            .when(!running && !self.last_command.is_empty(), |row| {
                let discuss =
                    Button::new("discuss-check")
                        .label("Discuss check")
                        .on_click(cx.listener(|view, _, _, cx| {
                            let start = view
                                .output
                                .ceil_char_boundary(view.output.len().saturating_sub(8192));
                            cx.emit(FollowUp(format!(
                            "Please review this check and address any failures.\n\n$ {}\n{}\n{}",
                            view.last_command, view.status, &view.output[start..]
                        )));
                        }));
                row.child(control!("Discuss check", discuss))
            });
        v_flex()
            .size_full()
            .min_h_0()
            .p_4()
            .gap_2()
            .child(div().text_sm().child("Shell command"))
            .child(controls)
            .child(div().text_sm().child(self.last_command.clone()))
            .child(
                div()
                    .id("check-output")
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .rounded_md()
                    .overflow_scroll()
                    .bg(cx.theme().sidebar)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_sm()
                    .child(control!(
                        format!("output/{}", self.output),
                        self.output.clone()
                    )),
            )
            .child(footer)
    }
}
