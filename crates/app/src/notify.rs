//! macOS notifications for an agent that needs you: a permission to grant,
//! a question to answer. Clicking one brings deck forward on that pane.
//!
//! Notification Center needs a real app bundle (an identifier to file the
//! notifications under), so a bare `cargo run` binary posts nothing rather
//! than crashing; the installed Deck.app does.

use block2::RcBlock;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Request identifiers are `deck-<pane>-<n>`; the pane is what a click opens.
const PREFIX: &str = "deck-";

static CLICKS: Mutex<Option<UnboundedSender<u64>>> = Mutex::new(None);
static DELEGATE: OnceLock<Retained<Delegate>> = OnceLock::new();

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "DeckNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Show it even while deck is in front: it is only posted when you
        /// are looking at another pane.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            done.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            done: &block2::DynBlock<dyn Fn()>,
        ) {
            let id = response.notification().request().identifier().to_string();
            if let Some(pane) = pane_of(&id) {
                if let Some(tx) = CLICKS.lock().ok().and_then(|t| t.clone()) {
                    let _ = tx.unbounded_send(pane);
                }
            }
            done.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        // SAFETY: NSObject's plain init; the class adds no state.
        unsafe { msg_send![Self::alloc(), init] }
    }
}

fn pane_of(identifier: &str) -> Option<u64> {
    identifier
        .strip_prefix(PREFIX)?
        .split('-')
        .next()?
        .parse()
        .ok()
}

/// Only an app with a bundle identifier can use Notification Center.
fn center() -> Option<Retained<UNUserNotificationCenter>> {
    NSBundle::mainBundle().bundleIdentifier()?;
    Some(UNUserNotificationCenter::currentNotificationCenter())
}

/// Ask for permission (macOS asks the user once) and route clicks; the
/// receiver yields the pane each clicked notification was about.
pub fn start() -> Option<UnboundedReceiver<u64>> {
    let center = center()?;
    let delegate = DELEGATE.get_or_init(Delegate::new);
    center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
    let asked = RcBlock::new(|granted: Bool, _error: *mut NSError| {
        if !granted.as_bool() {
            tracing::info!("notifications not allowed; deck will only mark tabs");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &asked,
    );
    let (tx, rx) = unbounded();
    if let Ok(mut c) = CLICKS.lock() {
        *c = Some(tx);
    }
    Some(rx)
}

/// Post a notification about `pane`. Later ones for the same pane are
/// grouped under it in Notification Center.
pub fn post(pane: u64, title: &str, body: &str) {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let Some(center) = center() else {
        return;
    };
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    content.setThreadIdentifier(&NSString::from_str(&format!("{PREFIX}{pane}")));
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let id = NSString::from_str(&format!("{PREFIX}{pane}-{n}"));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    center.addNotificationRequest_withCompletionHandler(&request, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_carry_their_pane() {
        assert_eq!(pane_of("deck-12-3"), Some(12));
        assert_eq!(pane_of("deck-7"), Some(7));
        assert_eq!(pane_of("other-12-3"), None);
    }
}
