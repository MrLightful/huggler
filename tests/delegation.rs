#![cfg(unix)]
#[path = "support/agents.rs"]
mod agents;
use agents::*;
use huggler::*;
use std::{path::Path, time::Duration};
use tokio::time::timeout;

async fn permission(rt: &Huggler, thread: &ThreadId) -> PermissionRequest {
    let (snapshot, mut events) = rt.subscribe(thread, None).unwrap();
    if let Some(request) = snapshot.permissions.into_iter().next() {
        return request;
    }
    loop {
        match events.next().await.unwrap().change {
            Change::PermissionRequested { request } => return request,
            Change::Run { run } if run.status.terminal() => panic!("expected permission: {run:?}"),
            _ => {}
        }
    }
}

async fn release(rt: &Huggler, thread: &ThreadId, live_tool: Option<&str>) {
    let request = permission(rt, thread).await;
    let choice = if let Some(tool) = live_tool {
        let input = &request.tool_call["rawInput"];
        assert_eq!(input["serverName"], "huggler");
        assert_eq!(input["_meta"]["tool_name"], tool);
        "accept"
    } else {
        "done"
    };
    rt.respond_to_permission(thread, &request.run_id, &request.id, choice.into())
        .unwrap();
}

async fn task(
    rt: &Huggler,
    thread: &ThreadId,
    index: usize,
    expected: impl Fn(&DelegatedTask) -> bool,
) -> DelegatedTask {
    let (mut snapshot, mut events) = rt.subscribe(thread, None).unwrap();
    loop {
        if let Some(task) = snapshot.tasks.get(index).filter(|task| expected(task)) {
            return task.clone();
        }
        events.next().await.unwrap();
        snapshot = rt.read(thread).unwrap();
    }
}

const LIVE_PROMPT: &str = "Use the huggler delegate_task tool once with clientRequestId review, title Reviewer, target.providerInstanceId codex, mode async and isolation shared. Set its task to exactly: First call the huggler thread_list tool once, then return exactly 2+2=4 as your final answer. After delegation finish this turn immediately. When Huggler delivers the child result, reply with the finding.";

async fn scenario(mode: &str, live: bool) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("checkout");
    checkout(&repo);
    let mut options = HugglerOptions::new(dir.path().join("state"));
    options.agent_tools = true;
    options.codex.request_approvals = live;
    options.claude_agent.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
    let rt = Huggler::open(options.clone()).await.unwrap();
    let project = rt.open_project(&repo).unwrap();
    let provider = if live {
        Provider::Codex
    } else {
        Provider::ClaudeAgent
    };
    let parent = launch(&rt, &project, "author", provider);
    let prompt = if live {
        LIVE_PROMPT.into()
    } else {
        format!("delegate {mode}")
    };
    let first = rt
        .send(CommandId("start".into()), parent.clone(), prompt)
        .unwrap()
        .run_id
        .unwrap();
    if live {
        release(&rt, &parent, Some("delegate_task")).await;
    }
    let initial = task(&rt, &parent, 0, |_| true).await;
    assert_eq!(initial.parent_run_id, first);
    let child = &initial.child_thread_id;
    assert_eq!(rt.read(child).unwrap().provider, provider);
    assert_eq!(rt.read(child).unwrap().project, project.id);

    match mode {
        "idle" | "nested" => {
            completed(&rt, &parent, &first).await;
            assert_eq!(rt.read(&parent).unwrap().runs.len(), 1);
            if mode == "nested" {
                let grandchild = task(&rt, child, 0, |_| true).await;
                completed(&rt, child, &initial.child_run_id).await;
                assert_eq!(task(&rt, &parent, 0, |_| true).await.result, None);
                release(&rt, &grandchild.child_thread_id, None).await;
            } else {
                release(&rt, child, live.then_some("thread_list")).await;
            }
        }
        "cancel" => {
            permission(&rt, child).await;
            release(&rt, &parent, None).await;
            completed(&rt, &parent, &first).await;
        }
        "ack" | "siblings" | "recover" => {
            permission(&rt, &parent).await;
            release(&rt, child, None).await;
            let pending = task(&rt, &parent, 0, |t| t.result.is_some()).await;
            assert_eq!(pending.delivery, CompletionDelivery::Pending);
            assert_eq!(pending.result.as_deref(), Some("2+2=4"));
            if mode == "siblings" {
                let sibling = task(&rt, &parent, 1, |_| true).await;
                release(&rt, &sibling.child_thread_id, None).await;
                task(&rt, &parent, 1, |t| t.result.is_some()).await;
            }
            if mode == "ack" {
                let stranger = launch(&rt, &project, "observer", provider);
                let run = rt
                    .send(
                        CommandId("inspect".into()),
                        stranger.clone(),
                        format!("inspect {}", initial.task_id),
                    )
                    .unwrap()
                    .run_id
                    .unwrap();
                completed(&rt, &stranger, &run).await;
                assert_eq!(rt.read(&parent).unwrap().tasks[0], pending);
            }
            if mode != "recover" {
                release(&rt, &parent, None).await;
                completed(&rt, &parent, &first).await;
            }
        }
        _ => unreachable!(),
    }

    let rt = if mode == "recover" {
        rt.close().await.unwrap();
        Huggler::open(options.clone()).await.unwrap()
    } else {
        rt
    };
    let settled = task(&rt, &parent, 0, |t| {
        t.delivery != CompletionDelivery::Pending && t.status.terminal()
    })
    .await;
    assert_eq!(settled.task_id, initial.task_id);
    match mode {
        "ack" | "cancel" => {
            assert_eq!(rt.read(&parent).unwrap().runs.len(), 1);
            assert!(settled.wake_run_id.is_none());
            assert_eq!(
                settled.delivery,
                if mode == "ack" {
                    CompletionDelivery::Acknowledged
                } else {
                    CompletionDelivery::Disposed
                }
            );
            assert_eq!(
                settled.status,
                if mode == "ack" {
                    RunStatus::Completed
                } else {
                    RunStatus::Interrupted
                }
            );
        }
        _ => {
            let wake = settled.wake_run_id.as_ref().unwrap();
            completed(&rt, &parent, wake).await;
            assert_eq!(settled.status, RunStatus::Completed);
            assert_eq!(settled.delivery, CompletionDelivery::Delivered);
            let snapshot = rt.read(&parent).unwrap();
            assert_eq!(snapshot.runs.len(), 2);
            if mode == "siblings" {
                assert_eq!(snapshot.tasks[1].wake_run_id.as_ref(), Some(wake));
                assert_eq!(
                    snapshot.messages.last().unwrap().text,
                    "Received: 2+2=4, 2+2=4"
                );
            }
            assert_eq!(
                settled.result.as_deref(),
                Some(if mode == "nested" {
                    "Received: 2+2=4"
                } else {
                    "2+2=4"
                })
            );
            assert!(snapshot.messages.last().unwrap().text.contains("2+2=4"));
        }
    }
    let before = rt.read(&parent).unwrap();
    rt.close().await.unwrap();
    let rt = Huggler::open(options).await.unwrap();
    let after = rt.read(&parent).unwrap();
    assert_eq!(after.tasks, before.tasks);
    assert_eq!(after.runs, before.runs);
    assert_eq!(after.messages, before.messages);
    assert_eq!(after.cursor, before.cursor);
    rt.close().await.unwrap();
}

#[tokio::test]
async fn completion_wakes_once_after_idle_or_restart_and_groups_siblings() {
    for mode in ["idle", "recover", "siblings", "nested"] {
        timeout(Duration::from_secs(15), scenario(mode, false))
            .await
            .expect(mode);
    }
}

#[tokio::test]
async fn only_the_parent_can_acknowledge_or_cancel_delivery() {
    for mode in ["ack", "cancel"] {
        timeout(Duration::from_secs(15), scenario(mode, false))
            .await
            .expect(mode);
    }
}

#[tokio::test]
#[ignore = "uses two live Codex sessions with an authenticated local CLI"]
async fn live_codex_delegation() {
    timeout(Duration::from_secs(240), scenario("idle", true))
        .await
        .unwrap();
}
