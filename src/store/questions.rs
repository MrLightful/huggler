//! Foreground user input. Permission IDs never enter this table.
use super::*;

pub(super) fn publish(
    tx: &Transaction,
    thread: &ThreadId,
    request: UserInputRequest,
) -> Result<()> {
    if tx.execute(
        "INSERT INTO user_question(id,run_id,questions)
         SELECT ?1,id,?3 FROM run WHERE id=?2 AND thread_id=?4 AND status='running' AND stop_requested=0",
        params![request.id.0, request.run_id.0, serde_json::to_string(&request.questions)?, thread.0],
    )? == 0 {
        return Err(Error::Interrupted);
    }
    event(tx, thread, Change::QuestionRequested { request })?;
    Ok(())
}

pub(super) fn close(
    tx: &Transaction,
    thread: &ThreadId,
    run: &RunId,
    id: &RequestId,
    answered: bool,
) -> Result<()> {
    let responder = tx
        .query_row(
            "UPDATE user_question SET state=CASE WHEN ?3 THEN 'answered' ELSE 'expired' END
         WHERE id=?1 AND run_id=?2 AND (state='answering' OR (state='pending' AND NOT ?3))
         RETURNING responder_run",
            params![id.0, run.0, answered],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?;
    if let Some(responder) = responder {
        event(
            tx,
            thread,
            Change::QuestionClosed {
                id: id.clone(),
                answered,
                responder_run: responder.filter(|_| answered).map(RunId),
            },
        )?;
    }
    Ok(())
}

pub(super) fn expire_run(tx: &Transaction, thread: &ThreadId, run: &RunId) -> Result<()> {
    let mut statement = tx.prepare(
        "SELECT id FROM user_question
         WHERE run_id=? AND state IN ('pending','answering') ORDER BY rowid",
    )?;
    let ids = statement
        .query_map([&run.0], |r| Ok(RequestId(r.get(0)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        close(tx, thread, run, &id, false)?;
    }
    Ok(())
}

impl Database {
    pub fn question_target(
        &self,
        token: &str,
        target: Option<&ThreadId>,
    ) -> Result<(ThreadId, RunId)> {
        let (caller, run, project) = self.caller(token)?;
        let thread = target.unwrap_or(&caller);
        self.require_project_thread(thread, &project)?;
        Ok((thread.clone(), run))
    }

    pub fn open_questions(&self, thread: &ThreadId) -> Result<Vec<UserInputRequest>> {
        let mut statement = self.db.prepare(
            "SELECT q.id, q.run_id, q.questions FROM user_question q JOIN run r ON r.id=q.run_id
             WHERE r.thread_id=? AND q.state IN ('pending','answering') ORDER BY q.rowid",
        )?;
        let rows = statement.query_map([&thread.0], |r| {
            Ok((
                RequestId(r.get(0)?),
                RunId(r.get(1)?),
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, run_id, questions) = row?;
            Ok(UserInputRequest {
                id,
                run_id,
                questions: serde_json::from_str(&questions)?,
            })
        })
        .collect()
    }

    /// Includes an accepted answer until the provider write completes. A racing
    /// responder can wait for that outcome without submitting another write.
    pub fn question(
        &self,
        thread: &ThreadId,
        id: &RequestId,
        expected_run: Option<&RunId>,
    ) -> Result<(UserInputRequest, bool)> {
        let row = self
            .db
            .query_row(
                "SELECT q.run_id,q.questions,q.state,r.stop_requested
             FROM user_question q JOIN run r ON r.id=q.run_id
             WHERE r.thread_id=?1 AND q.id=?2 AND (?3 IS NULL OR q.run_id=?3)",
                params![thread.0, id.0, expected_run.map(|r| &r.0)],
                |r| {
                    Ok((
                        RunId(r.get(0)?),
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        let (run_id, questions, state, stopped) =
            row.ok_or_else(|| Error::NotFound("user-input request".into()))?;
        if state == "answered" {
            return Err(Error::AlreadyAnswered);
        }
        if stopped || state == "expired" {
            return Err(Error::Invalid("user-input request expired".into()));
        }
        Ok((
            UserInputRequest {
                id: id.clone(),
                run_id,
                questions: serde_json::from_str(&questions)?,
            },
            state == "answering",
        ))
    }

    pub fn claim_question(
        &mut self,
        id: &RequestId,
        responder: Option<&RunId>,
        thread: &ThreadId,
        answers: &mut Option<UserAnswers>,
        attachments: &QuestionAttachments,
    ) -> Result<()> {
        if attachments.values().map(Vec::len).sum::<usize>() > 8 {
            return Err(Error::Invalid("answer accepts up to eight uploads".into()));
        }
        let tx = self.db.transaction()?;
        for (question, ids) in attachments {
            let answer = answers
                .as_mut()
                .and_then(|a| a.get_mut(question))
                .ok_or_else(|| {
                    Error::Invalid("attachment requires a matching question answer".into())
                })?;
            let mut values = match std::mem::replace(answer, UserAnswer::Choices(vec![])) {
                UserAnswer::Text(text) => vec![text],
                UserAnswer::Choices(values) => values,
            };
            values.extend(uploads::claim(&*self.output.text, &tx, thread, ids)?);
            *answer = UserAnswer::Choices(values);
        }
        if tx.execute(
            "UPDATE user_question SET state='answering',responder_run=? WHERE id=? AND state='pending'",
            params![responder.map(|r| &r.0), id.0],
        )? != 1 {
            return Err(Error::Invalid("question is no longer pending".into()));
        }
        Ok(tx.commit()?)
    }
}
