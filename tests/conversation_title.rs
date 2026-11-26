#![cfg(unix)]
use huggler::{providers::codex::generate_conversation_title, *};
use std::{fs, os::unix::fs::PermissionsExt, process};
use tempfile::tempdir;

#[tokio::test]
async fn host_can_opt_in_to_title_generation_without_overwriting_a_newer_title() -> Result<()> {
    let temp = tempdir()?;
    let root = temp.path().join("repo");
    assert!(
        process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&root)
            .status()?
            .success()
    );
    let executable = temp.path().join("codex");
    fs::write(&executable, include_bytes!("fixtures/title-codex.py"))?;
    fs::write(
        temp.path().join("title-codex-0.170.0.ndjson"),
        include_bytes!("fixtures/title-codex-0.170.0.ndjson"),
    )?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
    let instance = ProviderInstance::new("title-fixture", Provider::Codex, executable);
    let state = temp.path().join("state");
    let runtime = Huggler::open(HugglerOptions::new(&state)).await?;
    let project = runtime.open_project(&root)?;
    let thread = ThreadId("conversation".into());
    let seed = "Investigate the login retry problem";
    runtime.launch(
        CommandId("launch".into()),
        thread.clone(),
        project.id.clone(),
        seed.into(),
        None,
    )?;
    let (snapshot, mut events) = runtime.subscribe(&thread, None)?;
    assert_eq!(snapshot.name, seed);
    let original = runtime.thread_title(&thread)?;
    assert_eq!(original.as_str(), seed);
    let newer = ThreadId("newer-conversation".into());
    runtime.launch(
        CommandId("launch-newer".into()),
        newer.clone(),
        project.id,
        "Newer conversation".into(),
        None,
    )?;
    let list = runtime.conversation_list()?;

    let context = format!(
        "Investigate the login retry {} on slow connections",
        "🚀".repeat(3_000)
    );
    let request = ConversationTitleRequest {
        context: &context,
        project_title: Some("Example"),
        instructions: Some("Use sentence case."),
        ..Default::default()
    };
    let title = generate_conversation_title(
        &instance,
        runtime.output(),
        &root,
        Some("title-model"),
        request,
    )
    .await?
    .expect("suggested title");
    assert_eq!(title, "Investigate login retries on slow connections");
    assert_eq!(runtime.read(&thread)?.name, seed);
    assert!(runtime.set_thread_title_if_current(&original, &title)?);
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.next())
        .await
        .expect("title change event")?;
    assert!(
        matches!(event.change, Change::TitleChanged { title } if title == "Investigate login retries on slow connections")
    );
    let changed_list = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runtime.wait_for_conversation_list(list.revision),
    )
    .await
    .expect("updated conversation list")?;
    assert_eq!(changed_list.threads[0].id, newer);
    assert_eq!(changed_list.threads[1].name, title);
    let generated = runtime.thread_title(&thread)?;

    let regeneration = ConversationTitleRequest {
        context: "USER: Investigate login retries.\nASSISTANT: Reproduced.",
        previous_title: Some(generated.as_str()),
        ..request
    };
    let regenerated = generate_conversation_title(
        &instance,
        runtime.output(),
        &root,
        Some("title-model"),
        regeneration,
    )
    .await?
    .expect("regenerated title");
    assert_eq!(regenerated, "Login retry reliability");
    assert!(runtime.set_thread_title_if_current(&generated, &regenerated)?);
    assert!(!runtime.set_thread_title_if_current(&generated, "Late suggestion")?);

    let replacement = runtime.thread_title(&thread)?;
    assert_eq!(
        generate_conversation_title(
            &instance,
            runtime.output(),
            &root,
            Some("title-model"),
            ConversationTitleRequest {
                previous_title: Some(replacement.as_str()),
                ..regeneration
            },
        )
        .await?,
        None
    );
    assert!(!runtime.set_thread_title_if_current(&replacement, &regenerated)?);
    assert!(runtime.set_thread_title_if_current(&replacement, seed)?);
    runtime.retain_events(&thread, 1)?;
    runtime.close().await?;

    let reopened = Huggler::open(HugglerOptions::new(state)).await?;
    let snapshot = reopened.read(&thread)?;
    assert_eq!(snapshot.name, seed);
    assert!(!reopened.set_thread_title_if_current(&original, "Late suggestion after revert")?);
    let current = reopened.thread_title(&thread)?;
    assert!(reopened.set_thread_title_if_current(&current, &regenerated)?);
    assert_eq!(reopened.read(&thread)?.name, regenerated);
    assert!(snapshot.runs.is_empty() && snapshot.messages.is_empty());
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn existing_database_gets_title_revisions_without_losing_names() -> Result<()> {
    let temp = tempdir()?;
    let root = temp.path().join("repo");
    assert!(
        process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&root)
            .status()?
            .success()
    );
    let mut options = HugglerOptions::new(temp.path().join("state"));
    let database = temp.path().join("previous.sqlite");
    options.storage.database_path = database.clone();
    let runtime = Huggler::open(options.clone()).await?;
    let project = runtime.open_project(&root)?;
    let thread = ThreadId("existing".into());
    let title = "Old conversation title";
    runtime.launch(
        CommandId("existing-launch".into()),
        thread.clone(),
        project.id,
        title.into(),
        None,
    )?;
    let list = runtime.conversation_list()?;
    runtime.close().await?;

    rusqlite::Connection::open(database)?.execute_batch(
        "DROP INDEX thread_title_revision;
         ALTER TABLE thread_order DROP COLUMN title_revision;
         PRAGMA user_version=43;",
    )?;

    let reopened = Huggler::open(options).await?;
    assert_eq!(reopened.conversation_list()?, list);
    let snapshot = reopened.thread_title(&thread)?;
    assert_eq!(snapshot.as_str(), title);
    let custom_title = "Custom title ".repeat(20);
    assert!(reopened.set_thread_title_if_current(&snapshot, custom_title.trim())?);
    assert_eq!(reopened.read(&thread)?.name, custom_title.trim());
    reopened.close().await?;
    Ok(())
}
