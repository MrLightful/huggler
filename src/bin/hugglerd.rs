#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> huggler::Result<()> {
    use huggler::{Error, Huggler, HugglerOptions, machine};
    use std::{path::PathBuf, sync::Arc};

    let state = std::env::var_os("HUGGLER_MACHINE_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".huggler/daemon")))
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            Error::Invalid("set HUGGLER_MACHINE_STATE_DIR to an absolute path".into())
        })?;
    let socket = state.join("control.sock");
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "socket-path" => {
            println!("{}", serde_json::to_string(&socket)?);
        }
        [command] if command == "serve" => {
            use tokio::signal::unix::{SignalKind, signal};
            let mut terminate = signal(SignalKind::terminate())?;
            let mut interrupt = signal(SignalKind::interrupt())?;
            let mut options = HugglerOptions::new(state);
            options.agent_tools = true;
            options.codex.request_approvals = true;
            options.codex.workspace_write = true;
            if let Some(executable) = std::env::var_os("HUGGLER_CODEX") {
                options.codex.executable = executable.into();
            }
            let runtime = Arc::new(Huggler::open(options).await?);
            let result = machine::serve(runtime.clone(), socket, async move {
                tokio::select! { _ = terminate.recv() => (), _ = interrupt.recv() => () }
            })
            .await;
            let closed = runtime.close().await;
            result.and(closed)?;
        }
        _ => return Err(Error::Invalid("usage: hugglerd <serve|socket-path>".into())),
    }
    Ok(())
}

#[cfg(not(unix))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::other("hugglerd requires Unix sockets"))
}
