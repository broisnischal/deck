//! Usage, at the foot of the sidebar: how full a thread's context is, what
//! it has cost, and how much of the plan's allowance is left.
//!
//! Everything here is what the agents reported, kept in each thread's
//! transcript; nothing is estimated or fetched.

use crate::chat::compact_count;
use crate::icon::{icon, Icon};
use crate::theme::{self, u};
use crate::workspace::{PaneId, Workspace};
use agent_core::{Allowance, Limits, Usage};
use gpui::prelude::*;
use gpui::{div, relative, rgb, rgba, AnyElement, Context, ElementId, FontWeight, SharedString};
use std::time::Instant;

/// One chat's numbers.
pub struct Thread {
    pub pane: PaneId,
    pub title: SharedString,
    pub mark: Icon,
    pub agent: SharedString,
    pub model: Option<String>,
    pub usage: Usage,
    pub focused: bool,
    pub active: Instant,
}

/// The thread the panel speaks for: the focused chat, else the latest.
/// `threads` is newest first.
pub fn current(threads: &[Thread]) -> Option<&Thread> {
    threads
        .iter()
        .find(|t| t.focused)
        .or_else(|| threads.first())
}

/// The plan's allowance as the latest thread that saw it reported it, less
/// any window that has reset since.
fn plan(threads: &[Thread], now: i64) -> Option<Limits> {
    let l = threads.iter().find_map(|t| t.usage.limits)?;
    let live = |a: Option<Allowance>| a.filter(|a| a.resets_at > now);
    let l = Limits {
        session: live(l.session),
        week: live(l.week),
    };
    (l.session.is_some() || l.week.is_some()).then_some(l)
}

/// What the row in the sidebar says without opening: the context, else the
/// cost, else the tokens.
pub fn glance(t: &Thread) -> Option<(Option<f32>, String)> {
    let u = &t.usage;
    if let Some(f) = u.context_used() {
        return Some((Some(f), percent(f)));
    }
    if let Some(c) = u.cost_usd.filter(|c| *c > 0.) {
        return Some((None, dollars(c)));
    }
    (u.tokens() > 0).then(|| (None, compact_count(u.tokens())))
}

/// The panel above the row.
pub fn card(threads: &[Thread], cx: &mut Context<Workspace>) -> AnyElement {
    let now = unix_now();
    let mut card = div()
        .id("usage-card")
        .flex()
        .flex_col()
        .gap(u(16.))
        .mb(u(4.))
        .p(u(12.))
        .rounded(u(theme::CARD_RADIUS))
        .bg(rgb(theme::raised()))
        .border_1()
        .border_color(rgba(theme::hairline()))
        // A click on the card is not a click on the pane behind it.
        .occlude();
    match current(threads) {
        Some(t) => card = card.child(context(t)).child(thread(t)),
        None => {
            card = card.child(note(
                "No chats yet. What an agent spends shows here once it replies.",
            ))
        }
    }
    if let Some(l) = plan(threads, now) {
        card = card.child(allowance(l, now));
    }
    if threads.iter().filter(|t| t.usage.tokens() > 0).count() > 1 {
        card = card.child(everything(threads, cx));
    }
    card.into_any_element()
}

fn context(t: &Thread) -> AnyElement {
    let u_ = &t.usage;
    let used = u_.context_used();
    let held = match (u_.context, u_.context_window) {
        (0, _) => None,
        (n, Some(w)) => Some(format!("{} of {}", compact_count(n), compact_count(w))),
        (n, None) => Some(format!("{} tokens", compact_count(n))),
    };
    let who = match &t.model {
        Some(m) => format!("{} · {}", t.agent, model_name(m)),
        None => t.agent.to_string(),
    };
    let line = match held {
        Some(h) => format!("{h} · {who}"),
        None if u_.tokens() == 0 => format!("{who} · nothing reported yet"),
        None => who,
    };
    div()
        .flex()
        .flex_col()
        .gap(u(6.))
        .child(heading("Context", used.map(percent)))
        .child(meter(used.unwrap_or(0.)))
        .child(note(line))
        .into_any_element()
}

fn thread(t: &Thread) -> AnyElement {
    let u_ = &t.usage;
    let tile = |value: u64, label: &'static str| {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(u(1.))
            .child(
                div()
                    .text_size(u(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::text()))
                    .child(compact_count(value)),
            )
            .child(
                div()
                    .text_size(u(11.))
                    .text_color(rgb(theme::dim()))
                    .child(label),
            )
    };
    let pair = |a, b| div().flex().gap(u(12.)).child(a).child(b);
    div()
        .flex()
        .flex_col()
        .gap(u(8.))
        .child(heading("This thread", u_.cost_usd.map(dollars)))
        .child(pair(tile(u_.input, "Input"), tile(u_.output, "Output")))
        .child(pair(
            tile(u_.cache_read, "Cache read"),
            tile(u_.cache_write, "Cache write"),
        ))
        .into_any_element()
}

fn allowance(l: Limits, now: i64) -> AnyElement {
    let window = |name: &'static str, a: Allowance| {
        div()
            .flex()
            .flex_col()
            .gap(u(5.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(u(6.))
                    .text_size(u(12.))
                    .child(div().text_color(rgb(theme::text())).child(name))
                    .child(
                        div()
                            .flex_1()
                            .text_size(u(11.))
                            .text_color(rgb(theme::dim()))
                            .child(format!("resets in {}", until(a.resets_at - now))),
                    )
                    .child(
                        div()
                            .text_size(u(11.))
                            .text_color(rgb(theme::text_2()))
                            .child(percent(a.used)),
                    ),
            )
            .child(meter(a.used))
    };
    div()
        .flex()
        .flex_col()
        .gap(u(8.))
        .child(heading("Plan", None))
        .children(l.session.map(|a| window("Session", a)))
        .children(l.week.map(|a| window("Week", a)))
        .into_any_element()
}

/// Every thread that has spent anything, the heaviest first.
fn everything(threads: &[Thread], cx: &mut Context<Workspace>) -> AnyElement {
    let mut spent: Vec<&Thread> = threads.iter().filter(|t| t.usage.tokens() > 0).collect();
    spent.sort_by_key(|t| std::cmp::Reverse(t.usage.tokens()));
    let cost: f64 = spent.iter().filter_map(|t| t.usage.cost_usd).sum();
    let tokens: u64 = spent.iter().map(|t| t.usage.tokens()).sum();
    let total = if cost > 0. {
        format!("{} · {}", compact_count(tokens), dollars(cost))
    } else {
        compact_count(tokens)
    };
    let mut col = div()
        .flex()
        .flex_col()
        .gap(u(2.))
        .child(div().mb(u(4.)).child(heading("All threads", Some(total))));
    for t in spent.iter().take(5) {
        let pane = t.pane;
        let figure = match t.usage.cost_usd.filter(|c| *c > 0.) {
            Some(c) => dollars(c),
            None => compact_count(t.usage.tokens()),
        };
        col = col.child(
            div()
                .id(ElementId::NamedInteger("usage-thread".into(), pane))
                .flex()
                .items_center()
                .gap(u(8.))
                .h(u(28.))
                // Rows bleed to the card's padding so their hover has room.
                .mx(u(-6.))
                .px(u(6.))
                .rounded(u(6.))
                .text_size(u(12.))
                .cursor_pointer()
                .when(t.focused, |d| d.bg(rgb(theme::surface())))
                .when(!t.focused, |d| d.hover(|s| s.bg(rgb(theme::surface()))))
                .child(icon(t.mark, 13., theme::text_2()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(theme::text()))
                        .child(t.title.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(u(11.))
                        .text_color(rgb(theme::text_2()))
                        .child(figure),
                )
                .on_click(cx.listener(move |ws, _, window, cx| ws.focus_pane(pane, window, cx))),
        );
    }
    col.into_any_element()
}

/// A section's name, with its figure on the trailing edge.
fn heading(label: &'static str, figure: Option<String>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .text_size(u(11.))
        .child(
            div()
                .flex_1()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::dim()))
                .child(label),
        )
        .children(figure.map(|f| {
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::text()))
                .child(f)
        }))
}

fn note(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(u(11.))
        .line_height(u(15.))
        .text_color(rgb(theme::dim()))
        .child(text.into())
}

/// A bar filled to `frac`; past 80% it fills brighter, since that is when
/// the number starts to matter.
pub fn meter(frac: f32) -> gpui::Div {
    let frac = frac.clamp(0., 1.);
    div()
        .w_full()
        .h(u(4.))
        .rounded(u(2.))
        .bg(rgb(theme::surface()))
        .when(frac > 0., |d| {
            d.child(
                div()
                    .h_full()
                    // A sliver still shows that something is there.
                    .w(relative(frac.max(0.03)))
                    .rounded(u(2.))
                    .bg(rgb(if frac >= 0.8 {
                        theme::text()
                    } else {
                        theme::text_2()
                    })),
            )
        })
}

pub fn percent(f: f32) -> String {
    let p = f * 100.;
    if p > 0. && p < 1. {
        "<1%".into()
    } else {
        format!("{}%", p.round() as u32)
    }
}

fn dollars(c: f64) -> String {
    if c < 0.01 {
        "<$0.01".into()
    } else {
        format!("${c:.2}")
    }
}

/// "45m", "2h 10m", "3d 4h": how long until something, to the minute.
fn until(secs: i64) -> String {
    let m = (secs.max(0) + 59) / 60;
    match m {
        0..=59 => format!("{m}m"),
        60..=1439 if m % 60 == 0 => format!("{}h", m / 60),
        60..=1439 => format!("{}h {}m", m / 60, m % 60),
        _ if (m / 60) % 24 == 0 => format!("{}d", m / 1440),
        _ => format!("{}d {}h", m / 1440, (m / 60) % 24),
    }
}

/// `claude-opus-4-1-20250805` as "Opus 4.1", `claude-sonnet-4-5[1m]` as
/// "Sonnet 4.5 (1M)"; anything else as it came.
pub fn model_name(id: &str) -> String {
    let (id, long) = match id.strip_suffix("[1m]") {
        Some(base) => (base, true),
        None => (id, false),
    };
    let Some(rest) = id.strip_prefix("claude-") else {
        return id.to_string();
    };
    let mut parts = rest.split('-');
    let Some(family) = parts.next().filter(|f| !f.is_empty()) else {
        return id.to_string();
    };
    // A trailing date is a snapshot, not part of the name.
    let version: Vec<&str> = parts
        .filter(|p| !(p.len() == 8 && p.bytes().all(|b| b.is_ascii_digit())))
        .collect();
    let mut name = family[..1].to_uppercase() + &family[1..];
    if !version.is_empty() {
        name = format!("{name} {}", version.join("."));
    }
    if long {
        name.push_str(" (1M)");
    }
    name
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_read_like_the_product() {
        assert_eq!(model_name("claude-opus-4-1-20250805"), "Opus 4.1");
        assert_eq!(model_name("claude-haiku-5-5"), "Haiku 5.5");
        assert_eq!(model_name("claude-sonnet-4-5[1m]"), "Sonnet 4.5 (1M)");
        assert_eq!(model_name("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(model_name("claude-"), "claude-");
    }

    #[test]
    fn resets_read_to_the_minute() {
        assert_eq!(until(-5), "0m");
        assert_eq!(until(30), "1m");
        assert_eq!(until(45 * 60), "45m");
        assert_eq!(until(2 * 3600), "2h");
        assert_eq!(until(2 * 3600 + 10 * 60), "2h 10m");
        assert_eq!(until(3 * 86_400), "3d");
        assert_eq!(until(3 * 86_400 + 4 * 3600 + 1), "3d 4h");
    }

    #[test]
    fn small_figures_do_not_round_to_nothing() {
        assert_eq!(percent(0.004), "<1%");
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.376), "38%");
        assert_eq!(dollars(0.003), "<$0.01");
        assert_eq!(dollars(1.234), "$1.23");
    }

    fn thread(pane: PaneId, focused: bool, usage: Usage) -> Thread {
        Thread {
            pane,
            title: "t".into(),
            mark: Icon::Claude,
            agent: "Claude".into(),
            model: None,
            usage,
            focused,
            active: Instant::now(),
        }
    }

    #[test]
    fn the_panel_follows_the_focused_chat_and_the_freshest_plan() {
        let window = |used, resets_at| Allowance { used, resets_at };
        let old = Usage {
            limits: Some(Limits {
                session: Some(window(0.9, 50)),
                week: Some(window(0.4, 500)),
            }),
            ..Default::default()
        };
        let threads = [thread(1, false, Usage::default()), thread(2, true, old)];
        assert_eq!(current(&threads).map(|t| t.pane), Some(2));
        assert_eq!(current(&threads[..1]).map(|t| t.pane), Some(1));
        // The session window reset at 50; the week still runs.
        let l = plan(&threads, 100).expect("plan");
        assert_eq!(l.session, None);
        assert_eq!(l.week.map(|w| w.used), Some(0.4));
        assert!(plan(&threads, 1000).is_none());
    }
}
