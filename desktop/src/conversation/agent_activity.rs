use super::*;
use gpui_component::{
    WindowExt,
    notification::{Notification, NotificationType},
};

#[derive(Default)]
pub(super) struct AgentActivity {
    notifications: bool,
    #[cfg(target_os = "macos")]
    keep_awake: bool,
    #[cfg(target_os = "macos")]
    sleep_hold: Option<macos::SleepHold>,
}

impl AgentActivity {
    pub(super) fn update_preferences(
        &mut self,
        prefs: &Preferences,
        _threads: &[ConversationSummary],
    ) {
        #[cfg(target_os = "macos")]
        {
            if prefs.agent_notifications && !self.notifications {
                macos::authorize_notifications();
            }
            self.keep_awake = prefs.keep_awake;
            self.update_sleep_hold(_threads);
        }
        self.notifications = prefs.agent_notifications;
    }

    pub(super) fn update_threads(
        &mut self,
        previous: &[ConversationSummary],
        threads: &[ConversationSummary],
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(target_os = "macos")]
        self.update_sleep_hold(threads);
        if !self.notifications {
            return;
        }
        let previous: HashMap<_, _> = previous
            .iter()
            .map(|thread| (&thread.id, &thread.completed_run))
            .collect();
        for thread in threads {
            let Some(run) = &thread.completed_run else {
                continue;
            };
            if previous.get(&thread.id).and_then(|run| run.as_ref()) == Some(run) {
                continue;
            }
            #[cfg(target_os = "macos")]
            if !window.is_window_active()
                && let Some(delivery) = macos::notify_completed(&run.0, &thread.name)
            {
                let notification = completion_notification(thread);
                window
                    .spawn(cx, async move |cx| {
                        if !matches!(delivery.recv().await, Ok(true)) {
                            let _ = cx.update(|window, cx| {
                                window.push_notification(notification.autohide(false), cx);
                            });
                        }
                    })
                    .detach();
                continue;
            }
            let autohide = window.is_window_active();
            window.push_notification(completion_notification(thread).autohide(autohide), cx);
        }
    }

    #[cfg(target_os = "macos")]
    fn update_sleep_hold(&mut self, threads: &[ConversationSummary]) {
        use huggler::RunStatus;
        let wanted = self.keep_awake
            && threads.iter().any(|thread| {
                thread.active_children
                    || (thread.status == Some(RunStatus::Queued) && !thread.queue_held)
                    || matches!(
                        thread.status,
                        Some(RunStatus::Starting | RunStatus::Running | RunStatus::Waiting)
                    )
            });
        if wanted {
            if self.sleep_hold.is_none() {
                self.sleep_hold = macos::SleepHold::acquire();
            }
        } else {
            self.sleep_hold = None;
        }
    }
}

const COMPLETED: &str = "Conversation completed";

fn completion_notification(thread: &ConversationSummary) -> Notification {
    let name = SharedString::from(thread.name.clone());
    Notification::new()
        .with_type(NotificationType::Success)
        .title(COMPLETED)
        .id1::<AgentActivity>(SharedString::from(thread.id.0.clone()))
        .content(move |_, _, _| {
            control!(
                format!("conversation-completed/{name}"),
                div().text_sm().child(name.clone())
            )
            .into_any_element()
        })
}

#[cfg(target_os = "macos")]
mod macos {
    use block2::RcBlock;
    use core_foundation::{
        base::TCFType,
        string::{CFString, CFStringRef},
    };
    use objc2_foundation::{NSBundle, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNUserNotificationCenter,
    };

    fn has_bundle() -> bool {
        // Notification Center requires an application bundle. The standalone
        // executable uses the window's notification layer.
        NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    pub(super) fn authorize_notifications() {
        if has_bundle() {
            UNUserNotificationCenter::currentNotificationCenter()
                .requestAuthorizationWithOptions_completionHandler(
                    UNAuthorizationOptions::Alert,
                    &RcBlock::new(|_, _| {}),
                );
        }
    }

    pub(super) fn notify_completed(run: &str, name: &str) -> Option<smol::channel::Receiver<bool>> {
        if !has_bundle() {
            return None;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(super::COMPLETED));
        content.setBody(&NSString::from_str(name));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(run),
            &content,
            None,
        );
        let (sender, receiver) = smol::channel::bounded(1);
        center.addNotificationRequest_withCompletionHandler(
            &request,
            Some(&RcBlock::new(
                move |error: *mut objc2_foundation::NSError| {
                    let _ = sender.try_send(error.is_null());
                },
            )),
        );
        Some(receiver)
    }

    const ASSERTION_LEVEL_ON: u32 = 255;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            level: u32,
            name: CFStringRef,
            assertion_id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(assertion_id: u32) -> i32;
    }

    pub(super) struct SleepHold(u32);

    impl SleepHold {
        pub(super) fn acquire() -> Option<Self> {
            let assertion_type = CFString::new("PreventUserIdleSystemSleep");
            let name = CFString::new("Huggler agents are running");
            let mut id = 0;
            // The strings and writable ID stay alive for the call.
            let result = unsafe {
                IOPMAssertionCreateWithName(
                    assertion_type.as_concrete_TypeRef(),
                    ASSERTION_LEVEL_ON,
                    name.as_concrete_TypeRef(),
                    &mut id,
                )
            };
            if result != 0 {
                eprintln!("Could not prevent idle system sleep: {result:#x}");
                return None;
            }
            Some(Self(id))
        }
    }

    impl Drop for SleepHold {
        fn drop(&mut self) {
            // The process owns this assertion ID until release.
            unsafe { IOPMAssertionRelease(self.0) };
        }
    }
}
