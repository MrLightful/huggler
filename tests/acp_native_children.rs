#![cfg(unix)]
#[path = "support/agents.rs"]
mod agents;

use huggler::*;
use std::{path::Path, time::Duration};
use tokio::time::timeout;

const RELAY: &str = "agent-42";
const WRITER: &str = "agent-99";

async fn setup(dir: &Path) -> (HugglerOptions, Huggler, ThreadId) {
    let mut options = HugglerOptions::new(dir.join("state"));
    options.claude_agent.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-children-acp.py");
    let rt = Huggler::open(options.clone()).await.unwrap();
    let repo = dir.join("repo");
    agents::checkout(&repo);
    let project = rt.open_project(repo).unwrap();
    let thread = agents::launch(&rt, &project, "author", Provider::ClaudeAgent);
    (options, rt, thread)
}

fn send(rt: &Huggler, thread: &ThreadId, prompt: &str) -> RunId {
    rt.send(CommandId(prompt.into()), thread.clone(), prompt.into())
        .unwrap()
        .run_id
        .unwrap()
}

async fn permission(events: &mut Subscription, title: &str) -> PermissionRequest {
    loop {
        if let Change::PermissionRequested { request } = events.next().await.unwrap().change {
            assert_eq!(request.tool_call["title"], title);
            return request;
        }
    }
}

async fn closed(events: &mut Subscription, request: &PermissionRequest) {
    loop {
        if let Change::PermissionClosed { id, option } = events.next().await.unwrap().change
            && id == request.id
        {
            assert_eq!(option, None);
            return;
        }
    }
}

async fn wait_child(events: &mut Subscription, session: &str, status: NativeChildStatus) {
    loop {
        if let Change::NativeChild { child } = events.next().await.unwrap().change
            && child.thread_id.0 == session
            && child.status == status
        {
            return;
        }
    }
}

fn child<'a>(state: &'a Thread, session: &str) -> &'a NativeChild {
    state
        .native_children
        .iter()
        .find(|c| c.thread_id.0 == session)
        .unwrap()
}

#[tokio::test]
async fn nested_child_permissions_and_cancellation_outlive_the_foreground() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let (options, rt, thread) = setup(dir.path()).await;
        let (_, mut events) = rt.subscribe(&thread, None).unwrap();
        let run = send(&rt, &thread, "background");
        let writer_request = permission(&mut events, "old request").await;
        assert_eq!(writer_request.run_id, run);
        assert_eq!(
            writer_request.native_child,
            Some(NativeChildTurn {
                thread_id: NativeThreadId(WRITER.into()),
                turn_id: None
            })
        );
        let root_request = permission(&mut events, "finish root").await;
        assert!(root_request.native_child.is_none());
        let state = rt.read(&thread).unwrap();
        assert_eq!(state.native_children.len(), 3);
        let writer = child(&state, WRITER);
        let relay = child(&state, RELAY);
        assert_eq!(writer.parent_thread_id, relay.thread_id);
        assert_eq!(Some(&relay.parent_thread_id), state.native_thread.as_ref());
        assert_eq!(writer.path, "Writer");
        assert_eq!(writer.status, NativeChildStatus::RequiresAction);
        assert!(writer.can_interrupt);
        for c in &state.native_children {
            assert_eq!(c.run_id, run);
            assert!(c.turn_id.is_none() && c.parent_turn_id.is_none());
        }
        assert_eq!(child(&state, "orphaned").status, NativeChildStatus::Unknown);
        assert!(!child(&state, "orphaned").can_interrupt);
        for (owner, session, turn) in [
            (&run, "orphaned", None),
            (&run, "stranger", None),
            (&run, "root", None),
            (&RunId("wrong".into()), WRITER, None),
            (&run, WRITER, Some(&NativeTurnId("invented".into()))),
        ] {
            assert!(matches!(
                rt.interrupt_native_child(&thread, owner, &NativeThreadId(session.into()), turn)
                    .await,
                Err(Error::NotFound(_))
            ));
        }

        rt.respond_to_permission(&thread, &run, &root_request.id, "yes".into())
            .unwrap();
        agents::completed(&rt, &thread, &run).await;
        assert_eq!(
            rt.read(&thread).unwrap().messages.last().unwrap().text,
            "root answer"
        );
        rt.respond_to_permission(&thread, &run, &writer_request.id, "yes".into())
            .unwrap();
        let withdrawn = permission(&mut events, "withdrawn").await;
        closed(&mut events, &withdrawn).await;
        assert!(matches!(
            rt.respond_to_permission(&thread, &run, &withdrawn.id, "yes".into()),
            Err(Error::NotFound(_))
        ));
        let background = permission(&mut events, "background request").await;
        assert_eq!(background.native_child, writer_request.native_child);
        rt.respond_to_permission(&thread, &run, &background.id, "yes".into())
            .unwrap();
        wait_child(&mut events, WRITER, NativeChildStatus::Running).await;
        rt.interrupt_native_child(&thread, &run, &writer.thread_id, None)
            .await
            .unwrap();
        let finish = permission(&mut events, "finish relay").await;
        assert_eq!(finish.native_child.unwrap().thread_id, relay.thread_id);
        assert_eq!(
            child(&rt.read(&thread).unwrap(), WRITER).status,
            NativeChildStatus::Running
        );
        rt.respond_to_permission(&thread, &run, &finish.id, "yes".into())
            .unwrap();
        wait_child(&mut events, RELAY, NativeChildStatus::Completed).await;
        rt.close().await.unwrap();
        let rt = Huggler::open(options).await.unwrap();
        let state = rt.read(&thread).unwrap();
        assert_eq!(state.runs.len(), 1);
        assert_eq!(state.runs[0].status, RunStatus::Completed);
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].text, "root answer");
        assert!(state.permissions.is_empty());
        assert_eq!(child(&state, WRITER).status, NativeChildStatus::Interrupted);
        assert_eq!(child(&state, RELAY).result.as_deref(), Some("relay report"));
        assert!(state.native_children.iter().all(|c| !c.can_interrupt));
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn provider_loss_expires_a_background_childs_permission() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let (_, rt, thread) = setup(dir.path()).await;
        let (_, mut events) = rt.subscribe(&thread, None).unwrap();
        let run = send(&rt, &thread, "lost");
        let lost = permission(&mut events, "lost request").await;
        closed(&mut events, &lost).await;
        let state = rt.read(&thread).unwrap();
        assert_eq!(state.native_children.len(), 2);
        assert!(
            state
                .native_children
                .iter()
                .all(|c| c.status == NativeChildStatus::Disconnected && !c.can_interrupt)
        );
        assert!(state.permissions.is_empty());
        assert!(matches!(
            rt.respond_to_permission(&thread, &run, &lost.id, "yes".into()),
            Err(Error::NotFound(_))
        ));
        agents::completed(&rt, &thread, &run).await;
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn resumed_child_keeps_its_lineage_and_each_runs_own_answer() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let (options, rt, thread) = setup(dir.path()).await;
        let first = send(&rt, &thread, "first");
        agents::completed(&rt, &thread, &first).await;
        rt.close().await.unwrap();
        let rt = Huggler::open(options).await.unwrap();
        let (_, mut events) = rt.subscribe(&thread, None).unwrap();
        let second = send(&rt, &thread, "resume");
        let inspect = permission(&mut events, "inspect resumed writer").await;
        let state = rt.read(&thread).unwrap();
        let writer = state
            .native_children
            .iter()
            .find(|c| c.run_id == second)
            .unwrap();
        assert_eq!(writer.thread_id.0, WRITER);
        assert_eq!(writer.status, NativeChildStatus::Running);
        assert!(writer.can_interrupt);
        rt.respond_to_permission(&thread, &second, &inspect.id, "yes".into())
            .unwrap();
        agents::completed(&rt, &thread, &second).await;
        let state = rt.read(&thread).unwrap();
        assert_eq!(state.messages.len(), 4);
        assert_eq!(state.messages[1].text, "first");
        assert_eq!(state.messages[3].text, "resume");
        assert_eq!(state.native_children.len(), 3);
        assert_eq!(state.native_children[0].thread_id.0, RELAY);
        for (writer, owner, result) in [
            (&state.native_children[1], &first, "first finding"),
            (&state.native_children[2], &second, "resume finding"),
        ] {
            assert_eq!(writer.thread_id.0, WRITER);
            assert_eq!(writer.parent_thread_id.0, RELAY);
            assert_eq!(writer.path, "Writer");
            assert_eq!(&writer.run_id, owner);
            assert_eq!(writer.status, NativeChildStatus::Completed);
            assert_eq!(writer.result.as_deref(), Some(result));
        }
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn child_updates_can_remove_cancel_capability_or_settle_without_a_reason() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let (_, rt, thread) = setup(dir.path()).await;
        let (_, mut events) = rt.subscribe(&thread, None).unwrap();
        let run = send(&rt, &thread, "states");
        let inspect = permission(&mut events, "inspect writer").await;
        let state = rt.read(&thread).unwrap();
        let writer = child(&state, WRITER);
        assert_eq!(writer.status, NativeChildStatus::Running);
        assert_eq!(writer.path, WRITER);
        assert!(!writer.can_interrupt);
        assert!(matches!(
            rt.interrupt_native_child(&thread, &run, &writer.thread_id, None)
                .await,
            Err(Error::NotFound(_))
        ));
        rt.respond_to_permission(&thread, &run, &inspect.id, "yes".into())
            .unwrap();
        let expired = permission(&mut events, "ended request").await;
        closed(&mut events, &expired).await;
        loop {
            if let Change::NativeChild { child } = events.next().await.unwrap().change {
                assert_eq!(child.status, NativeChildStatus::Idle);
                assert!(!child.can_interrupt);
                break;
            }
        }
        agents::completed(&rt, &thread, &run).await;
        let state = rt.read(&thread).unwrap();
        let writer = child(&state, WRITER);
        assert_eq!(writer.status, NativeChildStatus::Failed);
        assert_eq!(writer.result.as_deref(), Some("review failed"));
        assert!(state.permissions.is_empty());
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}
