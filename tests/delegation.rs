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

fn options(state: &Path) -> HugglerOptions {
    let mut options = HugglerOptions::new(state);
    options.agent_tools = true;
    options.codex.request_approvals = true;
    options.claude_agent.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/peer-acp.py");
    options
}

async fn scenario(mode: &str, parent_provider: Provider, child_provider: Provider) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("checkout");
    checkout(&repo);
    let options = options(&dir.path().join("state"));
    let rt = Huggler::open(options.clone()).await.unwrap();
    let project = rt.open_project(&repo).unwrap();
    let parent = launch(&rt, &project, "author", parent_provider);
    let prompt = if parent_provider == Provider::Codex {
        let (instance, review) = match child_provider {
            Provider::Codex => (
                "codex",
                "First call the huggler thread_list tool once, then return exactly 2+2=4 as your final answer.",
            ),
            Provider::ClaudeAgent => ("claudeAgent", "held review"),
        };
        format!(
            "Use the huggler delegate_task tool once with clientRequestId review, title Reviewer, target.providerInstanceId {instance}, mode async and isolation shared. Set its task to exactly the following quoted text: \"{review}\". After delegation finish this turn immediately. When Huggler delivers the child result, reply with the finding."
        )
    } else {
        format!("delegate {mode}")
    };
    let first = rt
        .send(CommandId("start".into()), parent.clone(), prompt)
        .unwrap()
        .run_id
        .unwrap();
    if parent_provider == Provider::Codex {
        release(&rt, &parent, Some("delegate_task")).await;
    }
    let initial = task(&rt, &parent, 0, |_| true).await;
    assert_eq!(initial.parent_run_id, first);
    let child = &initial.child_thread_id;
    assert_eq!(rt.read(child).unwrap().provider, child_provider);
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
                release(
                    &rt,
                    child,
                    (child_provider == Provider::Codex).then_some("thread_list"),
                )
                .await;
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
                let stranger = launch(&rt, &project, "observer", parent_provider);
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
async fn queued_followup_supplies_the_final_result_once() {
    timeout(Duration::from_secs(12), async {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("checkout");
        checkout(&repo);
        let options = options(&dir.path().join("state"));
        let rt = Huggler::open(options.clone()).await.unwrap();
        let project = rt.open_project(&repo).unwrap();
        let parent = launch(&rt, &project, "author", Provider::ClaudeAgent);
        let first = rt
            .send(
                CommandId("start".into()),
                parent.clone(),
                "delegate followup".into(),
            )
            .unwrap()
            .run_id
            .unwrap();
        completed(&rt, &parent, &first).await;
        let initial = rt.read(&parent).unwrap().tasks.remove(0);
        let followup = rt.peer_receipt(&parent, "revise").unwrap().unwrap();
        assert_eq!(followup.thread_id, initial.child_thread_id);
        assert_eq!(followup.delivery, PeerDelivery::Queued);
        assert!(initial.result_run_id.is_none());
        release(&rt, &initial.child_thread_id, None).await;
        let settled = task(&rt, &parent, 0, |t| t.wake_run_id.is_some()).await;
        completed(&rt, &parent, settled.wake_run_id.as_ref().unwrap()).await;
        assert_eq!(settled.result_run_id, Some(followup.run_id.clone()));
        assert_eq!(settled.result.as_deref(), Some("Received revised finding"));
        assert_ne!(initial.child_run_id, followup.run_id);
        rt.close().await.unwrap();
        let rt = Huggler::open(options).await.unwrap();
        let reopened = rt.read(&parent).unwrap();
        assert_eq!(reopened.tasks, [settled]);
        assert_eq!(reopened.runs.len(), 2);
        assert_eq!(
            reopened.messages.last().unwrap().text,
            "Received: Received revised finding"
        );
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

async fn settled_review(rt: &Huggler, parent: &ThreadId) -> DelegatedTask {
    let first = rt
        .send(
            CommandId("start".into()),
            parent.clone(),
            "delegate rounds".into(),
        )
        .unwrap()
        .run_id
        .unwrap();
    task(rt, parent, 0, |t| t.status.terminal()).await;
    release(rt, parent, None).await;
    completed(rt, parent, &first).await;
    let original = rt.read(parent).unwrap().tasks.remove(0);
    assert_eq!(original.result.as_deref(), Some("2+2=4"));
    assert_eq!(original.delivery, CompletionDelivery::Acknowledged);
    original
}

#[tokio::test]
async fn owner_followups_keep_their_own_runs_receipts_and_results() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("checkout");
        checkout(&repo);
        let options = options(&dir.path().join("state"));
        let rt = Huggler::open(options.clone()).await.unwrap();
        let project = rt.open_project(&repo).unwrap();
        let parent = launch(&rt, &project, "author", Provider::ClaudeAgent);
        let original = settled_review(&rt, &parent).await;
        let child = &original.child_thread_id;
        let owner_run = rt
            .send(
                CommandId("rounds".into()),
                parent.clone(),
                format!("followups {child}"),
            )
            .unwrap()
            .run_id
            .unwrap();
        // The parent requests permission only after both sends and their retries.
        permission(&rt, &parent).await;
        let pending = rt.read(&parent).unwrap().tasks;
        assert_eq!(pending.len(), 3);
        assert_eq!(pending[0], original);
        assert_eq!(pending[1].status, RunStatus::Running);
        assert_eq!(pending[2].status, RunStatus::Queued);
        let child_snapshot = rt.read(child).unwrap();
        assert_eq!(child_snapshot.runs.len(), 3);
        let mut receipts = vec![];
        for (index, key, delivery) in [
            (1, "review", PeerDelivery::Started),
            (2, "next", PeerDelivery::Queued),
        ] {
            let receipt = rt.peer_receipt(&parent, key).unwrap().unwrap();
            let followup = &pending[index];
            assert_eq!(receipt.task_id.as_ref(), Some(&followup.task_id));
            assert_eq!(receipt.run_id, followup.child_run_id);
            assert_eq!(receipt.delivery, delivery);
            assert_eq!(followup.parent_run_id, owner_run);
            assert_eq!(&followup.child_thread_id, child);
            assert!(followup.result_run_id.is_none());
            let message = child_snapshot
                .messages
                .iter()
                .find(|m| m.id == receipt.message_id)
                .unwrap();
            assert_eq!(message.run_id, receipt.run_id);
            assert_eq!(
                message.origin,
                Some(MessageOrigin {
                    thread_id: parent.clone(),
                    run_id: owner_run.clone(),
                    request_key: key.into()
                })
            );
            receipts.push((key, receipt));
        }

        // ACP ends the first prompt with stopReason=cancelled. The queued
        // review is still awaiting permission when the interrupted result arrives.
        release(&rt, child, None).await;
        let interrupted = task(&rt, &parent, 1, |t| t.status.terminal()).await;
        assert_eq!(interrupted.status, RunStatus::Interrupted);
        assert_eq!(
            interrupted.result_run_id.as_ref(),
            Some(&pending[1].child_run_id)
        );
        assert_eq!(
            interrupted.result.as_deref(),
            Some("provider turn interrupted")
        );
        assert_eq!(
            rt.read(&parent).unwrap().tasks[2].status,
            RunStatus::Running
        );
        release(&rt, child, None).await;
        let second = task(&rt, &parent, 2, |t| t.status.terminal()).await;
        assert_eq!(second.result.as_deref(), Some("Second finding"));
        assert_eq!(
            second.result_run_id.as_ref(),
            Some(&pending[2].child_run_id)
        );

        release(&rt, &parent, None).await;
        completed(&rt, &parent, &owner_run).await;
        let delivered = task(&rt, &parent, 2, |t| t.wake_run_id.is_some()).await;
        completed(&rt, &parent, delivered.wake_run_id.as_ref().unwrap()).await;
        let saved = rt.read(&parent).unwrap();
        assert_eq!(saved.tasks[0], original);
        assert_eq!(saved.tasks[1].wake_run_id, delivered.wake_run_id);
        assert_eq!(saved.runs.len(), 3);
        assert_eq!(
            saved.messages.last().unwrap().text,
            "Received: provider turn interrupted, Second finding"
        );
        rt.close().await.unwrap();

        let rt = Huggler::open(options).await.unwrap();
        let retry = rt
            .send(
                CommandId("retry".into()),
                parent.clone(),
                format!("retry followups {child}"),
            )
            .unwrap()
            .run_id
            .unwrap();
        completed(&rt, &parent, &retry).await;
        assert_eq!(rt.read(&parent).unwrap().tasks, saved.tasks);
        assert_eq!(rt.read(child).unwrap().runs.len(), 3);
        for (key, receipt) in receipts {
            assert_eq!(rt.peer_receipt(&parent, key).unwrap(), Some(receipt));
        }
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn agent_interrupt_stops_only_the_addressed_followup() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("checkout");
        let outside_repo = dir.path().join("other");
        checkout(&repo);
        checkout(&outside_repo);
        let options = options(&dir.path().join("state"));
        let rt = Huggler::open(options.clone()).await.unwrap();
        let project = rt.open_project(&repo).unwrap();
        let parent = launch(&rt, &project, "author", Provider::ClaudeAgent);
        let outside = launch(
            &rt,
            &rt.open_project(&outside_repo).unwrap(),
            "outsider",
            Provider::ClaudeAgent,
        );
        let original = settled_review(&rt, &parent).await;
        let child = &original.child_thread_id;
        let owner_run = rt
            .send(
                CommandId("rounds".into()),
                parent.clone(),
                format!("interrupt followups {child}"),
            )
            .unwrap()
            .run_id
            .unwrap();
        permission(&rt, &parent).await;
        let first = permission(&rt, child).await.run_id;
        let second = rt.peer_receipt(&parent, "next").unwrap().unwrap().run_id;
        let before = rt.read(child).unwrap().cursor;

        let denied = rt
            .send(
                CommandId("outside".into()),
                outside.clone(),
                format!("outside interrupt {child} {first}"),
            )
            .unwrap()
            .run_id
            .unwrap();
        completed(&rt, &outside, &denied).await;
        assert_eq!(rt.read(child).unwrap().cursor, before);
        assert_eq!(permission(&rt, child).await.run_id, first);

        release(&rt, &parent, None).await;
        completed(&rt, &parent, &owner_run).await;
        let result = rt.wait(child, &first).await.unwrap();
        assert_eq!(result.status, RunStatus::Interrupted);
        assert_eq!(
            result.error.as_deref(),
            Some("Stopped by an agent: Continue with the second review")
        );
        assert_eq!(permission(&rt, child).await.run_id, second);
        release(&rt, child, None).await;
        let settled = task(&rt, &parent, 2, |t| t.wake_run_id.is_some()).await;
        completed(&rt, &parent, settled.wake_run_id.as_ref().unwrap()).await;
        assert_eq!(settled.result_run_id, Some(second.clone()));
        assert_eq!(settled.result.as_deref(), Some("Second finding"));
        rt.close().await.unwrap();

        let rt = Huggler::open(options).await.unwrap();
        let cursor = rt.read(child).unwrap().cursor;
        let retry = rt
            .send(
                CommandId("retry".into()),
                parent.clone(),
                format!("retry interrupt {child} {first} {second}"),
            )
            .unwrap()
            .run_id
            .unwrap();
        completed(&rt, &parent, &retry).await;
        assert_eq!(rt.read(child).unwrap().cursor, cursor);
        let tasks = rt.read(&parent).unwrap().tasks;
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0], original);
        assert_eq!(tasks[1].status, RunStatus::Interrupted);
        assert_eq!(tasks[1].result_run_id.as_ref(), Some(&first));
        assert_eq!(tasks[1].result, result.error);
        assert_eq!(tasks[2], settled);

        let (_, mut events) = rt.subscribe(child, Some(before)).unwrap();
        let mut requests = 0;
        loop {
            let event = events.next().await.unwrap();
            if let Change::RunInterruptRequested {
                run_id,
                caller_thread_id,
                caller_run_id,
                reason,
            } = event.change
            {
                assert_eq!(run_id, first);
                assert_eq!(caller_thread_id, parent);
                assert_eq!(caller_run_id, owner_run);
                assert_eq!(reason.as_deref(), Some("Continue with the second review"));
                requests += 1;
            }
            if event.sequence == cursor {
                break;
            }
        }
        assert_eq!(requests, 1);
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nested_results_wake_only_the_followup_that_requested_them() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("checkout");
        checkout(&repo);
        let rt = Huggler::open(options(&dir.path().join("state")))
            .await
            .unwrap();
        let project = rt.open_project(&repo).unwrap();
        let parent = launch(&rt, &project, "author", Provider::ClaudeAgent);
        let original = settled_review(&rt, &parent).await;
        let child = &original.child_thread_id;
        let owner_run = rt
            .send(
                CommandId("nested".into()),
                parent.clone(),
                format!("nested followups {child}"),
            )
            .unwrap()
            .run_id
            .unwrap();
        permission(&rt, &parent).await;
        let alpha = task(&rt, child, 0, |_| true).await;
        release(&rt, child, None).await;
        let beta = task(&rt, child, 1, |_| true).await;
        // Both reports arrive while the second follow-up occupies the child.
        for (index, leaf) in [&alpha, &beta].into_iter().enumerate() {
            release(&rt, &leaf.child_thread_id, None).await;
            task(&rt, child, index, |t| t.result.is_some()).await;
        }
        let pending = rt.read(&parent).unwrap().tasks;
        assert!(pending[1..].iter().all(|t| t.result.is_none()));
        release(&rt, child, None).await;
        for (i, answer) in ["Received: alpha", "Received: beta"]
            .into_iter()
            .enumerate()
        {
            let followup = task(&rt, &parent, i + 1, |t| t.result.is_some()).await;
            let nested = &rt.read(child).unwrap().tasks[i];
            assert_eq!(followup.child_run_id, nested.parent_run_id);
            assert_eq!(followup.result_run_id, nested.wake_run_id);
            assert_eq!(followup.result.as_deref(), Some(answer));
        }
        release(&rt, &parent, None).await;
        completed(&rt, &parent, &owner_run).await;
        let delivered = task(&rt, &parent, 2, |t| t.wake_run_id.is_some()).await;
        completed(&rt, &parent, delivered.wake_run_id.as_ref().unwrap()).await;
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_reaches_grandchildren_and_queued_work_even_after_a_result() {
    for mode in ["subtree", "settled"] {
        timeout(Duration::from_secs(8), async {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path().join("checkout");
            checkout(&repo);
            let options = options(&dir.path().join("state"));
            let rt = Huggler::open(options.clone()).await.unwrap();
            let project = rt.open_project(&repo).unwrap();
            let parent = launch(&rt, &project, "author", Provider::ClaudeAgent);
            let first = rt
                .send(
                    CommandId("start".into()),
                    parent.clone(),
                    format!("delegate {mode}"),
                )
                .unwrap()
                .run_id
                .unwrap();
            let original = task(&rt, &parent, 0, |t| {
                mode == "subtree" || t.status.terminal()
            })
            .await;
            let child = &original.child_thread_id;
            let branch_run = if mode == "settled" {
                rt.send(
                    CommandId("more".into()),
                    child.clone(),
                    "branch review".into(),
                )
                .unwrap()
                .run_id
                .unwrap()
            } else {
                original.child_run_id.clone()
            };
            let leaf = task(&rt, child, 0, |_| true).await;
            permission(&rt, child).await;
            permission(&rt, &leaf.child_thread_id).await;
            let queued = rt.peer_receipt(child, "revise").unwrap().unwrap();
            assert_eq!(queued.delivery, PeerDelivery::Queued);
            release(&rt, &parent, None).await;
            completed(&rt, &parent, &first).await;
            for (thread, run) in [
                (child, &branch_run),
                (&leaf.child_thread_id, &leaf.child_run_id),
                (&leaf.child_thread_id, &queued.run_id),
            ] {
                assert_eq!(
                    rt.wait(thread, run).await.unwrap().status,
                    RunStatus::Interrupted
                );
            }
            let nested = task(&rt, child, 0, |t| t.status.terminal()).await;
            assert_eq!(nested.delivery, CompletionDelivery::Disposed);
            assert_eq!(nested.status, RunStatus::Interrupted);
            let settled = task(&rt, &parent, 0, |t| t.status.terminal()).await;
            assert_eq!(settled.delivery, CompletionDelivery::Disposed);
            if mode == "settled" {
                assert_eq!(settled.result_run_id, original.result_run_id);
                assert_eq!(settled.result.as_deref(), Some("2+2=4"));
                assert_eq!(settled.status, RunStatus::Completed);
            } else {
                assert_eq!(settled.status, RunStatus::Interrupted);
            }
            rt.close().await.unwrap();
            let rt = Huggler::open(options).await.unwrap();
            assert_eq!(rt.read(&parent).unwrap().runs.len(), 1);
            assert_eq!(rt.read(&parent).unwrap().tasks, [settled.clone()]);
            assert_eq!(rt.read(child).unwrap().tasks, [nested]);
            let leaf_thread = rt.read(&leaf.child_thread_id).unwrap();
            assert!(leaf_thread.permissions.is_empty());
            assert!(leaf_thread.messages.iter().all(|m| m.role == Role::User));
            // An old cancel retried after restart must leave newly accepted work alone.
            let later = rt
                .send(CommandId("later".into()), child.clone(), "hold".into())
                .unwrap()
                .run_id
                .unwrap();
            permission(&rt, child).await;
            let retry = rt
                .send(
                    CommandId("retry".into()),
                    parent.clone(),
                    format!("retry stop {}", settled.task_id),
                )
                .unwrap()
                .run_id
                .unwrap();
            completed(&rt, &parent, &retry).await;
            release(&rt, child, None).await;
            completed(&rt, child, &later).await;
            rt.close().await.unwrap();
        })
        .await
        .expect(mode);
    }
}

#[tokio::test]
async fn waiting_for_a_task_handles_completion_timeout_approval_and_revocation() {
    for case in ["ready", "timeout", "approval", "revoke"] {
        timeout(Duration::from_secs(12), async {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path().join("checkout");
            checkout(&repo);
            let rt = Huggler::open(options(&dir.path().join("state")))
                .await
                .unwrap();
            let project = rt.open_project(&repo).unwrap();
            let parent = launch(&rt, &project, "author", Provider::ClaudeAgent);
            let run = rt
                .send(
                    CommandId("start".into()),
                    parent.clone(),
                    format!("wait {case}"),
                )
                .unwrap()
                .run_id
                .unwrap();
            let child = task(&rt, &parent, 0, |_| true).await.child_thread_id;
            if case == "revoke" {
                assert!(rt.revoke_agent_tools(&parent, &run).unwrap());
            }
            completed(&rt, &parent, &run).await;
            let snapshot = rt.read(&parent).unwrap();
            assert_eq!(
                snapshot.messages.last().unwrap().text,
                format!("wait returned {case}")
            );
            assert_eq!(snapshot.tasks.len(), 1);
            let delegated = &snapshot.tasks[0];
            assert_eq!(snapshot.runs.len(), 1);
            if case == "ready" {
                assert_eq!(delegated.result.as_deref(), Some("2+2=4"));
                assert_eq!(delegated.delivery, CompletionDelivery::Acknowledged);
                assert!(delegated.wake_run_id.is_none());
            } else {
                assert_eq!(delegated.delivery, CompletionDelivery::Pending);
                if case == "approval" {
                    release(&rt, &child, None).await;
                } else {
                    std::fs::write(repo.join("release-review"), "").unwrap();
                }
                let settled = task(&rt, &parent, 0, |t| t.wake_run_id.is_some()).await;
                completed(&rt, &parent, settled.wake_run_id.as_ref().unwrap()).await;
                assert_eq!(settled.result.as_deref(), Some("2+2=4"));
                assert_eq!(rt.read(&parent).unwrap().runs.len(), 2);
            }
            rt.close().await.unwrap();
        })
        .await
        .expect(case);
    }
}

#[tokio::test]
async fn completion_wakes_once_after_idle_or_restart_and_groups_siblings() {
    for mode in ["idle", "recover", "siblings", "nested"] {
        timeout(
            Duration::from_secs(15),
            scenario(mode, Provider::ClaudeAgent, Provider::ClaudeAgent),
        )
        .await
        .expect(mode);
    }
}

#[tokio::test]
async fn only_the_parent_can_acknowledge_or_cancel_delivery() {
    for mode in ["ack", "cancel"] {
        timeout(
            Duration::from_secs(15),
            scenario(mode, Provider::ClaudeAgent, Provider::ClaudeAgent),
        )
        .await
        .expect(mode);
    }
}

#[tokio::test]
#[ignore = "uses two live Codex sessions with an authenticated local CLI"]
async fn live_codex_delegation() {
    timeout(
        Duration::from_secs(240),
        scenario("idle", Provider::Codex, Provider::Codex),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "uses a live Codex parent with the scripted ACP reviewer"]
async fn live_codex_to_acp_delegation() {
    timeout(
        Duration::from_secs(240),
        scenario("idle", Provider::Codex, Provider::ClaudeAgent),
    )
    .await
    .unwrap();
}
