//! The right-hand inspector: what is running in the focused pane, where, and
//! what it is listening on, plus the project's files and git state.
//!
//! Collection shells out to `ps`, `lsof` and `git` (~80ms together), so it
//! always runs on the background executor; the UI renders the last result.

use crate::icon::{icon, Icon};
use crate::inspect::{self, Entry, GitInfo, Port, Proc};
use crate::theme::{self, u, OneLine};
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, ClipboardItem, Context, ElementId, FontWeight, SharedString,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Info,
    Files,
    Git,
}

#[derive(Default, Clone)]
pub struct Data {
    pub procs: Vec<Proc>,
    pub ports: Vec<Port>,
    pub root: Option<PathBuf>,
    pub git: Option<GitInfo>,
    /// Listings for the root and every expanded directory.
    pub dirs: HashMap<PathBuf, Vec<Entry>>,
}

pub struct Inspector {
    pub open: bool,
    pub section: Section,
    pub data: Data,
    pub expanded: HashSet<PathBuf>,
}

impl Default for Inspector {
    fn default() -> Self {
        Self {
            open: false,
            section: Section::Info,
            data: Data::default(),
            expanded: HashSet::new(),
        }
    }
}

/// Blocking: run on the background executor.
/// `git` is the one part that still spawns a process, so callers refresh it
/// less often and keep `previous` otherwise.
pub fn collect(
    pid: u32,
    fallback: PathBuf,
    expanded: Vec<PathBuf>,
    previous: Option<Option<GitInfo>>,
) -> Data {
    let mut procs = inspect::descendants(pid);
    // The setuid `login` wrapper is how every shell is started; the kernel
    // hides its stats from us, and it is not something the user ran.
    procs.retain(|p| !(p.name == "login" && p.rss == 0));
    let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
    let ports = inspect::listening_ports(&pids);
    // The PTY's child is setuid `login`, whose cwd the kernel will not tell
    // us; the deepest process (the foreground job, or the shell) is the one
    // whose directory the user is actually in.
    let cwd = procs
        .iter()
        .rev()
        .find_map(|p| inspect::cwd(p.pid))
        .unwrap_or(fallback);
    let root = inspect::project_root(&cwd);
    let git = previous.unwrap_or_else(|| inspect::git(&root));
    let mut dirs = HashMap::new();
    dirs.insert(root.clone(), inspect::list_dir(&root));
    for d in expanded.into_iter().filter(|d| d.starts_with(&root)) {
        let list = inspect::list_dir(&d);
        dirs.insert(d, list);
    }
    Data {
        procs,
        ports,
        root: Some(root),
        git,
        dirs,
    }
}

/// A section label, as the sidebar draws its own: sentence case, quiet.
fn header(label: &'static str, count: Option<usize>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(u(6.))
        .pt(u(20.))
        .pb(u(6.))
        .text_size(u(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::dim()))
        .child(div().flex_1().child(label))
        .children(count.map(|n| n.to_string()))
}

/// A small action beside a value: the icon says what it does.
fn action(id: &'static str, glyph: Icon) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(u(24.))
        .rounded(u(6.))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::surface())))
        .child(icon(glyph, 13., theme::text_2()))
}

/// What a section shows when it has nothing.
fn empty(text: &'static str) -> gpui::Div {
    div()
        .text_size(u(12.))
        .text_color(rgb(theme::dim()))
        .child(text)
}

/// `/Users/me/deck` → `~/deck`.
fn tilde(path: &std::path::Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if shown == home => "~".into(),
        Ok(home) => shown
            .strip_prefix(&format!("{home}/"))
            .map_or(shown.clone(), |rest| format!("~/{rest}")),
        Err(_) => shown,
    }
}

pub fn render(ws: &Workspace, title: SharedString, cx: &mut Context<Workspace>) -> AnyElement {
    let ins = &ws.inspector;
    // Concentric: 6px segments inside 2px of padding make an 8px track, 28
    // tall like the title bar's buttons beside it.
    let mut tabs = div()
        .flex()
        .flex_1()
        .gap(u(2.))
        .h(u(28.))
        .p(u(2.))
        .rounded(u(8.))
        .bg(rgb(theme::pane()))
        .border_1()
        .border_color(rgba(theme::hairline()));
    for (s, label, glyph) in [
        (Section::Info, "Info", Icon::Info),
        (Section::Files, "Files", Icon::Files),
        (Section::Git, "Git", Icon::Git),
    ] {
        let on = ins.section == s;
        tabs = tabs.child(
            div()
                .id(ElementId::Name(label.into()))
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(u(6.))
                .rounded(u(6.))
                .text_size(u(12.))
                .when(on, |d| {
                    d.bg(rgb(theme::surface())).text_color(rgb(theme::text()))
                })
                .when(!on, |d| {
                    d.text_color(rgb(theme::text_2()))
                        .hover(|s| s.text_color(rgb(theme::text())))
                })
                .cursor_pointer()
                .child(icon(
                    glyph,
                    12.,
                    if on { theme::text() } else { theme::text_2() },
                ))
                .child(label)
                .on_click(cx.listener(move |ws, _, _, cx| {
                    ws.inspector.section = s;
                    cx.notify();
                })),
        );
    }

    let body = match ins.section {
        Section::Info => match ws.focused_browser(cx) {
            Some((t, url)) => browser_info(t, url),
            None => info(ws, title, cx),
        },
        Section::Files => files(ws, cx),
        Section::Git => git(ws),
    };

    div()
        .id("inspector")
        .flex()
        .flex_col()
        .flex_none()
        .w(u(theme::INSPECTOR_PX))
        .h_full()
        .pl(u(theme::GUTTER + 4.))
        .pr(u(theme::GUTTER + 4.))
        .bg(rgb(theme::sidebar()))
        .text_color(rgb(theme::text()))
        // Same strip height as the title bar, so the two tops line up.
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(px(theme::TITLEBAR_PX))
                .child(tabs),
        )
        .child(
            div()
                .id("inspector-body")
                .flex_1()
                .overflow_y_scroll()
                .pb(u(12.))
                .child(body),
        )
        .into_any_element()
}

fn browser_info(title: SharedString, url: SharedString) -> AnyElement {
    let (u1, u2) = (url.to_string(), url.to_string());
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .pt(u(16.))
                .text_size(u(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().w_full().one_line().child(title)),
        )
        .child(header("Address", None))
        .child(
            div()
                .flex()
                .items_start()
                .gap(u(4.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pt(u(3.))
                        .text_size(u(12.))
                        .text_color(rgb(theme::text_2()))
                        .child(url),
                )
                .child(action("copy-url", Icon::Copy).on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(u1.clone()))
                }))
                .child(
                    action("open-default", Icon::ArrowUpRight)
                        .on_click(move |_, _, cx| cx.open_url(&u2)),
                ),
        )
        .into_any_element()
}

fn info(ws: &Workspace, title: SharedString, cx: &mut Context<Workspace>) -> AnyElement {
    let d = &ws.inspector.data;
    let pid = d.procs.first().map(|p| p.pid);
    let mut col = div().flex().flex_col().child(
        div()
            .flex()
            .flex_col()
            .gap(u(2.))
            .pt(u(16.))
            .child(
                div()
                    .w_full()
                    .text_size(u(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .one_line()
                    .child(title),
            )
            .when_some(pid, |c, pid| {
                c.child(
                    div()
                        .text_size(u(12.))
                        .text_color(rgb(theme::dim()))
                        .child(format!("pid {pid}")),
                )
            }),
    );

    if let Some(root) = d.root.clone() {
        let (r1, r2) = (root.clone(), root.display().to_string());
        col = col.child(header("Project", None)).child(
            div()
                .flex()
                .items_center()
                .gap(u(4.))
                .child(icon(Icon::Folder, 13., theme::text_2()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pl(u(4.))
                        .text_size(u(13.))
                        .one_line()
                        .child(tilde(&root)),
                )
                .child(
                    action("finder", Icon::ArrowUpRight)
                        .on_click(move |_, _, cx| cx.reveal_path(&r1)),
                )
                .child(action("copy", Icon::Copy).on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(r2.clone()))
                })),
        );
    }

    col = col.child(header("Processes", Some(d.procs.len())));
    // The tree reads by indent: the shell, what it started, and theirs.
    let mut depth: std::collections::HashMap<u32, usize> = Default::default();
    for p in &d.procs {
        let level = depth.get(&p.ppid).map_or(0, |d| d + 1);
        depth.insert(p.pid, level);
        let busy = p.cpu >= 1.;
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(u(6.))
                .h(u(24.))
                .pl(u(level.min(4) as f32 * 12.))
                .text_size(u(13.))
                .when(level > 0, |d| {
                    d.child(div().w(u(6.)).h(u(1.)).bg(rgba(theme::input_edge())))
                })
                .child(
                    div()
                        .flex_shrink()
                        .min_w_0()
                        .one_line()
                        .text_color(rgb(if level == 0 {
                            theme::text_2()
                        } else {
                            theme::text()
                        }))
                        .child(p.name.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(u(11.))
                        .text_color(rgb(theme::dim()))
                        .child(p.pid.to_string()),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .text_size(u(11.))
                        .text_color(rgb(if busy { theme::text_2() } else { theme::dim() }))
                        .child(format!("{:.0}% · {}", p.cpu, inspect::human_bytes(p.rss))),
                ),
        );
    }

    col = col.child(header("Ports", None));
    if d.ports.is_empty() {
        col = col.child(empty("None listening"));
    }
    for port in &d.ports {
        let url = format!("http://localhost:{}", port.port);
        col = col.child(
            div()
                .id(ElementId::NamedInteger("port".into(), port.port as u64))
                .flex()
                .items_center()
                .gap(u(8.))
                .h(u(28.))
                .px(u(6.))
                .rounded(u(6.))
                .text_size(u(13.))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::surface())))
                .child(
                    div()
                        .text_color(rgb(theme::text()))
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!(":{}", port.port)),
                )
                .child(
                    div()
                        .text_color(rgb(theme::text_2()))
                        .one_line()
                        .child(port.process.clone()),
                )
                .child(div().flex_1())
                .child(icon(Icon::ArrowUpRight, 13., theme::dim()))
                .on_click(
                    cx.listener(move |ws, _, window, cx| ws.open_url_beside(&url, window, cx)),
                ),
        );
    }
    col.into_any_element()
}

fn files(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let d = &ws.inspector.data;
    let Some(root) = &d.root else {
        return empty("No project").pt(u(16.)).into_any_element();
    };
    let mut col = div().flex().flex_col().pt(u(10.));
    let mut stack: Vec<(&Entry, usize)> = d
        .dirs
        .get(root)
        .map(|v| v.iter().rev().map(|e| (e, 0)).collect())
        .unwrap_or_default();
    let mut rows = 0;
    while let Some((e, depth)) = stack.pop() {
        rows += 1;
        if rows > 600 {
            break;
        }
        let open = ws.inspector.expanded.contains(&e.path);
        let path = e.path.clone();
        let is_dir = e.is_dir;
        col = col.child(
            div()
                .id(ElementId::Name(e.path.display().to_string().into()))
                .flex()
                .items_center()
                .gap(u(6.))
                .h(u(24.))
                .pl(u(6. + depth as f32 * 14.))
                .rounded(u(5.))
                .text_size(u(13.))
                .text_color(rgb(if is_dir {
                    theme::text()
                } else {
                    theme::text_2()
                }))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::surface())))
                .child(div().w(u(12.)).flex_none().when(is_dir, |d| {
                    d.child(icon(
                        if open {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        },
                        12.,
                        theme::dim(),
                    ))
                }))
                .child(icon(
                    if is_dir { Icon::Folder } else { Icon::File },
                    14.,
                    if is_dir {
                        theme::text_2()
                    } else {
                        theme::dim()
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().w_full().one_line().child(e.name.clone())),
                )
                .on_click(cx.listener(move |ws, _, _, cx| {
                    if is_dir {
                        if !ws.inspector.expanded.remove(&path) {
                            ws.inspector.expanded.insert(path.clone());
                        }
                        ws.refresh_inspector(cx);
                    } else {
                        cx.open_with_system(&path);
                    }
                })),
        );
        if is_dir && open {
            if let Some(children) = d.dirs.get(&e.path) {
                stack.extend(children.iter().rev().map(|c| (c, depth + 1)));
            }
        }
    }
    col.into_any_element()
}

fn git(ws: &Workspace) -> AnyElement {
    let Some(g) = &ws.inspector.data.git else {
        return empty("Not a git repository").pt(u(16.)).into_any_element();
    };
    let mut col = div().flex().flex_col().child(
        div()
            .flex()
            .items_center()
            .gap(u(8.))
            .pt(u(14.))
            .child(
                div()
                    .text_size(u(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(g.branch.clone()),
            )
            .when(g.ahead > 0, |d| {
                d.child(
                    div()
                        .text_color(rgb(theme::text_2()))
                        .child(format!("↑{}", g.ahead)),
                )
            })
            .when(g.behind > 0, |d| {
                d.child(
                    div()
                        .text_color(rgb(theme::text_2()))
                        .child(format!("↓{}", g.behind)),
                )
            }),
    );
    col = col.child(header("Changes", Some(g.files.len())));
    if g.files.is_empty() {
        col = col.child(
            div()
                .text_size(u(13.))
                .text_color(rgb(theme::text_2()))
                .child("Working tree clean"),
        );
    }
    for f in &g.files {
        let color = match f.status.as_str() {
            // The letter carries the meaning; weight of grey ranks it.
            "U" | "D" => theme::text(),
            "??" => theme::dim(),
            _ => theme::text_2(),
        };
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(u(8.))
                .h(u(24.))
                .text_size(u(13.))
                .child(
                    div()
                        .w(u(18.))
                        .text_size(u(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(color))
                        .child(f.status.clone()),
                )
                .child(
                    div()
                        .one_line()
                        .text_color(rgb(theme::text_2()))
                        .child(f.path.clone()),
                ),
        );
    }
    col.into_any_element()
}
