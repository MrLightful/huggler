//! Exit at SQLite commit boundaries without adding failpoints to the runtime.
use super::*;
use std::process::Command as Process;

fn send() -> Command {
    Command {
        id: CommandId("send".into()),
        thread_id: ThreadId("root".into()),
        action: Action::Send {
            text: "test".into(),
        },
    }
}

#[test]
#[ignore = "subprocess entry point"]
fn crash_at_commit() {
    let Some(path) = std::env::var_os("HUGGLER_COMMIT_TEST_DIR") else {
        return;
    };
    let mut db = Database::open(Path::new(&path)).unwrap();
    if std::env::var_os("HUGGLER_EXIT_BEFORE_COMMIT").is_some() {
        db.db
            .commit_hook(Some(|| -> bool { std::process::exit(71) }))
            .unwrap();
    }
    db.dispatch(&send()).unwrap();
    std::process::exit(72);
}

#[test]
fn command_commit_survives_process_loss_as_one_unit() -> Result<()> {
    for before in [true, false] {
        let dir = tempfile::tempdir()?;
        let mut db = Database::open(dir.path())?;
        let project = db.project(dir.path())?;
        let command = send();
        db.dispatch(&Command {
            id: CommandId("launch".into()),
            thread_id: command.thread_id.clone(),
            action: Action::Launch {
                project: project.id,
                name: "root".into(),
                model: None,
            },
        })?;
        drop(db);
        let mut child = Process::new(std::env::current_exe()?);
        child
            .args([
                "--exact",
                "store::crash_tests::crash_at_commit",
                "--ignored",
            ])
            .env("HUGGLER_COMMIT_TEST_DIR", dir.path())
            .env_remove("HUGGLER_EXIT_BEFORE_COMMIT");
        if before {
            child.env("HUGGLER_EXIT_BEFORE_COMMIT", "1");
        }
        assert_eq!(child.status()?.code(), Some(if before { 71 } else { 72 }));
        let mut db = Database::open(dir.path())?;
        let snapshot = db.read(&command.thread_id)?;
        assert_eq!(snapshot.messages.len(), usize::from(!before));
        assert_eq!(snapshot.runs.len(), usize::from(!before));
        assert!(db.ready_jobs()?.is_empty());
        assert!(!db.db.prepare("SELECT 1 FROM turn_effect")?.exists([])?);
        assert_eq!(
            db.db
                .prepare("SELECT 1 FROM receipt WHERE command_id='send'")?
                .exists([])?,
            !before
        );
        let (receipt, work) = db.dispatch(&command)?;
        assert_eq!(work.start.len(), usize::from(before));
        if !before {
            assert_eq!(snapshot.runs[0].status, RunStatus::Failed);
            assert!(
                snapshot.runs[0]
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("before provider claim")
            );
            assert_eq!(receipt.message_id, Some(snapshot.messages[0].id.clone()));
        }
    }
    Ok(())
}

#[test]
fn each_stage_five_schema_preserves_previous_messages_and_receipts() -> Result<()> {
    for version in 13..MIGRATIONS.len() {
        let dir = tempfile::tempdir()?;
        let disk = Connection::open(dir.path().join("huggler.sqlite3"))?;
        for sql in &MIGRATIONS[..version] {
            disk.execute_batch(sql)?;
        }
        let cmd = send();
        let old = Receipt {
            command_id: cmd.id.clone(),
            thread_id: cmd.thread_id.clone(),
            message_id: Some(MessageId("m".into())),
            run_id: Some(RunId("r".into())),
            event_sequences: vec![],
        };
        disk.execute_batch("INSERT INTO project VALUES('p','/repo');
            INSERT INTO thread(id,project,name,provider,provider_instance) VALUES('root','p','Example','codex','codex');
            INSERT INTO run(id,thread_id,ordinal,status) VALUES('r','root',1,'completed');
            INSERT INTO message(id,run_id,role,text) VALUES('m','r','user','test');")?;
        disk.execute(
            "INSERT INTO receipt VALUES('send',?,?)",
            params![
                serde_json::to_string(&cmd.action)?,
                serde_json::to_string(&old)?
            ],
        )?;
        drop(disk);
        let mut db = Database::open(dir.path())?;
        assert_eq!(db.dispatch(&cmd)?.0, old, "schema {version}");
        let page = db.history(&cmd.thread_id, None, "", 10)?;
        assert_eq!(page.messages[0].message.text, "test");
        assert!(db.fork_history(&cmd.thread_id)?.is_none());
    }
    Ok(())
}
