#![cfg(unix)]
use huggler::*;
use remote::{Access, Address, Peer, Reply, Request, Write};
use std::{net::SocketAddr, path::Path};
#[path = "support/agents.rs"]
mod agents;
use agents::*;

fn options(state: &Path) -> HugglerOptions {
    let mut options = HugglerOptions::new(state);
    options.codex.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/context-provider.py");
    options
}
async fn serve(rt: &Huggler) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let router = rt.remote_router();
    (
        endpoint,
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() }),
    )
}
fn read(target: &Address) -> Request {
    Request::Read {
        target: target.clone(),
        before: None,
    }
}

#[tokio::test]
async fn approved_hosts_resume_one_remote_delegation_and_revoke_access() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let local_config = options(&dir.path().join("a"));
    let remote_config = options(&dir.path().join("b"));
    let local = Huggler::open(local_config.clone()).await?;
    let host = Huggler::open(remote_config.clone()).await?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let parent = launch(
        &local,
        &local.open_project(&repo)?,
        "parent",
        Provider::Codex,
    );
    let project = host.open_project(&repo)?;
    let thread = launch(&host, &project, "remote", Provider::Codex);
    let sibling = launch(&host, &project, "sibling", Provider::Codex);
    let address = host.address(&thread)?;
    let sender = local.address(&parent)?;
    let run = local
        .send(CommandId("start".into()), parent.clone(), "start".into())?
        .run_id
        .unwrap();
    completed(&local, &parent, &run).await;
    let ro = host.approve_link(&thread, &sender, Access::Read)?;
    let rw = host.approve_link(&thread, &sender, Access::Operate)?;
    let (endpoint, server) = serve(&host).await;
    let reader = Peer::loopback(endpoint, ro.bearer)?;
    let operator = Peer::loopback(endpoint, rw.bearer.clone())?;
    let write = Request::Write {
        target: address.clone(),
        sender_run: run.clone(),
        key: "message".into(),
        action: Write::Send {
            text: "please review".into(),
        },
    };
    let before = host.read(&thread)?.cursor;
    assert!(matches!(
        reader.request(read(&address)).await?,
        Reply::History { .. }
    ));
    assert!(matches!(
        reader.request(write.clone()).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        operator.request(read(&host.address(&sibling)?)).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        operator
            .request(Request::Write {
                target: address.clone(),
                sender_run: run.clone(),
                key: "isolated".into(),
                action: Write::Delegate {
                    task: "review".into(),
                    worktree: true
                }
            })
            .await,
        Err(Error::Unsupported(_))
    ));
    assert_eq!(host.read(&thread)?.cursor, before);
    assert_eq!(host.threads()?.len(), 2);
    let Reply::Accepted { receipt, .. } = operator.request(write.clone()).await? else {
        panic!()
    };
    completed(&host, &thread, &receipt.run_id).await;
    operator.request(write.clone()).await?;
    let remote = host.read(&thread)?;
    assert_eq!(remote.runs.len(), 1);
    let origin = remote.messages[0].origin.as_ref().unwrap();
    assert_eq!(origin.environment_id.as_ref(), Some(&sender.environment));
    assert_eq!((&origin.thread_id, &origin.run_id), (&parent, &run));
    let child = local
        .delegate_remote(
            &operator,
            address.clone(),
            &parent,
            &run,
            "review",
            "check result".into(),
        )
        .await?;
    local.close().await?;
    let child_run = host.read(&child.thread)?.runs[0].id.clone();
    completed(&host, &child.thread, &child_run).await;
    server.abort();
    host.close().await?;
    let local = Huggler::open(local_config).await?;
    let host = Huggler::open(remote_config).await?;
    assert_eq!(host.address(&thread)?, address);
    let (endpoint, server) = serve(&host).await;
    let operator = Peer::loopback(endpoint, rw.bearer)?;
    assert_eq!(
        local
            .delegate_remote(
                &operator,
                address.clone(),
                &parent,
                &run,
                "review",
                "check result".into()
            )
            .await?,
        child
    );
    let wake = local
        .follow_remote(&operator, &parent, &run, "review")
        .await?
        .unwrap();
    completed(&local, &parent, &wake).await;
    assert_eq!(
        local
            .follow_remote(&operator, &parent, &run, "review")
            .await?,
        Some(wake)
    );
    let history = local.read(&parent)?;
    assert_eq!(history.runs.len(), 2);
    assert!(history.messages[2].text.contains("check result"));
    assert_eq!(
        history.messages[2].origin.as_ref().unwrap().thread_id,
        child.thread
    );
    assert!(host.revoke_link(&rw.id)?);
    assert!(matches!(
        operator.request(write).await,
        Err(Error::Forbidden)
    ));
    assert_eq!(host.read(&thread)?.cursor, remote.cursor);
    assert!(host.read(&sibling)?.runs.is_empty());
    server.abort();
    local.close().await?;
    host.close().await
}

#[tokio::test]
async fn single_use_guest_invite_and_moderated_inbox() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let rt = Huggler::open(options(&dir.path().join("state"))).await?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let project = rt.open_project(&repo)?;
    let thread = launch(&rt, &project, "shared", Provider::Codex);
    let other = launch(&rt, &project, "other", Provider::Codex);
    let invite = rt.invite_guest(&thread)?;
    let (endpoint, server) = serve(&rt).await;
    let client = reqwest::Client::new();
    let call = |token: &str, req: guest::Request| {
        client
            .post(format!("http://{endpoint}/v1/guest"))
            .bearer_auth(token)
            .json(&req)
            .send()
    };
    let redeem = || guest::Request::Redeem { name: "Sam".into() };
    let guest::Reply::Session { bearer, .. } = call(&invite.bearer, redeem())
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(call(&invite.bearer, redeem()).await.unwrap().status(), 403);
    let view = |thread: &ThreadId| guest::Request::View {
        thread: thread.clone(),
        before: None,
    };
    assert_eq!(call(&bearer, view(&other)).await.unwrap().status(), 403);
    let submit = |key: &str| guest::Request::Submit {
        thread: thread.clone(),
        key: key.into(),
        text: key.into(),
    };
    for key in ["hello", "dismiss", "hello"] {
        assert!(
            call(&bearer, submit(key))
                .await
                .unwrap()
                .status()
                .is_success()
        )
    }
    let pending = rt.guest_inbox(&thread)?;
    assert_eq!(pending.len(), 2);
    assert!(rt.read(&thread)?.runs.is_empty());
    let run = rt.moderate_guest(&thread, &pending[0].id, true)?.unwrap();
    rt.moderate_guest(&thread, &pending[1].id, false)?;
    assert_eq!(
        rt.moderate_guest(&thread, &pending[0].id, true)?,
        Some(run.clone())
    );
    completed(&rt, &thread, &run).await;
    let guest::Reply::View { messages, .. } = call(&bearer, view(&thread))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages[0].1, "Guest Sam: hello");
    assert!(rt.guest_inbox(&thread)?.is_empty());
    rt.close_share(&invite.id)?;
    assert_eq!(call(&bearer, submit("late")).await.unwrap().status(), 403);
    assert_eq!(call(&bearer, view(&thread)).await.unwrap().status(), 403);
    assert_eq!(rt.read(&thread)?.runs.len(), 1);
    server.abort();
    rt.close().await
}

#[cfg(feature = "application")]
#[tokio::test]
async fn durable_clock_and_signed_delivery() -> Result<()> {
    use automation::{Destination::*, Schedule, Trigger::*};
    use hmac::{Hmac, Mac};
    let dir = tempfile::tempdir()?;
    let config = options(&dir.path().join("state"));
    let rt = Huggler::open(config.clone()).await?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let thread = launch(&rt, &rt.open_project(&repo)?, "clock", Provider::Codex);
    let spec = |trigger, destination| Schedule {
        trigger,
        destination,
        prompt: "scheduled review".into(),
    };
    let once = rt.create_schedule(&thread, spec(Once { at: 10 }, Continue), 0, None)?;
    let interval = rt.create_schedule(
        &thread,
        spec(
            Interval {
                first: 20,
                seconds: 10,
            },
            Continue,
        ),
        0,
        None,
    )?;
    let daily = rt.create_schedule(
        &thread,
        spec(
            Daily {
                zone: "Europe/Oslo".into(),
                hour: 2,
                minute: 30,
            },
            FreshSession,
        ),
        1774656000,
        None,
    )?;
    assert!(rt.tick_schedule(&once, 9)?.is_none());
    let one = rt.tick_schedule(&once, 10)?.unwrap();
    completed(&rt, &thread, &one.run).await;
    let session = rt.read(&thread)?.native_thread.unwrap();
    rt.close().await?;
    let rt = Huggler::open(config).await?;
    assert!(rt.tick_schedule(&once, 100)?.is_none());
    let two = rt.tick_schedule(&interval, 55)?.unwrap();
    completed(&rt, &thread, &two.run).await;
    assert_eq!(rt.read(&thread)?.native_thread.as_ref(), Some(&session));
    assert!(rt.tick_schedule(&interval, 59)?.is_none());
    let utc = |date: &str| {
        chrono::DateTime::parse_from_rfc3339(date)
            .unwrap()
            .timestamp()
    };
    let three = rt
        .tick_schedule(&daily, utc("2026-03-28T01:30:00Z"))?
        .unwrap();
    completed(&rt, &thread, &three.run).await;
    assert_ne!(rt.read(&thread)?.native_thread.unwrap(), session);
    assert_eq!(
        rt.schedules(&thread)?
            .iter()
            .find(|row| row.0 == daily)
            .unwrap()
            .2,
        Some(utc("2026-03-30T00:30:00Z"))
    );
    let key = b"fixture signing key";
    let hook = rt.create_schedule(
        &thread,
        spec(
            Webhook {
                event: "opened".into(),
            },
            NewThread,
        ),
        0,
        Some(key),
    )?;
    let signed = |key: &[u8], body: &[u8]| {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    };
    let body = br#"{"id":"delivery","event":"opened"}"#;
    let filtered = br#"{"id":"other","event":"closed"}"#;
    let other = b"other signing key";
    assert!(matches!(
        rt.deliver_webhook(&hook, other, &signed(other, body), body),
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        rt.deliver_webhook(&hook, key, "00", body),
        Err(Error::Forbidden)
    ));
    assert!(
        rt.deliver_webhook(&hook, key, &signed(key, filtered), filtered)?
            .is_none()
    );
    let fire = || rt.deliver_webhook(&hook, key, &signed(key, body), body);
    let child = fire()?.unwrap();
    assert_ne!(child.thread, thread);
    assert_eq!(fire()?.as_ref(), Some(&child));
    completed(&rt, &child.thread, &child.run).await;
    assert_eq!(rt.schedule_history(&hook)?, [child.clone()]);
    rt.delete_thread(&thread)?;
    assert!(rt.schedule_history(&hook)?.is_empty());
    assert!(matches!(fire(), Err(Error::Forbidden)));
    assert_eq!(rt.read(&child.thread)?.runs.len(), 1);
    rt.close().await
}
