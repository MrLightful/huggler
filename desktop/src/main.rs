use huggler::{
    Command, Event, Huggler, HugglerOptions, Project, Receipt, RequestId, RunId, Thread, ThreadId,
    ThreadSummary,
};
use serde::Serialize;
use std::{path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Manager, RunEvent, State, ipc::Channel};

struct Host {
    runtime: Huggler,
    subscription: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum Frame {
    Snapshot(Thread),
    Event(Event),
}

#[tauri::command]
async fn open_project(host: State<'_, Host>, root: PathBuf) -> huggler::Result<Project> {
    host.runtime.open_project(root)
}

#[tauri::command]
async fn threads(host: State<'_, Host>) -> huggler::Result<Vec<ThreadSummary>> {
    host.runtime.threads()
}

#[tauri::command]
async fn dispatch(host: State<'_, Host>, command: Command) -> huggler::Result<Receipt> {
    host.runtime.dispatch(command)
}

#[tauri::command]
fn respond_to_permission(
    host: State<'_, Host>,
    thread: ThreadId,
    run: RunId,
    request: RequestId,
    option: String,
) -> huggler::Result<()> {
    host.runtime
        .respond_to_permission(&thread, &run, &request, option)
}

#[tauri::command]
fn unsubscribe(host: State<'_, Host>) {
    if let Some(task) = host.subscription.lock().unwrap().take() {
        task.abort();
    }
}

#[tauri::command]
fn subscribe(
    host: State<'_, Host>,
    thread: ThreadId,
    channel: Channel<Frame>,
) -> Result<(), String> {
    let mut active = host.subscription.lock().unwrap();
    if let Some(task) = active.take() {
        task.abort();
    }
    let (snapshot, mut stream) = host
        .runtime
        .subscribe(&thread, None)
        .map_err(|e| e.to_string())?;
    channel
        .send(Frame::Snapshot(snapshot))
        .map_err(|e| e.to_string())?;
    *active = Some(tauri::async_runtime::spawn(async move {
        while let Ok(event) = stream.next().await {
            if channel.send(Frame::Event(event)).is_err() {
                break;
            }
        }
    }));
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let mut options = HugglerOptions::new(app.path().app_data_dir()?);
            options.codex.request_approvals = true;
            if let Some(path) = std::env::var_os("HUGGLER_CODEX") {
                options.codex.executable = path.into();
            }
            if let Some(path) = std::env::var_os("HUGGLER_CLAUDE_ACP") {
                options.claude_agent.executable = path.into();
            }
            let runtime = tauri::async_runtime::block_on(Huggler::open(options))?;
            app.manage(Host {
                runtime,
                subscription: Mutex::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_project,
            threads,
            dispatch,
            respond_to_permission,
            subscribe,
            unsubscribe,
            quit
        ])
        .build(tauri::generate_context!())
        .expect("initialize Huggler desktop")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let host = app.state::<Host>();
                if let Some(task) = host.subscription.lock().unwrap().take() {
                    task.abort();
                }
                if let Err(e) = tauri::async_runtime::block_on(host.runtime.close()) {
                    eprintln!("close: {e}");
                }
            }
        });
}
