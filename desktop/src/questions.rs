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

#[derive(Clone)]
enum Backend {
    Local(Arc<Huggler>),
    #[cfg(unix)]
    Remote {
        client: huggler::machine::Client,
        executor: tokio::runtime::Handle,
        project: huggler::ProjectId,
    },
}

pub struct Questions {
    backend: Backend,
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
        Self::with_backend(Backend::Local(runtime), thread, request, window, cx)
    }

    #[cfg(unix)]
    pub(crate) fn remote(
        client: huggler::machine::Client,
        executor: tokio::runtime::Handle,
        project: huggler::ProjectId,
        thread: ThreadId,
        request: UserInputRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_backend(
            Backend::Remote {
                client,
                executor,
                project,
            },
            thread,
            request,
            window,
            cx,
        )
    }

    fn with_backend(
        backend: Backend,
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
            backend,
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
        let backend = self.backend.clone();
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
            let result = match backend {
                Backend::Local(runtime) => {
                    let result = if dismiss {
                        runtime
                            .dismiss_question(&thread, &request.run_id, &request.id)
                            .await
                    } else {
                        runtime
                            .respond_to_question(&thread, &request.run_id, &request.id, answers)
                            .await
                    };
                    result.map_err(|err| err.to_string())
                }
                #[cfg(unix)]
                Backend::Remote {
                    client,
                    executor,
                    project,
                } => {
                    use huggler::machine::{Reply, Request};
                    let request = Request::Answer {
                        project,
                        thread,
                        run: request.run_id,
                        request: request.id,
                        answers: (!dismiss).then_some(answers),
                    };
                    match executor
                        .spawn(async move { client.request(request).await })
                        .await
                    {
                        Ok(Ok(Reply::Responded)) => Ok(()),
                        Ok(Ok(_)) => Err("The machine returned an incompatible response.".into()),
                        Ok(Err(err)) => Err(err.to_string()),
                        Err(err) => Err(err.to_string()),
                    }
                }
            };
            if let Err(err) = result {
                let _ = view.update(cx, |view, cx| {
                    view.pending = false;
                    view.error = Some(err);
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
