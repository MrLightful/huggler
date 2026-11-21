// Only the installed qualification build needs names for the painted controls.
macro_rules! control {
    ($name:expr, $child:expr $(,)?) => {{
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let key = SharedString::from($name);
        let element = div().child($child);
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        let element = crate::qualification::observe(key, element);
        element
    }};
}

mod changes;
mod fonts;
mod git;
mod paths;
mod questions;
mod settings;
mod ui;

mod conversation;
#[cfg(all(feature = "qualification", target_os = "macos"))]
mod qualification;

use anyhow::{Context as _, Result};
use conversation::Desktop;
use gpui::{prelude::*, *};
use gpui_component::Root;
use huggler::Huggler;
use std::{path::PathBuf, sync::Arc};

const APP_ID: &str = "dev.huggler.stage2";
actions!(
    huggler_desktop,
    [
        Quit,
        OpenSettings,
        NewConversation,
        NewConversationWithSetup,
        FocusMessage
    ]
);

fn main() -> Result<()> {
    let paths = paths::StoragePaths::discover()?;
    #[cfg(all(feature = "qualification", target_os = "macos"))]
    let driver = qualification::load()?;

    #[cfg(all(feature = "qualification", target_os = "macos"))]
    let interactive = driver.is_none();
    #[cfg(not(all(feature = "qualification", target_os = "macos")))]
    let interactive = true;

    let (prefs, load_error) = match settings::Preferences::load(&paths.state_dir()) {
        Ok(prefs) => (prefs, None),
        Err(error) => (
            settings::Preferences::default(),
            Some(format!("Could not read settings. {error}")),
        ),
    };
    let mut options = paths.runtime_options();
    options.agent_tools = true;
    options.prompt_suggestions = std::env::var("HUGGLER_PROMPT_SUGGESTIONS").as_deref() == Ok("1");
    options.codex.request_approvals = true;
    options.codex.workspace_write = true;
    if let Some(path) = std::env::var_os("HUGGLER_CODEX")
        .map(PathBuf::from)
        .or_else(|| prefs.codex_executable.clone())
    {
        options.codex.executable = path;
    }
    if let Some(path) = std::env::var_os("HUGGLER_CLAUDE_ACP") {
        options.claude_agent.executable = path.into();
    }
    if let Some(path) = std::env::var_os("HUGGLER_PROVIDERS") {
        let file = std::fs::File::open(path).context("open provider instance configuration")?;
        options.instances =
            serde_json::from_reader(file).context("parse provider instance list")?;
    }
    let executor = tokio::runtime::Runtime::new()?;
    let runtime = Arc::new(executor.block_on(Huggler::open(options))?);

    #[cfg(unix)]
    runtime.set_service_host(
        huggler::services::Kind::Terminal,
        Some(Box::new(huggler::services::terminal::TerminalHost)),
    )?;

    let app = Application::new().with_assets(gpui_component_assets::Assets);
    app.run(move |cx| {
        gpui_component::init(cx);
        fonts::init(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-,", OpenSettings, None),
            KeyBinding::new("secondary-n", NewConversation, None),
            KeyBinding::new("secondary-shift-n", NewConversationWithSetup, None),
            KeyBinding::new(
                "shift-enter",
                gpui_component::input::Enter { secondary: true },
                Some(conversation::COMPOSER_CONTEXT),
            ),
            KeyBinding::new("secondary-l", FocusMessage, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Huggler".into(),
            items: vec![
                MenuItem::action("Create", NewConversation),
                MenuItem::action("Create with Setup…", NewConversationWithSetup),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit Huggler", Quit),
            ],
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

        let bounds = Bounds::centered(None, size(px(1120.), px(740.)), cx);
        let window_handle = cx
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
                    let view =
                        cx.new(|cx| Desktop::new(runtime, paths, prefs, load_error, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("open Huggler window");
        cx.on_action(move |_: &FocusMessage, cx| {
            // A dismissed popover can leave GPUI without a focused element to route through.
            cx.defer(move |cx| {
                let _ = window_handle.update(cx, |root, window, cx| {
                    if let Ok(desktop) = root.view().clone().downcast::<Desktop>() {
                        desktop.update(cx, |view, cx| view.focus_message(window, cx));
                    }
                });
            });
        });
        if interactive {
            cx.activate(true);
        }
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        if let Some(driver) = driver {
            qualification::run(driver, window_handle.into(), cx);
        }
    });
    Ok(())
}
