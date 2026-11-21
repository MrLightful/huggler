// Shared by the integration test and the standalone package consumer.
use huggler::*;
use serde_json::json;
use std::{
    fs, os::unix::fs::PermissionsExt, path::Path, process::Command as Process, time::Duration,
};
use tempfile::tempdir;
use tokio::time::timeout;

const NAME: &str = "Replay fixture: provider_thread_resume";
const MODEL: &str = "gpt-6-luna";
const NATIVE: &str = "01a0d5ef-62bd-7c20-861b-0528ff7d86bd";
const FIRST: &str = "Respond with exactly: provider thread resume fixture first turn complete";
const SECOND: &str = "Using the conversation history available in this resumed thread, first repeat the exact final answer you gave in the previous turn. Then on a new line write exactly: provider thread resume fixture second turn complete";
const ANSWER: &str = "provider thread resume fixture first turn complete";
const CONTINUATION: &str = "provider thread resume fixture first turn complete\nprovider thread resume fixture second turn complete";

fn case(dir: &Path, source: &Path, start: usize, sibling: bool) {
    let mut substitutions = json!({});
    if sibling {
        substitutions[NATIVE] = json!("another-native-thread");
    }
    fs::write(dir.join("case.json"), json!({
        "start":start, "fixture":source.join("docs/reference/codex-resume/codex_transcript.ndjson"),
        "version":env!("CARGO_PKG_VERSION"), "substitutions":substitutions
    }).to_string()).unwrap();
}

async fn completed(rt: &Huggler, receipt: &Receipt) -> Run {
    let run = timeout(
        Duration::from_secs(15),
        rt.wait(&receipt.thread_id, receipt.run_id.as_ref().unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    run
}

async fn verify_events(
    stream: &mut Subscription,
    receipt: &Receipt,
    through: i64,
    mut cursor: i64,
) {
    let mut statuses = Vec::new();
    let mut accepted = Vec::new();
    while cursor < through {
        let event = timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.thread_id, receipt.thread_id);
        assert!(event.sequence > cursor && event.sequence <= through);
        cursor = event.sequence;
        match event.change {
            Change::Run { run } => {
                assert_eq!(Some(&run.id), receipt.run_id.as_ref());
                if run.status == RunStatus::Starting {
                    accepted.push(cursor);
                }
                statuses.push(run.status);
            }
            Change::Message { message } => {
                assert_eq!(Some(&message.run_id), receipt.run_id.as_ref());
                if message.role == Role::User {
                    assert_eq!(Some(&message.id), receipt.message_id.as_ref());
                    accepted.push(cursor);
                }
            }
            _ => {}
        }
    }
    assert_eq!(accepted, receipt.event_sequences);
    assert_eq!(
        statuses,
        [
            RunStatus::Starting,
            RunStatus::Running,
            RunStatus::Waiting,
            RunStatus::Completed
        ]
    );
}

pub async fn scenario(source: &Path) {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project with spaces");
    assert!(
        Process::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let executable = temp.path().join("codex");
    fs::copy(source.join("tests/fixtures/replay-codex.py"), &executable).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = HugglerOptions::new(temp.path().join("app data"));
    options.codex.executable = executable;
    let thread = ThreadId("application-thread".into());
    let sibling = ThreadId("sibling-thread".into());
    let first = CommandId("first-send".into());
    case(temp.path(), source, 1, false);
    let rt = Huggler::open(options.clone()).await.unwrap();
    assert!(matches!(
        Huggler::open(options.clone()).await,
        Err(Error::AlreadyOpen)
    ));
    let project = rt.open_project(&root).unwrap();
    for id in [&thread, &sibling] {
        rt.launch(
            CommandId(format!("launch-{id}")),
            id.clone(),
            project.id.clone(),
            NAME.into(),
            Some(MODEL.into()),
        )
        .unwrap();
    }
    let (initial, mut events) = rt.subscribe(&thread, None).unwrap();
    let sent = rt
        .send(first.clone(), thread.clone(), FIRST.into())
        .unwrap();
    let first_run = completed(&rt, &sent).await;
    let cursor = rt.read(&thread).unwrap().cursor;
    verify_events(&mut events, &sent, cursor, initial.cursor).await;
    assert_eq!(
        fs::read_to_string(temp.path().join("cursor")).unwrap(),
        "40"
    );
    rt.close().await.unwrap();
    assert!(matches!(events.next().await, Err(Error::Closed)));

    case(temp.path(), source, 41, false);
    let rt = Huggler::open(options).await.unwrap();
    let (before, mut resumed) = rt.subscribe(&thread, Some(cursor)).unwrap();
    let (other, mut other_stream) = rt.subscribe(&sibling, None).unwrap();
    assert!(other.messages.is_empty());
    assert_eq!(before.id, thread);
    assert_eq!(before.messages.len(), 2);
    assert_eq!(before.messages[1].text, ANSWER);
    assert_eq!(rt.send(first, thread.clone(), FIRST.into()).unwrap(), sent);
    let second = rt
        .send(
            CommandId("second-send".into()),
            thread.clone(),
            SECOND.into(),
        )
        .unwrap();
    let second_run = completed(&rt, &second).await;
    assert_eq!(
        fs::read_to_string(temp.path().join("cursor")).unwrap(),
        "92"
    );
    let usage = rt.provider_usage().unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(
        (
            usage[0].instance.as_str(),
            usage[0].reported_runs,
            usage[0].input_tokens,
            usage[0].output_tokens
        ),
        ("codex", 2, 42897, 50)
    );
    let after = rt.read(&thread).unwrap();
    assert_eq!(after.native_thread, Some(NativeThreadId(NATIVE.into())));
    assert_eq!(after.runs, [first_run.clone(), second_run.clone()]);
    assert_eq!((first_run.ordinal, second_run.ordinal), (1, 2));
    assert_ne!(first_run.id, second_run.id);
    assert_eq!(
        first_run.native_turn.unwrap().0,
        "01a0d5ef-637e-7ba2-bf9a-5534a6f98a83"
    );
    assert_eq!(
        second_run.native_turn.unwrap().0,
        "01a0d5ef-77e2-7e22-a198-4864f687b8b5"
    );
    assert_eq!(
        after
            .messages
            .iter()
            .map(|m| (m.role, m.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (Role::User, FIRST),
            (Role::Assistant, ANSWER),
            (Role::User, SECOND),
            (Role::Assistant, CONTINUATION)
        ]
    );
    verify_events(&mut resumed, &second, after.cursor, cursor).await;
    case(temp.path(), source, 1, true);
    let sent = rt
        .send(
            CommandId("sibling-send".into()),
            sibling.clone(),
            FIRST.into(),
        )
        .unwrap();
    completed(&rt, &sent).await;
    let sibling_after = rt.read(&sibling).unwrap();
    assert_eq!(sibling_after.messages.len(), 2);
    assert_eq!(
        sibling_after.native_thread.unwrap().0,
        "another-native-thread"
    );
    verify_events(&mut other_stream, &sent, sibling_after.cursor, other.cursor).await;
    assert_eq!(rt.read(&thread).unwrap().cursor, after.cursor);
    rt.close().await.unwrap();
}
