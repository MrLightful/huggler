#![cfg(all(unix, feature = "machine"))]

use huggler::{
    CommandId, EnvironmentId, Huggler, HugglerOptions, Role, RunStatus, ThreadId,
    machine::{self, Client, ClientError, Reply, Request},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

fn git_init(path: &Path) {
    assert!(
        Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}

async fn identify(client: &Client) -> EnvironmentId {
    for _ in 0..100 {
        if let Ok(Reply::Machine(machine)) = client.request(Request::Identify).await {
            return machine.id;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("machine socket did not start");
}

#[tokio::test]
async fn machine_reconnect_reuses_the_receipt_and_recovers_the_turn() {
    let root = tempfile::Builder::new()
        .prefix("hg-test-")
        .tempdir_in("/tmp")
        .unwrap();
    let state = root.path().join("state");
    let checkout = root.path().join("remote-checkout");
    let other_checkout = root.path().join("other-checkout");
    git_init(&checkout);
    git_init(&other_checkout);
    fs::write(checkout.join("long-result"), "").unwrap();
    let socket = state.join("control.sock");
    let mut options = HugglerOptions::new(state);
    options.codex.executable =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/machine-codex.py");
    let runtime = Arc::new(Huggler::open(options).await.unwrap());
    let (stop, shutdown) = tokio::sync::oneshot::channel();
    let service = tokio::spawn(machine::serve(
        runtime.clone(),
        socket.clone(),
        async move {
            let _ = shutdown.await;
        },
    ));
    let unbound = Client::new(&socket).unwrap();
    let environment = identify(&unbound).await;
    let wrong = unbound
        .clone()
        .for_environment(EnvironmentId("other".into()));
    assert!(
        matches!(wrong.request(Request::OpenProject { checkout: checkout.clone() }).await, Err(ClientError::Rejected(message)) if message.contains("different Huggler environment"))
    );
    let client = unbound.for_environment(environment.clone());
    let Reply::Project(project) = client
        .request(Request::OpenProject {
            checkout: checkout.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("expected the remote project")
    };
    let Reply::Project(other) = client
        .request(Request::OpenProject {
            checkout: other_checkout,
        })
        .await
        .unwrap()
    else {
        panic!("expected the other project")
    };
    let thread = ThreadId("remote-thread".into());
    let Reply::Receipt(_) = client
        .request(Request::Launch {
            project: project.id.clone(),
            thread: thread.clone(),
            command: CommandId("open".into()),
            name: "Remote work".into(),
        })
        .await
        .unwrap()
    else {
        panic!("expected launch receipt")
    };
    assert!(
        matches!(client.request(Request::List { project: other.id.clone() }).await, Ok(Reply::Conversations(threads)) if threads.is_empty())
    );
    assert!(matches!(
        client
            .request(Request::Detail {
                project: other.id.clone(),
                thread: thread.clone(),
                after: None,
                before: None
            })
            .await,
        Err(ClientError::Rejected(_))
    ));
    let send = Request::Send {
        project: project.id.clone(),
        thread: thread.clone(),
        command: CommandId("first-message".into()),
        text: "Write the remote result.".into(),
    };
    let Reply::Receipt(receipt) = client.request(send.clone()).await.unwrap() else {
        panic!("expected send receipt")
    };
    let detail = |after| Request::Detail {
        project: project.id.clone(),
        thread: thread.clone(),
        after,
        before: None,
    };
    let Reply::Detail(initial) = client.request(detail(None)).await.unwrap() else {
        panic!("expected conversation detail")
    };
    drop(client);
    fs::write(checkout.join("finish-turn"), "").unwrap();

    let unbound = Client::new(&socket).unwrap();
    assert_eq!(identify(&unbound).await, environment);
    let client = unbound.for_environment(environment);
    assert!(
        matches!(client.request(send).await, Ok(Reply::Receipt(retried)) if retried == receipt)
    );
    let mut cursor = initial.history.sequence;
    let completed = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Reply::Detail(snapshot) = client.request(detail(Some(cursor))).await.unwrap() {
                cursor = snapshot.history.sequence;
                if snapshot
                    .latest_run
                    .as_ref()
                    .is_some_and(|run| run.status.terminal())
                {
                    break snapshot;
                }
            }
        }
    })
    .await
    .expect("remote turn should finish");
    assert_eq!(
        completed.latest_run.as_ref().unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(
        completed
            .history
            .messages
            .iter()
            .filter(|item| item.message.role == Role::User)
            .count(),
        1
    );
    let assistant = completed
        .history
        .messages
        .iter()
        .find(|item| item.message.role == Role::Assistant)
        .expect("assistant reply");
    assert!(assistant.truncated);
    let message_body = |project| Request::MessageBody {
        project,
        thread: thread.clone(),
        message: assistant.message.id.clone(),
    };
    assert!(
        matches!(client.request(message_body(project.id.clone())).await, Ok(Reply::MessageBody(body))
            if body.starts_with(&assistant.message.text) && body.ends_with("End of remote result."))
    );
    assert!(matches!(
        client.request(message_body(other.id)).await,
        Err(ClientError::Rejected(_))
    ));
    assert_eq!(
        fs::read_to_string(checkout.join("turns.txt")).unwrap(),
        "turn/start\n"
    );
    assert_eq!(
        fs::read_to_string(checkout.join("remote-result.txt")).unwrap(),
        "Written by the remote provider.\n"
    );
    drop(client);
    stop.send(()).unwrap();
    service.await.unwrap().unwrap();
    runtime.close().await.unwrap();
    assert!(!socket.exists());
}
