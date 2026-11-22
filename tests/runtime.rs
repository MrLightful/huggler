#![cfg(unix)]
use huggler::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command as Process,
    sync::{Arc, Barrier},
    time::Duration,
};
use tempfile::tempdir;
use tokio::time::timeout;

fn checkout(dir: &Path) {
    assert!(
        Process::new("git")
            .args(["init", "-q"])
            .arg(dir)
            .status()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn custom_database_preserves_ownership_and_history_across_state_roots() -> Result<()> {
    let temp = tempdir()?;
    let mut opts = HugglerOptions::new(temp.path().join("state"));
    opts.storage.database_path = "sessions/conversations.db".into();
    let rt = Huggler::open(opts.clone()).await?;
    let database = opts.state_dir.join(&opts.storage.database_path);
    assert!(database.is_file());
    assert!(
        database
            .with_file_name("conversations.db.runtime.lock")
            .is_file()
    );
    assert!(!opts.state_dir.join("huggler.sqlite3").exists());
    assert!(!opts.state_dir.join("runtime.lock").exists());

    let repo = temp.path().join("repo");
    checkout(&repo);
    let project = rt.open_project(&repo)?;
    let thread = ThreadId("saved".into());
    rt.launch(
        CommandId("launch".into()),
        thread.clone(),
        project.id,
        "Saved".into(),
        None,
    )?;
    let alias = temp.path().join("linked database");
    std::os::unix::fs::symlink(database, &alias)?;
    let mut reopen = HugglerOptions::new(temp.path().join("other state"));
    reopen.storage.database_path = alias;
    assert!(matches!(
        Huggler::open(reopen.clone()).await,
        Err(Error::AlreadyOpen)
    ));
    rt.close().await?;
    let rt = Huggler::open(reopen).await?;
    assert_eq!(rt.read(&thread)?.name, "Saved");
    rt.close().await
}

#[tokio::test]
async fn invalid_storage_names_are_rejected() -> Result<()> {
    let temp = tempdir()?;
    let mut opts = HugglerOptions::new(temp.path());
    for name in ["fixed", "../{id}", "nested/{id}", "{id}/"] {
        opts.storage.worktree_name = name.into();
        assert!(matches!(
            Huggler::open(opts.clone()).await,
            Err(Error::Invalid(_))
        ));
    }
    opts.storage = Default::default();
    opts.storage.worktree_root = WorktreeRoot::ProjectRelative(temp.path().into());
    assert!(matches!(
        Huggler::open(opts.clone()).await,
        Err(Error::Invalid(_))
    ));
    opts.storage = Default::default();
    for filename in [
        "runtime.lock",
        "other.db-wal",
        "other.db-shm",
        "other.db-journal",
    ] {
        opts.storage.database_path = filename.into();
        assert!(matches!(
            Huggler::open(opts.clone()).await,
            Err(Error::Invalid(_))
        ));
    }
    assert_eq!(fs::read_dir(temp.path())?.count(), 0);
    Ok(())
}

#[tokio::test]
async fn concurrent_receipts_preserve_the_winner_including_no_ops() {
    let temp = tempdir().unwrap();
    checkout(temp.path());
    let runtime = Arc::new(
        Huggler::open(HugglerOptions::new(temp.path().join("state")))
            .await
            .unwrap(),
    );
    let project = runtime.open_project(temp.path()).unwrap();
    let threads = [ThreadId("alpha".into()), ThreadId("beta".into())];
    for thread in &threads {
        runtime
            .launch(
                CommandId(format!("launch-{thread}")),
                thread.clone(),
                project.id.clone(),
                thread.to_string(),
                None,
            )
            .unwrap();
    }
    // The first race records an empty receipt, the second changes only the winner.
    for model in [None, Some("selected".to_string())] {
        let barrier = Arc::new(Barrier::new(2));
        let jobs = threads.clone().map(|id| {
            let barrier = barrier.clone();
            let runtime = runtime.clone();
            let command = Command {
                id: CommandId(format!("race-{model:?}")),
                thread_id: id,
                action: Action::SetModel {
                    model: model.clone(),
                },
            };
            std::thread::spawn(move || {
                barrier.wait();
                let result = runtime.dispatch(command.clone());
                (command, result)
            })
        });
        let outcomes = jobs.map(|job| job.join().unwrap());
        let (winner, receipt) = outcomes
            .iter()
            .find_map(|(cmd, res)| res.as_ref().ok().map(|receipt| (cmd, receipt)))
            .unwrap();
        assert_eq!(receipt.event_sequences.len(), usize::from(model.is_some()));
        let (loser, error) = outcomes
            .iter()
            .find_map(|(cmd, res)| res.as_ref().err().map(|err| (cmd, err)))
            .unwrap();
        assert!(
            matches!(error,Error::Conflict{original_thread,requested_thread,..}
            if original_thread==&winner.thread_id && requested_thread==&loser.thread_id)
        );
        assert_eq!(&runtime.dispatch(winner.clone()).unwrap(), receipt);
        assert_eq!(runtime.read(&winner.thread_id).unwrap().model, model);
        assert_eq!(runtime.read(&loser.thread_id).unwrap().model, None);
        let changed = Command {
            action: Action::Send {
                text: "different intent".into(),
            },
            ..winner.clone()
        };
        assert!(matches!(
            runtime.dispatch(changed),
            Err(Error::Conflict { .. })
        ));
    }
    runtime.close().await.unwrap();
}

async fn start_stalled(dir: &Path) -> (Huggler, Receipt) {
    checkout(dir);
    let shim = dir.join("codex");
    fs::write(&shim, include_str!("fixtures/stalled-codex.py")).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = HugglerOptions::new(dir.join("state"));
    options.codex.executable = shim;
    let rt = Huggler::open(options).await.unwrap();
    let project = rt.open_project(dir).unwrap();
    let id = ThreadId("active".into());
    rt.launch(
        CommandId("launch".into()),
        id.clone(),
        project.id,
        "Interrupted".into(),
        None,
    )
    .unwrap();
    let (_, mut stream) = rt.subscribe(&id, None).unwrap();
    let sent = rt
        .send(CommandId("send".into()), id, "one prompt".into())
        .unwrap();
    timeout(Duration::from_secs(10),async {
        while !matches!(stream.next().await.unwrap().change,Change::Run{run} if run.status==RunStatus::Running) {}
    }).await.unwrap();
    (rt, sent)
}

#[test]
fn dropping_the_executor_stops_the_provider_helper() {
    let dir = tempdir().unwrap();
    let lease = fs::File::create(dir.path().join("model.lock")).unwrap();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let (_rt, _) = start_stalled(dir.path()).await;
        assert!(matches!(
            lease.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
    });
    drop(executor);
    wait_for_helper_exit(&lease);
}

fn wait_for_helper_exit(lease: &fs::File) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match lease.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("provider helper retained its lock: {err}"),
        }
    }
}

#[tokio::test]
async fn protocol_failure_releases_the_provider_helpers_lock() {
    let dir = tempdir().unwrap();
    let lease = fs::File::create(dir.path().join("model.lock")).unwrap();
    fs::write(dir.path().join("invalid-frame"), "").unwrap();
    let (rt, receipt) = start_stalled(dir.path()).await;
    let run = rt
        .wait(&receipt.thread_id, receipt.run_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.error.unwrap().contains("invalid JSON-RPC frame"));
    wait_for_helper_exit(&lease);
    rt.close().await.unwrap();
}

#[tokio::test]
async fn close_reaps_an_active_child_with_observers_still_attached() {
    let dir = tempdir().unwrap();
    let lease = fs::File::create(dir.path().join("model.lock")).unwrap();
    let (rt, receipt) = start_stalled(dir.path()).await;
    assert!(matches!(
        rt.send(
            CommandId("busy".into()),
            receipt.thread_id.clone(),
            "too soon".into()
        ),
        Err(Error::Busy)
    ));
    let (_, mut stream) = rt.subscribe(&receipt.thread_id, None).unwrap();
    let pid: i32 = fs::read_to_string(dir.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    let (a, b, observed) = tokio::join!(rt.close(), rt.close(), stream.next());
    a.unwrap();
    b.unwrap();
    assert!(matches!(observed, Err(Error::Closed)));
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    // The provider's helper inherited the lock and lives in its process group.
    wait_for_helper_exit(&lease);
    let rt = Huggler::open(HugglerOptions::new(dir.path().join("state")))
        .await
        .unwrap();
    assert_eq!(
        rt.read(&receipt.thread_id).unwrap().runs[0].status,
        RunStatus::Failed
    );
    rt.close().await.unwrap();
}

#[tokio::test]
#[ignore = "subprocess entry point for process_loss_recovers_a_claimed_turn_without_reissuing_it"]
async fn crash_child() {
    let Some(dir) = std::env::var_os("HUGGLER_CRASH_CASE") else {
        return;
    };
    let (_rt, receipt) = start_stalled(dir.as_ref()).await;
    fs::write(
        Path::new(&dir).join("receipt"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    std::process::exit(73);
}

#[tokio::test]
async fn process_loss_recovers_a_claimed_turn_without_reissuing_it() {
    let dir = tempdir().unwrap();
    let exit = Process::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "crash_child"])
        .env("HUGGLER_CRASH_CASE", dir.path())
        .status()
        .unwrap();
    assert_eq!(exit.code(), Some(73));
    let original: Receipt =
        serde_json::from_slice(&fs::read(dir.path().join("receipt")).unwrap()).unwrap();
    let mut options = HugglerOptions::new(dir.path().join("state"));
    options.codex.executable = dir.path().join("codex");
    let rt = Huggler::open(options).await.unwrap();
    let state = rt.read(&original.thread_id).unwrap();
    assert_eq!(
        state.native_thread,
        Some(NativeThreadId("persisted".into()))
    );
    assert_eq!(state.runs.len(), 1);
    assert_eq!(state.runs[0].status, RunStatus::Failed);
    assert!(
        state.runs[0]
            .error
            .as_deref()
            .unwrap()
            .contains("delivery is uncertain")
    );
    assert_eq!(
        state.runs[0].native_turn,
        Some(NativeTurnId("accepted".into()))
    );
    assert_eq!(
        rt.send(
            original.command_id.clone(),
            original.thread_id.clone(),
            "one prompt".into()
        )
        .unwrap(),
        original
    );
    rt.close().await.unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("invocations")).unwrap(),
        "turn/start\n"
    );
}

#[tokio::test]
async fn bounded_history_and_retained_replay_match_the_reopened_snapshot() -> Result<()> {
    let temp = tempdir()?;
    let (rt, first) = start_stalled(temp.path()).await;
    let thread = &first.thread_id;
    let (_, mut lagged) = rt.subscribe(thread, None)?;
    let command = |id: String, action| Command {
        id: CommandId(id),
        thread_id: thread.clone(),
        action,
    };
    // Place a multibyte scalar at the preview boundary and a literal search hit after NUL.
    let huge = format!("{}🦀\0{}_% HIT", "a".repeat(4095), "b".repeat(300_000));
    for i in 0..69 {
        rt.dispatch(command(
            format!("q-{i}"),
            Action::Enqueue {
                text: if i == 12 {
                    huge.clone()
                } else {
                    format!("q{i}")
                },
            },
        ))?;
    }
    let with_file = rt.dispatch(command(
        "q-69".into(),
        Action::EnqueueAttachments {
            text: huge.clone(),
            files: vec![TextAttachment {
                name: "empty.txt".into(),
                content: String::new(),
            }],
        },
    ))?;
    let attached = rt
        .detail(thread, None, "", 1)?
        .history
        .messages
        .pop()
        .unwrap();
    let files = attached.files.unwrap();
    assert!(files.truncated);
    assert_eq!(files.caption.len(), 4095);
    let file_message = with_file.message_id.as_ref().unwrap();
    assert_eq!(rt.read_message_body(thread, file_message)?, huge);
    assert_eq!(rt.read_attachment(thread, file_message, 0)?, "");
    rt.stop_thread(CommandId("stop".into()), thread.clone())?;
    rt.wait(thread, first.run_id.as_ref().unwrap()).await?;
    let page = rt.detail(thread, None, "", 3)?.history;
    let old = page.older.unwrap();
    assert!(matches!(
        rt.detail(thread, Some(&old), "q", 3),
        Err(Error::Invalid(_))
    ));
    let snapshot = rt.read(thread)?;
    let edited = command(
        "edit".into(),
        Action::EditQueued {
            run: snapshot.runs.last().unwrap().id.clone(),
            text: "edited".into(),
        },
    );
    let receipt = rt.dispatch(edited.clone())?;
    assert!(
        rt.detail(thread, None, "", 1)?.history.messages[0]
            .files
            .is_none()
    );
    assert!(matches!(
        rt.read_attachment(thread, file_message, 0),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        rt.detail(thread, Some(&old), "", 3),
        Err(Error::HistoryChanged)
    ));
    let changed = rt.wait_for_change(thread, snapshot.cursor).await?;
    let (_, mut retained) = rt.subscribe(thread, Some(snapshot.cursor))?;
    rt.retain_events(thread, 1)?;
    assert!(matches!(lagged.next().await, Err(Error::CursorExpired)));
    let event = retained.next().await?;
    assert_eq!(event.sequence, changed);
    let mut rebuilt = snapshot.messages;
    let Change::Message { message } = event.change else {
        panic!("expected edit")
    };
    *rebuilt.iter_mut().find(|m| m.id == message.id).unwrap() = message.clone();
    rt.close().await?;
    let rt = Huggler::open(HugglerOptions::new(temp.path().join("state"))).await?;
    assert_eq!(rt.dispatch(edited)?, receipt);
    assert!(matches!(
        rt.subscribe(thread, Some(0)),
        Err(Error::CursorExpired)
    ));
    let authoritative = rt.read(thread)?;
    assert_eq!(rebuilt, authoritative.messages);
    let found = rt.detail(thread, None, "_% hit", 1)?.history.messages;
    assert_eq!(found.len(), 1);
    assert!(found[0].truncated);
    assert_eq!(found[0].message.text.len(), 4095);
    assert_eq!(rt.read_message(thread, &found[0].message.id)?.text, huge);
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let page = rt.detail(thread, cursor.as_ref(), "", 9)?.history;
        assert_eq!(page.sequence, authoritative.cursor);
        pages.push(
            page.messages
                .into_iter()
                .map(|item| {
                    if item.truncated {
                        rt.read_message(thread, &item.message.id).unwrap()
                    } else {
                        item.message
                    }
                })
                .collect::<Vec<_>>(),
        );
        cursor = page.older;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        pages.into_iter().rev().flatten().collect::<Vec<_>>(),
        authoritative.messages
    );
    rt.close().await
}

#[tokio::test]
async fn conversation_list_reports_human_requests_and_replays_settlement_across_reopen()
-> Result<()> {
    timeout(Duration::from_secs(15), async {
        let temp = tempdir()?;
        let repo = temp.path().join("repo");
        checkout(&repo);
        let mut options = HugglerOptions::new(temp.path().join("state"));
        options.codex.executable =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/context-provider.py");
        options.codex.request_approvals = true;
        let runtime = Huggler::open(options.clone()).await?;
        let project = runtime.open_project(&repo)?;
        let thread = ThreadId("review".into());
        let sibling = ThreadId("other".into());
        for id in [&thread, &sibling] {
            runtime.launch(
                CommandId(format!("launch-{id}")),
                id.clone(),
                project.id.clone(),
                id.to_string(),
                None,
            )?;
        }
        let initial_list = runtime.conversation_list()?;
        assert_eq!(initial_list.threads[0].id, sibling);
        let (_, mut events) = runtime.subscribe(&thread, None)?;
        let receipt = runtime.send(CommandId("ask".into()), thread.clone(), "callbacks".into())?;
        let run = receipt.run_id.unwrap();
        let (mut permission, mut question) = (None, None);
        let (permission, question) = loop {
            match events.next().await?.change {
                Change::PermissionRequested { request } => {
                    if let Some(question) = question {
                        break (request, question);
                    }
                    permission = Some(request);
                }
                Change::QuestionRequested { request } => {
                    if let Some(permission) = permission {
                        break (permission, request);
                    }
                    question = Some(request);
                }
                _ => {}
            }
        };
        let pending = runtime
            .wait_for_conversation_list(initial_list.revision)
            .await?;
        assert_eq!(pending.threads[0].id, thread);
        assert!(pending.threads[0].needs_answer);
        assert!(pending.threads[0].needs_approval);
        assert_eq!(pending.threads[0].completed_run, None);
        assert_eq!(pending.threads[1], initial_list.threads[0]);
        assert_eq!(
            runtime
                .detail(&thread, None, "", 1)?
                .foreground_run
                .unwrap()
                .id,
            run
        );

        let settle = Command {
            id: CommandId("settle".into()),
            thread_id: thread.clone(),
            action: Action::SetSettled { settled: true },
        };
        assert!(matches!(runtime.dispatch(settle.clone()), Err(Error::Busy)));
        assert_eq!(runtime.conversation_list()?, pending);
        runtime.respond_to_permission(&thread, &run, &permission.id, "accept".into())?;
        let approved = runtime.wait_for_conversation_list(pending.revision).await?;
        assert!(!approved.threads[0].needs_approval);
        assert!(approved.threads[0].needs_answer);
        assert_eq!(approved.threads[0].completed_run, None);
        let answers = question
            .questions
            .iter()
            .map(|question| {
                (
                    question.id.clone(),
                    UserAnswer::Text("Use the fixture".into()),
                )
            })
            .collect();
        runtime
            .respond_to_question(&thread, &run, &question.id, answers)
            .await?;
        assert_eq!(
            runtime.wait(&thread, &run).await?.status,
            RunStatus::Completed
        );
        let completed = runtime
            .wait_for_conversation_list(approved.revision)
            .await?;
        assert!(!completed.threads[0].needs_answer);
        assert_eq!(completed.threads[0].completed_run.as_ref(), Some(&run));

        let settled = runtime.dispatch(settle.clone())?;
        let settled_list = runtime
            .wait_for_conversation_list(completed.revision)
            .await?;
        assert!(settled_list.threads[0].settled);
        let cursor = runtime.read(&thread)?.cursor;
        runtime.close().await?;

        let runtime = Huggler::open(options).await?;
        let (snapshot, mut events) = runtime.subscribe(&thread, Some(cursor))?;
        assert!(snapshot.settled);
        assert_eq!(runtime.dispatch(settle)?, settled);
        assert_eq!(runtime.conversation_list()?, settled_list);
        let follow_up = runtime.send(
            CommandId("continue".into()),
            thread.clone(),
            "Continue".into(),
        )?;
        let reopened = events.next().await?;
        assert_eq!(Some(&reopened.sequence), follow_up.event_sequences.first());
        assert!(matches!(
            reopened.change,
            Change::SettlementChanged { settled: false }
        ));
        let follow_up = follow_up.run_id.unwrap();
        assert_eq!(
            runtime.wait(&thread, &follow_up).await?.status,
            RunStatus::Completed
        );
        let list = runtime
            .wait_for_conversation_list(settled_list.revision)
            .await?;
        assert!(!list.threads[0].settled);
        assert_eq!(list.threads[0].completed_run.as_ref(), Some(&follow_up));
        assert_eq!(list.threads[1], initial_list.threads[0]);
        runtime.close().await
    })
    .await
    .expect("conversation lifecycle should complete")
}
