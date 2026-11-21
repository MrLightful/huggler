use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use huggler::{Huggler, ThreadId, UserAnswer, UserAnswers, UserInputRequest};
use std::sync::Arc;

pub struct Questions {
    runtime: Arc<Huggler>,
    thread: ThreadId,
    request: UserInputRequest,
    inputs: Vec<Entity<InputState>>,
    pending: bool,
    error: Option<String>,
}

impl Questions {
    pub fn new(
        runtime: Arc<Huggler>,
        thread: ThreadId,
        request: UserInputRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = request
            .questions
            .iter()
            .map(|q| cx.new(|cx| InputState::new(window, cx).masked(q.is_secret)))
            .collect();
        Self {
            runtime,
            thread,
            request,
            inputs,
            pending: false,
            error: None,
        }
    }

    fn respond(&mut self, dismiss: bool, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let runtime = self.runtime.clone();
        let thread = self.thread.clone();
        let request = self.request.clone();
        let mut answers = UserAnswers::new();
        if !dismiss {
            for (q, input) in request.questions.iter().zip(&self.inputs) {
                let text = input.read(cx).value().to_string();
                if text.trim().is_empty() {
                    self.error = Some("Answer each question before submitting.".into());
                    cx.notify();
                    return;
                }
                answers.insert(q.id.clone(), UserAnswer::Text(text));
            }
        }
        self.pending = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |view, cx| {
            let result = if dismiss {
                runtime
                    .dismiss_question(&thread, &request.run_id, &request.id)
                    .await
            } else {
                runtime
                    .respond_to_question(&thread, &request.run_id, &request.id, answers)
                    .await
            };
            if let Err(err) = result {
                let _ = view.update(cx, |view, cx| {
                    view.pending = false;
                    view.error = Some(err.to_string());
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl Render for Questions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .p_2()
            .border_1()
            .rounded_md()
            .border_color(cx.theme().border)
            .children(
                self.request
                    .questions
                    .iter()
                    .zip(&self.inputs)
                    .enumerate()
                    .map(|(i, (q, input))| {
                        let title = q.question.as_ref().or(q.header.as_ref()).unwrap_or(&q.id);
                        let options = q.options.as_deref().unwrap_or_default();
                        v_flex()
                            .gap_1()
                            .child(title.clone())
                            .child(h_flex().gap_1().flex_wrap().children(
                                options.iter().enumerate().map(|(j, opt)| {
                                    let label = opt.label.clone();
                                    let input = input.clone();
                                    Button::new(SharedString::from(format!(
                                        "{}-{i}-{j}",
                                        self.request.id
                                    )))
                                    .label(label.clone())
                                    .selected(input.read(cx).value() == label.as_str())
                                    .when_some(opt.description.clone(), |b, d| b.tooltip(d))
                                    .disabled(self.pending)
                                    .on_click(cx.listener(move |_, _, w, cx| {
                                        input.update(cx, |s, cx| s.set_value(label.clone(), w, cx));
                                        cx.notify();
                                    }))
                                }),
                            ))
                            .when(q.is_other || options.is_empty(), |v| {
                                v.child(control!(format!("answer/{}", q.id), Input::new(input)))
                            })
                    }),
            )
            .when_some(self.error.clone(), |v, e| {
                v.child(div().text_color(cx.theme().danger).child(e))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(control!(
                        "Submit answers",
                        Button::new("answer-questions")
                            .label("Submit answers")
                            .primary()
                            .disabled(self.pending)
                            .on_click(cx.listener(|v, _, _, cx| v.respond(false, cx)))
                    ))
                    .child(control!(
                        "Dismiss questions",
                        Button::new("dismiss-questions")
                            .label("Dismiss")
                            .disabled(self.pending)
                            .on_click(cx.listener(|v, _, _, cx| v.respond(true, cx)))
                    )),
            )
    }
}
