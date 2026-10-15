//! A browser pane: a gpui toolbar above a native WKWebView parented into the
//! window and kept glued to the pane's layout bounds.
//!
//! The webview is a real NSView above gpui's Metal layer, so nothing gpui
//! draws can overlap it. Everything interactive is laid out beside it, and
//! while the address bar shows suggestions the page steps aside for them.
//! A pane's first page stays hidden until it has committed and painted (no
//! white flash); after that the old page stays up while the next one loads,
//! as in Safari. A blank tab is a gpui page, not an empty white document.

use crate::favicon;
use crate::field::{Field, FieldEvent};
use crate::history::{self, Visit};
use crate::icon::{icon, spinner, Icon};
use crate::settings::Search;
use crate::theme::{self, u};
use crate::workspace::icon_button;
use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use gpui::prelude::*;
use gpui::{
    canvas, div, img, relative, rgb, rgba, AnyElement, Bounds, Context, ElementId, Entity,
    EventEmitter, FontWeight, Image, MouseButton, Pixels, SharedString, Subscription, Task, Window,
};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wry::dpi::{LogicalPosition, LogicalSize};

pub enum BrowserEvent {
    /// The page has the keyboard now.
    Focused,
    /// The page asked for a new window (target=_blank, window.open), or a
    /// link was ⌘-clicked.
    OpenTab(String),
    /// The title, address or icon changed: things the sidebar shows.
    Retitled,
    Close,
}

pub enum Msg {
    Title(String),
    /// A navigation committed (`true`) or finished loading (`false`).
    Load(String, bool),
    /// The page changed its address without a load (pushState, #hash).
    Moved,
    Focused,
    NewWindow(String),
    /// A ⌘-click or middle click on a link.
    LinkTab(String),
    /// Where the page's icon is.
    Icon(String),
    DownloadStarted(PathBuf),
    DownloadDone(Option<PathBuf>, bool),
}

/// Safari's own, so sites serve their current code instead of the fallback
/// bundles an unknown WebKit gets (WKWebView's default has no `Safari/`).
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

/// Runs in every page before its own scripts. Clicks inside the page never
/// reach gpui, so it reports them; single-page apps move without loading, so
/// it reports that too (deck then reads the real address from WebKit rather
/// than trusting the page); and it names the page's best icon.
const PAGE_SCRIPT: &str = r#"(() => {
  const post = (m) => { try { window.ipc.postMessage(m); } catch (_) {} };
  addEventListener('mousedown', () => post('focus'), true);
  const moved = () => post('moved');
  for (const k of ['pushState', 'replaceState']) {
    const f = history[k];
    history[k] = function () { const r = f.apply(this, arguments); moved(); return r; };
  }
  addEventListener('popstate', moved);
  addEventListener('hashchange', moved);
  const icon = () => {
    let best = null, score = -1;
    for (const l of document.querySelectorAll('link[rel~="icon"], link[rel="apple-touch-icon"]')) {
      if (!l.href) continue;
      const svg = l.type === 'image/svg+xml' || /\.svg(\?|$)/.test(l.href);
      const size = parseInt((l.getAttribute('sizes') || '').split('x')[0], 10)
        || (l.rel.includes('apple') ? 180 : 16);
      const s = svg ? 1000 : (size >= 32 && size <= 192 ? 500 + size : size);
      if (s > score) { score = s; best = l.href; }
    }
    post('icon:' + (best || location.origin + '/favicon.ico'));
  };
  if (document.readyState === 'loading') addEventListener('DOMContentLoaded', icon);
  else icon();
  const tab = (e) => {
    const a = e.target && e.target.closest && e.target.closest('a[href]');
    if (!a || !/^https?:/.test(a.href)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    post('tab:' + a.href);
  };
  addEventListener('click', (e) => { if (e.metaKey && e.button === 0) tab(e); }, true);
  addEventListener('auxclick', (e) => { if (e.button === 1) tab(e); }, true);
})();"#;

const PAGE_TEXT: &str =
    "document.title + '\\n' + location.href + '\\n\\n' + (document.body ? document.body.innerText : '')";

/// Where the address pill starts and ends inside the toolbar, so the
/// suggestion list lines up under it: three 26 px buttons with their gaps
/// on the left, the close button on the right.
const PILL_LEFT: f32 = 96.;
const PILL_RIGHT: f32 = 38.;

fn is_blank(url: &str) -> bool {
    url.is_empty() || url == "about:blank"
}

fn is_web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// What the page sent over `window.ipc`. Anything else is ignored: the page
/// owns this channel too, so nothing it says is taken as an address to show.
fn parse_ipc(body: &str) -> Option<Msg> {
    if body == "focus" {
        return Some(Msg::Focused);
    }
    if body == "moved" {
        return Some(Msg::Moved);
    }
    if let Some(url) = body.strip_prefix("icon:") {
        return is_web(url).then(|| Msg::Icon(url.to_string()));
    }
    if let Some(url) = body.strip_prefix("tab:") {
        return is_web(url).then(|| Msg::LinkTab(url.to_string()));
    }
    None
}

#[derive(Clone)]
enum Row {
    /// Open this address: what Enter does when the text is one.
    Go {
        url: String,
        label: String,
    },
    Search(String),
    Page(Visit),
    Server(u16, String),
}

struct Suggest {
    rows: Vec<Row>,
    selected: usize,
}

struct FindBar {
    field: Entity<Field>,
    matches: Option<usize>,
    _sub: Subscription,
}

#[derive(Clone, Copy, PartialEq)]
enum DownloadState {
    Running,
    Done,
    Failed,
}

struct Download {
    name: String,
    path: Option<PathBuf>,
    state: DownloadState,
}

pub struct BrowserPane {
    webview: wry::WebView,
    pub url: SharedString,
    pub title: SharedString,
    pub loading: bool,
    /// 0–1 while loading, WebKit's own estimate.
    progress: f32,
    can_back: bool,
    can_forward: bool,
    pub address: Entity<Field>,
    /// Dev servers listening on this machine, offered on the blank page.
    pub servers: Vec<(u16, String)>,
    /// The app zoom, applied to the page so it scales with everything else.
    zoom: f64,
    bounds: Option<Bounds<Pixels>>,
    /// Whether this pane's tab is the one on screen.
    tab_visible: bool,
    /// Set once the first page has painted; until then the webview stays
    /// hidden and the pane shows black instead of WebKit's white.
    revealed: bool,
    shown: bool,
    dimmed: bool,
    favicon: Option<Arc<Image>>,
    /// What was typed in the address bar, without the inline completion.
    typed: String,
    /// The address bar is being edited, so a page moving on does not
    /// overwrite what is being typed.
    editing: bool,
    /// The inline completion: the text it shows and the URL it opens.
    completion: Option<(String, String)>,
    /// Text changes deck made itself and should not treat as typing.
    quiet: u32,
    /// The address bar was not focused when the press began, so the click
    /// selects the whole address instead of placing the caret.
    select_on_click: bool,
    suggest: Option<Suggest>,
    /// Google's suggestions, and the text they were for.
    remote: Vec<String>,
    remote_for: String,
    /// The new tab page's sites.
    top: Vec<Visit>,
    find: Option<FindBar>,
    download: Option<Download>,
    last_link_tab: Option<Instant>,
    watching: bool,
    _watch: Option<Task<()>>,
    _reveal: Option<Task<()>>,
    _icon: Option<Task<()>>,
    _remote: Option<Task<()>>,
    _download: Option<Task<()>>,
    _events: Task<()>,
    _subs: [Subscription; 2],
}

impl EventEmitter<BrowserEvent> for BrowserPane {}

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
        let (tx_title, tx_load, tx_ipc, tx_new, tx_dl, tx_done) = (
            tx.clone(),
            tx.clone(),
            tx.clone(),
            tx.clone(),
            tx.clone(),
            tx,
        );

        let webview = wry::WebViewBuilder::new()
            .with_url(&url)
            .with_bounds(rect(Bounds::default()))
            .with_visible(false)
            .with_user_agent(USER_AGENT)
            // Right-click › Inspect Element, and Safari's Develop menu.
            .with_devtools(true)
            .with_back_forward_navigation_gestures(true)
            .with_accept_first_mouse(true)
            .with_initialization_script(PAGE_SCRIPT)
            .with_ipc_handler(move |req| {
                if let Some(msg) = parse_ipc(req.body()) {
                    let _ = tx_ipc.unbounded_send(msg);
                }
            })
            .with_document_title_changed_handler(move |t| {
                let _ = tx_title.unbounded_send(Msg::Title(t));
            })
            .with_on_page_load_handler(move |ev, url| {
                let committed = matches!(ev, wry::PageLoadEvent::Started);
                let _ = tx_load.unbounded_send(Msg::Load(url, committed));
            })
            .with_new_window_req_handler(move |url, _| {
                let _ = tx_new.unbounded_send(Msg::NewWindow(url));
                wry::NewWindowResponse::Deny
            })
            // Into ~/Downloads, numbered rather than overwriting.
            .with_download_started_handler(move |_, path| {
                let _ = tx_dl.unbounded_send(Msg::DownloadStarted(path.clone()));
                true
            })
            .with_download_completed_handler(move |_, path, ok| {
                let _ = tx_done.unbounded_send(Msg::DownloadDone(path, ok));
            })
            .build_as_child(window)?;
        style_native(&webview, theme::CARD_RADIUS - 1.);
        Ok((webview, rx, url))
    }

    pub fn new(
        webview: wry::WebView,
        mut rx: UnboundedReceiver<Msg>,
        url: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                if this.update(cx, |p, cx| p.on_msg(msg, cx)).is_err() {
                    return;
                }
            }
        });

        let address = cx.new(|cx| {
            let mut f = Field::new(
                SharedString::from(format!(
                    "Search {} or enter address",
                    search_engine().name()
                )),
                cx,
            );
            f.bare = true;
            f
        });
        let typing = cx.subscribe_in(&address, window, Self::on_address);
        let focus = address.read(cx).focus.clone();
        let blur = cx.on_blur(&focus, window, |p, _, cx| p.stop_editing(cx));

        let mut pane = Self {
            webview,
            url: SharedString::default(),
            title: SharedString::default(),
            loading: !is_blank(&url),
            progress: 0.,
            can_back: false,
            can_forward: false,
            address,
            servers: Vec::new(),
            zoom: 1.0,
            bounds: None,
            tab_visible: true,
            revealed: false,
            shown: false,
            dimmed: false,
            favicon: None,
            typed: String::new(),
            editing: false,
            completion: None,
            quiet: 0,
            select_on_click: false,
            suggest: None,
            remote: Vec::new(),
            remote_for: String::new(),
            top: Vec::new(),
            find: None,
            download: None,
            last_link_tab: None,
            watching: false,
            _watch: None,
            _reveal: None,
            _icon: None,
            _remote: None,
            _download: None,
            _events: events,
            _subs: [typing, blur],
        };
        if is_blank(&url) {
            pane.top = history::top_sites(8);
        } else {
            pane.set_url(&url, cx);
            pane.watch_load(cx);
        }
        pane
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Title(t) => {
                history::set_title(&self.url, &t);
                self.title = t.into();
                cx.emit(BrowserEvent::Retitled);
            }
            Msg::Load(u, committed) => {
                // A new page's title has not arrived yet; the old one must
                // not be filed under the new address.
                if committed && u != self.url.as_ref() {
                    self.title = SharedString::default();
                }
                if !is_blank(&u) {
                    self.set_url(&u, cx);
                }
                if committed {
                    cx.emit(BrowserEvent::Retitled);
                    self.loading = true;
                    self.watch_load(cx);
                    self.reveal_soon(cx);
                } else {
                    history::record(&u, &self.title);
                    self.reveal(cx);
                    self.poll_load(cx);
                }
            }
            Msg::Moved => {
                if let Ok(u) = self.webview.url() {
                    if u != self.url.as_ref() && !is_blank(&u) {
                        self.set_url(&u, cx);
                        history::record(&u, "");
                        self.poll_load(cx);
                    }
                }
            }
            Msg::Focused => cx.emit(BrowserEvent::Focused),
            Msg::NewWindow(u) => cx.emit(BrowserEvent::OpenTab(u)),
            Msg::LinkTab(u) => {
                // The page can post this itself; a click is never faster
                // than this, a script flooding the channel is.
                let now = Instant::now();
                if self
                    .last_link_tab
                    .is_none_or(|t| now - t > Duration::from_millis(300))
                {
                    self.last_link_tab = Some(now);
                    cx.emit(BrowserEvent::OpenTab(u));
                }
            }
            Msg::Icon(url) => self.load_icon(url, cx),
            Msg::DownloadStarted(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Download".into());
                self._download = None;
                self.download = Some(Download {
                    name,
                    path: Some(path),
                    state: DownloadState::Running,
                });
            }
            Msg::DownloadDone(path, ok) => {
                if let Some(d) = &mut self.download {
                    d.state = if ok {
                        DownloadState::Done
                    } else {
                        DownloadState::Failed
                    };
                    if path.is_some() {
                        d.path = path;
                    }
                }
                self._download = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(8)).await;
                    let _ = this.update(cx, |p, cx| {
                        p.download = None;
                        cx.notify();
                    });
                }));
            }
        }
        cx.notify();
    }

    fn set_url(&mut self, url: &str, cx: &mut Context<Self>) {
        let host_changed = history::host(url) != history::host(&self.url);
        self.url = SharedString::from(url.to_string());
        if host_changed {
            self.favicon = favicon::get(history::host(url));
        }
        if !self.editing {
            self.set_address(url, cx);
        }
    }

    /// Put `text` in the address bar without it counting as typing.
    fn set_address(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.address.read(cx).text() != text {
            self.quiet += 1;
        }
        let shown = (!is_blank(text)).then(|| SharedString::from(pretty_url(text)));
        let text = if is_blank(text) { "" } else { text };
        self.address.update(cx, |f, cx| {
            f.set_text(text, cx);
            f.display = shown;
        });
    }

    pub fn is_blank(&self) -> bool {
        is_blank(&self.url)
    }

    pub fn favicon(&self) -> Option<Arc<Image>> {
        self.favicon.clone()
    }

    pub fn navigate(&mut self, input: &str, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let url = normalize_url(input);
        self.webview.load_url(&url)?;
        self.set_url(&url, cx);
        self.loading = true;
        self.progress = 0.;
        self.sync_visibility();
        self.watch_load(cx);
        cx.notify();
        Ok(())
    }

    /// Navigate because the user asked to, and hand the keyboard to the page.
    fn open(&mut self, url: &str, cx: &mut Context<Self>) {
        self.end_suggest(cx);
        self.editing = false;
        let _ = self.navigate(url, cx);
        self.focus_native();
        cx.emit(BrowserEvent::Focused);
    }

    // -- loading --------------------------------------------------------------

    /// Follow the load through WebKit's own state while it runs: progress,
    /// back and forward. A failed load reports nothing to wry, so this is
    /// also what notices a load has stopped.
    fn watch_load(&mut self, cx: &mut Context<Self>) {
        if self.watching {
            return;
        }
        self.watching = true;
        self._watch = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            match this.update(cx, |p, cx| p.poll_load(cx)) {
                Ok(true) => {}
                _ => break,
            }
        }));
    }

    /// Read the load state; true while it is still loading.
    fn poll_load(&mut self, cx: &mut Context<Self>) -> bool {
        use wry::WebViewExtMacOS;
        let wk = self.webview.webview();
        // SAFETY: plain property reads on the main thread.
        let (loading, progress) = unsafe { (wk.isLoading(), wk.estimatedProgress() as f32) };
        let back = self.webview.can_go_back().unwrap_or(false);
        let forward = self.webview.can_go_forward().unwrap_or(false);
        if loading != self.loading {
            // The sidebar spins a loading page's icon.
            cx.emit(BrowserEvent::Retitled);
        }
        if loading != self.loading
            || (progress - self.progress).abs() > 0.01
            || back != self.can_back
            || forward != self.can_forward
        {
            self.loading = loading;
            self.progress = progress;
            self.can_back = back;
            self.can_forward = forward;
            cx.notify();
        }
        if !loading {
            self.watching = false;
            self.reveal(cx);
        }
        loading
    }

    /// The first page has committed: show it once it has had a moment to
    /// paint, or when it finishes, whichever comes first.
    fn reveal_soon(&mut self, cx: &mut Context<Self>) {
        if self.revealed {
            return;
        }
        self._reveal = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
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

    fn load_icon(&mut self, icon_url: String, cx: &mut Context<Self>) {
        let host = history::host(&self.url).to_string();
        if host.is_empty() {
            return;
        }
        if let Some(found) = favicon::get(&host) {
            self.favicon = Some(found);
            if !favicon::stale(&host) {
                return;
            }
        }
        self._icon = Some(cx.spawn(async move |this, cx| {
            let found = favicon::fetch(host.clone(), icon_url).await;
            let _ = this.update(cx, |p, cx| {
                if let (Some(found), true) = (found, history::host(&p.url) == host) {
                    p.favicon = Some(found);
                    cx.emit(BrowserEvent::Retitled);
                    cx.notify();
                }
            });
        }));
    }

    // -- address bar ----------------------------------------------------------

    /// Put the keyboard in the address bar with its text selected (⌘L).
    pub fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let f = self.address.clone();
        window.focus(&f.read(cx).focus);
        reclaim_keyboard(window);
        f.update(cx, |f, cx| f.select_all(cx));
    }

    fn on_address(
        &mut self,
        _: &Entity<Field>,
        ev: &FieldEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            FieldEvent::Changed if self.quiet > 0 => self.quiet -= 1,
            FieldEvent::Changed if self.address.read(cx).focus.is_focused(window) => {
                self.on_typed(cx);
            }
            FieldEvent::PopupKey(k) => self.suggest_key(k, cx),
            FieldEvent::Submit(text) if !text.trim().is_empty() => {
                let url = match &self.completion {
                    Some((shown, url)) if shown == text => url.clone(),
                    _ => normalize_url(text),
                };
                self.open(&url, cx);
            }
            FieldEvent::Cancel => self.cancel_editing(cx),
            _ => {}
        }
    }

    /// The text changed under the user's hands: complete it inline when they
    /// added to the end, and refresh the suggestions.
    fn on_typed(&mut self, cx: &mut Context<Self>) {
        let (text, at_end) = {
            let f = self.address.read(cx);
            (f.text().to_string(), f.caret_at_end())
        };
        // Growing the text completes it; deleting (⌫ over the completion)
        // must not bring the completion straight back.
        let grew = text.len() > self.typed.len() && text.starts_with(&self.typed);
        self.typed = text.clone();
        self.editing = true;
        self.completion = None;
        if grew && at_end {
            if let Some((shown, url)) = history::complete(&text) {
                if let Some(rest) = shown.get(text.len()..).map(str::to_string) {
                    self.address.update(cx, |f, cx| f.complete_with(&rest, cx));
                    self.completion = Some((shown, url));
                }
            }
        }
        self.refresh_suggest(false, cx);
        self.fetch_remote(cx);
    }

    fn refresh_suggest(&mut self, keep_selection: bool, cx: &mut Context<Self>) {
        let typed = self.typed.trim().to_string();
        if typed.is_empty() {
            self.end_suggest(cx);
            return;
        }
        let mut rows = Vec::new();
        match &self.completion {
            Some((shown, url)) => rows.push(Row::Go {
                url: url.clone(),
                label: shown.clone(),
            }),
            None if looks_like_address(&typed) => rows.push(Row::Go {
                url: normalize_url(&typed),
                label: typed.clone(),
            }),
            None => rows.push(Row::Search(typed.clone())),
        }
        let first_url = match &rows[0] {
            Row::Go { url, .. } => Some(history::strip(url).trim_end_matches('/').to_string()),
            _ => None,
        };
        if first_url.is_some() && !typed.contains("://") {
            rows.push(Row::Search(typed.clone()));
        }
        let lower = typed.to_lowercase();
        for (port, process) in &self.servers {
            if format!("localhost:{port}").contains(&lower) {
                rows.push(Row::Server(*port, process.clone()));
            }
        }
        for v in history::search(&typed, 6)
            .into_iter()
            .filter(|v| first_url.as_deref() != Some(history::strip(&v.url).trim_end_matches('/')))
            .take(4)
        {
            rows.push(Row::Page(v));
        }
        if self.remote_for == typed {
            for s in self
                .remote
                .iter()
                .filter(|s| !s.eq_ignore_ascii_case(&typed))
                .take(5)
            {
                rows.push(Row::Search(s.clone()));
            }
        }
        let selected = match &self.suggest {
            Some(s) if keep_selection => s.selected.min(rows.len() - 1),
            _ => 0,
        };
        self.suggest = Some(Suggest { rows, selected });
        self.address.update(cx, |f, _| f.popup = true);
        self.sync_visibility();
        cx.notify();
    }

    /// Ask Google for suggestions a beat after typing pauses.
    fn fetch_remote(&mut self, cx: &mut Context<Self>) {
        let q = self.typed.trim().to_string();
        if q.is_empty() || q.contains("://") || q.starts_with("localhost") {
            self._remote = None;
            return;
        }
        self._remote = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(90))
                .await;
            let found = crate::net::suggestions(&q).await;
            let _ = this.update(cx, |p, cx| {
                if p.typed.trim() == q && p.suggest.is_some() {
                    p.remote = found;
                    p.remote_for = q;
                    p.refresh_suggest(true, cx);
                }
            });
        }));
    }

    fn suggest_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(s) = self.suggest.as_mut() else {
            return;
        };
        let n = s.rows.len();
        match key {
            "up" => s.selected = (s.selected + n - 1) % n,
            "down" => s.selected = (s.selected + 1) % n,
            // ⇥ takes the inline completion, as in Safari; past that it
            // walks the list.
            "tab" if self.completion.is_some() && s.selected == 0 => {
                self.address.update(cx, |f, cx| f.complete_with("", cx));
                if let Some((shown, _)) = &self.completion {
                    self.typed = shown.clone();
                }
            }
            "tab" => s.selected = (s.selected + 1) % n,
            "enter" => {
                let ix = s.selected;
                self.open_row(ix, cx);
                return;
            }
            "escape" => {
                self.cancel_editing(cx);
                return;
            }
            _ => {}
        }
        cx.notify();
    }

    fn open_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.suggest.as_ref().and_then(|s| s.rows.get(ix)).cloned() else {
            return;
        };
        let url = match row {
            Row::Go { url, .. } => url,
            Row::Search(q) => search_url(&q),
            Row::Page(v) => v.url,
            Row::Server(port, _) => format!("http://localhost:{port}"),
        };
        self.open(&url, cx);
    }

    fn end_suggest(&mut self, cx: &mut Context<Self>) {
        self.suggest = None;
        self._remote = None;
        self.address.update(cx, |f, _| f.popup = false);
        self.sync_visibility();
        cx.notify();
    }

    /// The address bar lost the keyboard: put the address back.
    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        self.end_suggest(cx);
        self.typed.clear();
        self.completion = None;
        if self.editing {
            self.editing = false;
            let url = self.url.clone();
            self.set_address(&url, cx);
        }
    }

    /// Esc: put the address back and give the page the keyboard again.
    fn cancel_editing(&mut self, cx: &mut Context<Self>) {
        self.editing = true;
        self.stop_editing(cx);
        if !self.is_blank() {
            self.focus_native();
            cx.emit(BrowserEvent::Focused);
        }
    }

    // -- find in page ---------------------------------------------------------

    /// ⌘F: a find bar under the toolbar.
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_blank() {
            return;
        }
        let field = match &self.find {
            Some(f) => f.field.clone(),
            None => {
                let field = cx.new(|cx| {
                    let mut f = Field::new("Find in page", cx);
                    // Enter, ↑↓ and Esc come to the pane.
                    f.popup = true;
                    f
                });
                let sub = cx.subscribe(&field, |p, _, ev: &FieldEvent, cx| match ev {
                    FieldEvent::Changed => p.find_run(false, true, cx),
                    FieldEvent::PopupKey("enter") | FieldEvent::PopupKey("down") => {
                        p.find_run(false, false, cx)
                    }
                    // ⇧⏎ arrives as a plain submit.
                    FieldEvent::Submit(_) | FieldEvent::PopupKey("up") => {
                        p.find_run(true, false, cx)
                    }
                    FieldEvent::PopupKey("escape") | FieldEvent::Cancel => p.close_find(cx),
                    _ => {}
                });
                self.find = Some(FindBar {
                    field: field.clone(),
                    matches: None,
                    _sub: sub,
                });
                field
            }
        };
        window.focus(&field.read(cx).focus);
        reclaim_keyboard(window);
        field.update(cx, |f, cx| f.select_all(cx));
        cx.notify();
    }

    /// ⌘G / ⇧⌘G while the find bar is open.
    pub fn find_step(&mut self, back: bool, cx: &mut Context<Self>) -> bool {
        if self.find.is_none() {
            return false;
        }
        self.find_run(back, false, cx);
        true
    }

    /// WebKit's own find moves the selection to the match and scrolls to it;
    /// the count is of the page's visible text.
    fn find_run(&mut self, back: bool, restart: bool, cx: &mut Context<Self>) {
        let Some(f) = &self.find else {
            return;
        };
        let q = serde_json::to_string(f.field.read(cx).text()).unwrap_or_default();
        let js = format!(
            "(() => {{ const q = {q}; const s = getSelection(); \
             if ({restart} || !q) s.removeAllRanges(); if (!q) return null; \
             window.find(q, false, {back}, true, false, false, false); \
             const t = (document.body ? document.body.innerText : '').toLowerCase(); \
             const l = q.toLowerCase(); let n = 0, i = 0; \
             while ((i = t.indexOf(l, i)) !== -1) {{ n++; i += l.length; }} return n; }})()"
        );
        let (tx, rx) = futures::channel::oneshot::channel();
        self.eval(&js, move |r| {
            let _ = tx.send(r);
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(v)) = rx.await {
                let _ = this.update(cx, |p, cx| {
                    if let Some(f) = &mut p.find {
                        f.matches = v.as_u64().map(|n| n as usize);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn close_find(&mut self, cx: &mut Context<Self>) {
        let _ = self
            .webview
            .evaluate_script("getSelection().removeAllRanges()");
        self.find = None;
        self.focus_native();
        cx.emit(BrowserEvent::Focused);
        cx.notify();
    }

    // -- the native view ------------------------------------------------------

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
        let show = self.tab_visible && self.revealed && !self.is_blank() && self.suggest.is_none();
        if show != self.shown {
            self.shown = show;
            let _ = self.webview.set_visible(show);
        }
    }

    /// An unfocused pane of a split steps back. gpui cannot paint a veil
    /// over a native view, so the view fades itself.
    pub fn set_dimmed(&mut self, dimmed: bool) {
        if dimmed != self.dimmed {
            self.dimmed = dimmed;
            use wry::WebViewExtMacOS;
            let wk = self.webview.webview();
            let view: &objc2_app_kit::NSView = &wk;
            view.setAlphaValue(if dimmed { 0.72 } else { 1.0 });
        }
    }

    pub fn set_zoom(&mut self, zoom: f64) {
        if (self.zoom - zoom).abs() > f64::EPSILON {
            self.zoom = zoom;
            let _ = self.webview.zoom(zoom);
        }
    }

    pub fn back(&self) {
        let _ = self.webview.go_back();
    }

    pub fn forward(&self) {
        let _ = self.webview.go_forward();
    }

    pub fn reload(&mut self) {
        if self.loading {
            use wry::WebViewExtMacOS;
            // SAFETY: main thread, a live view.
            unsafe { self.webview.webview().stopLoading() };
            self.loading = false;
        } else if !self.is_blank() {
            let _ = self.webview.reload();
            // It starts asynchronously; its commit restarts the watch.
            self.loading = true;
        }
    }

    /// ⌥⌘I: WebKit's inspector, docked to the page.
    pub fn toggle_devtools(&self) {
        if self.webview.is_devtools_open() {
            self.webview.close_devtools();
        } else {
            self.webview.open_devtools();
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

    // -- rendering ------------------------------------------------------------

    fn render_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.address.read(cx).focus.is_focused(window);
        let nav = |id: &'static str, glyph: Icon, enabled: bool| {
            icon_button(id, glyph).when(!enabled, |d| d.opacity(0.35))
        };
        let leading: AnyElement = match (&self.favicon, editing || self.is_blank()) {
            (Some(img_), false) => site_image(img_.clone(), 14.),
            _ => icon(Icon::Search, 14., theme::DIM),
        };
        let pill = div()
            .id("address")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h(u(30.))
            .pl(u(10.))
            .rounded(u(8.))
            .bg(rgb(if editing {
                theme::SURFACE_HOVER
            } else {
                theme::SURFACE
            }))
            .border_1()
            .border_color(rgba(if editing {
                theme::INPUT_EDGE
            } else {
                0xffffff00
            }))
            .cursor_text()
            // The first click into the address selects all of it, as in
            // Safari; the unfocused text is a shortened address anyway, so a
            // caret placed in it would land in the wrong spot.
            .capture_any_mouse_down(cx.listener(|p, _, window, cx| {
                p.select_on_click = !p.address.read(cx).focus.is_focused(window);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|p, _, window, cx| {
                    if std::mem::take(&mut p.select_on_click) {
                        p.focus_address(window, cx);
                    }
                }),
            )
            .child(leading)
            .child(self.address.clone());

        let download = self.download.as_ref().map(|d| {
            let path = d.path.clone();
            div()
                .id("download")
                .flex()
                .flex_none()
                .items_center()
                .gap(u(6.))
                .h(u(26.))
                .ml(u(4.))
                .px(u(8.))
                .rounded(u(6.))
                .text_size(u(12.))
                .text_color(rgb(theme::TEXT_2))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SURFACE)).text_color(rgb(theme::TEXT)))
                .child(match d.state {
                    DownloadState::Running => spinner("download-spin", 12., theme::TEXT_2),
                    DownloadState::Done => icon(Icon::Download, 13., theme::TEXT),
                    DownloadState::Failed => icon(Icon::Close, 13., theme::DIM),
                })
                .child(div().max_w(u(150.)).truncate().child(d.name.clone()))
                .on_click(move |_, _, _| {
                    // Show it in Finder, selected.
                    if let Some(p) = &path {
                        let _ = std::process::Command::new("/usr/bin/open")
                            .arg("-R")
                            .arg(p)
                            .spawn();
                    }
                })
        });

        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(2.))
            .h(u(theme::TOOLBAR_PX))
            .px(u(6.))
            .bg(rgb(theme::PANE))
            .child(
                nav("back", Icon::ArrowLeft, self.can_back)
                    .on_click(cx.listener(|p, _, _, _| p.back())),
            )
            .child(
                nav("forward", Icon::ArrowRight, self.can_forward)
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
                .mr(u(6.))
                .on_click(cx.listener(|p, _, _, cx| {
                    p.reload();
                    cx.notify();
                })),
            )
            .child(pill)
            .children(download)
            .child(
                icon_button("close-pane", Icon::Close)
                    .ml(u(4.))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(BrowserEvent::Close))),
            )
            // Real progress along the toolbar's bottom edge while loading.
            .when(self.loading && !self.is_blank(), |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .h(u(2.))
                        .w(relative(self.progress.clamp(0.08, 1.)))
                        .rounded(u(1.))
                        .bg(rgba(0xffffffcc)),
                )
            })
    }

    fn render_find(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let f = self.find.as_ref()?;
        let q_empty = f.field.read(cx).text().is_empty();
        let count = match (q_empty, f.matches) {
            (true, _) | (_, None) => String::new(),
            (false, Some(0)) => "No matches".into(),
            (false, Some(1)) => "1 match".into(),
            (false, Some(n)) => format!("{n} matches"),
        };
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(u(2.))
                .h(u(40.))
                .px(u(6.))
                .bg(rgb(theme::PANE))
                .border_t_1()
                .border_color(rgba(theme::HAIRLINE))
                .child(
                    div()
                        .w(u(PILL_LEFT - 6.))
                        .flex()
                        .justify_end()
                        .pr(u(8.))
                        .child(icon(Icon::Search, 14., theme::DIM)),
                )
                .child(f.field.clone())
                .child(
                    div()
                        .flex_none()
                        .min_w(u(78.))
                        .px(u(8.))
                        .text_size(u(12.))
                        .text_color(rgb(theme::DIM))
                        .child(count),
                )
                .child(
                    icon_button("find-up", Icon::ChevronUp)
                        .on_click(cx.listener(|p, _, _, cx| p.find_run(true, false, cx))),
                )
                .child(
                    icon_button("find-down", Icon::ChevronDown)
                        .on_click(cx.listener(|p, _, _, cx| p.find_run(false, false, cx))),
                )
                .child(
                    icon_button("find-close", Icon::Close)
                        .on_click(cx.listener(|p, _, _, cx| p.close_find(cx))),
                ),
        )
    }

    fn render_suggest(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .flex()
            .flex_col()
            .gap(u(1.))
            .pt(u(6.))
            .pl(u(PILL_LEFT))
            .pr(u(PILL_RIGHT));
        if let Some(s) = &self.suggest {
            for (i, row) in s.rows.iter().enumerate() {
                let selected = i == s.selected;
                let (lead, primary, secondary): (AnyElement, String, Option<String>) = match row {
                    Row::Go { url, label } => (site_icon(url, Icon::Globe), label.clone(), None),
                    Row::Search(q) => (
                        icon(Icon::Search, 14., theme::TEXT_2),
                        q.clone(),
                        Some(format!("Search {}", search_engine().name())),
                    ),
                    Row::Page(v) => {
                        let addr = pretty_url(&v.url);
                        if v.title.is_empty() {
                            (site_icon(&v.url, Icon::Clock), addr, None)
                        } else {
                            (site_icon(&v.url, Icon::Clock), v.title.clone(), Some(addr))
                        }
                    }
                    Row::Server(port, process) => (
                        icon(Icon::Terminal, 14., theme::TEXT_2),
                        format!("localhost:{port}"),
                        Some(process.clone()),
                    ),
                };
                list = list.child(
                    div()
                        .id(ElementId::NamedInteger("suggest".into(), i as u64))
                        .flex()
                        .items_center()
                        .gap(u(10.))
                        .h(u(34.))
                        .px(u(10.))
                        .rounded(u(8.))
                        .text_size(u(13.))
                        .cursor_pointer()
                        .when(selected, |d| d.bg(rgb(theme::SURFACE)))
                        .when(!selected, |d| d.hover(|s| s.bg(rgb(theme::ROW_HOVER))))
                        .child(
                            div()
                                .flex_none()
                                .w(u(16.))
                                .flex()
                                .justify_center()
                                .child(lead),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .items_center()
                                .gap(u(8.))
                                .child(
                                    div()
                                        .flex_shrink()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(rgb(theme::TEXT))
                                        .child(primary),
                                )
                                .children(secondary.map(|s| {
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(u(12.))
                                        .text_color(rgb(theme::DIM))
                                        .child(s)
                                })),
                        )
                        .when(selected, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_size(u(12.))
                                    .text_color(rgb(theme::DIM))
                                    .child("↵"),
                            )
                        })
                        .on_click(cx.listener(move |p, _, _, cx| p.open_row(i, cx))),
                );
            }
        }
        div().size_full().bg(rgb(theme::PANE)).child(list)
    }

    /// The new tab page: the sites you go back to, and the dev servers your
    /// terminals are running.
    fn render_blank(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = |text: &'static str| {
            div()
                .pl(u(4.))
                .pb(u(8.))
                .text_size(u(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::DIM))
                .child(text)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(u(28.))
            .w(u(536.))
            .max_w(relative(0.9));

        if !self.top.is_empty() {
            let mut grid = div().flex().flex_wrap().gap(u(4.));
            for (i, v) in self.top.iter().enumerate() {
                let url = v.url.clone();
                let host = history::host(&v.url).to_string();
                let mark: AnyElement = match favicon::get(&host) {
                    Some(img_) => site_image(img_, 22.),
                    None => div()
                        .text_size(u(17.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::TEXT_2))
                        .child(
                            host.chars()
                                .find(|c| c.is_alphanumeric())
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_default(),
                        )
                        .into_any_element(),
                };
                grid = grid.child(
                    div()
                        .id(ElementId::NamedInteger("top".into(), i as u64))
                        .group("tile")
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(u(8.))
                        .w(u(130.))
                        .py(u(12.))
                        .rounded(u(10.))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(theme::ROW_HOVER)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(u(44.))
                                .rounded(u(12.))
                                .bg(rgb(theme::SURFACE))
                                .border_1()
                                .border_color(rgba(theme::HAIRLINE))
                                .child(mark),
                        )
                        .child(
                            div()
                                .w(u(112.))
                                .truncate()
                                .text_center()
                                .text_size(u(12.))
                                .text_color(rgb(theme::TEXT_2))
                                .child(host),
                        )
                        .on_click(cx.listener(move |p, _, _, cx| p.open(&url, cx))),
                );
            }
            col = col.child(div().child(label("Frequently visited")).child(grid));
        }

        if !self.servers.is_empty() {
            let mut rows = div().flex().flex_col().gap(u(1.));
            for (port, process) in &self.servers {
                let url = format!("http://localhost:{port}");
                rows = rows.child(
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
                        .hover(|s| s.bg(rgb(theme::ROW_HOVER)))
                        .child(icon(Icon::Terminal, 14., theme::TEXT_2))
                        .child(
                            div()
                                .text_color(rgb(theme::TEXT))
                                .child(format!("localhost:{port}")),
                        )
                        .child(
                            div()
                                .text_size(u(12.))
                                .text_color(rgb(theme::DIM))
                                .child(process.clone()),
                        )
                        .child(div().flex_1())
                        .child(icon(Icon::ArrowUpRight, 13., theme::DIM))
                        .on_click(cx.listener(move |p, _, _, cx| p.open(&url, cx))),
                );
            }
            col = col.child(div().child(label("Running on this Mac")).child(rows));
        }

        if self.top.is_empty() && self.servers.is_empty() {
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(u(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(u(48.))
                            .rounded(u(24.))
                            .bg(rgb(theme::SURFACE))
                            .child(icon(Icon::Search, 20., theme::TEXT_2)),
                    )
                    .child(
                        div()
                            .text_size(u(14.))
                            .text_color(rgb(theme::TEXT_2))
                            .child(format!(
                                "Search {} or enter an address",
                                search_engine().name()
                            )),
                    )
                    .child(
                        div()
                            .text_size(u(12.))
                            .text_color(rgb(theme::DIM))
                            .child("Dev servers you start in a terminal show up here"),
                    ),
            );
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let toolbar = self.render_toolbar(window, cx);
        let find = self.render_find(cx);
        let body = if self.suggest.is_some() {
            self.render_suggest(cx).into_any_element()
        } else if self.is_blank() {
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
            .children(find)
            .child(div().flex_none().h(u(1.)).bg(rgba(theme::HAIRLINE)))
            .child(div().flex_1().min_h_0().child(body))
    }
}

/// A site's icon at `size`, or `fallback` when deck has none for it yet.
fn site_icon(url: &str, fallback: Icon) -> AnyElement {
    match favicon::get(history::host(url)) {
        Some(img_) => site_image(img_, 14.),
        None => icon(fallback, 14., theme::TEXT_2),
    }
}

pub fn site_image(image: Arc<Image>, size: f32) -> AnyElement {
    img(image)
        .size(u(size))
        .flex_none()
        .rounded(u(size / 5.))
        .into_any_element()
}

/// The webview sits inside the card: round its bottom corners (the card's
/// own clipping does not reach a native view) and keep it in dark
/// appearance whatever the system is set to, so pages that follow
/// `prefers-color-scheme`, their scrollbars and form controls match deck.
fn style_native(webview: &wry::WebView, radius: f32) {
    use objc2_app_kit::{NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua};
    use objc2_quartz_core::CACornerMask;
    use wry::WebViewExtMacOS;
    let wk = webview.webview();
    let view: &objc2_app_kit::NSView = &wk;
    // SAFETY: an AppKit constant.
    let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua });
    view.setAppearance(dark.as_deref());
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

/// Percent-encode for a query string, spaces as `+`.
pub fn encode_query(s: &str) -> String {
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
    q
}

/// The search engine picked in Settings; Google until told otherwise.
static ENGINE: Mutex<Search> = Mutex::new(Search::Google);

pub fn set_search_engine(engine: Search) {
    *ENGINE.lock().unwrap_or_else(|e| e.into_inner()) = engine;
}

pub fn search_engine() -> Search {
    *ENGINE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn search_url(query: &str) -> String {
    search_engine().url(&encode_query(query.trim()))
}

/// A host on the local network or this machine, where dev servers speak
/// plain HTTP.
fn is_local_host(s: &str) -> bool {
    let host = s.split(['/', '?', '#']).next().unwrap_or(s);
    let host = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    host == "localhost"
        || host == "[::1]"
        || host.ends_with(".localhost")
        || (host.split('.').count() == 4
            && host
                .split('.')
                .all(|p| !p.is_empty() && p.parse::<u8>().is_ok()))
}

/// Text the address bar opens rather than searches for.
fn looks_like_address(s: &str) -> bool {
    let s = s.trim();
    if s.starts_with('?') {
        return false;
    }
    s.contains("://")
        || s.starts_with("about:")
        || s.starts_with("data:")
        || is_local_host(s)
        || (!s.contains(char::is_whitespace) && s.contains('.') && !s.ends_with('.'))
}

/// What the address bar does: URLs pass through, hosts get a scheme (plain
/// HTTP for this machine and the local network), anything else is a search
/// with the chosen engine. A leading `?` forces a search.
pub fn normalize_url(input: &str) -> String {
    let s = input.trim();
    if let Some(q) = s.strip_prefix('?') {
        return search_url(q);
    }
    if !looks_like_address(s) {
        return search_url(s);
    }
    if s.contains("://") || s.starts_with("about:") || s.starts_with("data:") {
        return s.to_string();
    }
    if is_local_host(s) {
        return format!("http://{s}");
    }
    format!("https://{s}")
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
        assert_eq!(normalize_url("localhost"), "http://localhost");
        assert_eq!(
            normalize_url("192.168.1.20:8080/x"),
            "http://192.168.1.20:8080/x"
        );
        assert_eq!(
            normalize_url("app.localhost:5173"),
            "http://app.localhost:5173"
        );
        assert_eq!(
            normalize_url("rust gpui"),
            "https://www.google.com/search?q=rust+gpui"
        );
        assert_eq!(
            normalize_url("a&b"),
            "https://www.google.com/search?q=a%26b"
        );
        assert_eq!(
            normalize_url("?example.com"),
            "https://www.google.com/search?q=example.com"
        );
        assert_eq!(
            normalize_url("node."),
            "https://www.google.com/search?q=node."
        );
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
    fn page_messages_are_checked() {
        assert!(matches!(parse_ipc("focus"), Some(Msg::Focused)));
        assert!(matches!(parse_ipc("moved"), Some(Msg::Moved)));
        assert!(matches!(
            parse_ipc("tab:https://a.b/c"),
            Some(Msg::LinkTab(u)) if u == "https://a.b/c"
        ));
        assert!(parse_ipc("tab:javascript:alert(1)").is_none());
        assert!(parse_ipc("icon:file:///etc/passwd").is_none());
        assert!(parse_ipc("url:https://evil.example").is_none());
    }

    #[test]
    fn eval_results_unwrap_and_errors_surface() {
        assert_eq!(decode_eval(r#""{\"ok\":3}""#), Ok(Value::from(3)));
        assert_eq!(decode_eval(r#""{\"ok\":null}""#), Ok(Value::Null));
        assert_eq!(decode_eval(r#""{\"err\":\"boom\"}""#), Err("boom".into()));
    }
}
