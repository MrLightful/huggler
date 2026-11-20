use huggler::*;
use std::{path::Path, process::Command as Git};

pub fn checkout(path: &Path) {
    assert!(
        Git::new("git")
            .args(["init", "-q"])
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}

pub fn launch(rt: &Huggler, project: &Project, name: &str, provider: Provider) -> ThreadId {
    let id = ThreadId(name.into());
    rt.dispatch(Command {
        id: CommandId(format!("launch-{name}")),
        thread_id: id.clone(),
        action: Action::LaunchProvider {
            project: project.id.clone(),
            name: name.into(),
            model: None,
            provider,
        },
    })
    .unwrap();
    id
}

pub async fn completed(rt: &Huggler, thread: &ThreadId, run: &RunId) {
    let state = rt.wait(thread, run).await.unwrap();
    assert_eq!(
        state.status,
        RunStatus::Completed,
        "{thread}: {:?}",
        state.error
    );
}
