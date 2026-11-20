mod conversation;
#[cfg(all(feature = "qualification", target_os = "macos"))]
mod qualification;

use anyhow::{Context as _, Result};
use conversation::Desktop;
use gpui::{prelude::*, *};
use gpui_component::Root;
use huggler::{Huggler, HugglerOptions};
use std::{path::PathBuf, sync::Arc};

const APP_ID: &str = "dev.huggler.stage2";
actions!(huggler_desktop, [Quit]);

fn main() -> Result<()> {
    let state = std::env::var_os("HUGGLER_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|path| path.join(APP_ID)))
        .context("cannot locate Huggler app data")?;
    #[cfg(all(feature = "qualification", target_os = "macos"))]
    let driver = qualification::load()?;

    #[cfg(all(feature = "qualification", target_os = "macos"))]
    let interactive = driver.is_none();
    #[cfg(not(all(feature = "qualification", target_os = "macos")))]
    let interactive = true;

    let mut options = HugglerOptions::new(&state);
    options.codex.request_approvals = true;
    if let Some(path) = std::env::var_os("HUGGLER_CODEX") {
        options.codex.executable = path.into();
    }
    if let Some(path) = std::env::var_os("HUGGLER_CLAUDE_ACP") {
        options.claude_agent.executable = path.into();
    }
    let executor = tokio::runtime::Runtime::new()?;
    let runtime = Arc::new(executor.block_on(Huggler::open(options))?);

    Application::new().run(move |cx| {
        gpui_component::init(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "Huggler".into(),
            items: vec![MenuItem::action("Quit Huggler", Quit)],
        }]);

        let owner = runtime.clone();
        cx.on_app_quit(move |_| {
            // GPUI bounds the time it will poll a quit future. Join provider workers
            // here while the Tokio executor is still running, including on macOS Quit.
            if let Err(error) = executor.block_on(owner.close()) {
                eprintln!("Could not close Huggler: {error}");
            }
            std::future::ready(())
        })
        .detach();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(900.), px(660.)), cx);
        let _window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(640.), px(440.))),
                    focus: interactive,
                    app_id: Some(APP_ID.into()),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("Huggler");
                    let view = cx.new(|cx| Desktop::new(runtime, state, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("open Huggler window");
        if interactive {
            cx.activate(true);
        }
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        if let Some(driver) = driver {
            qualification::run(driver, _window.into(), cx);
        }
    });
    Ok(())
}
