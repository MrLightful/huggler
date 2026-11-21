#![cfg(unix)]
use huggler::*;
use serde_json::Value;
use std::{fs, path::Path};
#[path = "support/agents.rs"]
mod agents;
use agents::*;
fn options(state: &Path) -> HugglerOptions {
    let mut opts = HugglerOptions::new(state);
    let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/context-provider.py");
    opts.codex.executable = exe.clone();
    opts.claude_agent.executable = exe.clone();
    opts.agent_tools = false;
    opts.instances
        .push(ProviderInstance::new("alternate", Provider::Codex, exe));
    opts
}
fn command(id: &str, thread: &ThreadId, action: Action) -> Command {
    Command {
        id: CommandId(id.into()),
        thread_id: thread.clone(),
        action,
    }
}
fn frames(repo: &Path, method: &str) -> Vec<Value> {
    fs::read_to_string(repo.join("frames.ndjson"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|f| f["method"] == method)
        .map(|f| f["params"].clone())
        .collect()
}

#[tokio::test]
async fn forks_and_merge_use_selected_boundary_and_instance() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    fs::write(repo.join("dirty"), "keep me")?;
    let opts = options(&dir.path().join("state"));
    let rt = Huggler::open(opts.clone()).await?;
    let root = launch(&rt, &rt.open_project(&repo)?, "root", Provider::Codex);
    let alpha = rt
        .send(
            CommandId("alpha".into()),
            root.clone(),
            format!("alpha {}", "é".repeat(6000)),
        )?
        .run_id
        .unwrap();
    completed(&rt, &root, &alpha).await;
    let beta = rt
        .send(CommandId("beta".into()), root.clone(), "later beta".into())?
        .run_id
        .unwrap();
    completed(&rt, &root, &beta).await;
    let fork = |name: &str, instance: &str, mode| {
        command(
            name,
            &ThreadId(name.into()),
            Action::Fork {
                source: root.clone(),
                through: alpha.clone(),
                name: name.into(),
                instance: instance.into(),
                model: None,
                mode,
                text: "first fork prompt".into(),
            },
        )
    };
    assert!(matches!(
        rt.dispatch(fork("denied", "alternate", ForkMode::Native)),
        Err(Error::Unsupported(_))
    ));
    let native = fork("native", "codex", ForkMode::Native);
    let receipt = rt.dispatch(native.clone())?;
    completed(&rt, &native.thread_id, receipt.run_id.as_ref().unwrap()).await;
    let parent = rt.read(&root)?;
    let wire = frames(&repo, "thread/fork").pop().unwrap();
    assert_eq!(
        wire["lastTurnId"],
        parent.runs[0].native_turn.as_ref().unwrap().0
    );
    assert_eq!(wire["threadId"], parent.native_thread.as_ref().unwrap().0);
    assert_eq!(
        frames(&repo, "turn/start").last().unwrap()["input"][0]["text"],
        "first fork prompt"
    );
    fs::write(repo.join("bad-boundary"), "")?;
    let bad = fork("bad", "codex", ForkMode::Native);
    let rejected = rt.dispatch(bad.clone())?.run_id.unwrap();
    assert_eq!(
        rt.wait(&bad.thread_id, &rejected).await?.status,
        RunStatus::Failed
    );
    assert_eq!(frames(&repo, "turn/start").len(), 3);
    for (name, instance, method) in [
        ("other", "alternate", "turn/start"),
        ("handoff", "claudeAgent", "session/prompt"),
    ] {
        let cmd = fork(name, instance, ForkMode::Portable);
        let res = rt.dispatch(cmd.clone())?;
        completed(&rt, &cmd.thread_id, res.run_id.as_ref().unwrap()).await;
        assert_ne!(rt.read(&cmd.thread_id)?.native_thread, parent.native_thread);
        let history = rt.fork_history(&cmd.thread_id)?.unwrap();
        assert_eq!(history.through, alpha);
        assert!(history.truncated);
        assert_eq!(history.messages[0].id, parent.messages[0].id);
        let input = frames(&repo, method).pop().unwrap();
        let text = input[if method == "turn/start" {
            "input"
        } else {
            "prompt"
        }][0]["text"]
            .as_str()
            .unwrap();
        assert!(
            text.contains(&serde_json::to_string(&history)?) && text.ends_with("first fork prompt")
        );
        assert!(!text.contains("later beta"));
        assert!(text.len() < 17000);
        if name == "handoff" {
            let merge = Action::MergeBack {
                source: cmd.thread_id.clone(),
                through: res.run_id.unwrap(),
                text: "integrate".into(),
            };
            assert!(matches!(
                rt.dispatch(command("merge", &native.thread_id, merge.clone())),
                Err(Error::Forbidden)
            ));
            let merged = rt.dispatch(command("merge", &root, merge))?.run_id.unwrap();
            completed(&rt, &root, &merged).await;
            let input = frames(&repo, "turn/start").pop().unwrap();
            assert_eq!(input["threadId"], parent.native_thread.as_ref().unwrap().0);
            let text = input["input"][0]["text"].as_str().unwrap();
            assert!(text.contains("portable reply") && text.ends_with("integrate"));
        }
    }
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    assert_eq!(rt.dispatch(native.clone())?, receipt);
    assert_eq!(rt.fork_history(&native.thread_id)?.unwrap().through, alpha);
    assert_eq!(fs::read_to_string(repo.join("dirty"))?, "keep me");
    rt.close().await
}

#[tokio::test]
async fn attachments_are_claimed_once_and_answers_reach_the_owning_callback() -> Result<()> {
    use serde_json::json;
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let opts = options(&dir.path().join("state"));
    let rt = Huggler::open(opts.clone()).await?;
    let project = rt.open_project(&repo)?;
    let owner = launch(&rt, &project, "owner", Provider::Codex);
    let peer = launch(&rt, &project, "peer", Provider::Codex);
    let upload = |name: &str| rt.prepare_upload(&owner, name, "text/plain", name.as_bytes());
    assert!(matches!(
        rt.prepare_upload(&owner, "blob", "image/png", b""),
        Err(Error::Invalid(_))
    ));
    let discarded = upload("discard")?;
    rt.discard_upload(&owner, &discarded)?;
    rt.discard_upload(&owner, &discarded)?;
    let file = upload("project.txt")?;
    let send = |thread: &ThreadId, uploads| {
        command(
            "attach",
            thread,
            Action::SendAttachments {
                text: "read".into(),
                uploads,
            },
        )
    };
    for ids in [
        vec![file.clone(), discarded],
        vec![file.clone(); 2],
        vec![file.clone(); 9],
    ] {
        assert!(matches!(
            rt.dispatch(send(&owner, ids)),
            Err(Error::Invalid(_))
        ));
    }
    assert!(rt.dispatch(send(&peer, vec![file.clone()])).is_err());
    let cmd = send(&owner, vec![file]);
    let receipt = rt.dispatch(cmd.clone())?;
    completed(&rt, &owner, receipt.run_id.as_ref().unwrap()).await;
    assert_eq!(
        frames(&repo, "turn/start")[0]["input"][0]["text"],
        "read\n\nText attachment \"project.txt\"\nproject.txt"
    );
    for dismiss in [false, true] {
        let (_, mut stream) = rt.subscribe(&owner, None)?;
        let run = rt
            .send(
                CommandId(format!("ask-{dismiss}")),
                owner.clone(),
                "ask".into(),
            )?
            .run_id
            .unwrap();
        let request = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Change::QuestionRequested { request } = stream.next().await.unwrap().change {
                    break request;
                }
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            rt.dismiss_question(&peer, &run, &request.id).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            rt.dismiss_question(&owner, &RunId("wrong".into()), &request.id)
                .await,
            Err(Error::NotFound(_))
        ));
        if dismiss {
            rt.dismiss_question(&owner, &run, &request.id).await?;
        } else {
            let answers = UserAnswers::from([
                ("one".into(), UserAnswer::Text("first".into())),
                ("two".into(), UserAnswer::Choices(vec!["second".into()])),
            ]);
            let file = upload("question.txt")?;
            for ids in [vec![file.clone(); 2], vec![file.clone(); 9]] {
                assert!(matches!(
                    rt.respond_with_attachments(
                        &owner,
                        &run,
                        &request.id,
                        answers.clone(),
                        &QuestionAttachments::from([("two".into(), ids)])
                    )
                    .await,
                    Err(Error::Invalid(_))
                ));
            }
            rt.respond_with_attachments(
                &owner,
                &run,
                &request.id,
                answers,
                &QuestionAttachments::from([("two".into(), vec![file])]),
            )
            .await?;
        }
        assert!(
            matches!(stream.next().await?.change, Change::QuestionClosed{id,answered,..} if id==request.id && answered!=dismiss)
        );
        completed(&rt, &owner, &run).await;
        let retry = rt.dismiss_question(&owner, &run, &request.id).await;
        if dismiss {
            assert!(matches!(retry, Err(Error::Invalid(_))));
        } else {
            assert!(matches!(retry, Err(Error::AlreadyAnswered)));
        }
    }
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    assert_eq!(rt.dispatch(cmd)?, receipt);
    let id = receipt.message_id.as_ref().unwrap();
    let history = rt.detail(&owner, None, "", 32)?.history;
    let files = history
        .messages
        .iter()
        .find(|item| &item.message.id == id)
        .unwrap()
        .files
        .as_ref()
        .unwrap();
    assert_eq!(files.caption, "read");
    assert_eq!(files.names, ["project.txt"]);
    assert_eq!(rt.read_attachment(&owner, id, 0)?, "project.txt");
    assert!(matches!(
        rt.read_attachment(&peer, id, 0),
        Err(Error::NotFound(_))
    ));
    assert_eq!(frames(&repo, "turn/start").len(), 3);
    let replies = fs::read_to_string(repo.join("frames.ndjson"))?
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .filter(|f| f["id"] == "ask" && f.get("method").is_none())
        .map(|f| f["result"]["answers"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        replies,
        [
            json!({"one":{"answers":["first"]},"two":{"answers":["second","Text attachment \"question.txt\"\nquestion.txt"]}}),
            json!({})
        ]
    );
    rt.close().await
}

#[tokio::test]
async fn imports_copy_visible_history_and_continue_after_reopen() -> Result<()> {
    use serde_json::json;
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let mut opts = options(&dir.path().join("state"));
    opts.instances.push(ProviderInstance::new(
        "registry-test",
        Provider::AcpRegistry,
        opts.codex.executable.clone(),
    ));
    let rt = Huggler::open(opts.clone()).await?;
    let project = rt.open_project(&repo)?.id;
    let mut cases = vec![];
    for (name, rows) in [
        (
            "claude",
            vec![
                json!({"type":"user","sessionId":"archived","message":{"content":"old question"}}),
                json!({"type":"assistant","isSidechain":true,"message":{"content":"hidden"}}),
                json!({"type":"assistant","message":{"content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"old answer"}]}}),
            ],
        ),
        (
            "codex",
            vec![
                json!({"type":"session_meta","payload":{"id":"archived"}}),
                json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"old question"}]}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"old question"}}),
                json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"old answer"}]}}),
            ],
        ),
        (
            "pi",
            vec![
                json!({"type":"session","id":"archived"}),
                json!({"type":"message","id":"one","parentId":null,"message":{"role":"user","content":"old question"}}),
                json!({"type":"message","id":"abandoned","parentId":"one","message":{"role":"assistant","content":"hidden"}}),
                json!({"type":"message","id":"chosen","parentId":"one","message":{"role":"assistant","content":"old answer"}}),
            ],
        ),
        ("registry", vec![]),
    ] {
        let file = repo.join(name);
        fs::write(
            &file,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )?;
        let source = match name {
            "claude" => ImportSource::Claude(file.clone()),
            "codex" => ImportSource::Codex(file.clone()),
            "pi" => ImportSource::Pi(file.clone()),
            _ => ImportSource::AcpRegistry(NativeThreadId("archived".into())),
        };
        let thread = ThreadId(name.into());
        let native = name == "registry";
        let instance = ProviderInstanceId::from(if native { "registry-test" } else { "alternate" });
        let result = rt
            .import_history(&project, &thread, name, &instance, source.clone())
            .await?;
        assert_eq!(
            result.native_session,
            Some(NativeThreadId("archived".into()))
        );
        assert_eq!(
            rt.import_history(&project, &thread, name, &instance, source.clone())
                .await?,
            result
        );
        if native {
            assert!(matches!(
                rt.import_history(
                    &project,
                    &ThreadId("alias".into()),
                    name,
                    &instance,
                    source.clone()
                )
                .await,
                Err(Error::Invalid(_))
            ));
            fs::write(repo.join("changed"), "")?
        } else {
            use std::io::Write;
            writeln!(fs::OpenOptions::new().append(true).open(&file)?)?;
        }
        assert!(matches!(
            rt.import_history(&project, &thread, name, &instance, source.clone())
                .await,
            Err(Error::Invalid(_))
        ));
        if native {
            fs::write(repo.join("removed"), "")?
        } else {
            fs::remove_file(file)?;
        }
        assert!(
            rt.import_history(&project, &thread, name, &instance, source)
                .await
                .is_err()
        );
        cases.push((thread, result, native));
    }
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    for (thread, result, native) in cases {
        assert_eq!(rt.import_source(&thread)?, Some(result));
        assert_eq!(
            rt.read(&thread)?
                .messages
                .iter()
                .map(|m| m.text.as_str())
                .collect::<Vec<_>>(),
            ["old question", "old answer"]
        );
        let send = rt.send(
            CommandId(format!("continue-{thread}")),
            thread.clone(),
            "next".into(),
        )?;
        if native {
            assert_eq!(
                rt.wait(&thread, send.run_id.as_ref().unwrap())
                    .await?
                    .status,
                RunStatus::Failed
            );
            assert!(frames(&repo, "session/prompt").is_empty());
            fs::remove_file(repo.join("removed"))?;
            fs::remove_file(repo.join("changed"))?;
            let retry = rt.send(CommandId("restored".into()), thread.clone(), "next".into())?;
            completed(&rt, &thread, retry.run_id.as_ref().unwrap()).await;
            let frames = frames(&repo, "session/prompt");
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0]["sessionId"], "archived");
            assert_eq!(frames[0]["prompt"][0]["text"], "next");
        } else {
            completed(&rt, &thread, send.run_id.as_ref().unwrap()).await;
            let frame = frames(&repo, "turn/start").pop().unwrap();
            let prompt = frame["input"][0]["text"].as_str().unwrap();
            assert!(
                prompt.contains("old question")
                    && prompt.contains("old answer")
                    && prompt.ends_with("next")
            );
            assert!(!prompt.contains("hidden"));
        }
    }
    rt.close().await
}

#[tokio::test]
async fn checkpoint_forks_restore_only_the_destination_checkout() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    fs::write(repo.join(".git/info/exclude"), "frames.ndjson\n")?;
    let commit = |content: &str| {
        fs::write(repo.join("tracked"), content).unwrap();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .current_dir(&repo)
                    .args([
                        "-c",
                        "user.name=Test",
                        "-c",
                        "user.email=test@example.test",
                        "-c",
                        "commit.gpgsign=false"
                    ])
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            )
        };
        git(&["add", "tracked"]);
        git(&["commit", "-qm", content]);
    };
    let mut opts = options(&dir.path().join("state"));
    opts.storage.worktree_root = WorktreeRoot::Shared("host checkouts".into());
    let rt = Huggler::open(opts.clone()).await?;
    let project = rt.open_project(&repo)?;
    let root = launch(&rt, &project, "root", Provider::Codex);
    let peer = launch(&rt, &project, "peer", Provider::Codex);
    let mut captures = Vec::new();
    for content in ["alpha", "beta"] {
        commit(content);
        let run = rt
            .send(CommandId(content.into()), root.clone(), content.into())?
            .run_id
            .unwrap();
        completed(&rt, &root, &run).await;
        let cmd = command(
            &format!("capture-{content}"),
            &root,
            Action::CaptureCheckpoint {
                through: run.clone(),
            },
        );
        let receipt = rt.dispatch(cmd.clone())?;
        captures.push((run, cmd, receipt));
    }
    let (a, original, receipt) = &captures[0];
    let b = &captures[1].0;
    let diff = rt.checkpoint_diff(&root, a, b)?;
    assert!(diff.contains("-alpha") && diff.contains("+beta"));
    assert!(matches!(
        rt.checkpoint_diff(&peer, a, b),
        Err(Error::NotFound(_))
    ));
    fs::write(repo.join("tracked"), "dirty shared file")?;
    assert!(matches!(
        rt.dispatch(command(
            "dirty",
            &root,
            Action::CaptureCheckpoint { through: b.clone() }
        )),
        Err(Error::Invalid(_))
    ));
    assert_eq!(rt.dispatch(original.clone())?, *receipt);
    let mut forks = Vec::new();
    for (mode, name, expected) in [
        (ForkMode::Portable, "history", "dirty shared file"),
        (ForkMode::Checkpoint, "restored", "alpha"),
    ] {
        let cmd = command(
            name,
            &ThreadId(name.into()),
            Action::Fork {
                source: root.clone(),
                through: a.clone(),
                name: name.into(),
                instance: "codex".into(),
                model: None,
                mode,
                text: "continue here".into(),
            },
        );
        let receipt = rt.dispatch(cmd.clone())?;
        completed(&rt, &cmd.thread_id, receipt.run_id.as_ref().unwrap()).await;
        let cwd = rt
            .read(&cmd.thread_id)?
            .worktree_path
            .unwrap_or_else(|| repo.clone());
        if mode == ForkMode::Checkpoint {
            assert_eq!(
                cwd.parent().unwrap(),
                opts.state_dir.canonicalize()?.join("host checkouts")
            );
        }
        assert_eq!(fs::read_to_string(cwd.join("tracked"))?, expected);
        let start = frames(&cwd, "turn/start").pop().unwrap();
        let prompt = start["input"][0]["text"].as_str().unwrap();
        assert!(
            prompt.contains("alpha")
                && !prompt.contains("beta")
                && prompt.ends_with("continue here")
        );
        forks.push((cmd, receipt));
    }
    assert_eq!(rt.read(&root)?.runs.len(), 2);
    assert!(rt.read(&peer)?.worktree_path.is_none());
    assert_eq!(
        fs::read_to_string(repo.join("tracked"))?,
        "dirty shared file"
    );
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    assert_eq!(rt.checkpoint_diff(&root, a, b)?, diff);
    for (cmd, receipt) in forks {
        assert_eq!(rt.dispatch(cmd)?, receipt);
    }
    rt.close().await
}

async fn question(rt: &Huggler, thread: &ThreadId) -> Result<UserInputRequest> {
    let (snapshot, mut sub) = rt.subscribe(thread, None)?;
    if let Some(q) = snapshot.questions.into_iter().next() {
        return Ok(q);
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Change::QuestionRequested { request } = sub.next().await?.change {
                return Ok(request);
            }
        }
    })
    .await
    .expect("provider question")
}

#[tokio::test]
async fn restart_targets_one_active_run_and_keeps_queue_and_model_selection() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let opts = options(&dir.path().join("state"));
    let rt = Huggler::open(opts.clone()).await?;
    let project = rt.open_project(&repo)?;
    let root = launch(&rt, &project, "root", Provider::Codex);
    let run = rt
        .send(CommandId("active".into()), root.clone(), "ask".into())?
        .run_id
        .unwrap();
    assert_eq!(question(&rt, &root).await?.run_id, run);
    let queued = rt
        .dispatch(command(
            "queue",
            &root,
            Action::Enqueue {
                text: "queued".into(),
            },
        ))?
        .run_id
        .unwrap();
    rt.dispatch(command(
        "select",
        &root,
        Action::SetModel {
            model: Some("next".into()),
        },
    ))?;
    let restart = command(
        "restart",
        &root,
        Action::Restart {
            active: run.clone(),
            text: "replacement".into(),
        },
    );
    let receipt = rt.dispatch(restart.clone())?;
    assert_eq!(rt.dispatch(restart.clone())?, receipt);
    assert!(matches!(
        rt.dispatch(command("stale", &root, restart.action.clone())),
        Err(Error::Invalid(_))
    ));
    assert_eq!(rt.wait(&root, &run).await?.status, RunStatus::Interrupted);
    completed(&rt, &root, receipt.run_id.as_ref().unwrap()).await;
    completed(&rt, &root, &queued).await;
    let starts = frames(&repo, "turn/start");
    assert_eq!(
        starts
            .iter()
            .map(|f| f["input"][0]["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["ask", "replacement", "queued"]
    );
    assert_eq!(starts[1]["model"], "next");
    assert!(starts[0]["model"].is_null() && starts[2]["model"].is_null());
    assert_eq!(starts[0]["threadId"], starts[1]["threadId"]);
    let fresh = launch(&rt, &project, "fresh", Provider::Codex);
    let new = rt
        .send(CommandId("new".into()), fresh.clone(), "fresh start".into())?
        .run_id
        .unwrap();
    completed(&rt, &fresh, &new).await;
    let wire = frames(&repo, "turn/start").pop().unwrap();
    assert_ne!(wire["threadId"], starts[0]["threadId"]);
    assert_eq!(wire["input"][0]["text"], "fresh start");
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    assert_eq!(rt.dispatch(restart)?, receipt);
    assert_eq!(frames(&repo, "turn/start").len(), 4);
    rt.close().await
}

#[tokio::test]
#[ignore = "subprocess for outstanding_callbacks_expire_after_process_loss"]
async fn exit_with_callbacks() -> Result<()> {
    let Some(path) = std::env::var_os("HUGGLER_CALLBACK_FIXTURE") else {
        return Ok(());
    };
    let path = Path::new(&path);
    let repo = path.join("repo");
    checkout(&repo);
    let mut opts = options(&path.join("state"));
    opts.codex.request_approvals = true;
    let rt = Huggler::open(opts).await?;
    let thread = launch(&rt, &rt.open_project(&repo)?, "owner", Provider::Codex);
    rt.send(CommandId("send".into()), thread.clone(), "callbacks".into())?;
    let q = question(&rt, &thread).await?;
    let state = rt.read(&thread)?;
    assert_eq!(state.permissions.len(), 1);
    fs::write(
        path.join("callbacks"),
        serde_json::to_vec(&(q.run_id, q.id, &state.permissions[0].id, state.cursor))?,
    )?;
    std::process::exit(74)
}

#[tokio::test]
async fn outstanding_callbacks_expire_after_process_loss() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let exit = std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", "exit_with_callbacks", "--ignored"])
        .env("HUGGLER_CALLBACK_FIXTURE", dir.path())
        .status()?;
    assert_eq!(exit.code(), Some(74));
    let (run, q, p, cursor): (RunId, RequestId, RequestId, i64) =
        serde_json::from_slice(&fs::read(dir.path().join("callbacks"))?)?;
    let rt = Huggler::open(options(&dir.path().join("state"))).await?;
    let thread = ThreadId("owner".into());
    let (state, mut stream) = rt.subscribe(&thread, Some(cursor))?;
    assert!(state.questions.is_empty() && state.permissions.is_empty());
    assert_eq!(state.runs[0].status, RunStatus::Failed);
    assert!(
        matches!(stream.next().await?.change,Change::PermissionClosed{id,option:None} if id==p)
    );
    assert!(
        matches!(stream.next().await?.change,Change::QuestionClosed{id,answered:false,..} if id==q)
    );
    assert!(matches!(
        rt.dismiss_question(&thread, &run, &q).await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        rt.respond_to_permission(&thread, &run, &p, "accept".into()),
        Err(Error::NotFound(_))
    ));
    assert_eq!(
        rt.send(CommandId("send".into()), thread, "callbacks".into())?
            .run_id,
        Some(run)
    );
    rt.close().await?;
    let log = fs::read_to_string(dir.path().join("repo/frames.ndjson"))?;
    let frames: Vec<Value> = log
        .lines()
        .map(serde_json::from_str)
        .collect::<serde_json::Result<_>>()?;
    assert_eq!(
        frames
            .iter()
            .filter(|f| f["method"] == "turn/start")
            .count(),
        1
    );
    assert!(
        frames
            .iter()
            .all(|f| f["id"] != "approve" && f["id"] != "ask")
    );
    Ok(())
}

#[tokio::test]
async fn steering_settles_only_acknowledged_input_on_the_expected_turn() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let opts = options(&dir.path().join("state"));
    let rt = Huggler::open(opts.clone()).await?;
    let project = rt.open_project(&repo)?;
    let root = launch(&rt, &project, "root", Provider::Codex);
    let other = launch(&rt, &project, "other", Provider::ClaudeAgent);
    let steer = |thread: &ThreadId, run: &RunId, text: &str| {
        command(
            text,
            thread,
            Action::Steer {
                active: run.clone(),
                text: text.into(),
            },
        )
    };
    let mut replay = Vec::new();
    for (i, inputs) in [
        vec!["clarification", "reject", "finish before ack"],
        vec!["drop"],
    ]
    .iter()
    .enumerate()
    {
        let run = rt
            .dispatch(command(
                &format!("send-{i}"),
                &root,
                Action::SendAuto { text: "ask".into() },
            ))?
            .run_id
            .unwrap();
        question(&rt, &root).await?;
        let state = rt.read(&root)?;
        let native_turn = state.runs.last().unwrap().native_turn.as_ref().unwrap();
        let mut events = rt.subscribe(&root, Some(state.cursor))?.1;
        assert!(matches!(
            rt.dispatch(steer(&root, &RunId("stale".into()), "wrong")),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            rt.dispatch(steer(&other, &run, "unsupported")),
            Err(Error::Unsupported(_))
        ));
        for input in inputs {
            let cmd = if *input == "clarification" {
                command(
                    input,
                    &root,
                    Action::SendAuto {
                        text: (*input).into(),
                    },
                )
            } else {
                steer(&root, &run, input)
            };
            let receipt = rt.dispatch(cmd.clone())?;
            assert_eq!(receipt.run_id.as_ref(), Some(&run));
            assert_eq!(rt.dispatch(cmd.clone())?, receipt);
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    if let Change::Steer {
                        command_id,
                        run_id,
                        outcome,
                    } = events.next().await?.change
                        && command_id == cmd.id
                        && outcome != SteerOutcome::Pending
                    {
                        assert_eq!(run_id, run);
                        return Ok::<_, Error>(outcome);
                    }
                }
            })
            .await
            .expect("steer settlement")?;
            match *input {
                "clarification" => assert_eq!(outcome, SteerOutcome::Accepted),
                "reject" => assert!(
                    matches!(&outcome, SteerOutcome::Rejected(reason) if reason.contains("turn refused"))
                ),
                _ => assert_eq!(outcome, SteerOutcome::Uncertain),
            }
            assert!(matches!(
                rt.steer_outcome(&other, &cmd.id),
                Err(Error::NotFound(_))
            ));
            let wire = frames(&repo, "turn/steer").pop().unwrap();
            assert_eq!(wire["expectedTurnId"], native_turn.0);
            assert_eq!(wire["threadId"], state.native_thread.as_ref().unwrap().0);
            replay.push((cmd, receipt, outcome));
        }
        assert_eq!(
            rt.wait(&root, &run).await?.status,
            if i == 0 {
                RunStatus::Completed
            } else {
                RunStatus::Failed
            }
        );
    }
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    for (cmd, receipt, outcome) in replay {
        assert_eq!(rt.steer_outcome(&root, &cmd.id)?, outcome);
        assert_eq!(rt.dispatch(cmd)?, receipt);
    }
    let history = rt.read(&root)?.messages;
    assert_eq!(
        history.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["ask", "clarification", "reply ask", "ask"]
    );
    assert_eq!(frames(&repo, "turn/steer").len(), 4);
    rt.close().await
}

#[tokio::test]
async fn usage_reset_continues_once_and_respects_stop_and_newer_messages() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let mut opts = options(&dir.path().join("state"));
    let rt = Huggler::open(opts.clone()).await?;
    let project = rt.open_project(&repo)?;
    let mut limited = Vec::new();
    for (name, window) in [
        ("stopped", "soon"),
        ("manual", "soon"),
        ("stale", "stale"),
        ("unknown", "unknown"),
        ("resume", "soon"),
    ] {
        let thread = launch(&rt, &project, name, Provider::Codex);
        let run = rt
            .send(
                CommandId(format!("send-{name}")),
                thread.clone(),
                format!("quota {window}"),
            )?
            .run_id
            .unwrap();
        let failed = rt.wait(&thread, &run).await?;
        assert_eq!(failed.status, RunStatus::Failed);
        assert!(failed.error.unwrap().contains("Quota exhausted"));
        assert_eq!(failed.usage_limit_reset_at.is_some(), window != "unknown");
        if name == "stopped" {
            rt.stop_thread(CommandId("stop".into()), thread.clone())?;
        }
        if name == "manual" {
            let cmd = command(
                "manual-followup",
                &thread,
                Action::Send {
                    text: "manual continuation".into(),
                },
            );
            let receipt = rt.dispatch(cmd.clone())?;
            completed(&rt, &thread, receipt.run_id.as_ref().unwrap()).await;
            assert_eq!(rt.dispatch(cmd)?, receipt);
        }
        limited.push((thread, run));
    }
    rt.close().await?;
    opts.auto_resume_usage_limits = true;
    let rt = Huggler::open(opts.clone()).await?;
    let (thread, failed) = limited.last().unwrap();
    wait_for_limit_continuation(&rt, thread, failed).await?;
    let session = rt.read(thread)?.native_thread.unwrap();
    let starts = frames(&repo, "turn/start");
    assert_eq!(starts.len(), 7);
    assert_eq!(starts.last().unwrap()["threadId"], session.0);
    assert_eq!(
        starts.last().unwrap()["input"][0]["text"],
        "Continue where you left off."
    );
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    for (thread, _) in limited {
        let expected = if ["manual", "resume"].contains(&thread.0.as_str()) {
            2
        } else {
            1
        };
        assert_eq!(rt.read(&thread)?.runs.len(), expected);
    }
    assert_eq!(frames(&repo, "turn/start").len(), 7);
    let live = launch(&rt, &project, "live", Provider::Codex);
    let failed = rt
        .send(
            CommandId("live-limit".into()),
            live.clone(),
            "quota soon".into(),
        )?
        .run_id
        .unwrap();
    assert_eq!(rt.wait(&live, &failed).await?.status, RunStatus::Failed);
    wait_for_limit_continuation(&rt, &live, &failed).await?;
    assert_eq!(rt.read(&live)?.runs.len(), 2);
    rt.close().await
}

async fn wait_for_limit_continuation(
    rt: &Huggler,
    thread: &ThreadId,
    failed: &RunId,
) -> Result<()> {
    let mut stream = rt.subscribe(thread, Some(0))?.1;
    let next = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Change::UsageLimitContinuation {
                failed: previous,
                run_id,
            } = stream.next().await?.change
            {
                assert_eq!(&previous, failed);
                return Ok::<_, Error>(run_id);
            }
        }
    })
    .await
    .expect("recovery at the provider reset")?;
    completed(rt, thread, &next).await;
    Ok(())
}

#[tokio::test]
async fn command_output_is_scoped_bounded_and_available_after_reopen() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let opts = options(&dir.path().join("state"));
    let rt = Huggler::open(opts.clone()).await?;
    let project = rt.open_project(&repo)?;
    let thread = launch(&rt, &project, "root", Provider::Codex);
    let other = launch(&rt, &project, "sibling", Provider::Codex);
    let run = rt
        .send(
            CommandId("send".into()),
            thread.clone(),
            "command output".into(),
        )?
        .run_id
        .unwrap();
    let q = question(&rt, &thread).await?;
    assert_eq!(rt.command_items(&thread, &run)?, ["build"]);
    let live = rt.command_output(&thread, &run, "build", None).await?;
    assert!(live.truncated && !live.closed);
    assert!(live.text.len() <= 32768 && live.text.starts_with('界') && live.text.ends_with("end"));
    assert!(matches!(
        rt.command_output(&other, &run, "build", None).await,
        Err(Error::NotFound(_))
    ));
    let (result, output) = tokio::join!(
        rt.dismiss_question(&thread, &run, &q.id),
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            rt.command_output(&thread, &run, "build", Some(live.revision))
        )
    );
    result?;
    let finished = output.unwrap()?;
    assert!(finished.closed && !finished.truncated && finished.revision > live.revision);
    assert_eq!(finished.text, "final report");
    completed(&rt, &thread, &run).await;
    let (snapshot, mut events) = rt.subscribe(&thread, Some(0))?;
    loop {
        let event = events.next().await?;
        assert!(!serde_json::to_string(&event)?.contains('界'));
        if event.sequence == snapshot.cursor {
            break;
        }
    }
    rt.retain_events(&thread, 1)?;
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    assert_eq!(rt.command_items(&thread, &run)?, ["build"]);
    assert_eq!(
        rt.command_output(&thread, &run, "build", None).await?,
        finished
    );
    rt.close().await
}

#[tokio::test]
async fn prompt_suggestions_settle_independently_and_stay_with_the_latest_turn() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let mut opts = options(&dir.path().join("state"));
    let rt = Huggler::open(opts.clone()).await?;
    let root = launch(&rt, &rt.open_project(&repo)?, "root", Provider::Codex);
    let run = rt
        .send(CommandId("disabled".into()), root.clone(), "first".into())?
        .run_id
        .unwrap();
    completed(&rt, &root, &run).await;
    rt.close().await?;
    assert!(frames(&repo, "thread/sample").is_empty());

    #[derive(Debug)]
    struct SuggestionText;
    impl AgentText for SuggestionText {
        fn suggestion_prompt(&self) -> &str {
            "Propose the next question."
        }
    }
    opts.output.text = std::sync::Arc::new(SuggestionText);
    opts.output.name = "studio".into();
    opts.output.title = "Studio".into();
    opts.output.version = "7".into();
    opts.prompt_suggestions = true;
    let rt = Huggler::open(opts.clone()).await?;
    let send = |prompt: &str| {
        rt.send(CommandId(prompt.into()), root.clone(), prompt.into())
            .map(|receipt| receipt.run_id.unwrap())
    };
    let wait_file = |name: &str| {
        let file = repo.join(name);
        async move {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while !file.exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
    };
    for prompt in ["sample unavailable", "sample blank"] {
        let run = send(prompt)?;
        completed(&rt, &root, &run).await;
        wait_file(&format!("{prompt}.finished")).await;
        assert!(
            rt.detail(&root, None, "", 1)?
                .latest_run
                .unwrap()
                .prompt_suggestion
                .is_none()
        );
    }
    let slow = send("delayed sample")?;
    completed(&rt, &root, &slow).await;
    let newer = send("ask")?;
    let question = question(&rt, &root).await?;
    fs::write(repo.join("release-sample"), "")?;
    wait_file("delayed sample.finished").await;
    assert!(
        rt.read(&root)?
            .runs
            .iter()
            .all(|r| r.prompt_suggestion.is_none())
    );
    rt.dismiss_question(&root, &newer, &question.id).await?;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let detail = rt.detail(&root, None, "", 2)?;
            if let Some(hint) = detail.latest_run.unwrap().prompt_suggestion {
                assert_eq!(hint, "Explain the result.");
                break;
            }
            rt.wait_for_change(&root, detail.history.sequence).await?;
        }
        Ok::<_, Error>(())
    })
    .await
    .unwrap()?;
    let samples = frames(&repo, "thread/sample");
    assert_eq!(samples.len(), 4);
    assert!(
        samples
            .iter()
            .all(|p| p["prompt"] == "Propose the next question.")
    );
    let init = frames(&repo, "initialize").pop().unwrap();
    assert_eq!(
        init["clientInfo"],
        serde_json::json!({"name":"studio","title":"Studio","version":"7"})
    );
    rt.close().await?;
    let rt = Huggler::open(opts).await?;
    let detail = rt.detail(&root, None, "", 100)?;
    assert_eq!(
        detail.latest_run.unwrap().prompt_suggestion.as_deref(),
        Some("Explain the result.")
    );
    assert!(
        !detail
            .history
            .messages
            .iter()
            .any(|m| m.message.text == "Explain the result.")
    );
    rt.close().await
}

#[tokio::test]
async fn mixed_attachments_survive_a_held_queue_and_reach_the_provider() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let mut opts = options(&dir.path().join("state"));
    opts.storage.database_path = "attachments".into();
    let rt = Huggler::open(opts.clone()).await?;
    let project = rt.open_project(&repo)?;
    let thread = launch(&rt, &project, "mixed attachments", Provider::Codex);
    let peer = launch(&rt, &project, "other", Provider::Codex);

    let (_, mut changes) = rt.subscribe(&thread, None)?;
    let active = rt.send(CommandId("active".into()), thread.clone(), "ask".into())?;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if matches!(
                changes.next().await.unwrap().change,
                Change::QuestionRequested { .. }
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();

    let png = include_bytes!("fixtures/attachments/diagram.png");
    let pdf = include_bytes!("fixtures/attachments/report.pdf");
    let files = vec![
        Attachment::from_bytes("diagram".into(), png.to_vec())?,
        Attachment::from_bytes("report.pdf".into(), pdf.to_vec())?,
        Attachment::from_bytes("data.zip".into(), vec![0, 5, 255])?,
        Attachment::from_bytes("snippet.rs".into(), b"fn main() {}".to_vec())?,
    ];
    let queue = command(
        "mixed",
        &thread,
        Action::EnqueueFiles {
            text: String::new(),
            files: files.clone(),
        },
    );
    let receipt = rt.dispatch(queue.clone())?;
    assert_eq!(rt.dispatch(queue.clone())?, receipt);
    let mut different = queue.clone();
    if let Action::EnqueueFiles { files, .. } = &mut different.action {
        files[2].bytes[1] = 6;
    }
    assert!(matches!(
        rt.dispatch(different),
        Err(Error::Conflict { .. })
    ));

    let upload = rt.prepare_upload(&thread, "staged.png", "image/png", png)?;
    let staged = command(
        "staged",
        &thread,
        Action::SendAttachments {
            text: "Staged image".into(),
            uploads: vec![upload.clone()],
        },
    );
    assert!(matches!(rt.dispatch(staged.clone()), Err(Error::Busy)));

    rt.stop_thread(CommandId("hold".into()), thread.clone())?;
    rt.wait(&thread, active.run_id.as_ref().unwrap()).await?;
    drop(changes);
    rt.close().await?;

    let mut reopen = opts.clone();
    reopen.storage.database_path = opts.state_dir.join(&opts.storage.database_path);
    reopen.state_dir = dir.path().join("other state");
    let rt = Huggler::open(reopen).await?;
    assert_eq!(rt.dispatch(queue)?, receipt);
    let message = receipt.message_id.as_ref().unwrap();
    let row = rt
        .detail(&thread, None, "report.pdf", 8)?
        .history
        .messages
        .pop()
        .unwrap();
    assert_eq!(row.message.id, *message);
    assert!(row.files.unwrap().caption.is_empty());
    for (position, file) in files.iter().enumerate() {
        assert_eq!(rt.read_attachment_file(&thread, message, position)?, *file);
    }
    assert!(matches!(
        rt.read_attachment_file(&peer, message, 0),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        rt.read_attachment(&thread, message, 0),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(rt.read_attachment(&thread, message, 3)?, "fn main() {}");

    rt.resume_queue(CommandId("resume".into()), thread.clone())?;
    completed(&rt, &thread, receipt.run_id.as_ref().unwrap()).await;
    let input = frames(&repo, "turn/start").pop().unwrap()["input"].clone();
    let text = input[0]["text"].as_str().unwrap();
    assert!(text.contains("Attachment PDF fixture") && text.contains("fn main() {}"));
    assert!(text.contains("data.zip") && text.contains("application/zip"));
    assert_eq!(input.as_array().unwrap().len(), 2);
    assert_eq!(input[1]["type"], "localImage");
    let path = Path::new(input[1]["path"].as_str().unwrap());
    assert_eq!(fs::read(path)?, png);

    let sent = rt.dispatch(staged.clone())?;
    completed(&rt, &thread, sent.run_id.as_ref().unwrap()).await;
    assert_eq!(rt.dispatch(staged)?, sent);
    assert_eq!(
        rt.read_attachment_file(&thread, sent.message_id.as_ref().unwrap(), 0)?
            .bytes,
        png
    );

    rt.close().await?;
    #[cfg(feature = "application")]
    {
        let rt = Huggler::open(opts).await?;
        rt.delete_thread(&thread)?;
        assert!(!path.exists());
        assert!(matches!(
            rt.read_attachment_file(&thread, message, 0),
            Err(Error::NotFound(_))
        ));
        rt.close().await?;
    }
    Ok(())
}

#[tokio::test]
async fn acp_image_delivery_follows_the_advertised_capability() -> Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let dir = tempfile::tempdir()?;
    let repo = dir.path().join("repo");
    checkout(&repo);
    let rt = Huggler::open(options(&dir.path().join("state"))).await?;
    let thread = launch(
        &rt,
        &rt.open_project(&repo)?,
        "ACP images",
        Provider::ClaudeAgent,
    );
    let image = include_bytes!("fixtures/attachments/diagram.png");
    for native in [false, true] {
        if native {
            fs::write(repo.join("acp-vision"), "")?;
        }
        let sent = rt.dispatch(command(
            if native {
                "native-image"
            } else {
                "image-fallback"
            },
            &thread,
            Action::EnqueueFiles {
                text: "Look at this image".into(),
                files: vec![Attachment::from_bytes(
                    "diagram.png".into(),
                    image.to_vec(),
                )?],
            },
        ))?;
        completed(&rt, &thread, sent.run_id.as_ref().unwrap()).await;
        let wire = frames(&repo, "session/prompt").pop().unwrap();
        let parts = wire["prompt"].as_array().unwrap();
        assert!(parts[0]["text"].as_str().unwrap().contains("diagram.png"));
        if native {
            assert_eq!(
                parts[1],
                serde_json::json!({
                    "type": "image", "mimeType": "image/png", "data": STANDARD.encode(image)
                })
            );
        } else {
            assert_eq!(parts.len(), 1);
            let (_, encoded) = parts[0]["text"]
                .as_str()
                .unwrap()
                .rsplit_once("available at ")
                .unwrap();
            let path: String = serde_json::from_str(encoded)?;
            assert_eq!(fs::read(path)?, image);
        }
    }
    rt.close().await
}
