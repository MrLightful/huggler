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
        Err(Error::Unsupported(_))
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
