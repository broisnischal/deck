//! Popups. A page's `window.open` that asks for a window of its own (a
//! size, `popup`), or that starts a sign-in, gets one: a small panel over
//! Deck holding a page WebKit links to its opener, so the sign-in can post
//! its answer back and `window.close()` itself, as in Safari. Links with a
//! target and ⌘-clicks still open as Deck tabs.
//!
//! wry hands its new-window handler a request's address and size only, so
//! its UI delegate class is taught to note the rest first (see `teach`),
//! and to hear a page's `window.close()`.
//!
//! The panel is an NSPanel, which never becomes the main window, so Deck's
//! window stays the one gpui and its menus act on. It rides above Deck as a
//! child window and moves with it.

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Imp, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{class, define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSEvent, NSEventModifierFlags, NSPanel,
    NSResponder, NSView, NSWindow, NSWindowDelegate, NSWindowOrderingMode, NSWindowStyleMask,
    NSWindowTabbingMode,
};
use objc2_foundation::{NSNotification, NSNumber, NSObject, NSPoint, NSRect, NSSize, NSString};
use std::cell::{Cell, OnceCell, RefCell};

/// What WebKit says of a window a page asks for, besides its address.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ask {
    /// A link or form with a target was followed, not a script's
    /// `window.open`.
    pub link: bool,
    /// The size the page asked for, in points.
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// The window features ask for a popup: `popup`, or toolbars turned off.
    pub popup: bool,
    /// The new page keeps `window.opener` (the page did not say `noopener`).
    pub opener: bool,
}

/// Whether the window a page asked for opens as a popup linked to it,
/// rather than as a Deck tab, which the page cannot reach. A script that
/// asks for a sized window or a popup, opens an empty one to fill in, or
/// starts a sign-in gets a popup.
pub fn wants(url: &str, ask: Ask) -> bool {
    let sized = ask.width.is_some() || ask.height.is_some();
    !ask.link && ask.opener && (sized || ask.popup || is_blank(url) || is_sign_in(url))
}

fn is_blank(url: &str) -> bool {
    url.is_empty() || url == "about:blank"
}

/// An address that starts a sign-in: a provider's login page, an OAuth
/// step, or Firebase's auth handler.
pub fn is_sign_in(url: &str) -> bool {
    let Some((host, path)) = split(url) else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let path = path.to_ascii_lowercase();
    const HOSTS: &[&str] = &[
        "accounts.google.com",
        "appleid.apple.com",
        "login.microsoftonline.com",
        "login.live.com",
    ];
    HOSTS.contains(&host.as_str())
        || (host == "github.com" && (path == "/login" || path.starts_with("/login/")))
        || path.starts_with("/__/auth/")
        || path
            .split('/')
            .any(|step| matches!(step, "oauth" | "oauth2" | "authorize"))
}

/// An http(s) address's host (no port or user) and path (no query).
fn split(url: &str) -> Option<(&str, &str)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    let path = tail.split(['?', '#']).next().unwrap_or("");
    Some((host, path))
}

/// What a popup's title shows: the host of the page it is on.
fn host(url: &str) -> &str {
    split(url).map_or("", |(h, _)| h)
}

/// A page's own size when it asks for none, and the bounds an asked-for
/// size is kept to.
const SIZE: (f64, f64) = (500., 650.);
const MIN: (f64, f64) = (320., 320.);
const MAX: (f64, f64) = (1200., 1000.);
/// The title bar above a popup's page.
const TITLE_BAR: f64 = 28.;

/// Where a popup's page goes, in AppKit's screen points (y up): the size
/// asked for, or 500 × 650, kept to sensible bounds and to the screen,
/// centred on the opener's window.
fn place(width: Option<f64>, height: Option<f64>, over: NSRect, screen: Option<NSRect>) -> NSRect {
    let size = |v: Option<f64>, default: f64, min: f64, max: f64| {
        v.filter(|v| v.is_finite()).unwrap_or(default).clamp(min, max)
    };
    let mut w = size(width, SIZE.0, MIN.0, MAX.0);
    let mut h = size(height, SIZE.1, MIN.1, MAX.1);
    if let Some(s) = screen {
        w = w.min(s.size.width);
        h = h.min(s.size.height - TITLE_BAR).max(1.);
    }
    let mut x = over.origin.x + (over.size.width - w) / 2.;
    let mut y = over.origin.y + (over.size.height - h - TITLE_BAR) / 2.;
    if let Some(s) = screen {
        x = x.min(s.origin.x + s.size.width - w).max(s.origin.x);
        y = y.min(s.origin.y + s.size.height - h - TITLE_BAR).max(s.origin.y);
    }
    NSRect::new(
        NSPoint::new(x.round(), y.round()),
        NSSize::new(w.round(), h.round()),
    )
}

/// A popup on screen.
struct Popup {
    /// Its page, let go before the panel it is a view of.
    page: wry::WebView,
    /// The page's WKWebView, as WebKit names it in `webViewDidClose:`.
    view: usize,
    panel: Retained<Panel>,
    /// The WKWebView that opened it: a pane's page, or another popup's.
    opener: usize,
    /// The pane's page it came from, maybe by way of other popups.
    owner: usize,
}

thread_local! {
    static POPUPS: RefCell<Vec<Popup>> = const { RefCell::new(Vec::new()) };
    /// The request wry is answering, noted for its handler (see `teach`).
    static ASKED: Cell<Option<Ask>> = const { Cell::new(None) };
    static WATCH: OnceCell<Retained<Watch>> = const { OnceCell::new() };
}

type Obj = *mut AnyObject;
type CreateFn = unsafe extern "C-unwind" fn(Obj, Sel, Obj, Obj, Obj, Obj) -> Obj;
type CloseFn = unsafe extern "C-unwind" fn(Obj, Sel, Obj);

/// wry's own `webView:createWebViewWithConfiguration:…`, which ours wraps.
static WRY_CREATE: std::sync::OnceLock<CreateFn> = std::sync::OnceLock::new();

/// Teach wry's UI delegate class, once, to note what each window request
/// is before wry's handler answers it (see `asked`), and to close a popup
/// whose page calls `window.close()`. WebKit notes what a delegate can do
/// when it is set, so the caller sets `ui` on its webview again after this.
pub fn teach(ui: &AnyObject) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let cls = ui.class();
        let create = sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:);
        if let Some(method) = cls.instance_method(create) {
            // SAFETY: `create_webview` has the selector's signature, and
            // calls the implementation it replaces.
            unsafe {
                let wry = method.set_implementation(std::mem::transmute::<CreateFn, Imp>(create_webview));
                let _ = WRY_CREATE.set(std::mem::transmute::<Imp, CreateFn>(wry));
            }
        }
        // SAFETY: as above; a method the class already has is kept.
        unsafe {
            objc2::ffi::class_addMethod(
                cls as *const _ as *mut _,
                sel!(webViewDidClose:),
                std::mem::transmute::<CloseFn, Imp>(did_close),
                c"v@:@".as_ptr(),
            );
        }
    });
}

/// What the page asked for, read inside wry's new-window handler. Should
/// `teach` not have run, all that is known is the size wry passes on, and
/// only a sized window counts as a popup.
pub fn asked(features: &wry::NewWindowFeatures) -> Ask {
    ASKED.with(Cell::get).unwrap_or(Ask {
        link: features.size.is_none(),
        width: features.size.map(|s| s.width),
        height: features.size.map(|s| s.height),
        popup: false,
        opener: true,
    })
}

unsafe extern "C-unwind" fn create_webview(
    this: Obj,
    sel: Sel,
    webview: Obj,
    config: Obj,
    action: Obj,
    features: Obj,
) -> Obj {
    let Some(wry) = WRY_CREATE.get() else {
        return std::ptr::null_mut();
    };
    // SAFETY: what WebKit handed over, passed on as it came.
    unsafe {
        ASKED.with(|a| a.set(read_ask(action, features)));
        let made = wry(this, sel, webview, config, action, features);
        ASKED.with(|a| a.set(None));
        made
    }
}

/// SAFETY: a WKNavigationAction and its WKWindowFeatures, or null.
unsafe fn read_ask(action: Obj, features: Obj) -> Option<Ask> {
    let action = unsafe { action.as_ref() }?;
    let features = unsafe { features.as_ref() }?;
    let can = |o: &AnyObject, s: Sel| -> bool { unsafe { msg_send![o, respondsToSelector: s] } };
    // SAFETY: plain property reads; the private ones (popup, noopener,
    // both WebKit's own reading of the features) only where present.
    unsafe {
        // WKNavigationTypeLinkActivated and FormSubmitted.
        let kind: isize = msg_send![action, navigationType];
        let width: Option<Retained<NSNumber>> = msg_send![features, width];
        let height: Option<Retained<NSNumber>> = msg_send![features, height];
        let popup = if can(features, sel!(_wantsPopup)) {
            msg_send![features, _wantsPopup]
        } else {
            let bars: Option<Retained<NSNumber>> = msg_send![features, toolbarsVisibility];
            bars.is_some_and(|b| !b.boolValue())
        };
        let opener = !can(action, sel!(_hasOpener)) || msg_send![action, _hasOpener];
        Some(Ask {
            link: matches!(kind, 0 | 1),
            width: width.map(|n| n.doubleValue()),
            height: height.map(|n| n.doubleValue()),
            popup,
            opener,
        })
    }
}

/// The page called `window.close()`. Only a popup's page is closed; a
/// Deck tab stays as it was.
unsafe extern "C-unwind" fn did_close(_: Obj, _: Sel, webview: Obj) {
    let panel = POPUPS.with(|p| {
        p.try_borrow()
            .ok()?
            .iter()
            .find(|p| p.view == webview as usize)
            .map(|p| p.panel.clone())
    });
    if let Some(panel) = panel {
        close_soon(&panel);
    }
}

/// Close `panel` on the next turn of the run loop, clear of the WebKit or
/// AppKit call under way.
fn close_soon(panel: &Panel) {
    // SAFETY: -[NSObject performSelector:withObject:afterDelay:], which
    // keeps the panel until then.
    unsafe {
        let _: () = msg_send![
            panel,
            performSelector: sel!(close),
            withObject: None::<&AnyObject>,
            afterDelay: 0.0f64
        ];
    }
}

/// Open the popup WebKit asked for at `url`, and hand WebKit its page: a
/// panel centred on the opener's window, sized as the page asked, holding
/// a page made from WebKit's configuration (so `window.opener` is linked
/// and the cookies are the opener's) and set up by `setup`. `committed`
/// hears each address the popup moves to; `ready` sees the built page.
/// `None` when it could not be made.
pub fn open(
    url: &str,
    features: wry::NewWindowFeatures,
    setup: impl for<'a> FnOnce(wry::WebViewBuilder<'a>) -> wry::WebViewBuilder<'a>,
    committed: impl Fn(&str) + 'static,
    ready: fn(&wry::WebView),
) -> Option<wry::NewWindowResponse> {
    use wry::{WebViewBuilderExtMacos, WebViewExtMacOS};
    let mtm = MainThreadMarker::new()?;
    let ask = asked(&features);
    let opener = features.opener.webview;
    let parent = opener.window()?;
    let screen = parent.screen().map(|s| s.visibleFrame());
    let panel = Panel::new(mtm, place(ask.width, ask.height, parent.frame(), screen));
    panel.setTitle(&NSString::from_str(host(url)));
    let content = panel.contentView()?;

    // A controller of its own: the opener's would run the opener's
    // scripts in it a second time and send its messages to the opener's
    // pane. The rest of the configuration, its data store and the opener
    // link with it, stays as WebKit made it.
    let config = features.opener.target_configuration;
    // SAFETY: +[WKUserContentController new], set on WebKit's fresh copy
    // of the opener's configuration before any page is made from it.
    unsafe {
        let own: Retained<AnyObject> = msg_send![class!(WKUserContentController), new];
        let _: () = msg_send![&*config, setUserContentController: &*own];
    }
    let titled = Weak::from_retained(&panel);
    let builder = setup(wry::WebViewBuilder::new().with_webview_configuration(config))
        .with_on_page_load_handler(move |event, url| {
            if matches!(event, wry::PageLoadEvent::Started) {
                if let Some(p) = titled.load() {
                    p.setTitle(&NSString::from_str(host(&url)));
                }
                committed(&url);
            }
        });
    let at = std::ptr::NonNull::from(&*content).cast();
    let page = builder.build_as_child(&InView(at)).ok()?;

    let wk = page.webview();
    let view: &NSView = &wk;
    view.setFrame(content.bounds());
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    ready(&page);

    let view_id = Retained::as_ptr(&wk) as usize;
    let opener_id = Retained::as_ptr(&opener) as usize;
    let owner = POPUPS
        .with(|p| p.borrow().iter().find(|p| p.view == opener_id).map(|p| p.owner))
        .unwrap_or(opener_id);
    // SAFETY: main thread; both windows are alive.
    unsafe { parent.addChildWindow_ordered(&panel, NSWindowOrderingMode::Above) };
    panel.makeKeyAndOrderFront(None);
    let responder: &NSResponder = view;
    panel.makeFirstResponder(Some(responder));
    POPUPS.with(|p| {
        p.borrow_mut().push(Popup {
            page,
            view: view_id,
            panel,
            opener: opener_id,
            owner,
        })
    });
    Some(wry::NewWindowResponse::Create {
        webview: Retained::into_super(wk),
    })
}

/// Close the popups a pane's page opened, as the pane goes.
pub fn close_all(owner: usize) {
    let gone: Vec<Popup> = POPUPS.with(|p| {
        let Ok(mut p) = p.try_borrow_mut() else {
            return Vec::new();
        };
        let (gone, kept) = std::mem::take(&mut *p)
            .into_iter()
            .partition(|x| x.owner == owner);
        *p = kept;
        gone
    });
    for Popup { page, panel, .. } in gone {
        drop(page);
        panel.setDelegate(None);
        panel.close();
    }
}

/// A popup's panel is closing: let its page go, and close the popups it
/// opened in turn. The panel itself goes once AppKit is done closing it.
fn closed(panel: usize) {
    let gone = POPUPS.with(|p| {
        let mut p = p.try_borrow_mut().ok()?;
        let i = p
            .iter()
            .position(|x| Retained::as_ptr(&x.panel) as usize == panel)?;
        let gone = p.remove(i);
        for child in p.iter().filter(|x| x.opener == gone.view) {
            close_soon(&child.panel);
        }
        Some(gone)
    });
    if let Some(Popup { page, panel, .. }) = gone {
        drop(page);
        // AppKit has a pool in place while it closes a window.
        let _ = Retained::autorelease_ptr(panel);
    }
}

/// A view to build a page in, as wry takes one.
struct InView(std::ptr::NonNull<std::ffi::c_void>);

impl wry::raw_window_handle::HasWindowHandle for InView {
    fn window_handle(
        &self,
    ) -> Result<wry::raw_window_handle::WindowHandle<'_>, wry::raw_window_handle::HandleError> {
        use wry::raw_window_handle::{AppKitWindowHandle, RawWindowHandle, WindowHandle};
        let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(self.0));
        // SAFETY: the panel's content view, alive while the page is built.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

define_class!(
    /// A popup's window. ⌘W closes it here rather than reaching Deck's
    /// menu, which would close the pane behind it.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeckPopupPanel"]
    struct Panel;

    impl Panel {
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let mods = event.modifierFlags() & NSEventModifierFlags::DeviceIndependentFlagsMask;
            let key = event.charactersIgnoringModifiers();
            if mods == NSEventModifierFlags::Command && key.is_some_and(|k| k.to_string() == "w") {
                self.performClose(None);
                true
            } else {
                // SAFETY: NSWindow's own handling of the rest.
                unsafe { msg_send![super(self), performKeyEquivalent: event] }
            }
        }
    }
);

impl Panel {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let style =
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
        // SAFETY: NSWindow's designated initializer; the class adds no state.
        let panel: Retained<Self> = unsafe {
            msg_send![
                Self::alloc(mtm),
                initWithContentRect: frame,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        // SAFETY: the popup's `Retained` owns the panel, not AppKit.
        unsafe { panel.setReleasedWhenClosed(false) };
        // A sign-in stays up while the user fetches a code from elsewhere.
        panel.setHidesOnDeactivate(false);
        panel.setTabbingMode(NSWindowTabbingMode::Disallowed);
        panel.setContentMinSize(NSSize::new(MIN.0, MIN.1));
        let watch = WATCH.with(|w| {
            w.get_or_init(|| unsafe { msg_send![Watch::alloc(mtm), init] })
                .clone()
        });
        panel.setDelegate(Some(ProtocolObject::from_ref(&*watch)));
        panel
    }
}

define_class!(
    /// Hears a popup's panel close: its close button, ⌘W, or the page.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeckPopupWatch"]
    struct Watch;

    unsafe impl NSObjectProtocol for Watch {}

    unsafe impl NSWindowDelegate for Watch {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, note: &NSNotification) {
            if let Some(panel) = note.object() {
                closed(Retained::as_ptr(&panel) as usize);
            }
        }
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    fn script() -> Ask {
        Ask {
            opener: true,
            ..Ask::default()
        }
    }

    #[test]
    fn scripts_asking_for_a_window_get_a_popup() {
        let sized = Ask {
            width: Some(500.),
            height: Some(600.),
            ..script()
        };
        assert!(wants("https://example.com/popup", sized));
        assert!(wants("https://example.com/p", Ask { popup: true, ..script() }));
        // An empty window the script fills in or sends somewhere.
        assert!(wants("about:blank", script()));
        assert!(wants("", script()));
        // A sign-in, sized or not.
        assert!(wants("https://accounts.google.com/o/oauth2/v2/auth?x=1", script()));
        // A plain window.open is a tab, as is anything with noopener.
        assert!(!wants("https://example.com/", script()));
        assert!(!wants("https://example.com/", Ask { opener: false, ..sized }));
    }

    #[test]
    fn links_stay_tabs() {
        let link = Ask { link: true, ..script() };
        assert!(!wants("https://example.com/", link));
        assert!(!wants("https://accounts.google.com/", link));
        assert!(!wants("about:blank", link));
    }

    #[test]
    fn sign_in_addresses() {
        for url in [
            "https://accounts.google.com/",
            "https://Accounts.Google.com/signin/v2",
            "https://appleid.apple.com/auth/authorize?client_id=x",
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            "https://github.com/login",
            "https://github.com/login/oauth/authorize?client_id=x",
            "https://myapp.firebaseapp.com/__/auth/handler?apiKey=x",
            "https://auth.example.com/__/auth/handler",
            "https://www.facebook.com/v18.0/dialog/oauth?x=1",
            "https://example.auth0.com/authorize?response_type=code",
            "https://discord.com/oauth2/authorize?client_id=1",
            "http://localhost:8080/oauth/callback",
        ] {
            assert!(is_sign_in(url), "{url}");
        }
        for url in [
            "https://example.com/",
            "https://github.com/rust-lang/rust",
            "https://example.com/docs/oauth-guide",
            "https://example.com/?next=/oauth",
            "https://notgoogle.com/accounts.google.com",
            "deckcode://x/oauth",
            "about:blank",
        ] {
            assert!(!is_sign_in(url), "{url}");
        }
    }

    #[test]
    fn hosts_drop_user_port_and_path() {
        assert_eq!(host("https://accounts.google.com/o/oauth2"), "accounts.google.com");
        assert_eq!(host("http://me@127.0.0.1:5173/popup?x#y"), "127.0.0.1");
        assert_eq!(host("about:blank"), "");
    }

    #[test]
    fn popups_are_sized_and_centred() {
        let r = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let over = r(100., 100., 1400., 900.);
        let screen = r(0., 0., 1600., 1000.);
        // What the page asked for, centred on the opener.
        let p = place(Some(500.), Some(600.), over, Some(screen));
        assert_eq!((p.size.width, p.size.height), (500., 600.));
        assert_eq!(p.origin.x, 100. + (1400. - 500.) / 2.);
        assert_eq!(p.origin.y, 100. + (900. - 600. - TITLE_BAR) / 2.);
        // Nothing asked: 500 × 650.
        let p = place(None, None, over, Some(screen));
        assert_eq!((p.size.width, p.size.height), SIZE);
        // Too small, too large, or nonsense.
        let p = place(Some(10.), Some(f64::NAN), over, Some(screen));
        assert_eq!((p.size.width, p.size.height), (MIN.0, SIZE.1));
        let p = place(Some(5000.), Some(5000.), over, None);
        assert_eq!((p.size.width, p.size.height), MAX);
        // Kept on a small screen, title bar and all.
        let small = r(0., 0., 1000., 700.);
        let p = place(Some(1200.), Some(1000.), r(-300., 400., 600., 600.), Some(small));
        assert_eq!((p.size.width, p.size.height), (1000., 700. - TITLE_BAR));
        assert_eq!((p.origin.x, p.origin.y), (0., 0.));
        // Off any screen, it simply follows its opener.
        let p = place(None, None, r(-30000., -30000., 900., 700.), None);
        assert_eq!(p.origin.x, -30000. + 200.);
    }
}
