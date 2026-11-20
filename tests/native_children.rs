#![cfg(unix)]
#[path = "support/codex_replay.rs"]
mod replay;
use huggler::*;
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};
use tokio::time::timeout;

const TRANSCRIPT: &str = include_str!("fixtures/codex-subagents/codex_transcript.ndjson");

fn recording() -> Vec<Value> {
    parse(TRANSCRIPT)
}

fn parse(transcript: &str) -> Vec<Value> {
    transcript
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn start(dir: &Path, records: &[Value]) -> (HugglerOptions, Huggler, ThreadId, RunId) {
    let options = replay::config(dir, records, false);
    let rt = Huggler::open(options.clone()).await.unwrap();
    let repo = dir.join("repo");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&repo)
            .status()
            .unwrap()
            .success()
    );
    let project = rt.open_project(repo).unwrap();
    let thread = ThreadId("author".into());
    let prompt = &records
        .iter()
        .find(|r| r["type"] == "expect_outbound" && r["label"] == "turn/start")
        .unwrap()["frame"]["params"]["input"][0]["text"];
    rt.launch(
        CommandId("launch".into()),
        thread.clone(),
        project.id,
        format!(
            "Replay fixture: {}",
            records[0]["scenario"].as_str().unwrap()
        ),
        Some("gpt-6-luna".into()),
    )
    .unwrap();
    let run = rt
        .send(
            CommandId("send".into()),
            thread.clone(),
            prompt.as_str().unwrap().into(),
        )
        .unwrap()
        .run_id
        .unwrap();
    (options, rt, thread, run)
}

#[tokio::test]
async fn recorded_nested_children_retain_their_ancestry_and_results() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let records = recording();
        let (options, rt, thread, run) = start(dir.path(), &records).await;
        let result = rt.wait(&thread, &run).await.unwrap();
        assert_eq!(result.status, RunStatus::Completed, "{:?}", result.error);
        let snapshot = rt.read(&thread).unwrap();
        assert_eq!(snapshot.runs.len(), 1);
        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.messages[1].text, "Hello.");
        assert_eq!(snapshot.native_children.len(), 3);
        let mut parent = snapshot.native_thread.as_ref().unwrap();
        let mut parent_turn = result.native_turn.as_ref().unwrap();
        let mut turns = std::collections::HashSet::from([parent_turn.clone()]);
        for (child, path) in snapshot.native_children.iter().zip([
            "/root/chain_level_1",
            "/root/chain_level_1/chain_level_2",
            "/root/chain_level_1/chain_level_2/say_hello",
        ]) {
            assert_eq!(&child.parent_thread_id, parent);
            assert_eq!(&child.parent_turn_id, parent_turn);
            assert_eq!(&child.run_id, &run);
            assert_eq!(child.path, path);
            assert!(turns.insert(child.turn_id.clone().unwrap()));
            assert_eq!(child.status, NativeChildStatus::Completed);
            assert_eq!(child.result.as_deref(), Some("Hello."));
            parent = &child.thread_id;
            parent_turn = child.turn_id.as_ref().unwrap();
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("cursor")).unwrap(),
            (records.len() - 1).to_string()
        );
        rt.close().await.unwrap();
        let rt = Huggler::open(options).await.unwrap();
        let (reopened, mut replay) = rt.subscribe(&thread, Some(0)).unwrap();
        assert_eq!(reopened.native_children, snapshot.native_children);
        assert_eq!(reopened.cursor, snapshot.cursor);
        let mut completed = vec![];
        loop {
            let event = replay.next().await.unwrap();
            if let Change::NativeChild { child } = event.change {
                if child.status == NativeChildStatus::Completed {
                    completed.push(child);
                }
            }
            if event.sequence == reopened.cursor {
                break;
            }
        }
        completed.reverse();
        assert_eq!(completed, reopened.native_children);
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

// Keep the capture's preamble and first child, move the root's answer ahead
// of the child's, then gate the child's outcome on an observable host action.
fn after_root(interrupt: bool) -> Vec<Value> {
    let source = recording();
    let started = source
        .iter()
        .filter(|e| e["label"] == "turn/started")
        .take(2)
        .collect::<Vec<_>>();
    let root = &started[0]["frame"]["params"]["threadId"];
    let child = &started[1]["frame"]["params"]["threadId"];
    let turn = &started[1]["frame"]["params"]["turn"]["id"];
    let mut rows: Vec<_> = source
        .iter()
        .take_while(|e| *e != started[1])
        .cloned()
        .collect();
    rows.push(started[1].clone());
    let final_frames = |thread: &Value| {
        source
            .iter()
            .filter(|e| {
                let f = &e["frame"];
                f["params"]["threadId"] == *thread
                    && (f["method"] == "turn/completed"
                        || (f["method"] == "item/completed"
                            && f["params"]["item"]["phase"] == "final_answer"))
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    rows.extend(final_frames(root));
    rows.push(json!({"type":"await_release"}));
    let mut finish = final_frames(child);
    if interrupt {
        rows.push(
            json!({"type":"expect_outbound","frame":{"id":99,"method":"turn/interrupt",
            "params":{"threadId":child,"turnId":turn}}}),
        );
        let mut stopped = finish.pop().unwrap();
        stopped["frame"]["params"]["turn"]["status"] = json!("interrupted");
        rows.push(stopped);
        // A provider can settle the turn before acknowledging the interrupt.
        rows.push(json!({"type":"emit_inbound","frame":{"id":99,"result":{}}}));
    } else {
        rows.extend(finish);
    }
    rows.push(source.last().unwrap().clone());
    rows
}

#[tokio::test]
async fn background_child_can_finish_or_be_interrupted_after_root_completion() {
    for interrupt in [false, true] {
        timeout(Duration::from_secs(15), async {
            let dir = tempfile::tempdir().unwrap();
            let records = after_root(interrupt);
            let (options, rt, thread, run) = start(dir.path(), &records).await;
            let root = rt.wait(&thread, &run).await.unwrap();
            assert_eq!(root.status, RunStatus::Completed, "{:?}", root.error);
            let (snapshot, mut events) = rt.subscribe(&thread, None).unwrap();
            let child = &snapshot.native_children[0];
            assert_eq!(child.status, NativeChildStatus::Running);
            let turn = child.turn_id.as_ref().unwrap();
            assert!(matches!(
                rt.interrupt_native_child(&ThreadId("other".into()), &run, &child.thread_id, turn)
                    .await,
                Err(Error::NotFound(_))
            ));
            assert!(matches!(
                rt.interrupt_native_child(
                    &thread,
                    &run,
                    &child.thread_id,
                    &NativeTurnId("old".into())
                )
                .await,
                Err(Error::NotFound(_))
            ));
            fs::write(dir.path().join("release"), "").unwrap();
            let expected = if interrupt {
                rt.interrupt_native_child(&thread, &run, &child.thread_id, turn)
                    .await
                    .unwrap();
                NativeChildStatus::Interrupted
            } else {
                NativeChildStatus::Completed
            };
            loop {
                if let Change::NativeChild { child } = events.next().await.unwrap().change {
                    if child.status == expected {
                        break;
                    }
                }
            }
            rt.close().await.unwrap();
            let rt = Huggler::open(options).await.unwrap();
            let state = rt.read(&thread).unwrap();
            assert_eq!(state.runs, [root]);
            assert_eq!(state.messages, snapshot.messages);
            assert_eq!(state.native_children.len(), 1);
            let child = &state.native_children[0];
            assert_eq!(child.run_id, run);
            assert_eq!(child.status, expected);
            assert_eq!(child.result.as_deref(), (!interrupt).then_some("Hello."));
            rt.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn provider_loss_closes_a_background_approval_and_preserves_the_root_result() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let mut records = approval_recording(Some("emit_inbound"));
        // A request on the right native thread but the wrong turn must be
        // rejected on the wire before the real one reaches the host.
        let index = approval_index(&records, "emit_inbound");
        let mut stale = records[index].clone();
        stale["frame"]["id"] = json!(17);
        stale["frame"]["params"]["turnId"] = json!("stale-turn");
        let rejected = json!({"type":"expect_outbound","frame":{"id":17,"error":{
            "code":-32602,"message":"No matching interactive run"
        }}});
        records.splice(index..index, [stale, rejected]);
        let reply_index = approval_index(&records, "expect_outbound");
        records.truncate(reply_index);
        records.push(json!({"type":"await_release"}));
        records.push(json!({"type":"runtime_exit"}));
        let (options, rt, thread, run) = start(dir.path(), &records).await;
        let root = rt.wait(&thread, &run).await.unwrap();
        assert_eq!(root.status, RunStatus::Completed);
        let (snapshot, mut events) = rt.subscribe(&thread, Some(0)).unwrap();
        let request = loop {
            if let Change::PermissionRequested { request } = events.next().await.unwrap().change {
                break request;
            }
        };
        assert_eq!(snapshot.native_children.len(), 2);
        assert_eq!(
            request.native_child.as_ref().unwrap().thread_id,
            snapshot.native_children[1].thread_id
        );
        fs::write(dir.path().join("release"), "").unwrap();
        loop {
            if let Change::PermissionClosed { id, option } = events.next().await.unwrap().change {
                assert_eq!(id, request.id);
                assert!(option.is_none());
                break;
            }
        }
        assert!(matches!(
            rt.respond_to_permission(&thread, &run, &request.id, "accept".into()),
            Err(Error::NotFound(_))
        ));
        assert!(rt.read(&thread).unwrap().permissions.is_empty());
        rt.close().await.unwrap();
        let rt = Huggler::open(options).await.unwrap();
        let thread = rt.read(&thread).unwrap();
        assert_eq!(thread.runs, [root]);
        assert_eq!(thread.messages, snapshot.messages);
        assert!(thread.permissions.is_empty());
        assert_eq!(thread.native_children.len(), 2);
        for child in thread.native_children {
            assert_eq!(child.status, NativeChildStatus::Disconnected);
            assert!(child.result.is_none());
        }
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}

fn approval_index(records: &[Value], direction: &str) -> usize {
    records
        .iter()
        .position(|r| {
            r["type"] == direction && r["label"] == "item/commandExecution/requestApproval"
        })
        .unwrap()
}

// Optionally move foreground completion immediately before the request or its reply.
fn approval_recording(finish_root_before: Option<&str>) -> Vec<Value> {
    let mut records = parse(include_str!(
        "fixtures/codex-subagents/nested-approval.ndjson"
    ));
    if let Some(direction) = finish_root_before {
        let root = records
            .iter()
            .find(|r| r["label"] == "turn/started")
            .unwrap()["frame"]["params"]["threadId"]
            .clone();
        let root_answer: Vec<_> = records
            .extract_if(.., |r| {
                let p = &r["frame"]["params"];
                p["threadId"] == root
                    && (r["label"] == "turn/completed"
                        || (r["label"] == "item/completed" && p["item"]["phase"] == "final_answer"))
            })
            .collect();
        assert_eq!(root_answer.len(), 2);
        let index = approval_index(&records, direction);
        records.splice(index..index, root_answer);
    }
    records
}

#[tokio::test]
async fn grandchild_approval_reaches_its_provider_turn_even_after_the_root_finishes() {
    for finish_root_before in [None, Some("expect_outbound")] {
        timeout(Duration::from_secs(15), async {
            let dir = tempfile::tempdir().unwrap();
            let records = approval_recording(finish_root_before);
            let (options, rt, thread, run) = start(dir.path(), &records).await;
            let (_, mut events) = rt.subscribe(&thread, Some(0)).unwrap();
            let request = loop {
                if let Change::PermissionRequested { request } = events.next().await.unwrap().change
                {
                    break request;
                }
            };
            if finish_root_before.is_some() {
                assert_eq!(
                    rt.wait(&thread, &run).await.unwrap().status,
                    RunStatus::Completed
                );
            }
            let snapshot = rt.read(&thread).unwrap();
            assert_eq!(snapshot.native_children.len(), 2);
            let grandchild = &snapshot.native_children[1];
            let owner = request.native_child.as_ref().unwrap();
            assert_eq!(request.run_id, run);
            assert_eq!(grandchild.thread_id, owner.thread_id);
            assert_eq!(grandchild.turn_id.as_ref(), Some(&owner.turn_id));
            assert_eq!(snapshot.permissions[0].id, request.id);
            assert!(
                rt.respond_to_permission(
                    &thread,
                    &RunId("other".into()),
                    &request.id,
                    "accept".into()
                )
                .is_err()
            );
            rt.respond_to_permission(&thread, &run, &request.id, "accept".into())
                .unwrap();
            let root = rt.wait(&thread, &run).await.unwrap();
            assert_eq!(root.status, RunStatus::Completed, "{:?}", root.error);
            loop {
                if let Change::NativeChild { child } = events.next().await.unwrap().change
                    && child.thread_id == snapshot.native_children[0].thread_id
                    && child.status == NativeChildStatus::Completed
                {
                    break;
                }
            }
            rt.close().await.unwrap();
            let rt = Huggler::open(options).await.unwrap();
            let state = rt.read(&thread).unwrap();
            assert!(state.permissions.is_empty());
            assert_eq!(state.runs, [root]);
            assert_eq!(state.native_children.len(), 2);
            for child in &state.native_children {
                assert_eq!(child.status, NativeChildStatus::Completed);
                assert_eq!(child.result.as_deref(), Some("Written."));
            }
            rt.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
