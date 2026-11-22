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
    serve_router(rt.remote_router()).await
}
async fn serve_router(router: axum::Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
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
    // The operating host uses only the published HTTP contract, in its own process.
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    let mut client = tokio::process::Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-client.py"))
        .stdin(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let input = serde_json::json!({
        "url":format!("http://{endpoint}/v1/peer"), "bearer":rw.bearer,
        "target":address, "sender_run":run
    });
    client
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&input)?)
        .await?;
    assert!(client.wait().await?.success());
    let remote = host.read(&thread)?;
    assert_eq!(remote.runs.len(), 1);
    let origin = remote.messages[0].origin.as_ref().unwrap();
    assert_eq!(origin.environment_id.as_ref(), Some(&sender.environment));
    assert_eq!((&origin.thread_id, &origin.run_id), (&parent, &run));
    // A definitive rejection leaves the key free to use with corrected intent.
    #[cfg(feature = "application")]
    {
        assert!(matches!(
            host.setup_workspace(&project.id, Path::new("/usr/bin/false"), &[])
                .await?,
            workspace::SetupStatus::Failed(_)
        ));
        assert!(matches!(
            local
                .delegate_remote(
                    &operator,
                    address.clone(),
                    &parent,
                    &run,
                    "review",
                    "before setup".into()
                )
                .await,
            Err(Error::Invalid(_))
        ));
        assert_eq!(host.threads()?.len(), 2);
        assert_eq!(
            host.setup_workspace(&project.id, Path::new("/usr/bin/true"), &[])
                .await?,
            workspace::SetupStatus::Ready
        );
    }
    assert!(matches!(
        local
            .delegate_remote(
                &reader,
                address.clone(),
                &parent,
                &run,
                "review",
                "old draft".into()
            )
            .await,
        Err(Error::Forbidden)
    ));
    // Lose the receipt after the remote transaction commits. A denial on retry
    // must preserve the original intent until an operating grant recovers it.
    let lossy = host.remote_router().layer(axum::middleware::map_response(
        |response: axum::response::Response| async move {
            assert!(response.status().is_success());
            "incomplete receipt"
        },
    ));
    let (lossy_endpoint, lossy_server) = serve_router(lossy).await;
    let lossy_peer = Peer::loopback(lossy_endpoint, rw.bearer.clone())?;
    for (peer, text, expected) in [
        (&lossy_peer, "check result", "unknown"),
        (&reader, "check result", "denied"),
        (&operator, "changed task", "conflict"),
    ] {
        let result = local
            .delegate_remote(peer, address.clone(), &parent, &run, "review", text.into())
            .await;
        assert!(matches!(
            (result, expected),
            (Err(Error::Io(_)), "unknown")
                | (Err(Error::Forbidden), "denied")
                | (Err(Error::Conflict { .. }), "conflict")
        ));
    }
    lossy_server.abort();
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
    assert!(host.revoke_link(&rw.id)?);
    // The sender already saved this receipt and can recover it without another write.
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
    assert!(matches!(
        operator.request(write.clone()).await,
        Err(Error::Forbidden)
    ));
    let replacement = host.approve_link(&thread, &sender, Access::Operate)?;
    let operator = Peer::loopback(endpoint, replacement.bearer)?;
    // The receiver must also replay the receipt when the sender lost the response.
    let retry = |task: &str| Request::Write {
        target: address.clone(),
        sender_run: run.clone(),
        key: "review".into(),
        action: Write::Delegate {
            task: task.into(),
            worktree: false,
        },
    };
    let Reply::Accepted {
        address: replay,
        receipt,
    } = operator.request(retry("check result")).await?
    else {
        panic!("expected the original receipt");
    };
    assert_eq!(replay, child);
    assert_eq!(receipt.run_id, child_run);
    assert!(operator.request(retry("different task")).await.is_err());
    assert_eq!(host.threads()?.len(), 3);
    let unrelated = Address {
        thread: ThreadId("unrelated".into()),
        ..sender.clone()
    };
    for grant in [
        host.approve_link(&thread, &unrelated, Access::Operate)?,
        host.approve_link(&thread, &sender, Access::Read)?,
    ] {
        let peer = Peer::loopback(endpoint, grant.bearer)?;
        assert!(matches!(
            peer.request(read(&child)).await,
            Err(Error::Forbidden)
        ));
    }
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
    #[cfg(feature = "application")]
    {
        local.delete_thread(&parent)?;
        host.delete_thread(&child.thread)?;
        let Reply::Accepted { address: prior, .. } =
            operator.request(retry("check result")).await?
        else {
            panic!("deleted child must retain its original receipt");
        };
        assert_eq!(prior, child);
        assert_eq!(host.threads()?.len(), 2);
    }
    assert!(host.revoke_link(&replacement.id)?);
    assert!(matches!(
        operator.request(write).await,
        Err(Error::Forbidden)
    ));
    assert_eq!(host.read(&thread)?.cursor, remote.cursor);
    #[cfg(feature = "application")]
    host.delete_thread(&thread)?;
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
    assert_eq!(rt.schedule_history(&hook)?, std::slice::from_ref(&child));
    let second_body = br#"{"id":"second","event":"opened"}"#;
    let replay_second = || rt.deliver_webhook(&hook, key, &signed(key, second_body), second_body);
    let second = replay_second()?.unwrap();
    completed(&rt, &second.thread, &second.run).await;
    let list = rt.conversation_list()?;
    assert_eq!(list.threads[0].id, second.thread);
    assert_eq!(list.threads[0].status, Some(RunStatus::Completed));
    let (snapshot, mut subscription) = rt.subscribe(&second.thread, None)?;
    let observers = async {
        tokio::join!(
            rt.wait_for_change(&second.thread, snapshot.cursor),
            subscription.next(),
            rt.wait_for_conversation_list(list.revision),
        )
    };
    tokio::pin!(observers);
    tokio::select! {
        biased;
        _ = &mut observers => panic!("observers returned before deletion"),
        _ = std::future::ready(()) => {}
    }
    rt.delete_thread(&second.thread)?;
    let (change, event, new_list) =
        tokio::time::timeout(std::time::Duration::from_secs(2), observers)
            .await
            .expect("deletion should wake thread and list observers");
    let new_list = new_list?;
    assert!(new_list.revision > list.revision);
    assert_eq!(new_list.threads, list.threads[1..]);
    assert!(matches!(change, Err(Error::NotFound(id)) if id == second.thread.0));
    assert!(matches!(event, Err(Error::NotFound(id)) if id == second.thread.0));
    assert_eq!(replay_second()?.as_ref(), Some(&second));
    assert_eq!(rt.threads()?.len(), 2);
    rt.delete_thread(&thread)?;
    assert!(rt.schedule_history(&hook)?.is_empty());
    assert!(matches!(fire(), Err(Error::Forbidden)));
    assert_eq!(rt.read(&child.thread)?.runs.len(), 1);
    let later = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        rt.wait_for_threads_after(Some(&thread)),
    )
    .await
    .expect("a surviving child should be visible immediately")?;
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].id, child.thread);
    let unknown = ThreadId("never-created".into());
    assert!(matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            rt.wait_for_threads_after(Some(&unknown)),
        )
        .await
        .expect("unknown list cursor should fail immediately"),
        Err(Error::NotFound(id)) if id == unknown.0
    ));
    rt.close().await
}

#[cfg(feature = "application")]
#[tokio::test]
async fn prepare_promote_clone_and_release_workspaces() -> Result<()> {
    use workspace::SetupStatus;
    let dir = tempfile::tempdir()?;
    let mut opts = options(&dir.path().join("state"));
    let pool = dir.path().join("external pool");
    opts.storage.worktree_root = WorktreeRoot::Shared(pool.clone());
    let rt = Huggler::open(opts.clone()).await?;
    let scratch = rt.create_scratch(&dir.path().join("scratch"))?;
    std::fs::write(scratch.root.join("original"), "scratch")?;
    assert_eq!(rt.promote_scratch(&scratch.id)?.id, scratch.id);
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&scratch.root)
                .status()
                .unwrap()
                .success()
        )
    };
    git(&["add", "original"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=test@example.com",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "scratch",
    ]);
    let cloned = rt.clone_project(scratch.root.to_str().unwrap(), &dir.path().join("clone"))?;
    let external = dir.path().join("existing");
    git(&[
        "worktree",
        "add",
        "-q",
        "--detach",
        external.to_str().unwrap(),
    ]);
    for (name, checkout) in [
        ("root", cloned.clone()),
        ("existing", rt.open_project(&external)?),
    ] {
        let thread = launch(&rt, &checkout, name, Provider::Codex);
        let run = rt.send(
            CommandId(format!("send-{name}")),
            thread.clone(),
            name.into(),
        )?;
        completed(&rt, &thread, &run.run_id.unwrap()).await;
        assert!(checkout.root.join("frames.ndjson").exists());
        assert!(matches!(
            rt.release_workspace(&checkout.id),
            Err(Error::Forbidden)
        ));
    }
    let run_git = |root: &Path, args: &[&str]| -> Result<String> {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_owned())
    };
    run_git(&cloned.root, &["branch", "fixture-base"])?;
    let original = run_git(&cloned.root, &["rev-parse", "HEAD"])?;
    run_git(
        &cloned.root,
        &["symbolic-ref", "HEAD", "refs/heads/fixture-unborn"],
    )?;
    assert!(
        rt.allocate_workspace_at(&cloned.id, "refs/heads/missing")
            .is_err()
    );
    assert!(rt.allocated_workspaces(&cloned.id)?.is_empty());
    let project = rt.allocate_workspace_at(&cloned.id, "refs/heads/fixture-base")?;
    assert_eq!(run_git(&project.root, &["rev-parse", "HEAD"])?, original);
    assert_eq!(project.root.parent().unwrap(), pool.canonicalize()?);
    rt.close().await?;
    opts.storage.worktree_root = WorktreeRoot::Shared("new pool".into());
    let rt = Huggler::open(opts).await?;
    assert_eq!(rt.allocated_workspaces(&cloned.id)?[0].id, project.id);
    assert_eq!(rt.open_project(&project.root)?.id, project.id);
    let a = launch(&rt, &project, "a", Provider::Codex);
    let b = launch(&rt, &project, "b", Provider::Codex);
    let failed = rt
        .setup_workspace(&project.id, Path::new("/usr/bin/false"), &[])
        .await?;
    assert!(matches!(failed, SetupStatus::Failed(_)));
    assert_eq!(rt.workspace_setup_status(&project.id)?, Some(failed));
    let send = || rt.send(CommandId("release".into()), a.clone(), "ready".into());
    assert!(send().is_err());
    assert!(rt.read(&a)?.runs.is_empty());
    assert_eq!(
        rt.setup_workspace(&project.id, Path::new("/usr/bin/true"), &[])
            .await?,
        SetupStatus::Ready
    );
    completed(&rt, &a, &send()?.run_id.unwrap()).await;
    rt.delete_thread(&a)?;
    assert!(matches!(
        rt.release_workspace(&project.id),
        Err(Error::Busy)
    ));
    rt.delete_thread(&b)?;
    assert!(rt.release_workspace(&project.id).is_err());
    std::fs::remove_file(project.root.join("frames.ndjson"))?;
    rt.release_workspace(&project.id)?;
    assert!(!project.root.exists());
    assert!(rt.allocated_workspaces(&cloned.id)?.is_empty());
    assert_eq!(
        std::fs::read_to_string(cloned.root.join("original"))?,
        "scratch"
    );
    rt.close().await
}

#[cfg(feature = "application")]
#[tokio::test]
async fn closing_runtime_reaps_workspace_setup_and_records_failure() -> Result<()> {
    use std::{sync::Arc, time::Duration};
    use workspace::SetupStatus;
    let dir = tempfile::tempdir()?;
    let config = options(&dir.path().join("state"));
    let rt = Arc::new(Huggler::open(config.clone()).await?);
    let project = rt.create_scratch(&dir.path().join("scratch"))?;
    let worker = rt.clone();
    let id = project.id.clone();
    let setup = tokio::spawn(async move {
        worker
            .setup_workspace(
                &id,
                Path::new("/bin/sh"),
                &[
                    "-c".into(),
                    "echo $$ > started.tmp && mv started.tmp started && exec sleep 30".into(),
                ],
            )
            .await
    });
    let marker = project.root.join("started");
    let timeout = Duration::from_secs(5);
    tokio::time::timeout(timeout, async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("setup did not start");
    let pid = std::fs::read_to_string(marker)?
        .trim()
        .parse::<i32>()
        .unwrap();
    assert_eq!(
        rt.workspace_setup_status(&project.id)?,
        Some(SetupStatus::Running)
    );
    assert!(matches!(
        rt.setup_workspace(&project.id, Path::new("/bin/sh"), &[])
            .await,
        Err(Error::Busy)
    ));
    tokio::time::timeout(timeout, rt.close())
        .await
        .expect("setup blocked close")?;
    let status = setup.await.unwrap()?;
    assert!(matches!(status, SetupStatus::Failed(_)));
    // Signal zero checks whether the original process still exists.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    let reopened = Huggler::open(config).await?;
    assert_eq!(reopened.workspace_setup_status(&project.id)?, Some(status));
    reopened.close().await
}

#[tokio::test]
async fn allowlisted_tools_remain_callable_when_search_is_disabled() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut config = options(&dir.path().join("state"));
    config.agent_tools = true;
    config.agent_policy.defer_tools = true;
    config.agent_policy.allowed_tools = Some(
        ["send_to_user", "orchestrator_capabilities"]
            .map(String::from)
            .into(),
    );
    config.claude_agent.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
    let rt = Huggler::open(config).await?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let thread = launch(
        &rt,
        &rt.open_project(&repo)?,
        "sender",
        Provider::ClaudeAgent,
    );
    let run = rt
        .send(
            CommandId("notice".into()),
            thread.clone(),
            "direct notice".into(),
        )?
        .run_id
        .unwrap();
    completed(&rt, &thread, &run).await;
    assert_eq!(rt.user_notices(&thread)?.len(), 1);
    rt.close().await
}

#[tokio::test]
#[ignore = "uses the authenticated Codex app-server and a synthetic secret"]
async fn live_codex_managed_secret() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut config = HugglerOptions::new(dir.path().join("state"));
    config.agent_tools = true;
    config.codex.request_approvals = true;
    let allowed = [
        "orchestrator_capabilities",
        "request_secret",
        "send_to_user",
    ];
    config.agent_policy.allowed_tools = Some(allowed.map(String::from).into());
    config.agent_policy.secret_ttl_seconds = Some(120);
    let rt = Huggler::open(config).await?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let project = rt.open_project(&repo)?;
    let thread = launch(&rt, &project, "requester", Provider::Codex);
    let prompt = "Use only the Huggler MCP server. Call orchestrator_capabilities and send its tools array as JSON in a send_to_user notice with clientRequestId catalog. Call request_secret with clientRequestId fixture and label Qualification. Poll the same request until approved. Finish with just the reference string you received.";
    let run = rt
        .send(CommandId("qualify".into()), thread.clone(), prompt.into())?
        .run_id
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(180), async {
        loop {
            let snapshot = rt.read(&thread)?;
            for request in &snapshot.permissions {
                let input = &request.tool_call["rawInput"];
                assert_eq!(input["serverName"], "huggler");
                assert!(allowed.contains(&input["_meta"]["tool_name"].as_str().unwrap()));
                rt.respond_to_permission(&thread, &run, &request.id, "accept".into())?;
            }
            for secret in rt.secret_requests(&thread)? {
                if secret.state == "requested" {
                    assert_eq!(secret.label, "Qualification");
                    rt.answer_secret(&thread, &secret.id, Some(b"qualification-only".to_vec()))?;
                }
            }
            if snapshot.runs[0].status.terminal() {
                assert_eq!(
                    snapshot.runs[0].status,
                    RunStatus::Completed,
                    "{:?}",
                    snapshot.runs[0].error
                );
                let reference = snapshot.messages.last().unwrap().text.trim();
                assert_eq!(
                    rt.consume_secret(&project.id, reference)?,
                    b"qualification-only"
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok::<_, Error>(())
    })
    .await
    .expect("live Codex timed out")?;
    let notices = rt.user_notices(&thread)?;
    assert_eq!(notices.len(), 1);
    let mut tools: Vec<String> = serde_json::from_str(&notices[0].1)?;
    tools.sort();
    assert_eq!(tools, allowed);
    rt.close().await
}

#[tokio::test]
async fn managed_agent_requests_and_one_use_secret_references() -> Result<()> {
    use application::{Preferences, Theme};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let state = dir.path().join("state");
    let mut config = options(&state);
    config.agent_tools = true;
    config.agent_policy.defer_tools = true;
    config.agent_policy.allowed_tools = Some(
        [
            "tool_search",
            "orchestrator_capabilities",
            "send_to_user",
            "request_secret",
        ]
        .map(String::from)
        .into(),
    );
    config.agent_policy.secret_ttl_seconds = Some(3);
    config.claude_agent.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
    let rt = Huggler::open(config.clone()).await?;
    let prefs = Preferences {
        theme: Theme::Dark,
        reduce_motion: true,
    };
    rt.set_preferences(&prefs)?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let project = rt.open_project(&repo)?;
    let thread = launch(&rt, &project, "requester", Provider::ClaudeAgent);
    let sibling = launch(&rt, &project, "sibling", Provider::ClaudeAgent);
    let op = dir.path().join("op-fixture");
    std::fs::write(
        &op,
        "#!/bin/sh\n[ \"$1 $2 $3\" = 'read --no-newline op://Fixture/build/token' ] || exit 1\nprintf synthetic-secret-9876\n",
    )?;
    std::fs::set_permissions(&op, std::fs::Permissions::from_mode(0o700))?;
    for case in ["approved", "denied", "expired"] {
        let run = rt
            .send(
                CommandId(case.into()),
                thread.clone(),
                format!("managed secret {case}"),
            )?
            .run_id
            .unwrap();
        let request = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                if let Some(r) = rt
                    .secret_requests(&thread)
                    .unwrap()
                    .into_iter()
                    .find(|r| r.run == run)
                {
                    break r;
                }
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            rt.answer_secret(&sibling, &request.id, None),
            Err(Error::Forbidden)
        ));
        match case {
            "approved" => {
                rt.approve_onepassword(&thread, &request.id, &op, "op://Fixture/build/token")?
            }
            "denied" => rt.answer_secret(&thread, &request.id, None)?,
            _ => {}
        }
        completed(&rt, &thread, &run).await;
        let response = rt.read(&thread)?.messages.last().unwrap().text.clone();
        if case == "approved" {
            assert!(matches!(
                rt.consume_secret(&ProjectId("different".into()), &response),
                Err(Error::Forbidden)
            ));
            assert_eq!(
                rt.consume_secret(&project.id, &response)?,
                b"synthetic-secret-9876"
            );
            assert!(matches!(
                rt.consume_secret(&project.id, &response),
                Err(Error::Forbidden)
            ));
        } else {
            assert_eq!(response, case);
        }
    }
    assert_eq!(rt.user_notices(&thread)?.len(), 3);
    assert!(rt.user_notices(&sibling)?.is_empty());
    rt.close().await?;
    let rt = Huggler::open(config).await?;
    assert_eq!(rt.preferences()?, prefs);
    assert_eq!(
        rt.secret_requests(&thread)?
            .iter()
            .map(|r| r.state.as_str())
            .collect::<Vec<_>>(),
        ["consumed", "denied", "expired"]
    );
    for file in std::fs::read_dir(state)? {
        let path = file?.path();
        if path.is_file() {
            assert!(
                !std::fs::read(path)?
                    .windows(b"synthetic-secret-9876".len())
                    .any(|w| w == b"synthetic-secret-9876")
            );
        }
    }
    rt.close().await
}

#[cfg(feature = "application")]
#[tokio::test]
async fn project_services_and_ordered_watch_delivery() -> Result<()> {
    use axum::{
        Json, Router,
        body::Bytes,
        http::{HeaderMap, Method, StatusCode, Uri},
    };
    use repository::Driver::*;
    use serde_json::Value;
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/repository-services.json"))?;
    let pending = std::sync::Arc::new(std::sync::Mutex::new(cases));
    let requests = pending.clone();
    let router = Router::new().fallback(
        move |uri: Uri, method: Method, headers: HeaderMap, body: Bytes| {
            let requests = requests.clone();
            async move {
                if headers["authorization"] != "Bearer recorded" {
                    return (StatusCode::UNAUTHORIZED, Json(Value::Null));
                }
                let payload: Value = if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                };
                let query = payload["query"].as_str().unwrap_or_default();
                let path = uri.to_string();
                let mut remaining = requests.lock().unwrap();
                let index = remaining
                    .iter()
                    .position(|case| {
                        case["path"] == path
                            && method.as_str() == case["method"].as_str().unwrap_or("GET")
                            && case["operation"].as_str().is_none_or(|operation| {
                                query.starts_with(operation)
                                    && payload["variables"] == case["variables"]
                            })
                    })
                    .expect("unexpected or repeated fixture request");
                let case = remaining.swap_remove(index);
                if let Some(fields) = case["fields"].as_str() {
                    assert!(query.contains(fields));
                } else {
                    assert_eq!(payload, case["body"]);
                }
                (StatusCode::OK, Json(case["response"].clone()))
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = tempfile::tempdir()?;
    let config = options(&dir.path().join("state"));
    let rt = Huggler::open(config.clone()).await?;
    let root = dir.path().join("repo");
    checkout(&root);
    let project = rt.open_project(&root)?;
    let owner = launch(&rt, &project, "owner", Provider::Codex);
    let other = rt.create_scratch(&dir.path().join("other"))?;
    let outsider = launch(&rt, &other, "outsider", Provider::Codex);
    let mut watch = None;
    let mut chosen_link = None;
    for driver in [
        GitHub,
        GitLab,
        Bitbucket,
        AzureDevOps,
        Forgejo,
        GitCafe,
        CursorOrigin,
        Linear,
    ] {
        let repo = match driver {
            AzureDevOps => "org/acme/sdk",
            Linear => "SDK",
            _ => "acme/sdk",
        };
        let b = rt.bind_repository(&project.id, driver, &origin, repo)?;
        assert_eq!(b, rt.bind_repository(&project.id, driver, &origin, repo)?);
        assert!(matches!(
            rt.repository_client(&project.id, &b.id, None),
            Err(Error::LoginRequired)
        ));
        let client = rt.repository_client(&project.id, &b.id, Some("Bearer recorded"))?;
        let list = client.list().await?;
        assert_eq!(list.len(), 1);
        let key = if driver == Linear { "SDK-7" } else { "7" };
        assert_eq!(list[0].key, key);
        assert_eq!(list[0].title, "Review");
        assert!(!list[0].state.is_empty());
        let link = rt.link_repository_item(&owner, &b.id, key)?;
        assert!(matches!(
            rt.link_repository_item(&outsider, &b.id, key),
            Err(Error::Forbidden)
        ));
        if matches!(driver, GitHub | GitLab | Bitbucket | Forgejo) {
            assert_eq!(client.comment(7, "Approved by fixture").await?, 809);
        } else {
            assert!(matches!(
                client.comment(7, "denied").await,
                Err(Error::Unsupported(_))
            ));
        }
        if driver == Linear {
            client.comment_on_issue(key, "Approved by fixture").await?;
            assert!(matches!(
                client.comment_on_issue("SDK-8", "moved issue").await,
                Err(Error::Forbidden)
            ));
        } else {
            assert!(matches!(
                client.comment_on_issue(key, "denied").await,
                Err(Error::Unsupported(_))
            ));
        }
        if driver == GitHub {
            assert!(matches!(
                rt.repository_client(&other.id, &b.id, Some("Bearer recorded")),
                Err(Error::Forbidden)
            ));
            assert!(matches!(
                rt.repository_client(&project.id, &b.id, Some("expired"))?
                    .list()
                    .await,
                Err(Error::LoginRequired)
            ));
            assert_eq!(
                client.checks(&"a".repeat(40)).await?[0]["conclusion"],
                "success"
            );
            let w = rt.watch_repository_item(&owner, &link)?;
            assert_eq!(rt.sync_repository_watch(&w, 1, "open")?, None);
            watch = Some(w);
            chosen_link = Some(link);
        } else {
            assert!(matches!(
                client.checks(&"a".repeat(40)).await,
                Err(Error::Unsupported(_))
            ));
            assert!(rt.unlink_repository_item(&owner, &link)?);
        }
    }
    assert!(pending.lock().unwrap().is_empty());
    rt.close().await?;
    let rt = Huggler::open(config).await?;
    let w = watch.unwrap();
    let link = chosen_link.unwrap();
    assert_eq!(rt.repository_links(&owner)?[0].link, link);
    assert_eq!(rt.sync_repository_watch(&w, 2, "open")?, None);
    let run = rt.sync_repository_watch(&w, 3, "merged")?.unwrap();
    completed(&rt, &owner, &run).await;
    assert_eq!(rt.sync_repository_watch(&w, 2, "closed")?, None);
    assert_eq!(rt.sync_repository_watch(&w, 3, "merged")?, None);
    assert_eq!(rt.read(&owner)?.runs.len(), 1);
    assert!(!rt.unwatch_repository_item(&outsider, &w)?);
    assert!(rt.unlink_repository_item(&owner, &link)?);
    assert!(rt.repository_links(&owner)?.is_empty());
    assert!(matches!(
        rt.sync_repository_watch(&w, 4, "closed"),
        Err(Error::Forbidden)
    ));
    server.abort();
    rt.close().await
}

#[tokio::test]
async fn github_pull_request_updates_are_bound_and_revalidate_the_head() -> Result<()> {
    use axum::{Json, Router, http::HeaderMap, routing::post};
    use repository::{
        Driver,
        github::{MergeMethod, PullRequestUpdate},
    };
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    let ready = json!({
        "id": "pr-node", "number": 7, "url": "https://github.com/base/sdk/pull/7",
        "state": "OPEN", "isDraft": false, "baseRefName": "main", "headRefOid": "a".repeat(40),
        "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN", "canBeRebased": true,
        "viewerCanEnableAutoMerge": false, "isMergeQueueEnabled": false,
        "isCrossRepository": true, "headRepository": {"nameWithOwner": "acme/sdk"},
        "repository": {
            "nameWithOwner": "base/sdk", "viewerPermission": "WRITE", "mergeCommitAllowed": false,
            "squashMergeAllowed": true, "rebaseMergeAllowed": true,
        },
        "commits": {"nodes": [{"commit": {"statusCheckRollup": null}}]},
    });
    let mut moved = ready.clone();
    moved["headRefOid"] = json!("b".repeat(40));
    let mut queue_enabled = ready.clone();
    queue_enabled["isMergeQueueEnabled"] = json!(true);
    let observations = Arc::new(Mutex::new(VecDeque::from([
        ready.clone(),
        moved,
        queue_enabled,
        ready,
    ])));
    let remaining = observations.clone();
    let merges = Arc::new(Mutex::new(0));
    let writes = merges.clone();
    let router = Router::new().route("/graphql", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let observations = observations.clone();
        let merges = merges.clone();
        async move {
            assert_eq!(headers["authorization"], "Bearer recorded");
            let query = body["query"].as_str().unwrap();
            if query.trim_start().starts_with("query BranchPullRequests") {
                assert_eq!(body["variables"], json!({
                    "owner": "acme", "repo": "sdk", "head": "review", "ref": "refs/heads/review",
                }));
                let pr = observations.lock().unwrap().pop_front().expect("unexpected PR read");
                Json(json!({"data": {"repository": {
                    "ref": {"associatedPullRequests": {"nodes": [pr]}, "recent": {"nodes": []}},
                    "recent": {"nodes": []},
                }}}))
            } else {
                assert!(query.contains("mergePullRequest(input:"));
                assert!(query.contains("expectedHeadOid: $head"));
                assert_eq!(body["variables"], json!({"id": "pr-node", "head": "a".repeat(40), "method": "SQUASH"}));
                *merges.lock().unwrap() += 1;
                Json(json!({"data": {"mergePullRequest": {"pullRequest": {"id": "pr-node"}}}}))
            }
        }
    }));
    let (endpoint, server) = serve_router(router).await;
    let directory = tempfile::tempdir()?;
    let rt = Huggler::open(options(&directory.path().join("state"))).await?;
    let root = directory.path().join("repo");
    checkout(&root);
    let project = rt.open_project(&root)?;
    let thread = launch(&rt, &project, "review", Provider::Codex);
    let origin = format!("http://{endpoint}");
    let binding = rt.bind_thread_repository(&thread, Driver::GitHub, &origin, "acme/sdk")?;
    assert_eq!(binding.project, project.id);
    let client = rt.repository_client(&binding.project, &binding.id, Some("Bearer recorded"))?;
    let other = rt.bind_repository(&project.id, Driver::GitHub, &origin, "other/sdk")?;
    let other_client = rt.repository_client(&other.project, &other.id, Some("Bearer recorded"))?;

    let pr = client.github_pull_requests("review").await?.open.remove(0);
    assert_eq!(pr.repository(), "base/sdk");
    assert!(pr.can_merge_now());
    let merge = PullRequestUpdate::Merge(MergeMethod::Squash);
    assert!(matches!(
        other_client.update_github_pull_request(&pr, merge).await,
        Err(Error::Forbidden)
    ));
    for _ in 0..2 {
        assert!(matches!(
            client.update_github_pull_request(&pr, merge).await,
            Err(Error::Invalid(_))
        ));
    }
    client.update_github_pull_request(&pr, merge).await?;
    assert!(remaining.lock().unwrap().is_empty());
    assert_eq!(*writes.lock().unwrap(), 1);
    server.abort();
    rt.close().await
}
