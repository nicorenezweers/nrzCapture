//! Bilder/Text/Farben/Dateien als schwebende, immer-oben Fenster anheften.
//! Port von Capso `PinnedScreenshot*` + `ClipboardPinContentReader`.
//!
//! Jeder Pin ist ein eigenes Fenster `pin-<id>` (Route `pin`). Größe ändern hält das Seitenverhältnis;
//! Sperren schaltet Klick-durch (`set_ignore_cursor_events`) – nur der Schloss-Knopf bleibt klickbar.
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow,
};

use crate::core::{self, t, Win};

/// Capso: Breite beim Zoomen zwischen 160 und 1400 pt; Mindestgröße 100 pt.
const MIN_W: f64 = 160.0;
const MAX_W: f64 = 1400.0;
const MAX_LONG_SIDE: f64 = 800.0;
/// Klickbarer Bereich des Schloss-Knopfs oben rechts im gesperrten Zustand.
const LOCK_ZONE: f64 = 48.0;
const MAX_TEXT: usize = 100_000;

#[derive(Clone, Debug)]
struct Pin {
    /// Breite / Höhe.
    aspect: f64,
    /// Ausgangsbreite (100 %).
    base_w: f64,
    locked: bool,
    /// Inhalt fürs Kopieren/Sichern.
    data: Value,
}

static PINS: Mutex<Option<HashMap<String, Pin>>> = Mutex::new(None);
static HIDDEN: AtomicBool = AtomicBool::new(false);
static POLLING: AtomicBool = AtomicBool::new(false);

fn with_pins<R>(f: impl FnOnce(&mut HashMap<String, Pin>) -> R) -> R {
    f(PINS.lock().unwrap().get_or_insert_with(HashMap::new))
}

/// Sichtbarer Bereich (logisch) und Skalierung des Bildschirms unter dem Rechteck bzw. Mauszeiger.
fn screen_for(app: &AppHandle, rect: Option<[f64; 4]>) -> ([f64; 4], f64) {
    let screens = core::screens(app);
    let hit = rect.and_then(|[x, y, w, h]| {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        screens.iter().find(|s| cx >= s.x && cx < s.x + s.width && cy >= s.y && cy < s.y + s.height)
    });
    if let Some(s) = hit {
        return (s.work, s.scale);
    }
    match core::cursor_monitor(app) {
        Some(m) => {
            let (s, wa) = (m.scale_factor(), m.work_area());
            ([wa.position.x as f64 / s, wa.position.y as f64 / s, wa.size.width as f64 / s, wa.size.height as f64 / s], s)
        }
        None => ([0.0, 25.0, 1440.0, 875.0], 2.0),
    }
}

/// Startgröße eines Bild-Pins wie Capso: Bildpixel als Punkte, längste Seite ≤ 800, je Seite mindestens 100.
pub fn initial_size(px: (f64, f64)) -> (f64, f64) {
    let (w, h) = px;
    if w <= 0.0 || h <= 0.0 {
        return (320.0, 200.0);
    }
    let s = (MAX_LONG_SIDE / w.max(h)).min(1.0);
    ((w * s).max(100.0).round(), (h * s).max(100.0).round())
}

/// Zentriert im sichtbaren Bereich, mit 16 pt Rand.
pub fn centered(visible: [f64; 4], w: f64, h: f64) -> (f64, f64) {
    let [vx, vy, vw, vh] = visible;
    let x = (vx + vw / 2.0 - w / 2.0).max(vx + 16.0).min(vx + vw - w - 16.0).max(vx);
    let y = (vy + vh / 2.0 - h / 2.0).max(vy + 16.0).min(vy + vh - h - 16.0).max(vy);
    (x.round(), y.round())
}

/// Neue Breite beim Zoomen/Ziehen (Seitenverhältnis bleibt). Große Originale dürfen über 1400 pt bleiben.
pub fn clamp_width(w: f64, base_w: f64, aspect: f64) -> f64 {
    let min = MIN_W.max(if aspect < 1.0 { 100.0 * aspect } else { 100.0 }).min(base_w.max(100.0));
    w.clamp(min, MAX_W.max(base_w))
}

fn open_pin(app: &AppHandle, data: Value, size: (f64, f64), pos: (f64, f64)) {
    if HIDDEN.swap(false, Ordering::SeqCst) {
        set_all_visible(app, true);
    }
    let label = format!("pin-{}", core::unique_id());
    let (w, h) = size;
    with_pins(|p| p.insert(label.clone(), Pin { aspect: w / h, base_w: w, locked: false, data: data.clone() }));
    let o = Win { title: "nrzCapture Pin".into(), w, h, pos: Some(pos), shadow: true, focus: true, level: Some(core::LEVEL_FLOATING), ..Default::default() };
    let Some(win) = core::window(app, &label, "pin", o, data) else {
        with_pins(|p| p.remove(&label));
        return;
    };
    let (app2, l2) = (app.clone(), label.clone());
    win.on_menu_event(move |w, e| {
        if let Some(id) = e.id().as_ref().strip_prefix("pin:") {
            let (app, l, w, id) = (app2.clone(), l2.clone(), w.clone(), id.to_owned());
            std::thread::spawn(move || {
                if let Some(ww) = app.get_webview_window(&l) {
                    let _ = menu_action(&app, &ww, &id);
                }
                let _ = w;
            });
        }
    });
    let l3 = label.clone();
    let app3 = app.clone();
    win.on_window_event(move |e| {
        if matches!(e, tauri::WindowEvent::Destroyed) {
            with_pins(|p| p.remove(&l3));
            if with_pins(|p| p.is_empty()) && HIDDEN.swap(false, Ordering::SeqCst) {
                crate::actions::refresh_menu(&app3);
            }
        }
    });
    crate::actions::refresh_menu(app);
}

/// Bild als Pin öffnen – wie Capso `PinnedScreenshotWindow`: immer zentriert im sichtbaren Bereich des Anker-Bildschirms
/// (Bildschirm unter der Mitte von `rect`, sonst unter dem Mauszeiger), lange Seite auf 800 begrenzt.
/// `rect` (global logisch [x,y,w,h], z. B. aufgenommener Bereich) wählt nur den Bildschirm.
pub fn pin_image(app: &AppHandle, path: &Path, rect: Option<[f64; 4]>) {
    let (visible, _) = screen_for(app, rect);
    let kind = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif")) { "gif" } else { "image" };
    let data = json!({"kind": kind, "path": path});
    let px = core::image_size(path).map(|(w, h)| (w as f64, h as f64)).unwrap_or((640.0, 400.0));
    let (w, h) = initial_size(px);
    open_pin(app, data, (w, h), centered(visible, w, h));
}

// ───────────────────────────── Zwischenablage ─────────────────────────────

/// Farbe aus Text: `#RRGGBB`, `rgb(r, g, b)` oder `r, g, b` (0–255 bzw. 0–1). Port von Capso `clipboardColor`.
pub fn parse_color(text: &str) -> Option<(u8, u8, u8)> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        return Some(((v >> 16) as u8, (v >> 8) as u8, v as u8));
    }
    let lower = text.to_lowercase();
    let inner = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')).unwrap_or(&lower);
    let parts: Vec<&str> = inner.split(',').collect();
    let vals: Vec<f64> = parts.iter().filter_map(|s| s.trim().parse::<f64>().ok()).collect();
    if parts.len() != 3 || vals.len() != 3 || !vals.iter().all(|v| (0.0..=255.0).contains(v)) {
        return None;
    }
    let div = if vals.iter().all(|v| *v <= 1.0) { 1.0 } else { 255.0 };
    let c = |v: f64| ((v / div) * 255.0).round().clamp(0.0, 255.0) as u8;
    Some((c(vals[0]), c(vals[1]), c(vals[2])))
}

/// Hell genug für schwarze Schrift? (Capso: HSB-Helligkeit > 0.58)
pub fn is_light(rgb: (u8, u8, u8)) -> bool {
    rgb.0.max(rgb.1).max(rgb.2) as f64 / 255.0 > 0.58
}

/// HTML → reiner Text (Skripte/Styles raus, Block-Enden → Zeilenumbruch, Entities dekodieren, Leerzeilen weg).
pub fn html_to_text(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut out = String::new();
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            let Some(end) = html[i..].find('>').map(|e| i + e) else { break };
            let tag = lower[i + 1..end].trim().to_owned();
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if !tag.starts_with('/') && (name == "script" || name == "style") {
                let close = format!("</{name}");
                i = lower[end..].find(&close).and_then(|c| lower[end + c..].find('>').map(|g| end + c + g + 1)).unwrap_or(b.len());
                continue;
            }
            let block = name == "br" || (tag.starts_with('/') && (["p", "div", "li", "tr"].contains(&name.as_str()) || (name.len() == 2 && name.starts_with('h'))));
            if block {
                out.push('\n');
            }
            i = end + 1;
        } else {
            let next = html[i..].find('<').map(|n| i + n).unwrap_or(b.len());
            out.push_str(&decode_entities(&html[i..next]));
            i = next;
        }
    }
    let lines: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let semi = rest[1..].char_indices().take(12).find(|(_, c)| *c == ';' || *c == '&').filter(|(_, c)| *c == ';').map(|(i, _)| i + 1);
        let decoded = semi.and_then(|e| {
            let ent = rest[1..e].to_lowercase();
            let ch = match ent.as_str() {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
                e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            ch.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Geschätzte Kartengröße für Text (Monospace 18 pt, Ränder 24/26), Port von `renderCard`.
pub fn text_card_size(text: &str, visible_w: f64) -> (f64, f64) {
    const CW: f64 = 10.85;
    const LH: f64 = 23.0;
    let max_content = (visible_w * 0.72).clamp(320.0, 760.0);
    let per_line = (max_content / CW).floor().max(1.0) as usize;
    let mut longest = 0usize;
    let mut lines = 0usize;
    for l in text.lines() {
        let n = l.chars().count();
        longest = longest.max(n.min(per_line));
        lines += n.div_ceil(per_line).max(1);
    }
    let w = ((longest as f64 * CW).ceil() + 52.0).clamp(320.0, max_content + 52.0);
    let h = (lines.max(1) as f64 * LH + 48.0).clamp(120.0, 8000.0);
    (w.ceil(), h.ceil())
}

mod pb {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSPasteboard, NSWorkspace};
    use objc2_foundation::NSString;
    use std::path::{Path, PathBuf};

    fn board() -> Retained<NSPasteboard> {
        NSPasteboard::generalPasteboard()
    }

    pub fn string(ty: &str) -> Option<String> {
        board().stringForType(&NSString::from_str(ty)).map(|s| s.to_string())
    }

    pub fn has(ty: &str) -> bool {
        board().dataForType(&NSString::from_str(ty)).is_some()
    }

    pub fn data_len(ty: &str) -> usize {
        board().dataForType(&NSString::from_str(ty)).map(|d| d.len()).unwrap_or(0)
    }

    /// Alle kopierten Datei-URLs (Finder-Mehrfachauswahl).
    pub fn files() -> Vec<PathBuf> {
        let ty = NSString::from_str("public.file-url");
        let Some(items) = board().pasteboardItems() else { return vec![] };
        items
            .iter()
            .filter_map(|it| it.stringForType(&ty))
            .filter_map(|s| s.to_string().strip_prefix("file://").map(super::url_decode))
            .map(|p| PathBuf::from(p.trim_end_matches('/').to_owned()))
            .filter(|p| !p.as_os_str().is_empty())
            .collect()
    }

    /// Finder-Symbol einer Datei als PNG im Temp-Ordner.
    pub fn icon_png(path: &Path) -> Option<PathBuf> {
        let img = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
        let tiff = img.TIFFRepresentation()?;
        let t = crate::core::temp_file("tiff");
        std::fs::write(&t, tiff.to_vec()).ok()?;
        let p = t.with_extension("png");
        crate::core::sh("sips", &["-s", "format", "png", "-Z", "64", &t.to_string_lossy(), "--out", &p.to_string_lossy()]);
        let _ = std::fs::remove_file(&t);
        p.exists().then_some(p)
    }
}

pub fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const IMAGE_EXT: [&str; 10] = ["png", "jpg", "jpeg", "heic", "heif", "tiff", "tif", "gif", "webp", "bmp"];

fn is_image_file(p: &Path) -> bool {
    p.is_file() && p.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGE_EXT.contains(&e.to_lowercase().as_str()))
}

fn pin_text(app: &AppHandle, text: &str) {
    if text.chars().count() > MAX_TEXT {
        core::toast(app, t("Inhalt der Zwischenablage ist zu groß zum Anheften", "Clipboard content is too large to pin"));
        return;
    }
    let (visible, _) = screen_for(app, None);
    let (w, h) = text_card_size(text, visible[2]);
    let h = h.min(visible[3] * 0.85).max(120.0);
    open_pin(app, json!({"kind": "text", "text": text}), (w, h), centered(visible, w, h));
}

/// Inhalt der Zwischenablage (Bild, Text, Farbe, Datei) anheften. Reihenfolge wie Capso `ClipboardPinContentReader`.
pub fn pin_clipboard(app: &AppHandle) {
    let too_large = || core::toast(app, t("Inhalt der Zwischenablage ist zu groß zum Anheften", "Clipboard content is too large to pin"));
    if ["public.rtf", "com.apple.flat-rtfd", "public.html"].iter().any(|t| pb::data_len(t) > 4 * 1024 * 1024) {
        return too_large();
    }
    // 1. Dateien: einzelne Bilddatei als Bild, sonst Liste mit Symbol + Name.
    let files = pb::files();
    if files.len() == 1 && is_image_file(&files[0]) {
        return pin_image(app, &files[0], None);
    }
    if !files.is_empty() {
        let list: Vec<Value> = files
            .iter()
            .take(100)
            .map(|f| {
                json!({
                    "name": f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.to_string_lossy().into_owned()),
                    "path": f,
                    "icon": pb::icon_png(f),
                })
            })
            .collect();
        let more = files.len().saturating_sub(100);
        let (visible, _) = screen_for(app, None);
        let (w, h) = (420.0, (list.len() as f64 * 52.0 + 32.0 + if more > 0 { 28.0 } else { 0.0 }).clamp(100.0, visible[3] * 0.8));
        return open_pin(app, json!({"kind": "files", "files": list, "more": more}), (w, h), centered(visible, w, h));
    }
    let plain = pb::string("public.utf8-plain-text").map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    // 2. Formatierter Text (RTF) → als Text.
    if pb::has("public.rtf") || pb::has("com.apple.flat-rtfd") {
        if let Some(t) = &plain {
            return pin_text(app, t);
        }
    }
    // 3. Bilddaten.
    if let Some(img) = core::clipboard::read_image() {
        return pin_image(app, &img, None);
    }
    // 4. HTML ohne Klartext.
    if plain.is_none() {
        if let Some(t) = pb::string("public.html").and_then(|h| html_to_text(&h)) {
            return pin_text(app, &t);
        }
    }
    let Some(text) = plain else {
        core::toast(app, t("Nichts zum Anheften in der Zwischenablage", "Nothing to pin on the clipboard"));
        return;
    };
    // 5. Pfad zu einer Bilddatei.
    if text.starts_with('/') && is_image_file(Path::new(&text)) {
        return pin_image(app, Path::new(&text), None);
    }
    // 6. Farbe → Farbfeld.
    if let Some(rgb) = parse_color(&text) {
        let (visible, _) = screen_for(app, None);
        let hex = format!("#{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2);
        return open_pin(
            app,
            json!({"kind": "color", "text": text, "hex": hex, "light": is_light(rgb)}),
            (360.0, 180.0),
            centered(visible, 360.0, 180.0),
        );
    }
    // 7. Text.
    pin_text(app, &text);
}

// ───────────────────────────── Alle Pins ─────────────────────────────

fn pin_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows().into_iter().filter(|(l, _)| l.starts_with("pin-")).map(|(_, w)| w).collect()
}

fn set_all_visible(app: &AppHandle, visible: bool) {
    for w in pin_windows(app) {
        let _ = if visible { w.show() } else { w.hide() };
    }
}

/// Alle Pins aus-/einblenden.
pub fn toggle_all(app: &AppHandle) {
    if pin_windows(app).is_empty() {
        HIDDEN.store(false, Ordering::SeqCst);
    } else {
        let hide = !HIDDEN.load(Ordering::SeqCst);
        HIDDEN.store(hide, Ordering::SeqCst);
        set_all_visible(app, !hide);
    }
    crate::actions::refresh_menu(app);
}

pub fn all_hidden() -> bool {
    HIDDEN.load(Ordering::SeqCst)
}

/// Anzahl offener Pins.
pub fn count() -> usize {
    with_pins(|p| p.len())
}

/// Alle gesperrten Pins entsperren.
pub fn unlock_all(app: &AppHandle) {
    for w in pin_windows(app) {
        set_locked(app, &w, false);
    }
}

/// Alle Pins schließen.
pub fn close_all(app: &AppHandle) {
    for w in pin_windows(app) {
        let _ = w.close();
    }
}

// ───────────────────────────── Einzelner Pin ─────────────────────────────

fn set_locked(app: &AppHandle, w: &WebviewWindow, locked: bool) {
    with_pins(|p| {
        if let Some(pin) = p.get_mut(w.label()) {
            pin.locked = locked;
        }
    });
    let _ = w.set_ignore_cursor_events(locked);
    let _ = w.emit_to(w.label(), "pin-locked", locked);
    if locked && !POLLING.swap(true, Ordering::SeqCst) {
        let app = app.clone();
        std::thread::spawn(move || lock_poll(app));
    }
}

/// Gesperrte Pins: Klick-durch, beim Überfahren fast unsichtbar (Capso 0.08), nur der Schloss-Knopf reagiert.
fn lock_poll(app: AppHandle) {
    let mut state: HashMap<String, (bool, bool)> = HashMap::new();
    loop {
        let locked: Vec<String> = with_pins(|p| p.iter().filter(|(_, v)| v.locked).map(|(k, _)| k.clone()).collect());
        if locked.is_empty() {
            POLLING.store(false, Ordering::SeqCst);
            return;
        }
        let cur = app.cursor_position().ok();
        for l in &locked {
            let Some(w) = app.get_webview_window(l) else { continue };
            let (Ok(pos), Ok(size), Ok(scale)) = (w.outer_position(), w.outer_size(), w.scale_factor()) else { continue };
            let (inside, on_button) = match cur {
                Some(c) => {
                    let (x, y) = (c.x - pos.x as f64, c.y - pos.y as f64);
                    let inside = x >= 0.0 && y >= 0.0 && x < size.width as f64 && y < size.height as f64;
                    (inside, inside && x >= size.width as f64 - LOCK_ZONE * scale && y < LOCK_ZONE * scale)
                }
                None => (false, false),
            };
            if state.get(l) != Some(&(inside, on_button)) {
                state.insert(l.clone(), (inside, on_button));
                let _ = w.set_ignore_cursor_events(!on_button);
                let _ = w.emit_to(l.as_str(), "pin-hover", json!({"inside": inside, "button": on_button}));
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Größe ändern: `anchor_center` = um die Mitte (Zoom), sonst oben links fest (Griff).
fn resize(w: &WebviewWindow, width: f64, anchor_center: bool) -> Result<Value, String> {
    let Some(pin) = with_pins(|p| p.get(w.label()).cloned()) else { return Err("kein Pin".into()) };
    let scale = w.scale_factor().map_err(|e| e.to_string())?;
    let pos = w.outer_position().map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    let size = w.inner_size().map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    let nw = clamp_width(width, pin.base_w, pin.aspect).round();
    let nh = (nw / pin.aspect).round();
    let _ = w.set_size(LogicalSize::new(nw, nh));
    if anchor_center {
        let _ = w.set_position(LogicalPosition::new(pos.x + (size.width - nw) / 2.0, pos.y + (size.height - nh) / 2.0));
    }
    Ok(json!({"percent": (nw / pin.base_w * 100.0).round(), "w": nw, "h": nh}))
}

fn pin_data(w: &WebviewWindow) -> Value {
    with_pins(|p| p.get(w.label()).map(|p| p.data.clone())).unwrap_or(Value::Null)
}

fn copy(app: &AppHandle, w: &WebviewWindow) {
    let d = pin_data(w);
    let ok = match d["kind"].as_str() {
        Some("image") => core::clipboard::copy_image(Path::new(d["path"].as_str().unwrap_or(""))),
        Some("gif") => core::clipboard::copy_file(Path::new(d["path"].as_str().unwrap_or(""))),
        Some("color") => core::clipboard::copy_text(d["hex"].as_str().unwrap_or("")),
        Some("text") => core::clipboard::copy_text(d["text"].as_str().unwrap_or("")),
        Some("files") => d["files"][0]["path"].as_str().is_some_and(|p| core::clipboard::copy_file(Path::new(p))),
        _ => false,
    };
    if ok {
        core::toast(app, t("In die Zwischenablage kopiert", "Copied to clipboard"));
    }
}

fn save(app: &AppHandle, w: &WebviewWindow) -> Result<Value, String> {
    let d = pin_data(w);
    let s = crate::settings::get();
    let dst = match d["kind"].as_str() {
        Some("image") | Some("gif") => {
            let src = PathBuf::from(d["path"].as_str().unwrap_or(""));
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
            let dst = core::unique_path(&core::export_dir(), &core::file_name(&s.screenshot_filename_template, "", &ext));
            std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
            dst
        }
        Some("text") | Some("color") => {
            let dst = core::unique_path(&core::export_dir(), &core::file_name("nrzCapture Pin {timestamp}", "", "txt"));
            std::fs::write(&dst, d["text"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
            dst
        }
        _ => return Ok(Value::Null),
    };
    core::toast(app, t("Gesichert", "Saved"));
    Ok(json!(dst))
}

fn annotate(app: &AppHandle, w: &WebviewWindow) {
    let d = pin_data(w);
    if d["kind"] == "image" {
        crate::editor::open(app, Path::new(d["path"].as_str().unwrap_or("")));
    }
}

fn menu_action(app: &AppHandle, w: &WebviewWindow, id: &str) -> Result<Value, String> {
    match id {
        "copy" => copy(app, w),
        "save" => return save(app, w),
        "annotate" => annotate(app, w),
        "lock" => set_locked(app, w, true),
        "close" => {
            let _ = w.close();
        }
        o if o.starts_with("opacity-") => {
            let v: f64 = o[8..].parse().unwrap_or(100.0);
            let _ = w.emit_to(w.label(), "pin-opacity", v / 100.0);
        }
        _ => {}
    }
    Ok(Value::Null)
}

/// Kontextmenü wie Capso (Kopieren, Sichern, Schließen, Deckkraft) + Annotieren/Sperren.
fn context_menu(app: &AppHandle, w: &WebviewWindow) -> Result<(), String> {
    let e = |e: tauri::Error| e.to_string();
    let kind = pin_data(w)["kind"].as_str().unwrap_or("").to_owned();
    let item = |id: &str, text: &str, enabled: bool| MenuItem::with_id(app, format!("pin:{id}"), text, enabled, None::<&str>);
    let opacity = Submenu::with_items(
        app,
        t("Deckkraft", "Opacity"),
        true,
        &[&item("opacity-100", "100%", true).map_err(e)?, &item("opacity-75", "75%", true).map_err(e)?, &item("opacity-50", "50%", true).map_err(e)?, &item("opacity-25", "25%", true).map_err(e)?],
    )
    .map_err(e)?;
    let menu = Menu::with_items(
        app,
        &[
            &item("copy", t("Kopieren", "Copy"), true).map_err(e)?,
            &item("save", t("Sichern", "Save"), kind != "files").map_err(e)?,
            &item("annotate", t("Annotieren", "Annotate"), kind == "image").map_err(e)?,
            &PredefinedMenuItem::separator(app).map_err(e)?,
            &opacity,
            &item("lock", t("Sperren (Klicks durchlassen)", "Lock (Click Through)"), true).map_err(e)?,
            &PredefinedMenuItem::separator(app).map_err(e)?,
            &item("close", t("Schließen", "Close"), true).map_err(e)?,
        ],
    )
    .map_err(e)?;
    w.popup_menu(&menu).map_err(e)
}

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    match cmd {
        "info" => {
            let p = with_pins(|p| p.get(win.label()).cloned());
            Ok(p.map(|p| json!({"aspect": p.aspect, "baseW": p.base_w, "locked": p.locked, "data": p.data})).unwrap_or(Value::Null))
        }
        "startDrag" => {
            win.start_dragging().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        // Zoom um die Mitte: {factor}
        "zoom" => {
            let scale = win.scale_factor().map_err(|e| e.to_string())?;
            let cur = win.inner_size().map_err(|e| e.to_string())?.to_logical::<f64>(scale).width;
            resize(win, cur * a["factor"].as_f64().unwrap_or(1.0), true)
        }
        // Größe per Griff: {w} (oben links bleibt)
        "resize" => resize(win, a["w"].as_f64().unwrap_or(320.0), false),
        // Zurück auf 100 %
        "reset" => {
            let base = with_pins(|p| p.get(win.label()).map(|p| p.base_w)).unwrap_or(320.0);
            resize(win, base, true)
        }
        // Text-Pin an gemessene Inhaltsgröße anpassen: {w, h}
        "fit" => {
            let (w, h) = (a["w"].as_f64().unwrap_or(320.0).max(100.0), a["h"].as_f64().unwrap_or(120.0).max(60.0));
            with_pins(|p| {
                if let Some(pin) = p.get_mut(win.label()) {
                    pin.aspect = w / h;
                    pin.base_w = w;
                }
            });
            let _ = win.set_size(LogicalSize::new(w, h));
            Ok(Value::Null)
        }
        "lock" => {
            set_locked(app, win, a["locked"].as_bool().unwrap_or(true));
            Ok(Value::Null)
        }
        "contextMenu" => {
            context_menu(app, win)?;
            Ok(Value::Null)
        }
        "copy" | "save" | "annotate" | "close" => menu_action(app, win, cmd),
        _ => Err(format!("pin: unbekannter Befehl {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#FF8000"), Some((255, 128, 0)));
        assert_eq!(parse_color("#ff800"), None);
        assert_eq!(parse_color("ff8000"), None);
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some((10, 20, 30)));
        assert_eq!(parse_color("10,20,30"), Some((10, 20, 30)));
        assert_eq!(parse_color("1, 0.5, 0"), Some((255, 128, 0)));
        assert_eq!(parse_color("300, 0, 0"), None);
        assert_eq!(parse_color("1, 2"), None);
        assert_eq!(parse_color("hello"), None);
        assert!(is_light((255, 255, 0)));
        assert!(!is_light((20, 30, 120)));
    }

    #[test]
    fn html() {
        let h = "<style>p{}</style><p>Hallo &amp; <b>Welt</b></p><div>Zeile&nbsp;2 &#x41;&#66;</div><script>x()</script><br>  ";
        assert_eq!(html_to_text(h).as_deref(), Some("Hallo & Welt\nZeile 2 AB"));
        assert_eq!(html_to_text("<p> </p>"), None);
        assert_eq!(decode_entities("a & b &unknown; &lt;"), "a & b &unknown; <");
    }

    #[test]
    fn sizes() {
        assert_eq!(initial_size((2880.0, 1800.0)), (800.0, 500.0));
        assert_eq!(initial_size((400.0, 200.0)), (400.0, 200.0));
        assert_eq!(initial_size((100.0, 40.0)), (100.0, 100.0));
        let (x, y) = centered([0.0, 25.0, 1440.0, 875.0], 400.0, 300.0);
        assert_eq!((x, y), (520.0, 313.0));
        assert_eq!(clamp_width(50.0, 400.0, 2.0), 160.0);
        assert_eq!(clamp_width(5000.0, 400.0, 2.0), 1400.0);
        assert_eq!(clamp_width(5000.0, 2000.0, 2.0), 2000.0);
    }

    #[test]
    fn text_cards() {
        let (w, h) = text_card_size("kurz", 1440.0);
        assert_eq!((w, h), (320.0, 120.0));
        let (w, _) = text_card_size(&"x".repeat(2000), 1440.0);
        assert!(w <= 760.0 + 52.0);
        let (_, h) = text_card_size(&"zeile\n".repeat(20), 1440.0);
        assert_eq!(h, 20.0 * 23.0 + 48.0);
    }

    #[test]
    fn url_decoding() {
        assert_eq!(url_decode("/a%20b/%C3%A4.png"), "/a b/ä.png");
        assert_eq!(url_decode("100%"), "100%");
    }
}
