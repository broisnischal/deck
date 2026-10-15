//! A browser pane: a gpui toolbar above a native WKWebView parented into the
//! window and kept glued to the pane's layout bounds.
//!
//! The webview is a real NSView above gpui's Metal layer, so nothing gpui
//! draws can overlap it; everything interactive is laid out beside it. It is
//! kept hidden until it has something to show (no white flash on load), and
//! a blank tab is a gpui page instead of an empty white document.

use crate::field::{Field, FieldEvent};
use crate::icon::{icon, Icon};
use crate::theme::{self, u};
use crate::workspace::icon_button;
use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use gpui::prelude::*;
use gpui::{
    canvas, div, relative, rgb, rgba, Animation, AnimationExt, Bounds, Context, ElementId, Entity,
    EventEmitter, FontWeight, Pixels, SharedString, Subscription, Task, Window,
};
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;
use wry::dpi::{LogicalPosition, LogicalSize};

pub enum BrowserEvent {
    Focused,
    /// The page asked for a new window (target=_blank, window.open).
    OpenTab(String),
    Close,
}

pub enum Msg {
    Title(String),
    Url(String, bool),
    Focused,
    NewWindow(String),
}

pub struct BrowserPane {
    webview: wry::WebView,
    pub url: SharedString,
    pub title: SharedString,
    pub loading: bool,
    pub address: Entity<Field>,
    /// Dev servers listening on this machine, offered on the blank page.
    pub servers: Vec<(u16, String)>,
    /// The app zoom, applied to the page so it scales with everything else.
    zoom: f64,
    bounds: Option<Bounds<Pixels>>,
    /// Whether this pane's tab is the one on screen.
    tab_visible: bool,
    /// Set once the page has painted something; until then the webview stays
    /// hidden and the pane shows black instead of WebKit's white.
    revealed: bool,
    shown: bool,
    _reveal: Option<Task<()>>,
    _events: Task<()>,
    _address: Subscription,
}

impl EventEmitter<BrowserEvent> for BrowserPane {}

/// Clicks inside the page never reach gpui, so the page reports them itself.
const FOCUS_PROBE: &str =
    "window.addEventListener('mousedown', () => window.ipc.postMessage('focus'), true);";

const PAGE_TEXT: &str =
    "document.title + '\\n' + location.href + '\\n\\n' + (document.body ? document.body.innerText : '')";

fn is_blank(url: &str) -> bool {
    url.is_empty() || url == "about:blank"
}

impl BrowserPane {
    /// The fallible half: create the native view inside `window`.
    pub fn build(
        url: &str,
        window: &Window,
    ) -> anyhow::Result<(wry::WebView, UnboundedReceiver<Msg>, String)> {
        let url = if is_blank(url) {
            "about:blank".to_string()
        } else {
            normalize_url(url)
        };
        let (tx, rx) = futures::channel::mpsc::unbounded::<Msg>();
        let (tx_title, tx_load, tx_ipc, tx_new) = (tx.clone(), tx.clone(), tx.clone(), tx);

        let webview = wry::WebViewBuilder::new()
            .with_url(&url)
            .with_bounds(rect(Bounds::default()))
            .with_visible(false)
            .with_initialization_script(FOCUS_PROBE)
            .with_ipc_handler(move |req| {
                if req.body() == "focus" {
                    let _ = tx_ipc.unbounded_send(Msg::Focused);
                }
            })
            .with_document_title_changed_handler(move |t| {
                let _ = tx_title.unbounded_send(Msg::Title(t));
            })
            .with_on_page_load_handler(move |ev, url| {
                let started = matches!(ev, wry::PageLoadEvent::Started);
                let _ = tx_load.unbounded_send(Msg::Url(url, started));
            })
            .with_new_window_req_handler(move |url, _| {
                let _ = tx_new.unbounded_send(Msg::NewWindow(url));
                wry::NewWindowResponse::Deny
            })
            .build_as_child(window)?;
        round_bottom_corners(&webview, theme::CARD_RADIUS - 1.);
        Ok((webview, rx, url))
    }

    pub fn new(
        webview: wry::WebView,
        mut rx: UnboundedReceiver<Msg>,
        url: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let ok = this.update(cx, |p, cx| {
                    match msg {
                        Msg::Title(t) => p.title = t.into(),
                        Msg::Url(u, started) => {
                            p.loading = started && !is_blank(&u);
                            if !is_blank(&u) {
                                p.set_url(&u, cx);
                            }
                            if started {
                                p.reveal_soon(cx);
                            } else {
                                p.reveal(cx);
                            }
                        }
                        Msg::Focused => cx.emit(BrowserEvent::Focused),
                        Msg::NewWindow(u) => cx.emit(BrowserEvent::OpenTab(u)),
                    }
                    cx.notify();
                });
                if ok.is_err() {
                    return;
                }
            }
        });

        let address = cx.new(|cx| Field::new("Search or enter address", cx));
        let sub = cx.subscribe(&address, |p, _, ev: &FieldEvent, cx| match ev {
            FieldEvent::Submit(text) if !text.trim().is_empty() => {
                let _ = p.navigate(text, cx);
                p.focus_native();
            }
            FieldEvent::Submit(_) => {}
            FieldEvent::Cancel => {
                let url = p.url.clone();
                p.address.update(cx, |f, cx| {
                    f.set_text(if is_blank(&url) { "" } else { &url }, cx)
                });
                if !is_blank(&url) {
                    p.focus_native();
                }
            }
            _ => {}
        });

        let mut pane = Self {
            webview,
            url: SharedString::default(),
            title: SharedString::default(),
            loading: !is_blank(&url),
            address,
            servers: Vec::new(),
            zoom: 1.0,
            bounds: None,
            tab_visible: true,
            revealed: false,
            shown: false,
            _reveal: None,
            _events: events,
            _address: sub,
        };
        if !is_blank(&url) {
            pane.set_url(&url, cx);
        }
        pane
    }

    fn set_url(&mut self, url: &str, cx: &mut Context<Self>) {
        self.url = SharedString::from(url.to_string());
        let pretty = pretty_url(url);
        self.address.update(cx, |f, cx| {
            f.set_text(url, cx);
            f.display = Some(pretty.into());
        });
    }

    pub fn is_blank(&self) -> bool {
        is_blank(&self.url)
    }

    pub fn navigate(&mut self, input: &str, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let url = normalize_url(input);
        self.webview.load_url(&url)?;
        self.set_url(&url, cx);
        self.loading = true;
        self.revealed = false;
        self.sync_visibility();
        self.reveal_soon(cx);
        cx.notify();
        Ok(())
    }

    /// Show the page when it has painted, or after a moment if it is slow to
    /// finish (a long-polling page may never report "finished").
    fn reveal_soon(&mut self, cx: &mut Context<Self>) {
        if self.revealed {
            return;
        }
        self._reveal = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            let _ = this.update(cx, |p, cx| p.reveal(cx));
        }));
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        if !self.revealed && !self.is_blank() {
            self.revealed = true;
            self._reveal = None;
            self.sync_visibility();
            cx.notify();
        }
    }

    /// Put the keyboard in the address bar with its text selected (⌘L).
    pub fn focus_address(&self, window: &mut Window, cx: &mut Context<Self>) {
        let f = self.address.clone();
        window.focus(&f.read(cx).focus);
        reclaim_keyboard(window);
        f.update(cx, |f, cx| f.select_all(cx));
    }

    pub fn focus_native(&self) {
        if self.shown {
            let _ = self.webview.focus();
        }
    }

    /// Hidden tabs must hide their webview: it is a native view and would
    /// otherwise stay on screen over whatever tab is showing.
    pub fn set_visible(&mut self, visible: bool) {
        self.tab_visible = visible;
        self.sync_visibility();
    }

    fn sync_visibility(&mut self) {
        let show = self.tab_visible && self.revealed && !self.is_blank();
        if show != self.shown {
            self.shown = show;
            let _ = self.webview.set_visible(show);
        }
    }

    pub fn set_zoom(&mut self, zoom: f64) {
        if (self.zoom - zoom).abs() > f64::EPSILON {
            self.zoom = zoom;
            let _ = self.webview.zoom(zoom);
        }
    }

    pub fn back(&self) {
        let _ = self.webview.evaluate_script("history.back()");
    }

    pub fn forward(&self) {
        let _ = self.webview.evaluate_script("history.forward()");
    }

    pub fn reload(&mut self) {
        if self.loading {
            let _ = self.webview.evaluate_script("window.stop()");
            self.loading = false;
        } else if !self.is_blank() {
            let _ = self.webview.reload();
        }
    }

    /// Run `js` (an expression or statements) and hand back its JSON value.
    ///
    /// The result is stringified in the page so DOM nodes and cycles degrade
    /// to `{}` instead of failing the bridge, and exceptions come back as
    /// errors instead of never calling back.
    pub fn eval(&self, js: &str, done: impl FnOnce(Result<Value, String>) + Send + 'static) {
        let src = serde_json::to_string(js).unwrap_or_default();
        let wrapped = format!(
            "(() => {{ try {{ const v = (0, eval)({src}); \
             return JSON.stringify({{ ok: v === undefined ? null : v }}); }} \
             catch (e) {{ return JSON.stringify({{ err: String(e) }}); }} }})()"
        );
        let done = Mutex::new(Some(done));
        // On failure the callback is dropped unrun, which drops `done`; the
        // caller sees its reply channel close and reports that.
        let _ = self
            .webview
            .evaluate_script_with_callback(&wrapped, move |json| {
                if let Some(done) = done.lock().ok().and_then(|mut d| d.take()) {
                    done(decode_eval(&json));
                }
            });
    }

    pub fn read_text(&self, done: impl FnOnce(Result<Value, String>) + Send + 'static) {
        self.eval(PAGE_TEXT, done);
    }

    fn sync_bounds(&mut self, bounds: Bounds<Pixels>) {
        if self.bounds == Some(bounds) {
            return;
        }
        self.bounds = Some(bounds);
        let _ = self.webview.set_bounds(rect(bounds));
    }

    fn render_blank(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(u(2.))
            .w(u(420.))
            .max_w(relative(0.9))
            .child(
                div()
                    .pb(u(14.))
                    .text_size(u(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child("New Tab"),
            );
        if self.servers.is_empty() {
            col = col.child(
                div()
                    .text_size(u(13.))
                    .text_color(rgb(theme::TEXT_2))
                    .child("Type an address or a search above. Dev servers started in your terminals show up here."),
            );
        } else {
            col = col.child(
                div()
                    .pb(u(6.))
                    .text_size(u(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::DIM))
                    .child("RUNNING ON THIS MAC"),
            );
            for (port, process) in &self.servers {
                let url = format!("http://localhost:{port}");
                col = col.child(
                    div()
                        .id(ElementId::NamedInteger("server".into(), *port as u64))
                        .flex()
                        .items_center()
                        .gap(u(10.))
                        .h(u(36.))
                        .px(u(10.))
                        .rounded(u(8.))
                        .text_size(u(13.))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(theme::SURFACE)))
                        .child(icon(Icon::Globe, 15., theme::TEXT_2))
                        .child(
                            div()
                                .text_color(rgb(theme::TEXT))
                                .child(format!("localhost:{port}")),
                        )
                        .child(div().text_color(rgb(theme::DIM)).child(process.clone()))
                        .child(div().flex_1())
                        .child(icon(Icon::ArrowUpRight, 13., theme::DIM))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            let _ = p.navigate(&url, cx);
                        })),
                );
            }
        }
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(theme::PANE))
            .child(col)
    }
}

impl Render for BrowserPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(2.))
            .h(u(theme::TOOLBAR_PX))
            .px(u(6.))
            .bg(rgb(theme::PANE))
            .child(
                icon_button("back", Icon::ArrowLeft).on_click(cx.listener(|p, _, _, _| p.back())),
            )
            .child(
                icon_button("forward", Icon::ArrowRight)
                    .on_click(cx.listener(|p, _, _, _| p.forward())),
            )
            .child(
                icon_button(
                    "reload",
                    if self.loading {
                        Icon::Close
                    } else {
                        Icon::Refresh
                    },
                )
                .mr(u(4.))
                .on_click(cx.listener(|p, _, _, cx| {
                    p.reload();
                    cx.notify();
                })),
            )
            .child(self.address.clone())
            .child(
                icon_button("close-pane", Icon::Close)
                    .ml(u(4.))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(BrowserEvent::Close))),
            );

        // A thin indeterminate bar while loading: the toolbar's bottom edge.
        let edge = div()
            .relative()
            .h(u(1.))
            .w_full()
            .bg(rgba(theme::HAIRLINE))
            .overflow_hidden()
            .when(self.loading, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .h(u(1.))
                        .w(relative(0.3))
                        .bg(rgb(theme::TEXT))
                        .with_animation(
                            "loading",
                            Animation::new(Duration::from_millis(1100)).repeat(),
                            |d, t| d.left(relative(t * 1.3 - 0.3)),
                        ),
                )
            });

        let body = if self.is_blank() {
            self.render_blank(cx).into_any_element()
        } else {
            div()
                .size_full()
                .bg(rgb(theme::PANE))
                .child(
                    canvas(
                        move |bounds, _, cx| entity.update(cx, |p, _| p.sync_bounds(bounds)),
                        |_, _, _, _| {},
                    )
                    .size_full(),
                )
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(toolbar)
            .child(edge)
            .child(div().flex_1().min_h_0().child(body))
    }
}

/// Round the webview's bottom corners to sit inside the card. It is a native
/// view, so the card's own clipping does not reach it.
fn round_bottom_corners(webview: &wry::WebView, radius: f32) {
    use objc2_quartz_core::CACornerMask;
    use wry::WebViewExtMacOS;
    let wk = webview.webview();
    let view: &objc2_app_kit::NSView = &wk;
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setCornerRadius(radius as f64);
        // AppKit layers are not flipped: MinY is the bottom edge.
        layer.setMaskedCorners(
            CACornerMask::LayerMinXMinYCorner | CACornerMask::LayerMaxXMinYCorner,
        );
        layer.setMasksToBounds(true);
    }
}

fn rect(b: Bounds<Pixels>) -> wry::Rect {
    wry::Rect {
        position: LogicalPosition::new(f32::from(b.origin.x) as f64, f32::from(b.origin.y) as f64)
            .into(),
        size: LogicalSize::new(
            f32::from(b.size.width) as f64,
            f32::from(b.size.height) as f64,
        )
        .into(),
    }
}

/// wry hands back the JSON of what the page returned — here, a string that is
/// itself JSON.
fn decode_eval(json: &str) -> Result<Value, String> {
    let inner: String = serde_json::from_str(json).map_err(|e| format!("bad eval result: {e}"))?;
    let mut v: Value = serde_json::from_str(&inner).map_err(|e| format!("bad eval result: {e}"))?;
    if let Some(err) = v.get("err").and_then(Value::as_str) {
        return Err(err.to_string());
    }
    Ok(v.get_mut("ok").map(Value::take).unwrap_or(Value::Null))
}

/// The address as Safari shows it while you are not editing: no scheme, no
/// `www.`, no trailing slash.
fn pretty_url(url: &str) -> String {
    let s = url.split_once("://").map_or(url, |(_, r)| r);
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.strip_suffix('/').unwrap_or(s).to_string()
}

/// What the address bar does: URLs pass through, hosts get a scheme, anything
/// else is a search.
pub fn normalize_url(input: &str) -> String {
    let s = input.trim();
    if s.contains("://") || s.starts_with("about:") || s.starts_with("data:") {
        return s.to_string();
    }
    if s.starts_with("localhost") || s.starts_with("127.0.0.1") || s.starts_with("[::1]") {
        return format!("http://{s}");
    }
    if !s.contains(char::is_whitespace) && s.contains('.') {
        return format!("https://{s}");
    }
    let mut q = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                q.push(b as char)
            }
            b' ' => q.push('+'),
            _ => q.push_str(&format!("%{b:02X}")),
        }
    }
    format!("https://duckduckgo.com/?q={q}")
}

/// Give the keyboard back to gpui after a webview has held it.
///
/// gpui's view never claims first responder on click (it is normally the only
/// view in the window), so once WKWebView has it, keys keep going to the page
/// until someone hands them back.
pub fn reclaim_keyboard(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // Fully qualified: gpui's inherent `Window::window_handle` shadows the trait.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return;
    };
    // SAFETY: gpui's own content view, alive for as long as the window is.
    let view: &objc2_app_kit::NSView = unsafe { h.ns_view.cast().as_ref() };
    if let Some(win) = view.window() {
        let responder: &objc2_app_kit::NSResponder = view;
        win.makeFirstResponder(Some(responder));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_hosts_and_searches() {
        assert_eq!(normalize_url("https://a.b/c"), "https://a.b/c");
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("localhost:3000"), "http://localhost:3000");
        assert_eq!(
            normalize_url("rust gpui"),
            "https://duckduckgo.com/?q=rust+gpui"
        );
        assert_eq!(normalize_url("a&b"), "https://duckduckgo.com/?q=a%26b");
    }

    #[test]
    fn pretty_urls_drop_scheme_www_and_slash() {
        assert_eq!(pretty_url("https://www.example.com/"), "example.com");
        assert_eq!(
            pretty_url("http://localhost:5173/app"),
            "localhost:5173/app"
        );
    }

    #[test]
    fn eval_results_unwrap_and_errors_surface() {
        assert_eq!(decode_eval(r#""{\"ok\":3}""#), Ok(Value::from(3)));
        assert_eq!(decode_eval(r#""{\"ok\":null}""#), Ok(Value::Null));
        assert_eq!(decode_eval(r#""{\"err\":\"boom\"}""#), Err("boom".into()));
    }
}
