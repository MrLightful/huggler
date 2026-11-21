#![cfg(unix)]
use huggler::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command as Process,
    sync::{Arc, Barrier},
    time::Duration,
};
use tempfile::tempdir;
use tokio::time::timeout;

fn checkout(dir: &Path) {
    assert!(
        Process::new("git")
            .args(["init", "-q"])
            .arg(dir)
            .status()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn concurrent_receipts_preserve_the_winner_including_no_ops() {
    let temp = tempdir().unwrap();
    checkout(temp.path());
    let runtime = Arc::new(
        Huggler::open(HugglerOptions::new(temp.path().join("state")))
            .await
            .unwrap(),
    );
    let project = runtime.open_project(temp.path()).unwrap();
    let threads = [ThreadId("alpha".into()), ThreadId("beta".into())];
    for thread in &threads {
        runtime
            .launch(
                CommandId(format!("launch-{thread}")),
                thread.clone(),
                project.id.clone(),
                thread.to_string(),
                None,
            )
            .unwrap();
    }
    // The first race records an empty receipt, the second changes only the winner.
    for model in [None, Some("selected".to_string())] {
        let barrier = Arc::new(Barrier::new(2));
        let jobs = threads.clone().map(|id| {
            let barrier = barrier.clone();
            let runtime = runtime.clone();
            let command = Command {
                id: CommandId(format!("race-{model:?}")),
                thread_id: id,
                action: Action::SetModel {
                    model: model.clone(),
                },
            };
            std::thread::spawn(move || {
                barrier.wait();
                let result = runtime.dispatch(command.clone());
                (command, result)
            })
        });
        let outcomes = jobs.map(|job| job.join().unwrap());
        let (winner, receipt) = outcomes
            .iter()
            .find_map(|(cmd, res)| res.as_ref().ok().map(|receipt| (cmd, receipt)))
            .unwrap();
        assert_eq!(receipt.event_sequences.len(), usize::from(model.is_some()));
        let (loser, error) = outcomes
            .iter()
            .find_map(|(cmd, res)| res.as_ref().err().map(|err| (cmd, err)))
            .unwrap();
        assert!(
            matches!(error,Error::Conflict{original_thread,requested_thread,..}
            if original_thread==&winner.thread_id && requested_thread==&loser.thread_id)
        );
        assert_eq!(&runtime.dispatch(winner.clone()).unwrap(), receipt);
        assert_eq!(runtime.read(&winner.thread_id).unwrap().model, model);
        assert_eq!(runtime.read(&loser.thread_id).unwrap().model, None);
        let changed = Command {
            action: Action::Send {
                text: "different intent".into(),
            },
            ..winner.clone()
        };
        assert!(matches!(
            runtime.dispatch(changed),
            Err(Error::Conflict { .. })
        ));
    }
    runtime.close().await.unwrap();
}

async fn start_stalled(dir: &Path) -> (Huggler, Receipt) {
    checkout(dir);
    let shim = dir.join("codex");
    fs::write(&shim, include_str!("fixtures/stalled-codex.py")).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = HugglerOptions::new(dir.join("state"));
    options.codex.executable = shim;
    let rt = Huggler::open(options).await.unwrap();
    let project = rt.open_project(dir).unwrap();
    let id = ThreadId("active".into());
    rt.launch(
        CommandId("launch".into()),
        id.clone(),
        project.id,
        "Interrupted".into(),
        None,
    )
    .unwrap();
    let (_, mut stream) = rt.subscribe(&id, None).unwrap();
    let sent = rt
        .send(CommandId("send".into()), id, "one prompt".into())
        .unwrap();
    timeout(Duration::from_secs(10),async {
        while !matches!(stream.next().await.unwrap().change,Change::Run{run} if run.status==RunStatus::Running) {}
    }).await.unwrap();
    (rt, sent)
}

#[test]
fn dropping_the_executor_stops_the_provider_helper() {
    let dir = tempdir().unwrap();
    let lease = fs::File::create(dir.path().join("model.lock")).unwrap();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let (_rt, _) = start_stalled(dir.path()).await;
        assert!(matches!(
            lease.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
    });
    drop(executor);
    wait_for_helper_exit(&lease);
}

fn wait_for_helper_exit(lease: &fs::File) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match lease.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("provider helper retained its lock: {err}"),
        }
    }
}

#[tokio::test]
async fn protocol_failure_releases_the_provider_helpers_lock() {
    let dir = tempdir().unwrap();
    let lease = fs::File::create(dir.path().join("model.lock")).unwrap();
    fs::write(dir.path().join("invalid-frame"), "").unwrap();
    let (rt, receipt) = start_stalled(dir.path()).await;
    let run = rt
        .wait(&receipt.thread_id, receipt.run_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.error.unwrap().contains("invalid JSON-RPC frame"));
    wait_for_helper_exit(&lease);
    rt.close().await.unwrap();
}

#[tokio::test]
async fn close_reaps_an_active_child_with_observers_still_attached() {
    let dir = tempdir().unwrap();
    let lease = fs::File::create(dir.path().join("model.lock")).unwrap();
    let (rt, receipt) = start_stalled(dir.path()).await;
    assert!(matches!(
        rt.send(
            CommandId("busy".into()),
            receipt.thread_id.clone(),
            "too soon".into()
        ),
        Err(Error::Busy)
    ));
    let (_, mut stream) = rt.subscribe(&receipt.thread_id, None).unwrap();
    let pid: i32 = fs::read_to_string(dir.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    let (a, b, observed) = tokio::join!(rt.close(), rt.close(), stream.next());
    a.unwrap();
    b.unwrap();
    assert!(matches!(observed, Err(Error::Closed)));
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    // The provider's helper inherited the lock and lives in its process group.
    wait_for_helper_exit(&lease);
    let rt = Huggler::open(HugglerOptions::new(dir.path().join("state")))
        .await
        .unwrap();
    assert_eq!(
        rt.read(&receipt.thread_id).unwrap().runs[0].status,
        RunStatus::Failed
    );
    rt.close().await.unwrap();
}

#[tokio::test]
#[ignore = "subprocess entry point for process_loss_recovers_a_claimed_turn_without_reissuing_it"]
async fn crash_child() {
    let Some(dir) = std::env::var_os("HUGGLER_CRASH_CASE") else {
        return;
    };
    let (_rt, receipt) = start_stalled(dir.as_ref()).await;
    fs::write(
        Path::new(&dir).join("receipt"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    std::process::exit(73);
}

#[tokio::test]
async fn process_loss_recovers_a_claimed_turn_without_reissuing_it() {
    let dir = tempdir().unwrap();
    let exit = Process::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "crash_child"])
        .env("HUGGLER_CRASH_CASE", dir.path())
        .status()
        .unwrap();
    assert_eq!(exit.code(), Some(73));
    let original: Receipt =
        serde_json::from_slice(&fs::read(dir.path().join("receipt")).unwrap()).unwrap();
    let mut options = HugglerOptions::new(dir.path().join("state"));
    options.codex.executable = dir.path().join("codex");
    let rt = Huggler::open(options).await.unwrap();
    let state = rt.read(&original.thread_id).unwrap();
    assert_eq!(
        state.native_thread,
        Some(NativeThreadId("persisted".into()))
    );
    assert_eq!(state.runs.len(), 1);
    assert_eq!(state.runs[0].status, RunStatus::Failed);
    assert_eq!(
        state.runs[0].native_turn,
        Some(NativeTurnId("accepted".into()))
    );
    assert_eq!(
        rt.send(
            original.command_id.clone(),
            original.thread_id.clone(),
            "one prompt".into()
        )
        .unwrap(),
        original
    );
    rt.close().await.unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("invocations")).unwrap(),
        "turn/start\n"
    );
}
