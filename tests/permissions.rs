#![cfg(unix)]
use huggler::*;
use std::{path::Path, process::Command as Git, time::Duration};

#[tokio::test]
async fn only_the_owning_request_can_answer_and_load_omits_replayed_history() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Git::new("git")
            .args(["init", "-q"])
            .arg(dir.path())
            .status()
            .unwrap()
            .success()
    );
    let mut options = HugglerOptions::new(dir.path().join("state"));
    options.claude_agent.executable =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/permission-acp.py");
    let id = ThreadId("claude".into());
    for (ordinal, choice, answer) in [
        (1, Some("no"), "operation denied"),
        (2, Some("yes"), "operation approved"),
        (3, None, "operation approved"),
        (4, None, "persistent approval cancelled"),
    ] {
        if ordinal == 3 {
            let mut instance = ProviderInstance::new(
                "claudeAgent",
                Provider::ClaudeAgent,
                &options.claude_agent.executable,
            );
            instance.full_access = true;
            options.instances.push(instance);
        }
        let path = dir.path().join("approval-check.txt");
        if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
        let rt = Huggler::open(options.clone()).await.unwrap();
        if ordinal == 1 {
            let project = rt.open_project(dir.path()).unwrap();
            rt.dispatch(Command {
                id: CommandId("launch".into()),
                thread_id: id.clone(),
                action: Action::LaunchProvider {
                    project: project.id,
                    name: "Reviewer".into(),
                    provider: Provider::ClaudeAgent,
                    model: None,
                },
            })
            .unwrap();
        }
        let (_, mut stream) = rt.subscribe(&id, None).unwrap();
        let run = rt
            .send(
                CommandId(format!("edit-{ordinal}")),
                id.clone(),
                if ordinal == 4 {
                    "Offer only a persistent grant"
                } else {
                    "Create approval-check.txt"
                }
                .into(),
            )
            .unwrap()
            .run_id
            .unwrap();
        if let Some(choice) = choice {
            let request = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Change::PermissionRequested { request } =
                        stream.next().await.unwrap().change
                    {
                        break request;
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(rt.read(&id).unwrap().permissions[0].id, request.id);
            assert!(
                rt.respond_to_permission(
                    &ThreadId("other".into()),
                    &run,
                    &request.id,
                    "yes".into()
                )
                .is_err()
            );
            assert!(
                rt.respond_to_permission(&id, &run, &request.id, "unknown".into())
                    .is_err()
            );
            rt.respond_to_permission(&id, &run, &request.id, choice.into())
                .unwrap();
            assert!(
                rt.respond_to_permission(&id, &run, &request.id, choice.into())
                    .is_err()
            );
        }
        let result = tokio::time::timeout(Duration::from_secs(10), rt.wait(&id, &run))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.status, RunStatus::Completed, "{:?}", result.error);
        let snapshot = rt.read(&id).unwrap();
        assert!(snapshot.permissions.is_empty());
        assert_eq!(snapshot.messages.len(), ordinal * 2);
        assert_eq!(snapshot.messages.last().unwrap().text, answer);
        assert_eq!(path.exists(), matches!(ordinal, 2 | 3));
        rt.close().await.unwrap();
    }
}
