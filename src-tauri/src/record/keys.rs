//! Port von Capso `KeystrokeFormatter` (KeyCastr-Stil): Tastendruck → Anzeigetext für das Tasten-Overlay.
//! Die reine Logik (`format`) ist von der Tastaturbelegung entkoppelt und getestet; `translate_key_code`
//! übersetzt einen Tastencode über die aktuelle Belegung (TIS/UCKeyTranslate) wie im Original.

/// Modifikator-Flags wie `CGEventFlags` / `NSEvent.ModifierFlags`.
pub const SHIFT: u64 = 0x0002_0000;
pub const CONTROL: u64 = 0x0004_0000;
pub const OPTION: u64 = 0x0008_0000;
pub const COMMAND: u64 = 0x0010_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keystroke {
    pub key_code: u16,
    pub characters: String,
    pub characters_ignoring_modifiers: String,
    pub flags: u64,
}

impl Keystroke {
    /// ⌃ oder ⌘ gedrückt → „Befehl“ (beginnt eine neue Blase im Overlay).
    pub fn is_command(&self) -> bool {
        self.flags & (CONTROL | COMMAND) != 0
    }
    pub fn is_modified(&self) -> bool {
        self.flags & (CONTROL | COMMAND | OPTION | SHIFT) != 0
    }
}

const KVK_TAB: u16 = 48;

/// Reine Modifikatortasten erzeugen keine Anzeige.
pub fn is_modifier_key_code(code: u16) -> bool {
    matches!(code, 56 | 60 | 55 | 54 | 58 | 61 | 59 | 62 | 63 | 57)
}

fn special_key(code: u16) -> Option<&'static str> {
    Some(match code {
        126 => "↑",
        125 => "↓",
        124 => "→",
        123 => "←",
        48 => "⇥",
        53 => "⎋",
        71 => "⌧",
        51 => "⌫",
        117 => "⌦",
        115 => "↖",
        119 => "↘",
        116 => "⇞",
        121 => "⇟",
        36 | 76 => "↩",
        49 => "␣",
        122 => "F1",
        120 => "F2",
        99 => "F3",
        118 => "F4",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        100 => "F8",
        101 => "F9",
        109 => "F10",
        103 => "F11",
        111 => "F12",
        105 => "F13",
        107 => "F14",
        113 => "F15",
        106 => "F16",
        64 => "F17",
        79 => "F18",
        80 => "F19",
        90 => "F20",
        0x66 => "英数",
        0x68 => "かな",
        _ => return None,
    })
}

/// Anzeigetext für einen Tastendruck (None für reine Modifikatoren / Wiederholungen).
/// `translate`: Tastencode → Zeichen der aktuellen Belegung ohne Modifikatoren ("" falls unbekannt).
pub fn format(k: &Keystroke, is_repeat: bool, translate: impl Fn(u16) -> String) -> Option<String> {
    if is_repeat || is_modifier_key_code(k.key_code) {
        return None;
    }
    let f = k.flags;
    let (opt, shift, cmd) = (f & OPTION != 0, f & SHIFT != 0, k.is_command());
    let mut r = String::new();
    if f & CONTROL != 0 {
        r += "⌃";
    }
    if opt {
        r += "⌥";
    }
    if shift {
        r += "⇧";
    }
    if f & COMMAND != 0 {
        r += "⌘";
    }
    if shift && !cmd && !opt && k.key_code == KVK_TAB {
        return Some("⇧⇤".into());
    }
    if let Some(s) = special_key(k.key_code) {
        return Some(r + s);
    }
    let mut body = translate(k.key_code);
    if body.is_empty() {
        body = k.characters_ignoring_modifiers.clone();
    }
    if body.is_empty() {
        body = k.characters.clone();
    }
    if body.is_empty() {
        body = "?".into();
    }
    r += &body;
    if (cmd || shift) && k.key_code != 27 {
        r = r.to_uppercase();
    }
    Some(r)
}

// ───────────── Tastaturbelegung (Carbon TIS / UCKeyTranslate) ─────────────

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut std::ffi::c_void;
    fn TISGetInputSourceProperty(src: *mut std::ffi::c_void, key: *const std::ffi::c_void) -> *const std::ffi::c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const std::ffi::c_void;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const u8,
        key_code: u16,
        action: u16,
        modifier_state: u32,
        kbd_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_len: usize,
        actual_len: *mut usize,
        chars: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataGetBytePtr(d: *const std::ffi::c_void) -> *const u8;
    fn CFRelease(p: *const std::ffi::c_void);
}

/// Zeichen der Taste in der aktuellen Belegung (ohne Modifikatoren), "" falls nicht ermittelbar.
pub fn translate_key_code(code: u16) -> String {
    unsafe {
        let src = TISCopyCurrentKeyboardLayoutInputSource();
        if src.is_null() {
            return String::new();
        }
        let data = TISGetInputSourceProperty(src, kTISPropertyUnicodeKeyLayoutData);
        let mut out = String::new();
        if !data.is_null() {
            let layout = CFDataGetBytePtr(data);
            let (mut dead, mut len) = (0u32, 0usize);
            let mut chars = [0u16; 4];
            // kUCKeyActionDisplay = 3, kUCKeyTranslateNoDeadKeysBit = 0
            let st = UCKeyTranslate(layout, code, 3, 0, LMGetKbdType() as u32, 1, &mut dead, 4, &mut len, chars.as_mut_ptr());
            if st == 0 && len > 0 {
                out = String::from_utf16_lossy(&chars[..len]);
            }
        }
        CFRelease(src);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(code: u16, ch: &str, flags: u64) -> Keystroke {
        Keystroke { key_code: code, characters: ch.into(), characters_ignoring_modifiers: ch.into(), flags }
    }
    fn none(_: u16) -> String {
        String::new()
    }

    #[test]
    fn modifier_only_is_ignored() {
        assert_eq!(format(&ks(56, "", SHIFT), false, none), None);
        assert_eq!(format(&ks(55, "", COMMAND), false, none), None);
    }

    #[test]
    fn command_c_is_uppercased() {
        let l = format(&ks(8, "c", COMMAND), false, |_| "c".into()).unwrap();
        assert!(l.contains('⌘'));
        assert_eq!(l, "⌘C");
    }

    #[test]
    fn space_and_escape_glyphs() {
        assert_eq!(format(&ks(49, " ", 0), false, none).as_deref(), Some("␣"));
        assert_eq!(format(&ks(53, "\u{1b}", 0), false, none).as_deref(), Some("⎋"));
    }

    #[test]
    fn repeats_are_dropped() {
        assert_eq!(format(&ks(0, "a", 0), true, none), None);
    }

    #[test]
    fn shift_tab_and_modifier_order() {
        assert_eq!(format(&ks(48, "\t", SHIFT), false, none).as_deref(), Some("⇧⇤"));
        assert_eq!(format(&ks(48, "\t", 0), false, none).as_deref(), Some("⇥"));
        let all = CONTROL | OPTION | SHIFT | COMMAND;
        assert_eq!(format(&ks(126, "", all), false, none).as_deref(), Some("⌃⌥⇧⌘↑"));
    }

    #[test]
    fn plain_letter_falls_back_to_characters() {
        assert_eq!(format(&ks(0, "a", 0), false, none).as_deref(), Some("a"));
        assert_eq!(format(&ks(0, "", 0), false, none).as_deref(), Some("?"));
        assert_eq!(format(&ks(27, "-", SHIFT), false, |_| "-".into()).as_deref(), Some("⇧-"));
    }

    #[test]
    fn command_detection() {
        assert!(ks(8, "c", COMMAND).is_command());
        assert!(ks(8, "c", CONTROL).is_command());
        assert!(!ks(8, "c", 0).is_command());
        assert!(ks(8, "c", OPTION).is_modified());
    }
}
