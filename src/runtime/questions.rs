use super::*;

enum Responder<'a> {
    Agent {
        token: &'a str,
        target: Option<&'a ThreadId>,
    },
    Host {
        thread: &'a ThreadId,
        run: &'a RunId,
    },
}

impl Shared {
    pub(crate) async fn answer_as_agent(
        &self,
        token: &str,
        target: Option<&ThreadId>,
        id: &RequestId,
        answers: UserAnswers,
    ) -> Result<()> {
        self.answer_question(Responder::Agent { token, target }, id, answers)
            .await
    }

    async fn answer_question(
        &self,
        caller: Responder<'_>,
        id: &RequestId,
        answers: UserAnswers,
    ) -> Result<()> {
        let mut changes = self.changed.subscribe();
        loop {
            let sender = self.access(|db| {
                let (target, expected_run, responder) = match caller {
                    Responder::Agent { token, target } => {
                        let (thread, run) = db.question_target(token, target)?;
                        (thread, None, Some(run))
                    }
                    Responder::Host { thread, run } => (thread.clone(), Some(run), None),
                };
                let (request, answering) = db.question(&target, id, expected_run)?;
                if answering {
                    return Ok(None);
                }
                if answers.len() != request.questions.len()
                    || request
                        .questions
                        .iter()
                        .any(|q| !answers.contains_key(&q.id))
                {
                    return Err(Error::Invalid(
                        "provide an answer for each question ID".into(),
                    ));
                }
                let controls = self.controls.lock().unwrap();
                let tx = controls
                    .get(&(target, request.run_id))
                    .filter(|tx| !tx.is_closed())
                    .ok_or_else(|| Error::NotFound("live question responder".into()))?;
                db.claim_question(id, responder.as_ref())?;
                Ok(Some(tx.clone()))
            })?;
            if let Some(tx) = sender {
                let (reply, response) = tokio::sync::oneshot::channel();
                tx.send(Control::Answer {
                    id: id.clone(),
                    answers,
                    reply,
                })
                .map_err(|_| Error::Closed)?;
                return response.await.map_err(|_| Error::Closed)?;
            }
            // Another caller claimed the question. Recheck its committed outcome
            // and this caller's grant after the provider writes or disconnects.
            changes.changed().await.map_err(|_| Error::Closed)?;
        }
    }
}

impl Huggler {
    /// Answer a structured question on the foreground turn and await the provider write.
    pub async fn respond_to_question(
        &self,
        thread: &ThreadId,
        run: &RunId,
        id: &RequestId,
        answers: UserAnswers,
    ) -> Result<()> {
        self.shared
            .answer_question(Responder::Host { thread, run }, id, answers)
            .await
    }
}
