//! Lock (⇧⌘L): deck's window shows only a lock screen until you prove it is
//! you, with Touch ID or your Mac's password, through LocalAuthentication.
//! Terminals and agents keep running underneath; nothing of them is drawn.

use block2::RcBlock;
use futures::channel::oneshot;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::{NSError, NSString};
use std::sync::Mutex;

#[link(name = "LocalAuthentication", kind = "framework")]
extern "C" {}

/// Touch ID, Apple Watch or the Mac's password, whichever it offers.
const DEVICE_OWNER: isize = 2;

/// Ask macOS to confirm it is you. True when it did.
pub async fn authenticate(reason: &str) -> bool {
    let Some(class) = AnyClass::get(c"LAContext") else {
        return false;
    };
    // SAFETY: LAContext's plain `new`.
    let context: Retained<AnyObject> = unsafe { msg_send![class, new] };
    let (tx, rx) = oneshot::channel::<bool>();
    let tx = Mutex::new(Some(tx));
    let reply = RcBlock::new(move |ok: Bool, _error: *mut NSError| {
        if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
            let _ = tx.send(ok.as_bool());
        }
    });
    let reason = NSString::from_str(reason);
    // SAFETY: the documented selector; the block only touches a
    // Mutex-guarded sender, so it may run on LocalAuthentication's queue.
    unsafe {
        let _: () = msg_send![
            &*context,
            evaluatePolicy: DEVICE_OWNER,
            localizedReason: &*reason,
            reply: &*reply
        ];
    }
    let ok = rx.await.unwrap_or(false);
    drop(context);
    ok
}
