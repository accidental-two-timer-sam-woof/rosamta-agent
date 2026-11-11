//! System notifications: UNUserNotificationCenter from the helper app on
//! macOS (ADR 0004), the desktop notification service over D-Bus on Linux.
//! Where neither is available nothing is shown; the review queue and the
//! shell reminder still are.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::i18n::{self, Lang};

use super::Event;

/// What a click on a review notification asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Adopt,
    Ignore,
    Review,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Adopt, Action::Ignore, Action::Review];

    pub fn id(self) -> &'static str {
        match self {
            Action::Adopt => "adopt",
            Action::Ignore => "ignore",
            Action::Review => "review",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Action::ALL.into_iter().find(|action| action.id() == id)
    }

    pub fn label(self, lang: Lang) -> String {
        i18n::text(lang, &format!("action.{}", self.id()), &[])
    }

    /// A button, or a click on the notification itself, which opens the
    /// app like Show Diff.
    fn from_response(id: &str) -> Option<Self> {
        match id {
            "default" | "com.apple.UNNotificationDefaultActionIdentifier" => Some(Action::Review),
            id => Action::from_id(id),
        }
    }
}

/// With nothing to decide, the one button only opens the app.
fn view_label(lang: Lang) -> String {
    i18n::text(lang, "action.view", &[])
}

/// One notification. `items` are the review queue IDs its actions apply
/// to; a notification about a problem has none and no actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub body: String,
    pub items: Vec<u32>,
    /// Items that are not decisions, such as templates waiting for a
    /// terminal, offer only View.
    pub view_only: bool,
}

impl Note {
    fn actions(&self) -> &'static [Action] {
        match (self.items.is_empty(), self.view_only) {
            (true, _) => &[],
            (false, true) => &[Action::Review],
            (false, false) => &Action::ALL,
        }
    }
}

/// `review:3,4` carries the queue IDs through the notification system. A
/// problem is named by its text: two shown at once must not replace each
/// other, while the same one shown again does.
fn identifier(note: &Note) -> String {
    if note.items.is_empty() {
        return format!("problem:{}", note.body);
    }
    let ids: Vec<String> = note.items.iter().map(u32::to_string).collect();
    format!("review:{}", ids.join(","))
}

fn items(identifier: &str) -> Vec<u32> {
    identifier
        .strip_prefix("review:")
        .map(|ids| ids.split(',').filter_map(|id| id.parse().ok()).collect())
        .unwrap_or_default()
}

pub trait Notifier: Send {
    /// Whether the note reached the notification service.
    fn show(&self, note: &Note) -> bool;
}

/// Stops trying a note the service keeps refusing: a session bus with no
/// notification service on it (a server) never takes one, and the note
/// would stay unannounced for good.
pub struct GiveUp {
    inner: Box<dyn Notifier>,
    /// How long a note is tried, however many times in that time.
    limit: Duration,
    /// Without a try for this long, a note that reads the same is a new one.
    pause: Duration,
    /// By note text: when it was first and last refused.
    refused: std::sync::Mutex<std::collections::BTreeMap<String, (Instant, Instant)>>,
}

impl GiveUp {
    pub fn new(inner: Box<dyn Notifier>, limit: Duration, pause: Duration) -> Self {
        Self {
            inner,
            limit,
            pause,
            refused: Default::default(),
        }
    }

    /// Whether the note refused at `now` has been tried for long enough.
    pub(super) fn done(&self, body: &str, now: Instant) -> bool {
        let mut refused = self.refused.lock().unwrap();
        refused.retain(|_, (_, last)| now.duration_since(*last) <= self.pause);
        let (first, last) = refused.entry(body.to_string()).or_insert((now, now));
        *last = now;
        let done = now.duration_since(*first) >= self.limit;
        if done {
            refused.remove(body);
        }
        done
    }
}

impl Notifier for GiveUp {
    fn show(&self, note: &Note) -> bool {
        if self.inner.show(note) {
            self.refused.lock().unwrap().remove(&note.body);
            return true;
        }
        self.done(&note.body, Instant::now())
    }
}

/// For hosts without a notification service: there is nothing to wait for.
pub struct Silent;

impl Notifier for Silent {
    fn show(&self, _note: &Note) -> bool {
        true
    }
}

#[cfg(target_os = "linux")]
pub use linux::Desktop;

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::{Mutex, OnceLock};

    use notify_rust::{ActionResponse, NotificationHandle};

    use super::*;

    pub struct Desktop {
        lang: Lang,
        events: Sender<Event>,
        /// Asked on first use: the notification service may start after
        /// the daemon does.
        actions: OnceLock<bool>,
        /// The review notification still waiting for a click. A newer one
        /// closes it, so one thread at most waits.
        waiting: Mutex<Option<NotificationHandle>>,
    }

    /// Whether there is a session bus to ask, where the bus library looks
    /// for one: the address, the runtime directory, or its default place.
    fn session_bus() -> bool {
        // SAFETY: geteuid has no preconditions and cannot fail.
        let default = format!("/run/user/{}", unsafe { libc::geteuid() });
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|address| !address.is_empty())
            || std::env::var_os("XDG_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .into_iter()
                .chain([default.into()])
                .any(|dir| dir.join("bus").exists())
    }

    impl Desktop {
        pub fn new(lang: Lang, events: Sender<Event>) -> Self {
            Self {
                lang,
                events,
                actions: OnceLock::new(),
                waiting: Mutex::new(None),
            }
        }

        /// Whether the server shows buttons; `None` while no notification
        /// service answers on the session bus.
        fn actions(&self) -> Option<bool> {
            if let Some(actions) = self.actions.get() {
                return Some(*actions);
            }
            let capabilities = notify_rust::get_capabilities().ok()?;
            Some(
                *self
                    .actions
                    .get_or_init(|| capabilities.iter().any(|cap| cap == "actions")),
            )
        }
    }

    impl Notifier for Desktop {
        fn show(&self, note: &Note) -> bool {
            // Without a bus (a container, a bare server) no notification
            // service will ever answer: there is nothing to wait for.
            if !session_bus() {
                return true;
            }
            let Some(actions) = self.actions() else {
                return false;
            };
            let mut notification = notify_rust::Notification::new();
            notification
                .appname("rosamta")
                .summary("rosamta")
                .body(&note.body);
            let ask = actions && !note.items.is_empty();
            if ask {
                for &action in note.actions() {
                    let label = match note.view_only {
                        true => view_label(self.lang),
                        false => action.label(self.lang),
                    };
                    notification.action(action.id(), &label);
                }
            }
            let Ok(handle) = notification.show() else {
                return false;
            };
            if !ask {
                return true;
            }
            let id = handle.id();
            let ids = items(&identifier(note));
            let events = self.events.clone();
            // Blocks until the notification is acted on or closed.
            std::thread::spawn(move || {
                let _ = notify_rust::handle_action(id, |response| {
                    if let ActionResponse::Custom(name) = response
                        && let Some(action) = Action::from_response(name)
                    {
                        let _ = events.send(Event::Action(action, ids));
                    }
                });
            });
            let previous = self
                .waiting
                .lock()
                .ok()
                .and_then(|mut waiting| waiting.replace(handle));
            if let Some(previous) = previous {
                previous.close();
            }
            true
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::{UserNotifications, in_app_bundle, run_app};

#[cfg(target_os = "macos")]
mod macos {
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::{
        MainThreadMarker, NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSSet, NSString,
    };
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
        UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    use std::path::PathBuf;

    use super::*;

    const CATEGORY: &str = "review";
    const VIEW_ONLY: &str = "review-view";

    /// UNUserNotificationCenter works only in a process with a bundle ID,
    /// which the daemon has when launchd starts it inside the helper app.
    pub fn in_app_bundle() -> bool {
        NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    pub struct Ivars {
        events: Sender<Event>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "RosamtaNotificationDelegate"]
        #[ivars = Ivars]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                handler: &block2::DynBlock<dyn Fn()>,
            ) {
                let id = response.notification().request().identifier().to_string();
                if let Some(action) =
                    Action::from_response(&response.actionIdentifier().to_string())
                {
                    let _ = self.ivars().events.send(Event::Action(action, items(&id)));
                }
                handler.call(());
            }

            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
            }
        }
    );

    /// Posts to the notification center; results go to `daemon.log` in
    /// `state`, since launchd discards the daemon's output.
    pub struct UserNotifications {
        state: PathBuf,
    }

    impl UserNotifications {
        /// Register the review actions and ask for permission once; a
        /// refusal is remembered by the system and later notes are dropped.
        pub fn set_up(lang: Lang, events: Sender<Event>, state: PathBuf) -> Self {
            // Check in as an app before using the center, the order the
            // spike verified (ADR 0004).
            let mtm = MainThreadMarker::new().expect("set up on the main thread");
            let app = NSApplication::sharedApplication(mtm);
            // No Dock icon, no menu bar.
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let delegate: Retained<Delegate> = {
                let this = Delegate::alloc().set_ivars(Ivars { events });
                unsafe { msg_send![super(this), init] }
            };
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            // The center holds its delegate weakly.
            std::mem::forget(delegate);
            let category = |identifier: &str, actions: &[Action], view_only: bool| {
                let actions: Vec<_> = actions
                    .iter()
                    .map(|&action| {
                        let label = match view_only {
                            true => view_label(lang),
                            false => action.label(lang),
                        };
                        let options = if action == Action::Review {
                            UNNotificationActionOptions::Foreground
                        } else {
                            UNNotificationActionOptions::empty()
                        };
                        UNNotificationAction::actionWithIdentifier_title_options(
                            &NSString::from_str(action.id()),
                            &NSString::from_str(&label),
                            options,
                        )
                    })
                    .collect();
                UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                    &NSString::from_str(identifier),
                    &NSArray::from_retained_slice(&actions),
                    &NSArray::new(),
                    UNNotificationCategoryOptions::empty(),
                )
            };
            center.setNotificationCategories(&NSSet::from_retained_slice(&[
                category(CATEGORY, &Action::ALL, false),
                category(VIEW_ONLY, &[Action::Review], true),
            ]));
            let log_state = state.clone();
            let done = RcBlock::new(move |granted: Bool, err: *mut NSError| {
                let reason = describe(err);
                if !granted.as_bool() {
                    super::super::log_to(
                        &log_state,
                        &format!("notifications are not allowed: {reason}"),
                    );
                }
            });
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &done,
            );
            Self { state }
        }
    }

    impl Notifier for UserNotifications {
        fn show(&self, note: &Note) -> bool {
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str("rosamta"));
            content.setBody(&NSString::from_str(&note.body));
            let category = match note.actions() {
                [] => None,
                [Action::Review] => Some(VIEW_ONLY),
                _ => Some(CATEGORY),
            };
            if let Some(category) = category {
                content.setCategoryIdentifier(&NSString::from_str(category));
            }
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&identifier(note)),
                &content,
                None,
            );
            let state = self.state.clone();
            let done = RcBlock::new(move |err: *mut NSError| {
                if !err.is_null() {
                    super::super::log_to(
                        &state,
                        &format!("notification not shown: {}", describe(err)),
                    );
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, Some(&done));
            // Delivery is reported later, to the log.
            true
        }
    }

    fn describe(err: *mut NSError) -> String {
        if err.is_null() {
            return "no reason given".into();
        }
        // SAFETY: a non-null error from a completion handler is a valid NSError.
        unsafe { (*err).localizedDescription() }.to_string()
    }

    /// Run the AppKit loop that delivers notification clicks; never returns.
    pub fn run_app() -> ! {
        let mtm = MainThreadMarker::new().expect("the daemon's main thread runs the app");
        NSApplication::sharedApplication(mtm).run();
        std::process::exit(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_carry_queue_ids() {
        let note = Note {
            body: String::new(),
            items: vec![3, 14],
            view_only: false,
        };
        assert_eq!(identifier(&note), "review:3,14");
        assert_eq!(items("review:3,14"), [3, 14]);
        assert!(items("problem:1").is_empty());
        // Two problems shown at once must not replace each other.
        let problem = |body: &str| {
            identifier(&Note {
                body: body.into(),
                items: Vec::new(),
                view_only: false,
            })
        };
        assert_ne!(problem("Pulling failed"), problem("Publishing failed"));
        assert!(items(&problem("Pulling failed")).is_empty());
        assert_eq!(Action::from_id("ignore"), Some(Action::Ignore));
        assert_eq!(Action::from_id("default"), None);
        assert_eq!(Action::from_response("default"), Some(Action::Review));
    }
}
