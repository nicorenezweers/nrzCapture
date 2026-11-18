//! Reine Logik für Tastenkürzel: Tastendruck → Tauri-Accelerator ("Alt+Shift+1"), Normalisierung, Konflikte.

/// Ergebnis eines Tastendrucks im Aufnahmefeld.
#[derive(Clone, Debug, PartialEq)]
pub enum Recorded {
    /// Gültige Kombination.
    Accel(String),
    /// Esc ohne Modifikator: Aufnahme abbrechen.
    Cancel,
    /// Backspace/Entf ohne Modifikator: Kürzel löschen.
    Clear,
    /// Nur Modifikatoren oder unbekannte Taste – weiter warten.
    Pending,
    /// Taste braucht einen Modifikator (außer F-Tasten).
    NeedsModifier,
}

/// `KeyboardEvent.code` → Tasten-Name im Accelerator-Format von tauri-plugin-global-shortcut.
pub fn key_name(code: &str) -> Option<String> {
    if let Some(c) = code.strip_prefix("Key") {
        return (c.len() == 1).then(|| c.to_owned());
    }
    if let Some(d) = code.strip_prefix("Digit") {
        return (d.len() == 1).then(|| d.to_owned());
    }
    if code.starts_with('F') && code[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) {
        return Some(code.to_owned());
    }
    if let Some(n) = code.strip_prefix("Numpad") {
        return Some(format!("Numpad{n}"));
    }
    let k = match code {
        "Space" => "Space",
        "Enter" => "Enter",
        "Tab" => "Tab",
        "Escape" => "Escape",
        "Backspace" => "Backspace",
        "Delete" => "Delete",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "PageUp",
        "PageDown" => "PageDown",
        "Minus" => "Minus",
        "Equal" => "Equal",
        "BracketLeft" => "BracketLeft",
        "BracketRight" => "BracketRight",
        "Backslash" => "Backslash",
        "Semicolon" => "Semicolon",
        "Quote" => "Quote",
        "Comma" => "Comma",
        "Period" => "Period",
        "Slash" => "Slash",
        "Backquote" => "Backquote",
        "IntlBackslash" => "IntlBackslash",
        _ => return None,
    };
    Some(k.to_owned())
}

/// Tastendruck auswerten (Modifikatoren in macOS-Reihenfolge ⌃⌥⇧⌘).
pub fn record(code: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Recorded {
    let any_mod = ctrl || alt || shift || meta;
    if code.starts_with("Shift") || code.starts_with("Control") || code.starts_with("Alt") || code.starts_with("Meta") || code.starts_with("OS") || code == "CapsLock" || code == "Fn" {
        return Recorded::Pending;
    }
    if !any_mod {
        match code {
            "Escape" => return Recorded::Cancel,
            "Backspace" | "Delete" => return Recorded::Clear,
            _ => {}
        }
    }
    let Some(key) = key_name(code) else { return Recorded::Pending };
    let fkey = key.starts_with('F') && key.len() > 1 && key[1..].parse::<u8>().is_ok();
    // Wie Capso/KeyboardShortcuts: ohne Modifikator nur F-Tasten; nur ⇧ reicht nicht (würde Tippen blockieren).
    if !fkey && (!any_mod || (shift && !ctrl && !alt && !meta)) {
        return Recorded::NeedsModifier;
    }
    let mut parts: Vec<&str> = vec![];
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    if meta {
        parts.push("Cmd");
    }
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(&key);
    Recorded::Accel(s)
}

/// Accelerator normalisieren (Reihenfolge, Groß-/Kleinschreibung, Synonyme) für Vergleiche.
pub fn normalize(accel: &str) -> String {
    let mut mods: Vec<&str> = vec![];
    let mut key = String::new();
    for p in accel.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match p.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "meta" | "cmdorctrl" | "commandorcontrol" => mods.push("Cmd"),
            "ctrl" | "control" => mods.push("Ctrl"),
            "alt" | "option" => mods.push("Alt"),
            "shift" => mods.push("Shift"),
            k => key = k.trim_start_matches("key").trim_start_matches("digit").to_ascii_uppercase(),
        }
    }
    let mut out: Vec<String> = ["Ctrl", "Alt", "Shift", "Cmd"].iter().filter(|m| mods.contains(m)).map(|m| m.to_string()).collect();
    out.push(key);
    out.join("+")
}

/// Andere Aktion mit demselben Kürzel (oder "macOS" bei System-Kürzeln). `None` = frei.
pub fn conflict<'a>(shortcuts: impl IntoIterator<Item = (&'a str, &'a str)>, action: &str, accel: &str, replace_system: bool) -> Option<String> {
    let n = normalize(accel);
    for (a, v) in shortcuts {
        if a != action && !v.trim().is_empty() && normalize(v) == n {
            return Some(a.to_owned());
        }
    }
    if ["Shift+Cmd+3", "Shift+Cmd+4", "Shift+Cmd+5"].contains(&n.as_str()) {
        return Some(if replace_system { "replaceSystemShortcuts".into() } else { "macOS".into() });
    }
    ["Cmd+Q", "Cmd+W", "Cmd+TAB", "Cmd+SPACE", "Ctrl+SPACE", "Cmd+H", "Cmd+M"].contains(&n.as_str()).then(|| "macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_combinations() {
        assert_eq!(record("Digit1", false, true, true, false), Recorded::Accel("Alt+Shift+1".into()));
        assert_eq!(record("KeyT", true, false, false, true), Recorded::Accel("Ctrl+Cmd+T".into()));
        assert_eq!(record("ArrowUp", false, false, true, true), Recorded::Accel("Shift+Cmd+Up".into()));
        assert_eq!(record("F5", false, false, false, false), Recorded::Accel("F5".into()));
        assert_eq!(record("Escape", false, false, false, false), Recorded::Cancel);
        assert_eq!(record("Backspace", false, false, false, false), Recorded::Clear);
        assert_eq!(record("Delete", false, false, false, false), Recorded::Clear);
        assert_eq!(record("Backspace", false, true, false, false), Recorded::Accel("Alt+Backspace".into()));
        assert_eq!(record("ShiftLeft", false, false, true, false), Recorded::Pending);
        assert_eq!(record("MetaRight", false, false, false, true), Recorded::Pending);
        assert_eq!(record("KeyA", false, false, false, false), Recorded::NeedsModifier);
        assert_eq!(record("KeyA", false, false, true, false), Recorded::NeedsModifier);
        assert_eq!(record("Unidentified", true, false, false, false), Recorded::Pending);
    }

    #[test]
    fn normalizes_and_detects_conflicts() {
        assert_eq!(normalize("shift+alt+1"), "Alt+Shift+1");
        assert_eq!(normalize("Command+Shift+KeyA"), "Shift+Cmd+A");
        let list = [("captureArea", "Alt+Shift+1"), ("captureWindow", "")];
        assert_eq!(conflict(list, "captureText", "Shift+Alt+1", false), Some("captureArea".into()));
        assert_eq!(conflict(list, "captureArea", "Alt+Shift+1", false), None);
        assert_eq!(conflict(list, "captureText", "Cmd+Shift+3", false), Some("macOS".into()));
        assert_eq!(conflict(list, "captureText", "Cmd+Shift+3", true), Some("replaceSystemShortcuts".into()));
        assert_eq!(conflict(list, "captureText", "Ctrl+Alt+K", false), None);
    }
}
