#![cfg(unix)]
#[path = "support/agents.rs"]
mod agents;
use agents::*;
#[path = "support/reviewer.rs"]
mod reviewer;
use huggler::*;
use reviewer::{Reviewer, allow_once, requested_huggler_tool};
use std::{path::Path, time::Duration};
use tokio::time::timeout;

async fn permission(rt: &Huggler, thread: &ThreadId) -> PermissionRequest {
    let (snapshot, mut events) = rt.subscribe(thread, None).unwrap();
    if let Some(request) = snapshot.permissions.into_iter().next() {
        return request;
    }
    if !snapshot.runs.is_empty() && snapshot.runs.iter().all(|r| r.status.terminal()) {
        panic!(
            "expected a pending permission on {thread}, got {:?}",
            snapshot.runs.last()
        );
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
        assert_eq!(
            requested_huggler_tool(&request, rt.read(thread).unwrap().provider),
            tool
        );
        allow_once(&request)
    } else {
        "done".into()
    };
    rt.respond_to_permission(thread, &request.run_id, &request.id, choice)
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

async fn scenario(mode: &str, parent_provider: Provider, reviewer: Reviewer) {
    let child_provider = reviewer.provider();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("checkout");
    checkout(&repo);
    let mut options = options(&dir.path().join("state"));
    reviewer.configure(&mut options, &repo);
    let rt = Huggler::open(options.clone()).await.unwrap();
    let project = rt.open_project(&repo).unwrap();
    let parent = launch(&rt, &project, "author", parent_provider);
    let prompt = if parent_provider == Provider::Codex {
        let instance = child_provider.key();
        let review = "First call the huggler thread_list tool once. When it returns, finish the turn by copying the five-character review finding verbatim as plain text: 2+2=4";
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
                    (parent_provider == Provider::Codex).then_some("thread_list"),
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
                    environment_id: None,
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
async fn stop_thread_holds_owned_queues_across_reopen_until_each_resumes() {
    #[derive(Debug)]
    struct StudioStops;
    impl AgentText for StudioStops {
        fn label(&self, text: &str) -> String {
            match text {
                "task cancelled" => "Assignment withdrawn".into(),
                "Stopped by the host" => "Studio stopped this turn".into(),
                _ => text.into(),
            }
        }
    }
    timeout(Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("checkout");
        checkout(&repo);
        let mut options = options(&dir.path().join("state"));
        options.output.text = std::sync::Arc::new(StudioStops);
        let rt = Huggler::open(options.clone()).await.unwrap();
        let project = rt.open_project(&repo).unwrap();
        let parent = launch(&rt, &project, "author", Provider::ClaudeAgent);
        let peer = launch(&rt, &project, "peer", Provider::ClaudeAgent);
        let send = |target: &ThreadId, key: &str, prompt: &str| {
            rt.send(CommandId(key.into()), target.clone(), prompt.into())
                .unwrap()
                .run_id
                .unwrap()
        };

        let first = send(&parent, "branch", "start branch review");
        completed(&rt, &parent, &first).await;
        let branch = task(&rt, &parent, 0, |_| true).await;
        let child = &branch.child_thread_id;
        let leaf = task(&rt, child, 0, |_| true).await;
        let grandchild = &leaf.child_thread_id;
        permission(&rt, child).await;
        permission(&rt, grandchild).await;
        let revision = rt.peer_receipt(child, "revise").unwrap().unwrap().run_id;

        // The stop must reach the first turn's descendants and preserve a
        // completed result awaiting delivery to the active second turn.
        let active = send(&parent, "settled", "delegate settled");
        permission(&rt, &parent).await;
        let review = task(&rt, &parent, 1, |t| t.status.terminal()).await;
        assert_eq!(review.delivery, CompletionDelivery::Pending);
        let peer_run = send(&peer, "queue", "queue-two");
        let peer_request = permission(&rt, &peer).await;
        let queued =
            ["first", "second"].map(|key| rt.peer_receipt(&peer, key).unwrap().unwrap().run_id);
        let old_resume = rt
            .resume_queue(CommandId("early".into()), parent.clone())
            .unwrap();
        let stop = rt
            .stop_thread(CommandId("stop".into()), parent.clone())
            .unwrap();
        for (owner, run) in [
            (&parent, &active),
            (child, &branch.child_run_id),
            (grandchild, &leaf.child_run_id),
        ] {
            let result = rt.wait(owner, run).await.unwrap();
            assert_eq!(result.status, RunStatus::Interrupted);
            assert_eq!(result.error.as_deref(), Some("Studio stopped this turn"));
            assert!(rt.read(owner).unwrap().permissions.is_empty());
        }
        let peer_snapshot = rt.read(&peer).unwrap();
        assert!(matches!(peer_snapshot.permissions.as_slice(), [request] if request.id == peer_request.id && request.run_id == peer_run));
        rt.revoke_agent_tools(&peer, &peer_run).unwrap();
        release(&rt, &peer, None).await;
        completed(&rt, &peer, &peer_run).await;

        let saved = rt.read(&parent).unwrap();
        assert_eq!(saved.runs.len(), 4);
        assert_eq!(saved.runs[0].status, RunStatus::Completed);
        assert_eq!(
            saved.tasks[0].result_run_id.as_ref(),
            Some(&branch.child_run_id)
        );
        assert_eq!(saved.tasks[0].status, RunStatus::Interrupted);
        assert_eq!(saved.tasks[0].result.as_deref(), Some("Assignment withdrawn"));
        assert_eq!(saved.tasks[1].result_run_id, review.result_run_id);
        assert_eq!(saved.tasks[1].result.as_deref(), Some("2+2=4"));
        assert!(
            saved
                .tasks
                .iter()
                .all(|t| t.delivery == CompletionDelivery::Disposed && t.wake_run_id.is_none())
        );
        let nested = rt.read(child).unwrap().tasks.remove(0);
        assert_eq!(nested.result_run_id.as_ref(), Some(&leaf.child_run_id));
        assert_eq!(nested.delivery, CompletionDelivery::Disposed);
        rt.close().await.unwrap();

        options.output = AgentOutput::default();
        let rt = Huggler::open(options).await.unwrap();
        assert_eq!(
            rt.resume_queue(old_resume.command_id.clone(), parent.clone())
                .unwrap(),
            old_resume
        );
        let reopened = rt.read(&parent).unwrap();
        assert_eq!(reopened.tasks[0].result, saved.tasks[0].result);
        assert_eq!(reopened.cursor, saved.cursor);
        for (run, expected) in reopened.runs[2..].iter().zip(&queued) {
            assert_eq!(&run.id, expected);
            assert!(run.queue_held && run.status == RunStatus::Queued);
        }
        let resume = rt
            .resume_queue(CommandId("resume".into()), parent.clone())
            .unwrap();
        assert_eq!(resume.run_id.as_ref(), Some(&queued[0]));
        for run in &queued {
            completed(&rt, &parent, run).await;
        }
        let resumed = rt.read(&parent).unwrap();
        let answers: Vec<_> = resumed
            .messages
            .iter()
            .filter(|m| m.role == Role::Assistant && queued.contains(&m.run_id))
            .map(|m| (&m.run_id, m.text.as_str()))
            .collect();
        assert_eq!(
            answers,
            [
                (&queued[0], "Received first"),
                (&queued[1], "Received second")
            ]
        );
        assert_eq!(resumed.tasks, saved.tasks);
        assert_eq!(resumed.runs.len(), 4);
        let held = rt.read(grandchild).unwrap().runs.pop().unwrap();
        assert_eq!(held.id, revision);
        assert!(held.queue_held && held.status == RunStatus::Queued);

        let later = rt
            .send(CommandId("later".into()), parent.clone(), "hold".into())
            .unwrap()
            .run_id
            .unwrap();
        permission(&rt, &parent).await;
        assert_eq!(
            rt.stop_thread(stop.command_id.clone(), parent.clone())
                .unwrap(),
            stop
        );
        release(&rt, &parent, None).await;
        completed(&rt, &parent, &later).await;
        rt.resume_queue(CommandId("revise".into()), grandchild.clone())
            .unwrap();
        completed(&rt, grandchild, &revision).await;
        assert_eq!(
            rt.read(grandchild).unwrap().messages.last().unwrap().text,
            "Received revised finding"
        );
        assert_eq!(rt.read(child).unwrap().tasks, [nested]);
        assert_eq!(rt.read(&parent).unwrap().tasks, saved.tasks);
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
            assert_eq!(
                rt.read(&parent).unwrap().tasks,
                std::slice::from_ref(&settled)
            );
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
            scenario(mode, Provider::ClaudeAgent, Reviewer::ScriptedAcp),
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
            scenario(mode, Provider::ClaudeAgent, Reviewer::ScriptedAcp),
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
        scenario("idle", Provider::Codex, Reviewer::Codex),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "uses a live Codex parent with the scripted ACP reviewer"]
async fn live_codex_to_acp_delegation() {
    timeout(
        Duration::from_secs(240),
        scenario("idle", Provider::Codex, Reviewer::ScriptedAcp),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "uses a live Codex parent and an authenticated standalone Claude Agent ACP reviewer"]
async fn live_codex_to_claude_delegation() {
    timeout(
        Duration::from_secs(240),
        scenario("idle", Provider::Codex, Reviewer::ClaudeAgent),
    )
    .await
    .unwrap();
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim_end().into()
}

async fn review_round(rt: &Huggler, parent: &ThreadId, key: &str, prompt: String) -> DelegatedTask {
    let (before, mut events) = rt.subscribe(parent, None).unwrap();
    let run = rt
        .send(CommandId(key.into()), parent.clone(), prompt)
        .unwrap()
        .run_id
        .unwrap();
    let mut delegated = false;
    let settle = async {
        completed(rt, parent, &run).await;
        let snapshot = rt.read(parent).unwrap();
        assert_eq!(
            snapshot.tasks.len(),
            before.tasks.len() + 1,
            "{:?}",
            snapshot.messages.last()
        );
        let result = task(rt, parent, before.tasks.len(), |t| {
            t.status.terminal() && t.delivery != CompletionDelivery::Pending
        })
        .await;
        assert_eq!(result.parent_thread_id, *parent);
        assert_eq!(result.parent_run_id, run);
        assert_eq!(result.status, RunStatus::Completed, "{result:?}");
        assert_eq!(result.result_run_id.as_ref(), Some(&result.child_run_id));
        match result.delivery {
            CompletionDelivery::Acknowledged => assert!(result.wake_run_id.is_none()),
            CompletionDelivery::Delivered => {
                completed(rt, parent, result.wake_run_id.as_ref().unwrap()).await
            }
            other => panic!("unexpected review delivery {other:?}"),
        }
        result
    };
    tokio::pin!(settle);
    loop {
        tokio::select! {
            result = &mut settle => {
                assert!(delegated, "{key} did not call delegate_task");
                return result;
            }
            event = events.next() => {
                if let Change::PermissionRequested { request } = event.unwrap().change {
                    let input = &request.tool_call["rawInput"];
                    assert_eq!(input["serverName"], "huggler");
                    match input["_meta"]["tool_name"].as_str() {
                        Some("delegate_task") => {
                            assert_eq!(request.run_id, run);
                            delegated = true;
                        }
                        Some("task_status" | "thread_list") => {}
                        other => panic!("{key} chose {other:?}"),
                    }
                    rt.respond_to_permission(parent, &request.run_id, &request.id, "accept".into()).unwrap();
                }
            }
        }
    }
}

async fn delegated_review_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("checkout");
    checkout(&repo);
    let cache = repo.join("cache.py");
    let original = "def decode_usage(entry):
    return entry['usage'] if entry['version'] == 5 else {}

def cache_path(directory, app_version):
    return directory / 'usage.json'
";
    std::fs::write(&cache, original).unwrap();
    git(&repo, &["add", "cache.py"]);
    git(&repo, &["commit", "-qm", "Persist usage per file"]);

    let options = options(&dir.path().join("state"));
    let rt = Huggler::open(options.clone()).await.unwrap();
    let project = rt.open_project(&repo).unwrap();
    let author = launch(&rt, &project, "author", Provider::Codex);
    let first = review_round(
        &rt,
        &author,
        "round-one",
        "Have a delegated Codex reviewer in Huggler review cache.py, read-only. \
         Requirements: v4 and v5 entries must retain usage even for deleted files, and \
         different app versions must be able to persist usage independently in the same directory. \
         Identify affected cache filenames and versions in the findings. \
         Wait for its verdict. Report the findings and stop until I request another round."
            .into(),
    )
    .await;
    let fixed = original
        .replace("== 5", "in (4, 5)")
        .replace("'usage.json'", "f'usage-{app_version}.json'");
    std::fs::write(&cache, fixed).unwrap();
    git(
        &repo,
        &[
            "commit",
            "-qam",
            "Read v4 usage and separate cache files by app version",
        ],
    );
    let fix = git(&repo, &["rev-parse", "--short", "HEAD"]);
    let second = review_round(
        &rt,
        &author,
        "round-two",
        format!("Both are fixed in commit {fix}. Run round 2 of the Codex review."),
    )
    .await;
    rt.close().await.unwrap();

    let rt = Huggler::open(options).await.unwrap();
    assert_ne!(first.task_id, second.task_id);
    assert_ne!(first.child_thread_id, second.child_thread_id);
    assert_ne!(first.child_run_id, second.child_run_id);
    assert_ne!(first.parent_run_id, second.parent_run_id);
    for round in [&first, &second] {
        let reviewer = rt.read(&round.child_thread_id).unwrap();
        assert_eq!(reviewer.provider, Provider::Codex);
        assert_eq!(reviewer.project, project.id);
        assert_eq!(reviewer.runs.len(), 1);
        assert_eq!(reviewer.runs[0].id, round.child_run_id);
        let answer = reviewer.messages.last().unwrap();
        assert_eq!(answer.role, Role::Assistant);
        assert_eq!(answer.run_id, round.child_run_id);
        assert_eq!(round.result.as_deref(), Some(answer.text.as_str()));
    }
    let reviewer = rt.read(&second.child_thread_id).unwrap();
    let brief = &reviewer.messages[0];
    assert_eq!(brief.role, Role::User);
    assert_eq!(brief.run_id, second.child_run_id);
    // The follow-up names only the commit. The shared filename comes from the first review.
    for report in [first.result.as_deref().unwrap(), brief.text.as_str()] {
        let text = report.to_lowercase();
        assert!(
            text.contains("v4") || text.contains("version 4"),
            "missing v4 finding: {report}"
        );
        assert!(
            text.contains("usage.json"),
            "missing shared cache filename: {report}"
        );
    }
    assert_eq!(rt.read(&author).unwrap().tasks, [first, second]);
    rt.close().await.unwrap();
}

#[tokio::test]
#[ignore = "uses a live Codex author and two delegated Codex reviewers"]
async fn live_codex_delegated_review_rounds() {
    timeout(Duration::from_secs(360), delegated_review_rounds())
        .await
        .expect("Codex review rounds timed out");
}

async fn acp_prompt(rt: &Huggler, thread: &ThreadId, key: &str, text: &str) -> String {
    let run = rt
        .send(CommandId(key.into()), thread.clone(), text.into())
        .unwrap()
        .run_id
        .unwrap();
    completed(rt, thread, &run).await;
    rt.read(thread)
        .unwrap()
        .messages
        .last()
        .unwrap()
        .text
        .clone()
}

async fn checkout_task(rt: &Huggler, caller: &ThreadId, key: &str, prompt: &str) -> DelegatedTask {
    serde_json::from_str(&acp_prompt(rt, caller, key, prompt).await).unwrap()
}

async fn exercise_worktree_delegation() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repository with spaces");
    checkout(&repo);
    let mut options = options(&dir.path().join("state"));
    options.storage.worktree_root = WorktreeRoot::ProjectRelative(".my-host/checkouts".into());
    options.storage.worktree_name = "agent-{id}".into();
    options.instances.push(ProviderInstance::new(
        "review-profile",
        Provider::ClaudeAgent,
        &options.claude_agent.executable,
    ));
    let rt = Huggler::open(options.clone()).await.unwrap();
    let project = rt.open_project(&repo).unwrap();
    let author = launch(&rt, &project, "author", Provider::ClaudeAgent);
    let isolated = r#"checkout task {"clientRequestId":"isolate","task":"edit tracked isolated","isolation":"worktree","target":{"providerInstanceId":"review-profile"}}"#;
    let rejected: serde_json::Value =
        serde_json::from_str(&acp_prompt(&rt, &author, "unborn", isolated).await).unwrap();
    assert_eq!(rejected["code"], "invalid");
    assert!(rt.read(&author).unwrap().tasks.is_empty());
    assert_eq!(rt.threads().unwrap().len(), 1);

    let tracked = repo.join("tracked.txt");
    std::fs::write(&tracked, "initial").unwrap();
    git(&repo, &["add", "tracked.txt"]);
    git(&repo, &["commit", "-qm", "Base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let shared = r#"checkout task {"clientRequestId":"share","task":"edit tracked shared"}"#;
    let shared = checkout_task(&rt, &author, "shared", shared).await;
    assert_eq!(shared.result.as_deref(), Some("initial"));
    assert!(
        rt.read(&shared.child_thread_id)
            .unwrap()
            .worktree_path
            .is_none()
    );
    assert_eq!(std::fs::read_to_string(&tracked).unwrap(), "shared");

    let task = checkout_task(&rt, &author, "isolated", isolated).await;
    assert_eq!(task.result.as_deref(), Some("initial"));
    let child = &task.child_thread_id;
    let path = rt.read(child).unwrap().worktree_path.unwrap();
    assert_eq!(
        path.parent().unwrap(),
        project.root.join(".my-host/checkouts")
    );
    let folder = path.file_name().unwrap().to_str().unwrap();
    let id = folder.strip_prefix("agent-").unwrap();
    assert!((4..=8).contains(&id.len()));
    assert!(id.bytes().all(|b| b.is_ascii_lowercase()));
    assert!(!options.state_dir.join("worktrees").exists());
    assert_eq!(git(&path, &["rev-parse", "HEAD"]), base);
    assert_eq!(
        std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
        "isolated"
    );
    assert_eq!(std::fs::read_to_string(&tracked).unwrap(), "shared");
    let worktrees = git(&repo, &["worktree", "list", "--porcelain"]);
    assert_eq!(
        worktrees
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count(),
        2
    );
    rt.close().await.unwrap();

    // An accepted retry does not need a resolvable parent HEAD.
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/unborn"]);
    options.storage.worktree_root = WorktreeRoot::ProjectRelative(".my-host/new checkouts".into());
    let rt = Huggler::open(options).await.unwrap();
    let retry = checkout_task(&rt, &author, "retry", isolated).await;
    assert_eq!(retry, task);
    assert_eq!(
        rt.read(child).unwrap().provider_instance.as_str(),
        "review-profile"
    );
    assert_eq!(rt.read(child).unwrap().worktree_path.as_ref(), Some(&path));
    assert_eq!(rt.read(child).unwrap().runs.len(), 1);
    #[cfg(feature = "application")]
    {
        use workspace::SetupStatus;
        let checkout = rt.open_project(&path).unwrap();
        let fail = Path::new("/usr/bin/false");
        let pass = Path::new("/usr/bin/true");
        assert!(matches!(
            rt.setup_workspace(&checkout.id, fail, &[]).await.unwrap(),
            SetupStatus::Failed(_)
        ));
        assert!(matches!(
            rt.send(CommandId("blocked".into()), child.clone(), "hold".into()),
            Err(Error::Invalid(_))
        ));
        acp_prompt(&rt, &author, "root-ready", "echo ready").await;
        assert_eq!(
            rt.setup_workspace(&checkout.id, pass, &[]).await.unwrap(),
            SetupStatus::Ready
        );
        let run = rt
            .send(CommandId("blocked".into()), child.clone(), "hold".into())
            .unwrap()
            .run_id
            .unwrap();
        permission(&rt, child).await;
        assert!(matches!(
            rt.setup_workspace(&checkout.id, pass, &[]).await,
            Err(Error::Busy)
        ));
        assert!(matches!(
            rt.setup_workspace(&project.id, fail, &[]).await.unwrap(),
            SetupStatus::Failed(_)
        ));
        release(&rt, child, None).await;
        completed(&rt, child, &run).await;
        acp_prompt(&rt, child, "child-ready", "echo ready").await;
        assert_eq!(
            rt.setup_workspace(&project.id, pass, &[]).await.unwrap(),
            SetupStatus::Ready
        );
    }
    assert_eq!(
        acp_prompt(&rt, child, "continue", "edit tracked resumed").await,
        "isolated"
    );
    git(&path, &["commit", "-qam", "Child commit"]);
    let child_base = git(&path, &["rev-parse", "HEAD"]);
    assert_ne!(child_base, base);

    // Shared descendants and independent peers use the caller's checkout.
    let inherit = r#"checkout task {"clientRequestId":"shared-child","task":"edit tracked inherited","isolation":"shared"}"#;
    let sub = checkout_task(&rt, child, "inherit", inherit).await;
    assert_eq!(sub.result.as_deref(), Some("resumed"));
    assert_eq!(
        rt.read(&sub.child_thread_id)
            .unwrap()
            .provider_instance
            .as_str(),
        "review-profile"
    );
    assert_eq!(
        rt.read(&sub.child_thread_id)
            .unwrap()
            .worktree_path
            .as_ref(),
        Some(&path)
    );
    let peers: serde_json::Value = serde_json::from_str(
        &acp_prompt(
            &rt,
            child,
            "batch",
            r#"create peers {"clientRequestId":"peer","threads":[{"title":"Peer"}]}"#,
        )
        .await,
    )
    .unwrap();
    let peer = ThreadId(peers["threads"][0]["threadId"].as_str().unwrap().into());
    assert_eq!(
        acp_prompt(&rt, &peer, "peer-edit", "edit tracked peer").await,
        "inherited"
    );

    let nested = checkout_task(&rt, child, "nested", isolated).await;
    let nested_path = rt
        .read(&nested.child_thread_id)
        .unwrap()
        .worktree_path
        .unwrap();
    assert_eq!(
        nested_path.parent().unwrap(),
        project.root.join(".my-host/new checkouts")
    );
    assert_eq!(git(&nested_path, &["rev-parse", "HEAD"]), child_base);
    assert_eq!(nested.result.as_deref(), Some("resumed"));
    assert_eq!(
        std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
        "peer"
    );
    assert_eq!(std::fs::read_to_string(&tracked).unwrap(), "shared");

    // Losing a prepared checkout must not re-create it or dispatch in the parent.
    std::fs::rename(&nested_path, dir.path().join("moved")).unwrap();
    let missing = rt
        .send(
            CommandId("missing".into()),
            nested.child_thread_id.clone(),
            "edit tracked wrong".into(),
        )
        .unwrap()
        .run_id
        .unwrap();
    assert_eq!(
        rt.wait(&nested.child_thread_id, &missing)
            .await
            .unwrap()
            .status,
        RunStatus::Failed
    );
    assert!(!nested_path.exists());
    std::fs::rename(dir.path().join("moved"), &nested_path).unwrap();
    assert_eq!(
        acp_prompt(
            &rt,
            &nested.child_thread_id,
            "repair",
            "edit tracked repaired"
        )
        .await,
        "isolated"
    );
    assert_eq!(
        std::fs::read_to_string(nested_path.join("tracked.txt")).unwrap(),
        "repaired"
    );
    assert_eq!(std::fs::read_to_string(&tracked).unwrap(), "shared");
    rt.close().await.unwrap();
}

#[tokio::test]
async fn worktree_delegation_keeps_the_callers_checkout_and_retries_after_reopen() {
    timeout(Duration::from_secs(30), exercise_worktree_delegation())
        .await
        .expect("worktree delegation");
}

#[tokio::test]
async fn host_queue_edits_survive_reopen_in_delivery_order() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        checkout(&repo);
        let config = options(&dir.path().join("state"));
        let rt = Huggler::open(config.clone()).await.unwrap();
        let project = rt.open_project(&repo).unwrap();
        let thread = launch(&rt, &project, "queue", Provider::ClaudeAgent);
        let cmd = |id: &str, action| Command {
            id: CommandId(id.into()),
            thread_id: thread.clone(),
            action,
        };
        let enqueue = |text: &str| cmd(text, Action::Enqueue { text: text.into() });
        let active = rt.dispatch(enqueue("hold")).unwrap().run_id.unwrap();
        permission(&rt, &thread).await;
        assert!(matches!(
            rt.dispatch(cmd(
                "busy",
                Action::Send {
                    text: "busy".into()
                }
            )),
            Err(Error::Busy)
        ));
        let automatic = cmd(
            "auto",
            Action::SendAuto {
                text: "echo first".into(),
            },
        );
        let received = rt.dispatch(automatic.clone()).unwrap();
        let a = received.run_id.clone().unwrap();
        assert_eq!(
            rt.read(&thread).unwrap().runs.last().unwrap().status,
            RunStatus::Queued
        );
        let b = rt.dispatch(enqueue("echo second")).unwrap().run_id.unwrap();
        rt.dispatch(cmd("stop", Action::StopThread)).unwrap();
        assert_eq!(
            rt.wait(&thread, &active).await.unwrap().status,
            RunStatus::Interrupted
        );
        let c = rt
            .dispatch(enqueue("echo cancelled"))
            .unwrap()
            .run_id
            .unwrap();
        rt.dispatch(cmd("cancel", Action::CancelQueued { run: c.clone() }))
            .unwrap();
        assert_eq!(
            rt.wait(&thread, &c).await.unwrap().status,
            RunStatus::Interrupted
        );
        let edit = cmd(
            "edit",
            Action::EditQueued {
                run: b.clone(),
                text: "echo changed".into(),
            },
        );
        let moved = cmd(
            "move",
            Action::MoveQueued {
                run: b.clone(),
                before: Some(a.clone()),
            },
        );
        let edited = rt.dispatch(edit.clone()).unwrap();
        rt.dispatch(moved.clone()).unwrap();
        rt.close().await.unwrap();
        let rt = Huggler::open(config).await.unwrap();
        assert_eq!(rt.dispatch(edit).unwrap(), edited);
        assert_eq!(
            rt.dispatch(cmd("resume", Action::ResumeQueue))
                .unwrap()
                .run_id,
            Some(b.clone())
        );
        completed(&rt, &thread, &a).await;
        assert_eq!(rt.dispatch(automatic).unwrap(), received);
        assert_eq!(
            std::fs::read_to_string(repo.join("delivery-log")).unwrap(),
            "echo changed\necho first\n"
        );
        assert!(matches!(
            rt.dispatch(cmd(
                "late",
                Action::MoveQueued {
                    run: b,
                    before: None
                }
            )),
            Err(Error::Invalid(_))
        ));
        rt.dispatch(moved).unwrap();
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}
