//! Shortcuts as people read them: `cmd-shift-k` is shown as ⇧⌘K, with the
//! modifiers in the order macOS menus use (⌃⌥⇧⌘).

/// A keystroke in gpui's form, as symbols.
pub fn pretty(keys: &str) -> String {
    let Ok(k) = gpui::Keystroke::parse(keys) else {
        return keys.to_string();
    };
    let m = k.modifiers;
    let mut out = String::new();
    if m.control {
        out.push('⌃');
    }
    if m.alt {
        out.push('⌥');
    }
    if m.shift {
        out.push('⇧');
    }
    if m.platform {
        out.push('⌘');
    }
    if m.function {
        out.push_str("fn ");
    }
    out.push_str(&key_name(&k.key));
    out
}

fn key_name(key: &str) -> String {
    match key {
        "enter" => "↩".into(),
        "escape" => "⎋".into(),
        "tab" => "⇥".into(),
        "space" => "Space".into(),
        "backspace" => "⌫".into(),
        "delete" => "⌦".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "home" => "↖".into(),
        "end" => "↘".into(),
        "pageup" => "⇞".into(),
        "pagedown" => "⇟".into(),
        "-" => "−".into(),
        k if k.len() > 1 && k.starts_with('f') && k[1..].chars().all(|c| c.is_ascii_digit()) => {
            k.to_uppercase()
        }
        k => k.to_uppercase(),
    }
}

/// Whether a recorded key may be a shortcut: it needs ⌘, ⌃ or ⌥ (or is an
/// F-key), or it would take a plain key away from typing.
pub fn allowed(k: &gpui::Keystroke) -> bool {
    let m = k.modifiers;
    m.platform
        || m.control
        || m.alt
        || (k.key.len() > 1
            && k.key.starts_with('f')
            && k.key[1..].chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_like_menus() {
        assert_eq!(pretty("cmd-shift-k"), "⇧⌘K");
        assert_eq!(pretty("alt-cmd-i"), "⌥⌘I");
        assert_eq!(pretty("ctrl-tab"), "⌃⇥");
        assert_eq!(pretty("cmd--"), "⌘−");
        assert_eq!(pretty("cmd-,"), "⌘,");
        assert_eq!(pretty("f5"), "F5");
        assert!(allowed(&gpui::Keystroke::parse("cmd-j").unwrap()));
        assert!(allowed(&gpui::Keystroke::parse("f6").unwrap()));
        assert!(!allowed(&gpui::Keystroke::parse("shift-j").unwrap()));
        assert!(!allowed(&gpui::Keystroke::parse("j").unwrap()));
    }
}
