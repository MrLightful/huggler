use super::*;
use crate::contracts::agent_commands::{CreateThreads, CreatedPeer};

pub(super) fn instance_driver(tx: &Transaction, instance: &str) -> Result<String> {
    tx.query_row(
        "SELECT driver FROM configured_provider WHERE id=?",
        [instance],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("provider instance {instance}")))
}

/// Agent-created threads inherit their caller's checkout. A different provider
/// starts with its own default model.
pub(super) fn create_agent_thread(
    tx: &Transaction,
    caller: &ThreadId,
    name: &str,
    provider: Option<&str>,
) -> Result<ThreadId> {
    let thread = ThreadId(new_id());
    let driver = provider.map(|id| instance_driver(tx, id)).transpose()?;
    tx.execute(
        "INSERT INTO thread(id,project,name,provider_instance,model,worktree_path,provider)
         SELECT ?1,project,?2,coalesce(?3,provider_instance),CASE WHEN ?3 IS NULL OR ?3=provider_instance THEN model END,worktree_path,coalesce(?5,provider)
         FROM thread WHERE id=?4",
        params![thread.0, name, provider, caller.0, driver],
    )?;
    event(tx, &thread, Change::Launched)?;
    Ok(thread)
}

struct PriorCreation {
    peer: CreatedPeer,
    title: Option<String>,
    provider: Option<String>,
    prompt: Option<String>,
}

fn prior_creations(tx: &Transaction, caller: &ThreadId, key: &str) -> Result<Vec<PriorCreation>> {
    Ok(tx
        .prepare(
            "SELECT c.thread_id,c.initial_run,c.requested_title,c.requested_provider,
                (SELECT text FROM message WHERE run_id=c.initial_run AND role='user' ORDER BY rowid LIMIT 1)
             FROM peer_creation c
             WHERE c.caller_thread=? AND c.request_key=? ORDER BY c.position",
        )?
        .query_map(params![caller.0, key], |r| {
            Ok(PriorCreation {
                peer: CreatedPeer {
                    thread_id: ThreadId(r.get(0)?),
                    run_id: r.get::<_, Option<String>>(1)?.map(RunId),
                },
                title: r.get(2)?,
                provider: r.get(3)?,
                prompt: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

impl Database {
    pub fn create_threads(
        &mut self,
        token: &str,
        input: CreateThreads,
    ) -> Result<(Vec<CreatedPeer>, Vec<RunJob>)> {
        let (caller, caller_run, _) = self.caller(token)?;
        let key = input.client_request_id;
        if key.trim().is_empty() || !(1..=20).contains(&input.threads.len()) {
            return Err(Error::Invalid(
                "expected clientRequestId and 1 to 20 threads".into(),
            ));
        }
        let tx = self.db.transaction()?;
        let prior = prior_creations(&tx, &caller, &key)?;
        let conflict = || Error::Conflict {
            command: CommandId(key.clone()),
            original_thread: caller.clone(),
            requested_thread: caller.clone(),
        };
        if !prior.is_empty() && prior.len() != input.threads.len() {
            return Err(conflict());
        }
        let mut prior = prior.into_iter();
        let mut peers = Vec::with_capacity(input.threads.len());
        let mut jobs = Vec::new();
        for (position, request) in input.threads.into_iter().enumerate() {
            let title = request.title.as_deref();
            let prompt = request.prompt.as_deref();
            let provider = request
                .target
                .as_ref()
                .map(|t| t.provider_instance_id.as_str());
            if title.is_some_and(|t| t.trim().is_empty()) {
                return Err(Error::Invalid("empty thread title".into()));
            }
            let mut peer = if let Some(previous) = prior.next() {
                if previous.title != request.title
                    || previous.provider.as_deref() != provider
                    || prompt
                        .zip(previous.prompt.as_deref())
                        .is_some_and(|(requested, accepted)| requested != accepted)
                {
                    return Err(conflict());
                }
                previous.peer
            } else {
                let name = title
                    .map(crate::output::short_title)
                    .unwrap_or_else(|| self.output.text.peer_title(prompt, position + 1));
                let thread_id = create_agent_thread(&tx, &caller, &name, provider)?;
                tx.execute(
                    "INSERT INTO peer_creation(caller_thread,request_key,position,thread_id,requested_title,requested_provider)
                     VALUES(?,?,?,?,?,?)",
                    params![caller.0, key, position as i64, thread_id.0, title, provider],
                )?;
                event(
                    &tx,
                    &caller,
                    Change::PeerCreated {
                        caller_run_id: caller_run.clone(),
                        peer_thread_id: thread_id.clone(),
                    },
                )?;
                CreatedPeer {
                    thread_id,
                    run_id: None,
                }
            };
            if peer.run_id.is_none()
                && let Some(prompt) = prompt
            {
                let model = tx.query_row(
                    "SELECT model FROM thread WHERE id=?",
                    [&peer.thread_id.0],
                    |r| r.get(0),
                )?;
                let origin = MessageOrigin {
                    environment_id: None,
                    thread_id: caller.clone(),
                    run_id: caller_run.clone(),
                    request_key: key.clone(),
                };
                let (run, _, _, delivery) =
                    start_message(&tx, &peer.thread_id, prompt, model, Some(origin), false)?;
                if delivery == PeerDelivery::Queued {
                    return Err(Error::Busy);
                }
                tx.execute(
                    "UPDATE peer_creation SET initial_run=? WHERE thread_id=?",
                    params![run.0, peer.thread_id.0],
                )?;
                jobs.push((peer.thread_id.clone(), run.clone()));
                peer.run_id = Some(run);
            }
            peers.push(peer);
        }
        tx.commit()?;
        Ok((peers, jobs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn setup(dir: &Path) -> Result<(Database, String)> {
        let mut db = Database::open(dir)?;
        let project = db.project(dir)?;
        let caller = ThreadId("caller".into());
        db.dispatch(&Command {
            id: CommandId("launch".into()),
            thread_id: caller.clone(),
            action: Action::Launch {
                project: project.id,
                name: "caller".into(),
                model: Some("parent-model".into()),
            },
        })?;
        let (sent, _) = db.dispatch(&Command {
            id: CommandId("start".into()),
            thread_id: caller,
            action: Action::Send {
                text: "create peers".into(),
            },
        })?;
        let token = db.grant(&sent.run_id.unwrap())?;
        Ok((db, token))
    }

    fn batch(entries: Value) -> CreateThreads {
        serde_json::from_value(json!({"clientRequestId":"batch", "threads": entries})).unwrap()
    }

    #[test]
    fn peer_delivery_recovers_until_claimed_and_keeps_its_receipt() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut db, token) = setup(dir.path())?;
        let (peers, _) = db.create_threads(&token, batch(json!([{}])))?;
        let target = &peers[0].thread_id;
        let (first, _) = db.send_peer(&token, target, "first", "Review")?;
        let (second, _) = db.send_peer(&token, target, "second", "Follow up")?;
        assert_eq!(first.delivery, PeerDelivery::Started);
        assert_eq!(second.delivery, PeerDelivery::Queued);
        drop(db);

        let mut db = Database::open(dir.path())?;
        assert_eq!(db.ready_jobs()?, [(target.clone(), first.run_id.clone())]);
        db.claim(&first.run_id)?;
        drop(db);

        let mut db = Database::open(dir.path())?;
        assert_eq!(
            db.find_run(target, &first.run_id)?.status,
            RunStatus::Failed
        );
        assert_eq!(db.ready_jobs()?, [(target.clone(), second.run_id.clone())]);
        drop(db);

        // The queued follow-up was promoted, but its worker hadn't claimed it.
        let mut db = Database::open(dir.path())?;
        assert_eq!(db.ready_jobs()?, [(target.clone(), second.run_id.clone())]);
        let (retry, _) = db.dispatch(&Command {
            id: CommandId("retry".into()),
            thread_id: ThreadId("caller".into()),
            action: Action::Send {
                text: "retry delivery".into(),
            },
        })?;
        let token = db.grant(&retry.run_id.unwrap())?;
        assert_eq!(
            db.send_peer(&token, target, "second", "Follow up")?,
            (second.clone(), false)
        );
        assert!(db.claim(&second.run_id)?.text.ends_with("Follow up"));
        assert_eq!(db.read(target)?.runs.len(), 2);
        Ok(())
    }

    #[test]
    fn batch_recovery_starts_only_unclaimed_prompts() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut db, token) = setup(dir.path())?;
        let (_, jobs) = db.create_threads(&token, batch(json!([
            {"prompt":"pending"}, {"prompt":"claimed","target":{"providerInstanceId":"claudeAgent"}}
        ])))?;
        let [pending, claimed] = jobs.as_slice() else {
            panic!("expected two jobs")
        };
        assert!(db.claim(&claimed.1)?.model.is_none());
        drop(db);

        let mut reopened = Database::open(dir.path())?;
        assert_eq!(reopened.ready_jobs()?, std::slice::from_ref(pending));
        assert_eq!(
            reopened.find_run(&claimed.0, &claimed.1)?.status,
            RunStatus::Failed
        );
        assert_eq!(
            reopened.claim(&pending.1)?.model.as_deref(),
            Some("parent-model")
        );
        assert!(reopened.ready_jobs()?.is_empty());
        Ok(())
    }

    #[test]
    fn batch_errors_roll_back_earlier_entries_and_prompts() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut db, token) = setup(dir.path())?;
        let invalid = batch(json!([{"prompt":"valid"}, {"prompt":"  "}]));
        assert!(matches!(
            db.create_threads(&token, invalid),
            Err(Error::Invalid(_))
        ));
        assert_eq!(db.threads_after(0)?.len(), 1);
        let (peers, jobs) = db.create_threads(&token, batch(json!([{}, {"prompt":"original"}])))?;
        assert_eq!(jobs.len(), 1);
        for entries in [
            json!([{"prompt":"late"}, {"prompt":"changed"}]),
            json!([{"prompt":"late"}, {"title":"changed"}]),
            json!([{"prompt":"late"}, {"target":{"providerInstanceId":"claudeAgent"}}]),
            json!([{"prompt":"late"}]),
        ] {
            assert!(matches!(
                db.create_threads(&token, batch(entries)),
                Err(Error::Conflict { .. })
            ));
        }
        assert!(db.read(&peers[0].thread_id)?.runs.is_empty());
        assert_eq!(db.read(&peers[1].thread_id)?.runs.len(), 1);
        Ok(())
    }
}
