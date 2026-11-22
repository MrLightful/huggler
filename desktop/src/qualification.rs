//! Qualification builds drive the installed window using its painted layout and native input handlers.
use anyhow::{Result, ensure};
use gpui::*;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::{MainThreadMarker, NSPoint};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    time::{Duration, Instant},
};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Click(String),
    Hover(String),
    DragX(String, i16),
    CenterX(String, i16),
    NextFileSelection(Vec<std::path::PathBuf>),
    NextSavePath(std::path::PathBuf),
    FileSize(std::path::PathBuf, u64),
    Resize([u16; 2]),
    Type(String),
    Key(String),
    See(String),
    RightOf([String; 2]),
    Above([String; 2]),
    InLowerHalf(String),
    InUpperHalf(String),
    MaxHeight(String, u16),
    Contains(String),
    Absent(String),
    SeeAny(Vec<String>),
    PermissionFrom(String),
}

pub fn load() -> Result<Option<Vec<Step>>> {
    let Some(path) = std::env::var_os("HUGGLER_UI_SCRIPT") else {
        return Ok(None);
    };
    ensure!(
        std::env::var_os("HUGGLER_STATE_DIR").is_some(),
        "qualification needs isolated state"
    );
    Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
}

#[derive(Default)]
struct Controls(HashMap<SharedString, Bounds<Pixels>>);
impl Global for Controls {}

// Script the OS picker boundary while exercising the real composer and filesystem.
#[derive(Default)]
struct FileSelection(Option<Vec<std::path::PathBuf>>);
impl Global for FileSelection {}

pub fn take_file_selection(cx: &mut App) -> Option<Vec<std::path::PathBuf>> {
    cx.default_global::<FileSelection>().0.take()
}

#[derive(Default)]
struct SavePath(Option<std::path::PathBuf>);
impl Global for SavePath {}

pub fn take_save_path(cx: &mut App) -> Option<std::path::PathBuf> {
    cx.default_global::<SavePath>().0.take()
}

pub fn begin_frame(cx: &mut App) {
    cx.default_global::<Controls>().0.clear();
}

pub fn observe(key: SharedString, element: Div) -> Div {
    element.on_children_prepainted(move |bounds, _, cx| {
        if let Some(bounds) = bounds.first() {
            observe_bounds(key.clone(), *bounds, cx);
        }
    })
}

pub(crate) fn observe_bounds(key: SharedString, bounds: Bounds<Pixels>, cx: &mut App) {
    cx.global_mut::<Controls>().0.insert(key, bounds);
}

fn advance(step: &Step, window: &mut Window, cx: &mut App) -> bool {
    // AppKit can suspend display-link callbacks for a background window.
    // Lay out a fresh frame before hit-testing or dispatching to the focused editor.
    window.draw(cx).clear();
    let controls = &cx.default_global::<Controls>().0;
    match step {
        Step::NextFileSelection(paths) => {
            cx.default_global::<FileSelection>().0 = Some(paths.clone());
            true
        }
        Step::NextSavePath(path) => {
            cx.default_global::<SavePath>().0 = Some(path.clone());
            true
        }
        Step::FileSize(path, size) => fs::metadata(path).is_ok_and(|file| file.len() == *size),
        Step::Resize([width, height]) => {
            window.resize(size(px(f32::from(*width)), px(f32::from(*height))));
            true
        }
        Step::CenterX(key, x) => controls
            .get(key.as_str())
            .is_some_and(|bounds| (f32::from(bounds.center().x) - f32::from(*x)).abs() < 2.),
        Step::Click(key) | Step::DragX(key, _) | Step::Hover(key) => {
            let Some(bounds) = controls.get(key.as_str()) else {
                return false;
            };
            let position = bounds.center();
            let app =
                NSApplication::sharedApplication(MainThreadMarker::new().expect("GPUI thread"));
            let Some(native) = app
                .windows()
                .iter()
                .find(|candidate| candidate.title().to_string() == "Huggler")
            else {
                return false;
            };
            let events = match step {
                Step::Hover(_) => vec![(NSEventType::MouseMoved, 0.)],
                Step::DragX(_, delta) => {
                    let dx = f64::from(*delta);
                    vec![
                        (NSEventType::LeftMouseDown, 0.),
                        (NSEventType::LeftMouseDragged, dx / 2.),
                        (NSEventType::LeftMouseDragged, dx),
                        (NSEventType::LeftMouseUp, dx),
                    ]
                }
                _ => vec![
                    (NSEventType::LeftMouseDown, 0.),
                    (NSEventType::LeftMouseUp, 0.),
                ],
            };
            for (kind, offset) in events {
                let location = NSPoint::new(
                    f64::from(position.x) + offset,
                    (window.viewport_size().height - position.y).into(),
                );

                let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                    kind, location, NSEventModifierFlags::empty(), 0., native.windowNumber(), None, 0, 1, 1.
                ).expect("create click event");
                app.postEvent_atStart(&event, false);
            }
            true
        }
        Step::Key(key) => window.dispatch_keystroke(
            Keystroke::parse(key).expect("valid qualification keystroke"),
            cx,
        ),
        Step::Type(value) => window.dispatch_keystroke(
            Keystroke {
                key: "text".into(),
                key_char: Some(value.clone()),
                modifiers: Default::default(),
            },
            cx,
        ),
        Step::See(key) => controls.contains_key(key.as_str()),
        Step::InLowerHalf(key) => controls.get(key.as_str()).is_some_and(|bounds| {
            let height = window.viewport_size().height;
            bounds.center().y >= height / 2. && bounds.bottom() <= height
        }),
        Step::InUpperHalf(key) => controls.get(key.as_str()).is_some_and(|bounds| {
            bounds.top() >= px(0.) && bounds.bottom() <= window.viewport_size().height / 2.
        }),
        Step::MaxHeight(key, height) => controls.get(key.as_str()).is_some_and(|bounds| {
            bounds.size.height > px(0.) && bounds.size.height <= px(f32::from(*height))
        }),
        Step::RightOf([item, reference]) => controls
            .get(item.as_str())
            .zip(controls.get(reference.as_str()))
            .is_some_and(|(item, reference)| item.left() >= reference.right()),
        Step::Above([item, reference]) => controls
            .get(item.as_str())
            .zip(controls.get(reference.as_str()))
            .is_some_and(|(item, reference)| item.bottom() <= reference.top()),
        Step::Contains(text) => controls.keys().any(|key| key.contains(text.as_str())),
        Step::SeeAny(keys) => keys.iter().any(|key| controls.contains_key(key.as_str())),
        Step::Absent(key) => !controls.contains_key(key.as_str()),
        Step::PermissionFrom(path) => controls.keys().any(|key| {
            key.starts_with("permission-title/") && key.ends_with(&format!(" ({path})"))
        }),
    }
}

pub fn run(steps: Vec<Step>, window: AnyWindowHandle, cx: &App) {
    cx.spawn(async move |cx| {
        for step in steps {
            let deadline = Instant::now() + Duration::from_secs(100);
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                if window.update(cx, |_, window, cx| advance(&step, window, cx))? {
                    break;
                }
                let error = cx.update(|cx| {
                    cx.global::<Controls>()
                        .0
                        .keys()
                        .find_map(|key| key.strip_prefix("error/").map(str::to_owned))
                })?;
                if let Some(error) = error {
                    eprintln!("Desktop error at {step:?}: {error}");
                    cx.update(|cx| cx.quit())?;
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    let keys = cx.update(|cx| format!("{:?}", cx.global::<Controls>().0.keys()))?;
                    eprintln!("Timed out at {step:?}. Rendered controls: {keys}");
                    cx.update(|cx| cx.quit())?;
                    return Ok(());
                }
            }
        }
        println!("GPUI workflow passed");
        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}
