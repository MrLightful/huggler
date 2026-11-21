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
