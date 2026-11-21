use super::*;
use crate::application::{Preferences, SecretRequest};
use serde_json::{Value, json};

pub(super) struct Secret {
    reference: String,
    value: Vec<u8>,
    expires: i64,
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
impl Database {
    pub fn preferences(&self) -> Result<Preferences> {
        let value: String = self
            .db
            .query_row("SELECT preferences FROM environment", [], |r| r.get(0))?;
        Ok(serde_json::from_str(&value)?)
    }
    pub fn set_preferences(&self, prefs: &Preferences) -> Result<()> {
        self.db.execute(
            "UPDATE environment SET preferences=?",
            [serde_json::to_string(prefs)?],
        )?;
        Ok(())
    }
    pub fn discovered(&self, token: &str, name: &str, reveal: bool) -> Result<bool> {
        let (_, run, _) = self.caller(token)?;
        if reveal {
            self.db.execute(
                "INSERT OR IGNORE INTO discovered_tool VALUES(?,?)",
                params![run.0, name],
            )?;
        }
        Ok(self
            .db
            .prepare("SELECT 1 FROM discovered_tool WHERE run=? AND name=?")?
            .exists(params![run.0, name])?)
    }
    pub fn notify_user(&mut self, token: &str, key: &str, text: &str) -> Result<()> {
        let (thread, run, _) = self.caller(token)?;
        if text.trim().is_empty() || text.len() > 16384 || key.trim().is_empty() {
            return Err(Error::Invalid(
                "expected a request key and up to 16 KiB of text".into(),
            ));
        }
        let tx = self.db.transaction()?;
        if tx.execute(
            "INSERT INTO agent_notice VALUES(?,?,?) ON CONFLICT DO NOTHING",
            params![run.0, key, text],
        )? == 1
        {
            event(
                &tx,
                &thread,
                Change::UserNotice {
                    run_id: run,
                    text: text.into(),
                },
            )?;
        } else if !tx
            .prepare("SELECT 1 FROM agent_notice WHERE run=? AND request_key=? AND body=?")?
            .exists(params![run.0, key, text])?
        {
            return Err(Error::Invalid("notice key already used".into()));
        }
        tx.commit()?;
        Ok(())
    }
    pub fn user_notices(&self, thread: &ThreadId) -> Result<Vec<(RunId, String)>> {
        self.thread_position(thread)?;
        Ok(self.db.prepare("SELECT n.run,n.body FROM agent_notice n JOIN run r ON r.id=n.run WHERE r.thread_id=? ORDER BY n.rowid")?.query_map([&thread.0],|r|Ok((RunId(r.get(0)?),r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }
    fn expire_secrets(&mut self) -> Result<()> {
        let now = now();
        self.secrets.retain(|_, s| s.expires > now);
        self.db.execute("UPDATE secret_request SET state='expired' WHERE (state IN ('requested','approved') AND expires<=?) OR (state='requested' AND run NOT IN (SELECT run_id FROM agent_grant))",[now])?;
        Ok(())
    }
    pub fn request_secret(
        &mut self,
        token: &str,
        key: &str,
        label: &str,
        ttl: u32,
    ) -> Result<Value> {
        let (_, run, _) = self.caller(token)?;
        if key.trim().is_empty() || label.trim().is_empty() || label.len() > 256 {
            return Err(Error::Invalid(
                "expected a request key and short secret label".into(),
            ));
        }
        self.expire_secrets()?;
        self.db.execute("INSERT INTO secret_request VALUES(?,?,?,?,'requested',?) ON CONFLICT(run,request_key) DO NOTHING",params![new_id(),run.0,key,label,now()+i64::from(ttl)])?;
        let (id, state): (String, String) = self
            .db
            .query_row(
                "SELECT id,state FROM secret_request WHERE run=? AND request_key=? AND label=?",
                params![run.0, key, label],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid("secret request key already used".into()))?;
        let reference = self.secrets.get(&id).map(|s| &s.reference);
        Ok(json!({"id":id,"state":state,"reference":reference}))
    }
    pub fn secret_requests(&mut self, thread: &ThreadId) -> Result<Vec<SecretRequest>> {
        self.thread_position(thread)?;
        self.expire_secrets()?;
        Ok(self.db.prepare("SELECT s.id,s.run,s.label,s.state,s.expires FROM secret_request s JOIN run r ON r.id=s.run WHERE r.thread_id=? ORDER BY s.rowid")?.query_map([&thread.0],|r|Ok(SecretRequest{id:r.get(0)?,run:RunId(r.get(1)?),label:r.get(2)?,state:r.get(3)?,expires:r.get(4)?}))?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn answer_secret(
        &mut self,
        thread: &ThreadId,
        id: &str,
        value: Option<Vec<u8>>,
    ) -> Result<()> {
        self.expire_secrets()?;
        let expires=self.db.query_row("UPDATE secret_request SET state=? WHERE id=? AND state='requested' AND expires>? AND run IN (SELECT r.id FROM agent_grant g JOIN run r ON r.id=g.run_id WHERE r.thread_id=?) RETURNING expires",params![if value.is_some(){"approved"}else{"denied"},id,now(),thread.0],|r|r.get(0)).optional()?.ok_or(Error::Forbidden)?;
        if let Some(value) = value {
            self.secrets.insert(
                id.into(),
                Secret {
                    value,
                    expires,
                    reference: new_id(),
                },
            );
        }
        Ok(())
    }
    pub fn consume_secret(&mut self, project: &ProjectId, reference: &str) -> Result<Vec<u8>> {
        self.expire_secrets()?;
        let id = self
            .secrets
            .iter()
            .find(|(_, s)| s.reference == reference)
            .map(|(k, _)| k.clone())
            .ok_or(Error::Forbidden)?;
        if self.db.execute("UPDATE secret_request SET state='consumed' WHERE id=? AND state='approved' AND expires>? AND run IN (SELECT r.id FROM run r JOIN thread t ON t.id=r.thread_id WHERE t.project=?)",params![id,now(),project.0])? !=1 {return Err(Error::Forbidden);}
        Ok(self.secrets.remove(&id).unwrap().value)
    }
}
