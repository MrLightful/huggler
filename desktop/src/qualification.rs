//! Qualification builds drive the installed window using its painted layout and native input handlers.
mod screenshot;

use anyhow::{Result, ensure};
use gpui::*;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::{MainThreadMarker, NSPoint, NSString};
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
    DragY(String, i16),
    CenterX(String, i16),
    NextFileSelection(Vec<std::path::PathBuf>),
    NextSavePath(std::path::PathBuf),
    NextPromptAnswer(usize),
    OpenedUrl(String),
    FileSize(std::path::PathBuf, u64),
    ClipboardText(String),
    ClipboardContains(String),
    ClipboardWrite(String),
    FileText(std::path::PathBuf, String),
    Resize([u16; 2]),
    Screenshot(screenshot::Screenshot),
    Type(String),
    Key(String),
    ReleaseEnter,
    See(String),
    RightOf([String; 2]),
    SameLine([String; 2]),
    RememberX(String),
    SameX(String),
    Above([String; 2]),
    InLowerHalf(String),
    InUpperHalf(String),
    MinHeight(String, u16),
    MaxHeight(String, u16),
    Contains(String),
    Absent(String),
    SeeAny(Vec<String>),
    PermissionFrom(String),
}

impl Step {
    pub fn captures_window(&self) -> bool {
        matches!(self, Self::Screenshot(shot) if shot.path.is_some())
    }
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

struct ScreenshotMode;
impl Global for ScreenshotMode {}

pub(crate) fn capturing_screenshots(cx: &App) -> bool {
    cx.has_global::<ScreenshotMode>()
}

#[derive(Default)]
struct RememberedX(Option<Pixels>);
impl Global for RememberedX {}

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

#[derive(Default)]
struct PromptAnswer(Option<usize>);
impl Global for PromptAnswer {}

pub fn take_prompt_answer(cx: &mut App) -> Option<usize> {
    cx.default_global::<PromptAnswer>().0.take()
}

pub(crate) fn scripted() -> bool {
    std::env::var_os("HUGGLER_UI_SCRIPT").is_some()
}

#[derive(Default)]
struct OpenedUrl(Option<String>);
impl Global for OpenedUrl {}

pub fn record_open_url(url: &str, cx: &mut App) -> bool {
    if !scripted() {
        return false;
    }
    cx.default_global::<OpenedUrl>().0 = Some(url.to_owned());
    true
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

fn huggler_window_number(app: &NSApplication) -> Option<isize> {
    app.windows()
        .iter()
        .find(|window| window.title().to_string() == "Huggler")
        .map(|window| window.windowNumber())
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
        Step::NextPromptAnswer(answer) => {
            cx.default_global::<PromptAnswer>().0 = Some(*answer);
            true
        }
        Step::OpenedUrl(url) => cx.default_global::<OpenedUrl>().0.as_ref() == Some(url),
        Step::ClipboardText(expected) => {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_ref()
                == Some(expected)
        }
        Step::ClipboardContains(expected) => cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .is_some_and(|text| text.contains(expected)),
        Step::ClipboardWrite(text) => {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            true
        }
        Step::FileText(path, expected) => fs::read_to_string(path).as_ref().ok() == Some(expected),
        Step::FileSize(path, size) => fs::metadata(path).is_ok_and(|file| file.len() == *size),
        Step::Resize([width, height]) => {
            window.resize(size(px(f32::from(*width)), px(f32::from(*height))));
            true
        }
        Step::Screenshot(_) => unreachable!("screenshots run after presenting the frame"),
        Step::CenterX(key, x) => controls
            .get(key.as_str())
            .is_some_and(|bounds| (f32::from(bounds.center().x) - f32::from(*x)).abs() < 2.),
        Step::Click(key) | Step::DragX(key, _) | Step::DragY(key, _) | Step::Hover(key) => {
            let Some(bounds) = controls.get(key.as_str()) else {
                return false;
            };
            let position = bounds.center();
            let app =
                NSApplication::sharedApplication(MainThreadMarker::new().expect("GPUI thread"));
            let Some(number) = huggler_window_number(&app) else {
                return false;
            };
            let events = match step {
                Step::Hover(_) => vec![(NSEventType::MouseMoved, 0.)],
                Step::DragX(_, delta) | Step::DragY(_, delta) => {
                    let offset = f64::from(*delta);
                    vec![
                        (NSEventType::LeftMouseDown, 0.),
                        (NSEventType::LeftMouseDragged, offset / 2.),
                        (NSEventType::LeftMouseDragged, offset),
                        (NSEventType::LeftMouseUp, offset),
                    ]
                }
                _ => vec![
                    (NSEventType::LeftMouseDown, 0.),
                    (NSEventType::LeftMouseUp, 0.),
                ],
            };
            for (kind, offset) in events {
                let (dx, dy) = if matches!(step, Step::DragY(..)) {
                    (0., offset)
                } else {
                    (offset, 0.)
                };
                let location = NSPoint::new(
                    f64::from(position.x) + dx,
                    f64::from(window.viewport_size().height - position.y) - dy,
                );

                let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                    kind, location, NSEventModifierFlags::empty(), 0., number, None, 0, 1, 1.
                ).expect("create click event");
                app.postEvent_atStart(&event, false);
            }
            true
        }
        Step::Key(key) => window.dispatch_keystroke(
            Keystroke::parse(key).expect("valid qualification keystroke"),
            cx,
        ),
        Step::ReleaseEnter => {
            // GPUI buttons activate on key release. dispatch_keystroke only sends key down.
            let app =
                NSApplication::sharedApplication(MainThreadMarker::new().expect("GPUI thread"));
            let Some(number) = huggler_window_number(&app) else {
                return false;
            };
            let enter = NSString::from_str("\r");
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyUp, NSPoint::new(0., 0.), NSEventModifierFlags::empty(),
                0., number, None, &enter, &enter, false, 36,
            ).expect("create Return release event");
            app.postEvent_atStart(&event, false);
            true
        }
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
        Step::MinHeight(key, height) => controls
            .get(key.as_str())
            .is_some_and(|bounds| bounds.size.height >= px(f32::from(*height))),
        Step::MaxHeight(key, height) => controls.get(key.as_str()).is_some_and(|bounds| {
            bounds.size.height > px(0.) && bounds.size.height <= px(f32::from(*height))
        }),
        Step::RightOf([item, reference]) => controls
            .get(item.as_str())
            .zip(controls.get(reference.as_str()))
            .is_some_and(|(item, reference)| item.left() >= reference.right()),
        Step::SameLine([item, reference]) => controls
            .get(item.as_str())
            .zip(controls.get(reference.as_str()))
            .is_some_and(|(item, reference)| {
                (item.center().y - reference.center().y).abs() < px(2.)
            }),
        Step::RememberX(key) => {
            let Some(bounds) = controls.get(key.as_str()) else {
                return false;
            };
            let x = bounds.center().x;
            cx.default_global::<RememberedX>().0 = Some(x);
            true
        }
        Step::SameX(key) => {
            let x = controls.get(key.as_str()).map(|bounds| bounds.center().x);
            let remembered = cx.default_global::<RememberedX>().0;
            x.zip(remembered)
                .is_some_and(|(x, saved)| (x - saved).abs() < px(2.))
        }
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

pub fn run(steps: Vec<Step>, window: AnyWindowHandle, cx: &mut App) {
    if steps.iter().any(Step::captures_window) {
        cx.set_global(ScreenshotMode);
    }
    cx.spawn(async move |cx| {
        for step in steps {
            if let Step::Screenshot(screenshot) = step {
                let mut cx = cx.clone();
                if let Err(error) = screenshot.run(window, &mut cx).await {
                    eprintln!("Screenshot {} failed: {error:#}", screenshot.name);
                    cx.update(|cx| cx.quit())?;
                    return Ok(());
                }
                continue;
            }
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
