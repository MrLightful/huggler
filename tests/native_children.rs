#![cfg(unix)]
use huggler::*;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio::time::timeout;

const TRANSCRIPT: &str = include_str!("fixtures/codex-subagents/codex_transcript.ndjson");

fn recording() -> Vec<Value> {
    TRANSCRIPT
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn config(dir: &Path, records: &[Value]) -> HugglerOptions {
    let executable = dir.join("codex");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/replay-codex.py"),
        &executable,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let fixture = dir.join("replay.ndjson");
    fs::write(
        &fixture,
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    fs::write(
        dir.join("case.json"),
        json!({"fixture":fixture,"version":env!("CARGO_PKG_VERSION"),"start":1}).to_string(),
    )
    .unwrap();
    let mut options = HugglerOptions::new(dir.join("state"));
    options.codex.executable = executable;
    options
}

async fn start(dir: &Path, records: &[Value]) -> (HugglerOptions, Huggler, ThreadId, RunId) {
    let options = config(dir, records);
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
        "Replay fixture: subagent_v2_nested".into(),
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
async fn losing_a_background_provider_preserves_the_foreground_result() {
    timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        let mut records = after_root(false);
        let gate = records
            .iter()
            .position(|r| r["type"] == "await_release")
            .unwrap();
        records.truncate(gate + 1);
        records.push(json!({"type":"runtime_exit"}));
        let (options, rt, thread, run) = start(dir.path(), &records).await;
        let root = rt.wait(&thread, &run).await.unwrap();
        assert_eq!(root.status, RunStatus::Completed);
        let (snapshot, mut events) = rt.subscribe(&thread, None).unwrap();
        assert_eq!(
            snapshot.native_children[0].status,
            NativeChildStatus::Running
        );
        fs::write(dir.path().join("release"), "").unwrap();
        let Change::NativeChild { child } = events.next().await.unwrap().change else {
            panic!("expected a disconnected child");
        };
        assert_eq!(child.status, NativeChildStatus::Disconnected);
        assert!(child.result.is_none());
        rt.close().await.unwrap();
        let rt = Huggler::open(options).await.unwrap();
        let thread = rt.read(&thread).unwrap();
        assert_eq!(thread.runs, [root]);
        assert_eq!(thread.messages, snapshot.messages);
        assert_eq!(thread.native_children, [child]);
        rt.close().await.unwrap();
    })
    .await
    .unwrap();
}
