use super::remote::digest;
use super::*;
use crate::guest::{Invitation, Reply, Request, Submission};

impl Database {
    pub fn invite_guest(&self, thread: &ThreadId) -> Result<Invitation> {
        self.thread_position(thread)?;
        let invite = Invitation {
            id: new_id(),
            bearer: new_id(),
        };
        self.db.execute(
            "INSERT INTO guest_share(id,thread,invite) VALUES(?,?,?)",
            params![invite.id, thread.0, digest(&invite.bearer)],
        )?;
        Ok(invite)
    }
    pub fn close_share(&self, id: &str) -> Result<bool> {
        Ok(self
            .db
            .execute("DELETE FROM guest_share WHERE id=?", [id])?
            != 0)
    }
    fn guest_session(&self, token: &str, thread: &ThreadId) -> Result<String> {
        self.db
            .query_row(
                "SELECT id FROM guest_share WHERE session=? AND thread=?",
                params![digest(token), thread.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::Forbidden)
    }
    pub fn guest_request(&self, token: &str, request: Request) -> Result<Reply> {
        match request {
            Request::Redeem { name } => {
                if name.trim().is_empty() || name.len() > 256 {
                    return Err(Error::Invalid("invalid guest name".into()));
                }
                let bearer = new_id();
                let thread=self.db.query_row("UPDATE guest_share SET invite=NULL,session=?,name=? WHERE invite=? RETURNING thread",params![digest(&bearer),name,digest(token)],|r|Ok(ThreadId(r.get(0)?))).optional()?.ok_or(Error::Forbidden)?;
                Ok(Reply::Session { bearer, thread })
            }
            Request::View { thread, before } => {
                self.guest_session(token, &thread)?;
                let history = self.history(&thread, before.as_ref(), "", 50)?;
                Ok(Reply::View {
                    thread,
                    messages: history
                        .messages
                        .into_iter()
                        .map(|m| (m.message.role, m.message.text))
                        .collect(),
                    older: history.older,
                })
            }
            Request::Submit { thread, key, text } => {
                let share = self.guest_session(token, &thread)?;
                if key.trim().is_empty()
                    || key.len() > 256
                    || text.trim().is_empty()
                    || text.len() > 16384
                {
                    return Err(Error::Invalid("invalid guest submission".into()));
                }
                if let Some((id, prior)) = self
                    .db
                    .query_row(
                        "SELECT id,text FROM guest_submission WHERE share=? AND request_key=?",
                        params![share, key],
                        |r| Ok((r.get(0)?, r.get::<_, String>(1)?)),
                    )
                    .optional()?
                {
                    if prior != text {
                        return Err(Error::Conflict {
                            command: CommandId(key),
                            original_thread: thread.clone(),
                            requested_thread: thread,
                        });
                    }
                    return Ok(Reply::Submitted { id });
                }
                let id = new_id();
                self.db.execute(
                    "INSERT INTO guest_submission(id,share,request_key,text) VALUES(?,?,?,?)",
                    params![id, share, key, text],
                )?;
                Ok(Reply::Submitted { id })
            }
        }
    }
    pub fn guest_inbox(&self, thread: &ThreadId) -> Result<Vec<Submission>> {
        Ok(self.db.prepare("SELECT s.id,g.name,s.text FROM guest_submission s JOIN guest_share g ON s.share=g.id WHERE g.thread=? AND decision IS NULL ORDER BY s.rowid")?.query_map([&thread.0],|r|Ok(Submission{id:r.get(0)?,name:r.get(1)?,text:r.get(2)?}))?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn moderate_guest(
        &mut self,
        thread: &ThreadId,
        id: &str,
        accept: bool,
    ) -> Result<(Option<RunId>, Option<RunJob>)> {
        let tx = self.db.transaction()?;
        let (prior,run,name,text,model)=tx.query_row("SELECT s.decision,s.run,g.name,s.text,t.model FROM guest_submission s JOIN guest_share g ON g.id=s.share JOIN thread t ON t.id=g.thread WHERE s.id=? AND t.id=?",params![id,thread.0],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?.map(RunId),r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get(4)?))).optional()?.ok_or(Error::Forbidden)?;
        let decision = if accept { "accepted" } else { "dismissed" };
        if let Some(prior) = prior {
            if prior != decision {
                return Err(Error::AlreadyAnswered);
            }
            return Ok((run, None));
        }
        let (run, job) = if accept {
            let (run, _, _, delivery) = start_message(
                &tx,
                thread,
                &format!("Guest {name}: {text}"),
                model,
                None,
                true,
            )?;
            tx.execute(
                "UPDATE turn_effect SET durable_launch=1 WHERE run_id=?",
                [&run.0],
            )?;
            (
                Some(run.clone()),
                (delivery == PeerDelivery::Started).then(|| (thread.clone(), run)),
            )
        } else {
            (None, None)
        };
        tx.execute(
            "UPDATE guest_submission SET decision=?,run=? WHERE id=?",
            params![decision, run.as_ref().map(|r| &r.0), id],
        )?;
        tx.commit()?;
        Ok((run, job))
    }
}
