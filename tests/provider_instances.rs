#![cfg(unix)]
use huggler::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command as Process,
    time::Duration,
};
use tempfile::{TempDir, tempdir};
use tokio::time::timeout;

fn checkout() -> (TempDir, PathBuf) {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project with spaces");
    assert!(
        Process::new("git")
            .args(["init", "--quiet"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    (temp, root.canonicalize().unwrap())
}

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn launch(rt: &Huggler, root: &Path, instance: &str, model: Option<String>) -> ThreadId {
    let thread = ThreadId(format!("thread-{instance}"));
    rt.dispatch(Command {
        id: CommandId(format!("launch-{instance}")),
        thread_id: thread.clone(),
        action: Action::LaunchInstance {
            project: rt.open_project(root).unwrap().id,
            instance: instance.into(),
            name: format!("{instance} qualification"),
            model,
        },
    })
    .unwrap();
    thread
}

async fn turn(rt: &Huggler, thread: &ThreadId, prompt: &str) -> Run {
    let receipt = rt
        .send(
            CommandId(uuid::Uuid::new_v4().to_string()),
            thread.clone(),
            prompt.into(),
        )
        .unwrap();
    timeout(
        Duration::from_secs(90),
        rt.wait(thread, receipt.run_id.as_ref().unwrap()),
    )
    .await
    .unwrap()
    .unwrap()
}

async fn answer(rt: &Huggler, thread: &ThreadId, prompt: &str) -> String {
    let run = turn(rt, thread, prompt).await;
    assert_eq!(run.status, RunStatus::Completed, "{run:?}");
    rt.read(thread)
        .unwrap()
        .messages
        .last()
        .unwrap()
        .text
        .clone()
}

#[tokio::test]
#[ignore = "uses the signed-in cloud Codex subscription"]
async fn cloud_codex_with_missing_acp() {
    let (temp, root) = checkout();
    let mut options = HugglerOptions::new(temp.path().join("state"));
    options.instances.push(ProviderInstance::new(
        "missing",
        Provider::Cursor,
        temp.path().join("missing"),
    ));
    let rt = Huggler::open(options).await.unwrap();
    assert!(matches!(
        rt.probe_provider("missing", &root).await,
        Err(Error::MissingExecutable(_))
    ));
    let report = rt.probe_provider("codex", &root).await.unwrap();
    eprintln!(
        "{} {:?}, {} models",
        report.transport,
        report.version,
        report.models.len()
    );
    assert_eq!(report.authenticated, Some(true));
    assert!(!report.models.is_empty());
    let thread = launch(&rt, &root, "codex", None);
    assert_eq!(
        answer(
            &rt,
            &thread,
            "Without using tools, reply with exactly: qualification passed"
        )
        .await
        .trim(),
        "qualification passed"
    );
    rt.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the Muse executable with its deterministic echo provider"]
async fn installed_muse_echo() {
    let (temp, root) = checkout();
    let path = temp.path().join("echo-muse");
    executable(
        &path,
        r#"#!/bin/sh
dir=$(dirname "$0")/home
export HOME="$dir" XDG_CONFIG_HOME="$dir/config" XDG_DATA_HOME="$dir/data" XDG_STATE_HOME="$dir/state" XDG_CACHE_HOME="$dir/cache"
exec muse "$@" --provider echo
"#,
    );
    let mut options = HugglerOptions::new(temp.path().join("state"));
    options
        .instances
        .push(ProviderInstance::new("echo", Provider::Muse, path));
    let rt = Huggler::open(options.clone()).await.unwrap();
    let report = rt.probe_provider("echo", &root).await.unwrap();
    eprintln!(
        "{} {:?}, models {:?}",
        report.transport, report.version, report.models
    );
    let thread = launch(&rt, &root, "echo", report.models.first().cloned());
    assert!(
        answer(&rt, &thread, "echo first")
            .await
            .contains("echo first")
    );
    let native = rt.read(&thread).unwrap().native_thread.unwrap();
    rt.close().await.unwrap();
    let rt = Huggler::open(options).await.unwrap();
    assert!(
        answer(&rt, &thread, "echo second")
            .await
            .contains("echo second")
    );
    assert_eq!(rt.read(&thread).unwrap().native_thread, Some(native));
    rt.close().await.unwrap();
}

/// Only initialize ACP and open an empty session, using fresh provider homes.
#[tokio::test]
#[ignore = "requires the Cursor and Antigravity CLIs"]
async fn installed_acp_login_challenges() {
    let (dir, root) = checkout();
    let mut options = HugglerOptions::new(dir.path().join("state"));
    for (driver, variable, default) in [
        (Provider::Cursor, "HUGGLER_TEST_CURSOR", "agent"),
        (Provider::Antigravity, "HUGGLER_TEST_ANTIGRAVITY", "agy"),
    ] {
        let home = dir.path().join(driver.key());
        fs::create_dir(&home).unwrap();
        let wrapper = home.join("launcher");
        executable(
            &wrapper,
            r#"#!/bin/sh
dir=$(dirname "$0")
exec env -i PATH="$PATH" HOME="$dir" XDG_CONFIG_HOME="$dir" TMPDIR="$dir" TERM=dumb "$@"
"#,
        );
        let mut instance = ProviderInstance::new(driver.key(), driver, wrapper);
        instance.args = vec![
            std::env::var(variable).unwrap_or_else(|_| default.into()),
            "acp".into(),
        ];
        options.instances.push(instance);
    }
    let rt = Huggler::open(options).await.unwrap();
    for driver in [Provider::Cursor, Provider::Antigravity] {
        let result = rt.probe_provider(driver.key(), &root).await;
        assert!(
            matches!(result, Err(Error::LoginRequired)),
            "{driver:?} returned {result:?}"
        );
    }
    rt.close().await.unwrap();
}

/// The wrapper preserves protocol argv ordering, including the version probe.
fn peer(dir: &Path, flavor: &str, driver: Provider) -> ProviderInstance {
    let wrapper = dir.join(flavor);
    let fixture = if matches!(driver, Provider::OpenCode) {
        "open_server.py"
    } else {
        "replay.py"
    };
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/providers")
        .join(fixture);
    executable(
        &wrapper,
        &format!(
            "#!/usr/bin/env python3\nimport os,sys\nos.execvp('python3',['python3','-B',{},{},*sys.argv[1:]])\n",
            serde_json::to_string(&source).unwrap(),
            serde_json::to_string(flavor).unwrap()
        ),
    );
    ProviderInstance::new(flavor, driver, wrapper)
}

async fn observe_turn(
    rt: &Huggler,
    thread: &ThreadId,
    prompt: &str,
    approval: bool,
    streamed: bool,
) -> String {
    let (_, mut stream) = rt.subscribe(thread, None).unwrap();
    let run_id = rt
        .send(
            CommandId(uuid::Uuid::new_v4().to_string()),
            thread.clone(),
            prompt.into(),
        )
        .unwrap()
        .run_id
        .unwrap();
    let mut output = String::new();
    let mut requests = 0;
    loop {
        let event = timeout(Duration::from_secs(15), stream.next())
            .await
            .expect("fixture stalled")
            .unwrap();
        match event.change {
            Change::PermissionRequested { request } => {
                let deny = request
                    .options
                    .iter()
                    .find(|o| o.kind == "reject_once")
                    .unwrap();
                rt.respond_to_permission(thread, &run_id, &request.id, deny.option_id.clone())
                    .unwrap();
                requests += 1;
            }
            Change::OutputDelta {
                run_id: source,
                text,
            } => {
                assert_eq!(source, run_id);
                output.push_str(&text);
            }
            Change::Run { run } if run.id == run_id && run.status.terminal() => {
                assert_eq!(run.status, RunStatus::Completed, "{thread:?}: {run:?}");
                break;
            }
            _ => {}
        }
    }
    assert_eq!(requests, usize::from(approval));
    let snapshot = rt.read(thread).unwrap();
    let answer = snapshot.messages.last().unwrap().text.clone();
    if streamed {
        assert_eq!(output, answer);
    } else {
        assert!(output.is_empty());
    }
    answer
}

#[tokio::test]
async fn acp_flavors_discover_approve_stream_and_continue() {
    let (dir, root) = checkout();
    let flavors = [
        ("fx", Provider::Fx, "0.0.13", Some((19540, 9))),
        ("grok", Provider::Grok, "1.0.41", Some((22229, 34))),
        ("kiro", Provider::Kiro, "2.32.0", None),
        ("registry", Provider::AcpRegistry, "1", None),
        ("cursor", Provider::Cursor, "2026.11.19-f005e9d", None),
        (
            "antigravity",
            Provider::Antigravity,
            "agy_acp_server_1.1.1",
            None,
        ),
        ("antigravity-v1", Provider::Antigravity, "1.5.13", None),
    ];
    let mut opts = HugglerOptions::new(dir.path().join("state"));
    opts.agent_tools = true;
    add_stopper(&mut opts, dir.path());
    for (name, driver, _, _) in flavors {
        opts.instances.push(peer(dir.path(), name, driver));
    }
    for name in ["early-login", "login", "incompatible"] {
        opts.instances
            .push(peer(dir.path(), name, Provider::AcpRegistry));
    }
    for (name, driver) in [
        ("cursor-logged-out", Provider::Cursor),
        ("antigravity-logged-out", Provider::Antigravity),
    ] {
        opts.instances.push(peer(dir.path(), name, driver));
    }
    let rt = Huggler::open(opts.clone()).await.unwrap();
    for name in [
        "early-login",
        "login",
        "cursor-logged-out",
        "antigravity-logged-out",
    ] {
        assert!(matches!(
            rt.probe_provider(name, &root).await,
            Err(Error::LoginRequired)
        ));
    }
    assert!(matches!(
        rt.probe_provider("incompatible", &root).await,
        Err(Error::IncompatibleProvider(_))
    ));
    let stopper = launch(&rt, &root, "stopper", None);
    let mut threads = vec![];
    for (id, driver, version, usage) in flavors {
        let report = rt.probe_provider(id, &root).await.unwrap();
        assert!(report.resume);
        assert_eq!(report.agent_tools, id != "antigravity-v1");
        assert_eq!(report.authenticated, None);
        assert_eq!(report.version.as_deref(), Some(version));
        assert_eq!(report.models.is_empty(), driver == Provider::Kiro);
        let thread = launch(&rt, &root, id, report.models.first().cloned());
        assert_eq!(
            observe_turn(&rt, &thread, "fresh", true, true).await,
            "fixture simple ok"
        );
        let snapshot = rt.read(&thread).unwrap();
        assert_eq!(snapshot.provider_instance.as_str(), id);
        threads.push((thread, snapshot.native_thread.unwrap(), id, usage));
    }
    rt.close().await.unwrap();
    let rt = Huggler::open(opts).await.unwrap();
    for (thread, native, id, usage) in threads {
        assert_eq!(
            observe_turn(&rt, &thread, "resume", true, true).await,
            "fixture simple ok"
        );
        assert_eq!(rt.read(&thread).unwrap().native_thread, Some(native));
        let totals = rt
            .provider_usage()
            .unwrap()
            .into_iter()
            .find(|r| r.instance.as_str() == id);
        assert_eq!(
            totals.map(|r| (r.reported_runs, r.input_tokens, r.output_tokens)),
            usage.map(|(i, o)| (2, i * 2, o * 2))
        );
        interrupt_held(&rt, &root, &thread, &stopper, "hold").await;
    }
    rt.close().await.unwrap();
}

#[tokio::test]
async fn optional_capabilities_are_checked_per_instance() {
    let (dir, root) = checkout();
    let mut opts = HugglerOptions::new(dir.path().join("state"));
    opts.agent_tools = true;
    opts.instances
        .push(peer(dir.path(), "limited", Provider::AcpRegistry));
    let mut pi = peer(dir.path(), "pi", Provider::Pi);
    pi.pi_extension = Some(dir.path().join("missing.mjs"));
    opts.instances.push(pi);
    let rt = Huggler::open(opts.clone()).await.unwrap();
    let error = rt.probe_provider("pi", &root).await.unwrap_err();
    assert!(error.to_string().contains("install the Pi extension"));
    let report = rt.probe_provider("limited", &root).await.unwrap();
    assert!(!report.resume && !report.agent_tools);
    let thread = launch(&rt, &root, "limited", report.models.first().cloned());
    assert_eq!(
        observe_turn(&rt, &thread, "fresh", true, true).await,
        "fixture simple ok"
    );
    rt.close().await.unwrap();
    let rt = Huggler::open(opts).await.unwrap();
    assert!(
        turn(&rt, &thread, "resume")
            .await
            .error
            .unwrap()
            .contains("cannot load the saved session")
    );
    rt.close().await.unwrap();
}

#[tokio::test]
async fn private_pi_instances_validate_both_native_identifiers_and_home() {
    let (dir, root) = checkout();
    let mut opts = HugglerOptions::new(dir.path().join("state"));
    opts.agent_tools = true;
    for id in ["first", "second"] {
        let mut pi = ProviderInstance::new(id, Provider::Pi, "python3");
        pi.args = vec![
            "-B".into(),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/providers/replay.py"
            )
            .into(),
            "pi".into(),
        ];
        pi.home = Some(dir.path().join(format!("{id}-home")));
        opts.instances.push(pi);
    }
    let rt = Huggler::open(opts.clone()).await.unwrap();
    let report = rt.probe_provider("first", &root).await.unwrap();
    assert!(!report.agent_tools);
    assert_eq!(report.version.as_deref(), Some("1.0.0"));
    let model = "openrouter/deepseek/deepseek-v4-flash".to_owned();
    assert!(report.models.contains(&model));
    let a = launch(&rt, &root, "first", Some(model.clone()));
    let b = launch(&rt, &root, "second", Some(model));
    for thread in [&a, &b] {
        assert_eq!(
            observe_turn(&rt, thread, "fresh", false, true).await,
            "fixture simple ok"
        );
    }
    let native = rt.read(&a).unwrap().native_thread.unwrap();
    assert_eq!(rt.read(&b).unwrap().native_thread, Some(native.clone()));
    rt.close().await.unwrap();
    let rt = Huggler::open(opts.clone()).await.unwrap();
    for mismatch in ["wrong-id", "wrong-path"] {
        let marker = dir.path().join("first-home").join(mismatch);
        fs::write(&marker, "").unwrap();
        let run = turn(&rt, &a, "resume").await;
        assert!(
            run.error
                .unwrap()
                .contains("different session file or identity")
        );
        fs::remove_file(marker).unwrap();
    }
    assert_eq!(
        observe_turn(&rt, &a, "resume", false, true).await,
        "fixture simple ok"
    );
    assert_eq!(rt.read(&a).unwrap().native_thread, Some(native));
    rt.close().await.unwrap();
    let other = dir.path().join("other-home");
    opts.instances[0].home = Some(other.clone());
    let rt = Huggler::open(opts).await.unwrap();
    let run = turn(&rt, &a, "resume").await;
    assert!(run.error.unwrap().contains("original configuration"));
    assert!(!other.exists());
    assert_eq!(
        observe_turn(&rt, &b, "resume", false, true).await,
        "fixture simple ok"
    );
    let totals = rt.provider_usage().unwrap();
    assert_eq!(totals.len(), 2);
    for usage in totals {
        assert_eq!(
            (usage.reported_runs, usage.input_tokens, usage.output_tokens),
            (2, 3572, 56)
        );
    }
    rt.close().await.unwrap();
}

#[tokio::test]
#[ignore = "runs the Pi adapter extension in Node.js against a fixture, without inference"]
async fn pi_extension_calls_scoped_tool() {
    let (dir, root) = checkout();
    let mut opts = HugglerOptions::new(dir.path().join("state"));
    let mut pi = peer(dir.path(), "pi", Provider::Pi);
    pi.home = Some(dir.path().join("pi-home"));
    pi.pi_extension = Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/pi/huggler.mjs"));
    opts.output.name = "atlas".into();
    opts.output.tools.insert(
        "thread_list".into(),
        ToolPresentation {
            name: Some("directory".into()),
            title: Some("Find collaborators".into()),
            ..Default::default()
        },
    );
    opts.instances.push(pi);
    opts.agent_tools = true;
    let rt = Huggler::open(opts).await.unwrap();
    assert!(rt.probe_provider("pi", &root).await.unwrap().agent_tools);
    let thread = launch(&rt, &root, "pi", None);
    assert_eq!(
        observe_turn(&rt, &thread, "fresh", false, true).await,
        "fixture simple ok"
    );
    rt.close().await.unwrap();
}

#[tokio::test]
async fn muse_observes_early_events_and_rejects_unhandled_dialogs() {
    let (dir, root) = checkout();
    let mut opts = HugglerOptions::new(dir.path().join("state"));
    opts.instances
        .push(peer(dir.path(), "muse", Provider::Muse));
    opts.agent_tools = true;
    add_stopper(&mut opts, dir.path());
    let mut pi = peer(dir.path(), "pi", Provider::Pi);
    pi.home = Some(dir.path().join("pi-home"));
    opts.instances.push(pi);
    let rt = Huggler::open(opts.clone()).await.unwrap();
    let report = rt.probe_provider("muse", &root).await.unwrap();
    assert_eq!(report.version.as_deref(), Some("1.4.3"));
    assert!(report.resume && report.agent_tools);
    let thread = launch(&rt, &root, "muse", Some(report.models[0].clone()));
    assert_eq!(
        observe_turn(&rt, &thread, "fresh", false, true).await,
        "fixture simple ok"
    );
    let native = rt.read(&thread).unwrap().native_thread.unwrap();
    rt.close().await.unwrap();
    let rt = Huggler::open(opts).await.unwrap();
    assert_eq!(
        observe_turn(&rt, &thread, "resume", false, true).await,
        "fixture simple ok"
    );
    assert_eq!(rt.read(&thread).unwrap().native_thread, Some(native));
    let usage = &rt.provider_usage().unwrap()[0];
    assert_eq!(
        (usage.reported_runs, usage.input_tokens, usage.output_tokens),
        (2, 64700, 48)
    );
    assert!(
        turn(&rt, &thread, "form")
            .await
            .error
            .unwrap()
            .contains("interactive input")
    );
    let stopper = launch(&rt, &root, "stopper", None);
    interrupt_held(&rt, &root, &thread, &stopper, "hold").await;
    let pi = launch(&rt, &root, "pi", None);
    interrupt_held(&rt, &root, &pi, &stopper, "hold").await;
    rt.close().await.unwrap();
}

struct External(std::process::Child);

impl Drop for External {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn external(instance: &mut ProviderInstance, root: &Path, deny: bool) -> External {
    use std::io::{BufRead, BufReader};
    let mut child = Process::new(&instance.executable)
        .arg(if deny { "deny" } else { "external" })
        .current_dir(root)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut banner = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut banner)
        .unwrap();
    instance.server_url = Some(
        banner
            .trim()
            .strip_prefix("server listening on ")
            .unwrap()
            .to_string(),
    );
    External(child)
}

fn add_stopper(options: &mut HugglerOptions, dir: &Path) {
    let mut stopper = peer(dir, "registry", Provider::AcpRegistry);
    stopper.id = "stopper".into();
    options.instances.push(stopper);
}

async fn interrupt_held(
    rt: &Huggler,
    root: &Path,
    thread: &ThreadId,
    stopper: &ThreadId,
    prompt: &str,
) -> RunId {
    let id = rt
        .send(
            CommandId(uuid::Uuid::new_v4().to_string()),
            thread.clone(),
            prompt.into(),
        )
        .unwrap()
        .run_id
        .unwrap();
    timeout(Duration::from_secs(8), async {
        while !root.join("holding").exists() {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("peer did not receive the prompt");
    fs::remove_file(root.join("holding")).unwrap();
    let prompt = format!("interrupt {} {}", thread.0, id.0);
    assert_eq!(answer(rt, stopper, &prompt).await, "fixture simple ok");
    let run = timeout(Duration::from_secs(6), rt.wait(thread, &id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::Interrupted, "{run:?}");
    id
}

#[tokio::test]
async fn opencode_both_generations_in_managed_and_external_modes() {
    for v2 in [false, true] {
        for managed in [true, false] {
            let (dir, root) = checkout();
            let flavor = if v2 { "open2" } else { "open1" };
            let mut opts = HugglerOptions::new(dir.path().join("state"));
            opts.agent_tools = true;
            let mut instance = peer(dir.path(), flavor, Provider::OpenCode);
            let _external = (!managed).then(|| external(&mut instance, &root, false));
            opts.instances.push(instance);
            add_stopper(&mut opts, dir.path());
            let rt = Huggler::open(opts.clone()).await.unwrap();
            let report = rt.probe_provider(flavor, &root).await.unwrap();
            assert_eq!(report.agent_tools, managed);
            assert!(report.resume && report.authenticated.is_none());
            assert_eq!(
                report.version.as_deref(),
                Some(if v2 { "2.0.18" } else { "1.14.39" })
            );
            let thread = launch(&rt, &root, flavor, Some(report.models[0].clone()));
            let first = observe_turn(&rt, &thread, "fresh", false, v2).await;
            assert!(first.contains(if v2 { "√391" } else { "fixture simple ok" }));
            let native = rt.read(&thread).unwrap().native_thread.unwrap();
            rt.close().await.unwrap();
            let rt = Huggler::open(opts.clone()).await.unwrap();
            assert_eq!(observe_turn(&rt, &thread, "resume", false, v2).await, first);
            assert_eq!(rt.read(&thread).unwrap().native_thread, Some(native));
            let usage = rt.provider_usage().unwrap();
            if v2 {
                assert_eq!(
                    (
                        usage[0].reported_runs,
                        usage[0].input_tokens,
                        usage[0].output_tokens
                    ),
                    (2, 17402, 226)
                );
                for (prompt, error) in [
                    ("disconnect", "disconnected"),
                    ("queued-disconnect", "disconnected"),
                    ("truncated", "HTTP exchange failed"),
                    ("wrong-session", "admission"),
                    ("wrong-id", "admission"),
                    ("form", "interactive form"),
                    ("refused", "409"),
                    ("joined", "another prompt"),
                ] {
                    let aborted = root.join("aborted");
                    let cancelled = root.join("cancelled-inbox");
                    let _ = fs::remove_file(&aborted);
                    let _ = fs::remove_file(&cancelled);
                    assert!(
                        turn(&rt, &thread, prompt)
                            .await
                            .error
                            .unwrap()
                            .contains(error)
                    );
                    assert_eq!(aborted.exists(), matches!(prompt, "disconnect" | "form"));
                    assert_eq!(
                        cancelled.exists(),
                        matches!(
                            prompt,
                            "queued-disconnect" | "truncated" | "wrong-id" | "wrong-session"
                        )
                    );
                }
            } else {
                assert_eq!(
                    (
                        usage[0].reported_runs,
                        usage[0].input_tokens,
                        usage[0].output_tokens
                    ),
                    (2, 1142, 18)
                );
            }
            if !v2 {
                assert!(
                    turn(&rt, &thread, "refused")
                        .await
                        .error
                        .unwrap()
                        .contains("409")
                );
                assert!(!root.join("aborted").exists());
                assert!(
                    turn(&rt, &thread, "truncated")
                        .await
                        .error
                        .unwrap()
                        .contains("HTTP exchange failed")
                );
                assert!(root.join("aborted").exists());
            }
            let aborted = root.join("aborted");
            if aborted.exists() {
                fs::remove_file(&aborted).unwrap();
            }
            let stopper = launch(&rt, &root, "stopper", None);
            if v2 {
                interrupt_held(&rt, &root, &thread, &stopper, "hold-admission").await;
                assert!(root.join("cancelled-inbox").exists());
                assert!(!aborted.exists());
            }
            let run = interrupt_held(&rt, &root, &thread, &stopper, "hold").await;
            rt.close().await.unwrap();
            assert!(
                aborted.exists(),
                "{flavor} did not abort the outstanding prompt"
            );
            let port = fs::read_to_string(root.join("last-port")).unwrap();
            assert_eq!(
                std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok(),
                !managed
            );
            let rt = Huggler::open(opts).await.unwrap();
            assert_eq!(
                rt.wait(&thread, &run).await.unwrap().status,
                RunStatus::Interrupted
            );
            rt.close().await.unwrap();
        }
    }
}

#[tokio::test]
async fn external_server_rejects_invalid_credentials() {
    let (dir, root) = checkout();
    let mut opts = HugglerOptions::new(dir.path().join("state"));
    let mut server = peer(dir.path(), "open2", Provider::OpenCode);
    let _external = external(&mut server, &root, true);
    opts.instances.push(server);
    let rt = Huggler::open(opts).await.unwrap();
    assert!(matches!(
        rt.probe_provider("open2", &root).await,
        Err(Error::ServerAccessDenied)
    ));
    rt.close().await.unwrap();
}
