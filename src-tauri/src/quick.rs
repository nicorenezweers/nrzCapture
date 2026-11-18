//! Quick Access: schwebende Vorschau-Karten nach jeder Aufnahme (kopieren, sichern, annotieren, OCR, anheften,
//! hochladen, übersetzen, ziehen) und Toast-Hinweise. Port von Capso `QuickAccess*` + `ToastWindow`.
//!
//! Alle Karten liegen in *einem* transparenten Fenster `quick` (Stapel, älteste unten). Das Frontend hält die Liste,
//! das Backend liefert neue Karten (`pending`), Fenstergeometrie, Aktionen und den nativen Datei-Drag.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{mpsc, Mutex, OnceLock},
    time::{Duration, SystemTime},
};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow};

use crate::core::{self, Win};

/// Kartengröße wie Capso (`QuickAccessWindow`: 288×200), Abstand 12, Randabstand 16.
pub const CARD_W: f64 = 288.0;
pub const CARD_H: f64 = 200.0;
pub const SPACING: f64 = 12.0;
pub const INSET: f64 = 16.0;
/// Transparenter Rand im Fenster, damit der Kartenschatten nicht abgeschnitten wird.
pub const PAD: f64 = 14.0;
/// Capso `maxQuickAccessStackSize`.
pub const MAX_STACK: usize = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickItem {
    pub path: PathBuf,
    /// "image" | "video" | "gif"
    pub kind: String,
    /// Bereits am Zielort gespeichert (sonst Temp-Datei).
    pub saved: bool,
}

/// Neue Karten je Stapelfenster, die das Frontend noch nicht abgeholt hat (vermeidet das Rennen beim ersten Öffnen).
static PENDING: Mutex<Option<HashMap<String, Vec<Value>>>> = Mutex::new(None);
/// Zuletzt ungenutzt geschlossene Karte (für „Zuletzt geschlossene wieder öffnen“). Temp-Dateien bleiben erhalten.
static LAST_CLOSED: Mutex<Option<Value>> = Mutex::new(None);
/// Zustand je Stapelfenster (ein Stapel pro Bildschirm wie Capso `restackQuickAccessWindows`).
static STACKS: Mutex<Option<HashMap<String, Stack>>> = Mutex::new(None);

#[derive(Clone, Copy, Default)]
struct Stack {
    /// Sichtbarer Bereich (logisch, oben links) des Bildschirms.
    visible: [f64; 4],
    /// Anzahl der Karten (vom Frontend gemeldet).
    n: usize,
    /// Mauszeiger über einer Karte (vom Frontend gemeldet).
    hover: bool,
}

fn with_stacks<R>(f: impl FnOnce(&mut HashMap<String, Stack>) -> R) -> R {
    f(STACKS.lock().unwrap().get_or_insert_with(HashMap::new))
}

fn push_pending(label: &str, card: Value) {
    PENDING.lock().unwrap().get_or_insert_with(HashMap::new).entry(label.to_owned()).or_default().push(card);
}

/// Fensterlabel des Stapels für einen Bildschirm.
fn stack_label(screen_id: u32) -> String {
    format!("quick-s{screen_id}")
}

/// Alle offenen Stapelfenster.
fn stack_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows().into_iter().filter(|(l, _)| l.starts_with("quick-s")).map(|(_, w)| w).collect()
}

fn quick_window(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    let w = core::window(
        app,
        label,
        "quick",
        Win { w: CARD_W + 2.0 * PAD, h: CARD_H + 2.0 * PAD, visible: false, level: Some(core::LEVEL_POPUP), ..Default::default() },
        Value::Null,
    )?;
    core::exclude_from_capture(&w, true);
    Some(w)
}

/// Index des Bildschirms, der den Punkt enthält.
fn screen_index_at(screens: &[core::Screen], (px, py): (f64, f64)) -> Option<usize> {
    screens.iter().position(|s| px >= s.x && px < s.x + s.width && py >= s.y && py < s.y + s.height)
}

/// Bildschirm der Aufnahme: enthält die Mitte von `source`, sonst der unter dem Mauszeiger (Capso: `result.displayID`).
fn target_screen(app: &AppHandle, source: Option<[f64; 4]>) -> Option<core::Screen> {
    let screens = core::screens(app);
    let hit = source.and_then(|[x, y, w, h]| screen_index_at(&screens, (x + w / 2.0, y + h / 2.0)));
    let hit = hit.or_else(|| {
        let m = core::cursor_monitor(app)?;
        let (s, p) = (m.scale_factor(), m.position());
        screens.iter().position(|sc| (sc.x - p.x as f64 / s).abs() < 1.0 && (sc.y - p.y as f64 / s).abs() < 1.0)
    });
    hit.or((!screens.is_empty()).then_some(0)).map(|i| screens[i].clone())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Kartendaten fürs Frontend: Pfad, Art, Vorschaubild, Abmessungen, Dauer, Zeitpunkt.
fn card_json(item: &QuickItem) -> Value {
    let p = &item.path;
    let (thumb, size, duration) = match item.kind.as_str() {
        "video" => {
            let t = video_thumbnail(p);
            let size = t.as_deref().and_then(core::image_size).or_else(|| mdls_size(p));
            (t.unwrap_or_default(), size, mdls_num(p, "kMDItemDurationSeconds"))
        }
        _ => (p.clone(), core::image_size(p), None),
    };
    json!({
        "path": p,
        "kind": item.kind,
        "saved": item.saved,
        "thumb": thumb,
        "width": size.map(|s| s.0),
        "height": size.map(|s| s.1),
        "duration": duration,
        "created": now_ms(),
    })
}

fn mdls_num(p: &Path, key: &str) -> Option<f64> {
    core::sh("mdls", &["-raw", "-name", key, &p.to_string_lossy()])?.trim().parse().ok()
}

fn mdls_size(p: &Path) -> Option<(u32, u32)> {
    Some((mdls_num(p, "kMDItemPixelWidth")? as u32, mdls_num(p, "kMDItemPixelHeight")? as u32))
}

/// Vorschaubild eines Videos über Quick Look (`qlmanage -t`), zwischengespeichert im Temp-Ordner.
fn video_thumbnail(p: &Path) -> Option<PathBuf> {
    let dir = core::temp_dir().join("thumbs");
    let _ = std::fs::create_dir_all(&dir);
    let name = format!("{}.png", p.file_name()?.to_string_lossy());
    let out = dir.join(&name);
    if !out.exists() {
        core::sh("qlmanage", &["-t", "-s", "640", "-o", &dir.to_string_lossy(), &p.to_string_lossy()]);
    }
    out.exists().then_some(out)
}

/// Neue Vorschau zeigen (ohne Einflug-Animation, ohne Auto-Upload). = `show_from(app, item, None)` ohne Auto-Upload.
pub fn show(app: &AppHandle, item: QuickItem) {
    present(app, item, None, false);
}

/// Neue Vorschau nach einer Aufnahme zeigen. `source` = aufgenommener Bereich (global logisch [x, y, w, h]):
/// bestimmt den Bildschirm des Stapels (Mitte von `source`, sonst Mauszeiger) und den Startpunkt der Einflug-Animation.
/// Lädt bei `cloudShareAutoUpload` + eingerichteter Cloud-Freigabe ein Bild selbst hoch (siehe [`auto_upload_handles`]).
pub fn show_from(app: &AppHandle, item: QuickItem, source: Option<[f64; 4]>) {
    present(app, item, source, true);
}

/// `true`, wenn die Quick-Access-Karte den automatischen Upload nach der Aufnahme selbst übernimmt (wie Capso
/// `QuickAccessView.startAutoUploadIfNeeded`): Auto-Upload an, Cloud eingerichtet, Quick Access aktiv. Dann darf
/// `capture::deliver` nach `show_from` (Bild) nicht zusätzlich hochladen; sonst lädt `deliver` selbst hoch.
pub fn auto_upload_handles() -> bool {
    let s = crate::settings::get();
    s.cloud_share_auto_upload && s.screenshot_show_preview && crate::share::is_configured()
}

fn present(app: &AppHandle, item: QuickItem, source: Option<[f64; 4]>, auto: bool) {
    let Some(screen) = target_screen(app, source) else { return };
    let label = stack_label(screen.id);
    let n_after = with_stacks(|m| {
        let st = m.entry(label.clone()).or_default();
        st.visible = screen.work;
        (st.n + 1).min(MAX_STACK)
    });
    let source = source.filter(|r| r[2] >= 1.0 && r[3] >= 1.0);
    let mut card = card_json(&item);
    card["fly"] = json!(source.is_some());
    card["autoUpload"] = json!(auto && item.kind == "image" && auto_upload_handles());
    push_pending(&label, card.clone());
    if quick_window(app, &label).is_some() {
        let _ = app.emit_to(label.as_str(), "quick-add", ());
    }
    if let Some(src) = source {
        let pos = crate::settings::get().quick_access_position;
        let [x, y, _, _] = stack_frame(&pos, screen.work, n_after);
        // Neueste Karte liegt oben im Stapel.
        let to = [x + PAD, y + PAD, CARD_W, CARD_H];
        let app = app.clone();
        std::thread::spawn(move || fly_in(&app, &label, card, src, to));
    }
}

/// Seitenverhältnis-treu in `rect` einpassen und zentrieren (Capso `aspectFitFrame`).
pub fn aspect_fit_frame(size: (f64, f64), rect: [f64; 4]) -> [f64; 4] {
    let [x, y, w, h] = rect;
    if size.0 <= 0.0 || size.1 <= 0.0 || w <= 0.0 || h <= 0.0 {
        return rect;
    }
    let s = (w / size.0).min(h / size.1);
    let (fw, fh) = (size.0 * s, size.1 * s);
    [x + w / 2.0 - fw / 2.0, y + h / 2.0 - fh / 2.0, fw, fh]
}

/// Kubische Bézier-Easing-Kurve wie `CAMediaTimingFunction(controlPoints:)`.
pub fn bezier_ease(t: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let c = |a: f64, b: f64, u: f64| 3.0 * a * u * (1.0 - u).powi(2) + 3.0 * b * u * u * (1.0 - u) + u.powi(3);
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if c(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    c(y1, y2, (lo + hi) / 2.0)
}

/// Einflug: Kartenabbild fliegt vom aufgenommenen Bereich an seinen Platz im Stapel (Capso
/// `QuickAccessTransitionWindow`, 0,48 s, Kurve 0.22/0.75/0.2/1). Danach wird die echte Karte sichtbar (`quick-landed`).
fn fly_in(app: &AppHandle, label: &str, card: Value, src: [f64; 4], to: [f64; 4]) {
    const DUR: Duration = Duration::from_millis(480);
    let from = aspect_fit_frame((to[2], to[3]), src);
    let o = Win { w: from[2], h: from[3], pos: Some((from[0], from[1])), visible: false, level: Some(core::LEVEL_POPUP), ..Default::default() };
    let landed = |app: &AppHandle| {
        let _ = app.emit_to(label, "quick-landed", card["created"].clone());
    };
    let Some(w) = core::window(app, "quick-fly", "quick-fly", o, card.clone()) else { return landed(app) };
    core::exclude_from_capture(&w, true);
    let _ = w.set_ignore_cursor_events(true);
    let _ = w.set_size(LogicalSize::new(from[2], from[3]));
    let _ = w.set_position(LogicalPosition::new(from[0], from[1]));
    // Dem WebView kurz Zeit zum Zeichnen geben.
    std::thread::sleep(Duration::from_millis(40));
    let _ = w.show();
    let start = std::time::Instant::now();
    loop {
        let t = (start.elapsed().as_secs_f64() / DUR.as_secs_f64()).min(1.0);
        let e = bezier_ease(t, 0.22, 0.75, 0.2, 1.0);
        let f: Vec<f64> = (0..4).map(|i| from[i] + (to[i] - from[i]) * e).collect();
        let _ = w.set_size(LogicalSize::new(f[2], f[3]));
        let _ = w.set_position(LogicalPosition::new(f[0], f[1]));
        if t >= 1.0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(12));
    }
    landed(app);
    std::thread::sleep(Duration::from_millis(60));
    let _ = w.hide();
}

/// Fenster zum Key-Window machen (Tastaturfokus für die neue Karte, Capso `makeKey()`), ohne die App zu aktivieren.
fn make_key(w: &WebviewWindow) {
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        if let Ok(ptr) = w2.ns_window() {
            use objc2_app_kit::NSWindow;
            let nw = unsafe { &*(ptr as *const NSWindow) };
            nw.makeKeyWindow();
        }
    });
}

/// Globales Übersetzen-Kürzel: übersetzt die Karte im fokussierten bzw. gehoverten Quick-Access-Stapel
/// (Capso `invokeQuickAccessTranslateIfKey`). `true` = erledigt, sonst normale Übersetzen-Aufnahme starten.
pub fn translate_focused(app: &AppHandle) -> bool {
    let candidates: Vec<(String, bool)> = with_stacks(|m| m.iter().filter(|(_, s)| s.n > 0).map(|(l, s)| (l.clone(), s.hover)).collect());
    let pick = candidates.iter().find(|(_, hover)| *hover).or_else(|| {
        candidates.iter().find(|(l, _)| app.get_webview_window(l).is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false)))
    });
    match pick {
        Some((label, _)) => app.emit_to(label.as_str(), "quick-translate", ()).is_ok(),
        None => false,
    }
}

/// Während einer Aufnahme ausblenden (`hide = true`) und danach wieder zeigen.
pub fn hide_for_capture(app: &AppHandle, hide: bool) {
    let mut slept = false;
    for w in stack_windows(app) {
        let key = format!("{}-was-visible", w.label());
        if hide {
            if w.is_visible().unwrap_or(false) {
                let _ = w.hide();
                core::set_window_data(&key, json!(true));
                slept = true;
            }
        } else if core::window_data(&key) == json!(true) {
            core::set_window_data(&key, Value::Null);
            let _ = w.show();
        }
    }
    if slept {
        std::thread::sleep(Duration::from_millis(120));
    }
}

/// Alle Karten sichern (Menü „Alle in Quick Access sichern“).
pub fn save_all(app: &AppHandle) {
    for w in stack_windows(app) {
        let _ = w.emit_to(w.label(), "quick-save-all", ());
    }
}

/// Alle Karten schließen (ohne „wieder öffnen“ zu setzen – wie Capso).
pub fn close_all(app: &AppHandle) {
    for w in stack_windows(app) {
        let _ = w.emit_to(w.label(), "quick-close-all", ());
    }
}

/// Zuletzt ungenutzt geschlossene Karte wiederherstellen (auf dem Bildschirm unter dem Mauszeiger).
pub fn reopen_last(app: &AppHandle) {
    let Some(mut card) = LAST_CLOSED.lock().unwrap().take() else { return };
    if !card["path"].as_str().is_some_and(|p| Path::new(p).exists()) {
        return;
    }
    let Some(screen) = target_screen(app, None) else { return };
    let label = stack_label(screen.id);
    with_stacks(|m| m.entry(label.clone()).or_default().visible = screen.work);
    card["created"] = json!(now_ms());
    card["fly"] = json!(false);
    card["autoUpload"] = json!(false);
    push_pending(&label, card);
    if quick_window(app, &label).is_some() {
        let _ = app.emit_to(label.as_str(), "quick-add", ());
    }
}

/// Gibt es eine Karte zum Wiederherstellen? (für Menüs)
pub fn can_reopen() -> bool {
    LAST_CLOSED.lock().unwrap().is_some()
}

// ───────────────────────────── Geometrie ─────────────────────────────

/// Fensterrahmen (logisch, oben links) für einen Stapel aus `n` Karten – Port von `QuickAccessStackGeometry`.
/// `visible` = Arbeitsbereich [x, y, w, h]. Enthält den Schattenrand `PAD`.
pub fn stack_frame(position: &str, visible: [f64; 4], n: usize) -> [f64; 4] {
    let n = n.max(1) as f64;
    let [vx, vy, vw, vh] = visible;
    let stack_h = n * CARD_H + (n - 1.0) * SPACING;
    let x = match position {
        "bottomLeft" => vx + INSET,
        "centerScreen" => vx + vw / 2.0 - CARD_W / 2.0,
        _ => vx + vw - CARD_W - INSET,
    };
    let y = match position {
        // Capso: Mitte minus 48 pt (in Cocoa-Koordinaten) → etwas unterhalb der Mitte; oberer Rand hat Vorrang.
        "centerScreen" => (vy + vh / 2.0 - stack_h / 2.0 + 48.0).min(vy + vh - INSET - stack_h).max(vy + INSET),
        _ => vy + vh - INSET - stack_h,
    };
    [x - PAD, y - PAD, CARD_W + 2.0 * PAD, stack_h + 2.0 * PAD]
}

/// Größe/Position des Fensters an die Anzahl der Karten anpassen.
fn layout(app: &AppHandle, window: &WebviewWindow, n: usize) {
    let label = window.label().to_owned();
    let visible = with_stacks(|m| {
        let st = m.entry(label.clone()).or_default();
        st.n = n;
        if n == 0 {
            st.hover = false;
        }
        (st.visible[2] > 0.0).then_some(st.visible)
    });
    if n == 0 {
        let _ = window.hide();
        return;
    }
    let Some(visible) = visible.or_else(|| core::screens(app).first().map(|s| s.work)) else { return };
    let pos = crate::settings::get().quick_access_position;
    let [x, y, w, h] = stack_frame(&pos, visible, n);
    let _ = window.set_size(LogicalSize::new(w, h));
    let _ = window.set_position(LogicalPosition::new(x, y));
    if core::window_data(&format!("{label}-was-visible")) != json!(true) {
        let _ = window.show();
    }
}

// ───────────────────────────── Aktions-Layout ─────────────────────────────

/// Alle Aktions-IDs (Capso `QuickAccessActionKind`).
pub const ACTIONS: [&str; 8] = ["drag", "copy", "save", "upload", "annotate", "ocr", "translate", "pin"];

/// Sichtbare Knöpfe + Überlauf-Menü. Port von `QuickAccessActionLayout`.
/// Einstellung `quickAccessActions`: `null` (Standard), Liste sichtbarer IDs oder `{visible: [...], overflow: [...]}`.
pub fn action_layout(setting: &Value, sharing: bool) -> (Vec<String>, Vec<String>) {
    let valid = |id: &str| ACTIONS.contains(&id) && (sharing || id != "upload");
    let list = |v: &Value| -> Option<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for id in v.as_array()?.iter().filter_map(Value::as_str) {
            if valid(id) && !out.iter().any(|o| o == id) {
                out.push(id.to_owned());
            }
        }
        Some(out)
    };
    let (visible, overflow) = match setting {
        Value::Array(_) => (list(setting), None),
        Value::Object(o) => (o.get("visible").and_then(list), o.get("overflow").and_then(list)),
        _ => (None, None),
    };
    let visible = visible.unwrap_or_else(|| {
        let mut v: Vec<String> = vec!["drag".into(), "copy".into(), "save".into()];
        if sharing {
            v.push("upload".into());
        }
        v.extend(["annotate".into(), "pin".into()]);
        v
    });
    let overflow = overflow
        .map(|o| o.into_iter().filter(|id| !visible.contains(id)).collect())
        .unwrap_or_else(|| ACTIONS.iter().filter(|id| valid(id) && !visible.iter().any(|v| v == *id)).map(|s| s.to_string()).collect());
    (visible, overflow)
}

// ───────────────────────────── Aktionen ─────────────────────────────

fn item_from(a: &Value) -> Result<QuickItem, String> {
    let path = a["path"].as_str().filter(|p| !p.is_empty()).ok_or("path fehlt")?;
    Ok(QuickItem { path: PathBuf::from(path), kind: a["kind"].as_str().unwrap_or("image").to_owned(), saved: a["saved"].as_bool().unwrap_or(false) })
}

/// Dateiname am Ziel gemäß Vorlage (Screenshot- bzw. Aufnahme-Vorlage).
fn target_name(item: &QuickItem) -> String {
    let s = crate::settings::get();
    let ext = item.path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    let template = if item.kind == "image" { &s.screenshot_filename_template } else { &s.recording_filename_template };
    core::file_name(template, "", &ext)
}

/// Karte sichern: Temp-Datei → Exportordner (Dateinamen-Vorlage). Gibt den neuen Pfad zurück.
pub fn save_item(item: &QuickItem) -> Result<PathBuf, String> {
    if item.saved && item.path.exists() {
        return Ok(item.path.clone());
    }
    let dst = core::unique_path(&core::export_dir(), &target_name(item));
    std::fs::copy(&item.path, &dst).map_err(|e| e.to_string())?;
    after_save(item, &dst);
    Ok(dst)
}

fn after_save(item: &QuickItem, dst: &Path) {
    let s = crate::settings::get();
    if item.kind == "image" && s.screenshot_auto_copy && s.screenshot_clipboard_content == "filePath" {
        core::clipboard::copy_file(dst);
    }
}

/// „Sichern unter …“ über den System-Dateidialog (AppleScript, ohne zusätzliches Plugin).
fn save_as(item: &QuickItem) -> Result<Option<PathBuf>, String> {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"POSIX path of (choose file name with prompt "{}" default name "{}" default location (POSIX file "{}"))"#,
        esc(core::t("Sichern unter", "Save As")),
        esc(&target_name(item)),
        esc(&core::export_dir().to_string_lossy())
    );
    let Some(out) = core::sh("osascript", &["-e", &script]) else { return Ok(None) };
    let dst = PathBuf::from(out.trim());
    std::fs::copy(&item.path, &dst).map_err(|e| e.to_string())?;
    after_save(item, &dst);
    Ok(Some(dst))
}

/// Kopieren wie Capso `copyScreenshotToClipboard`: Bilder gemäß `screenshotClipboardContent` als Bild im Format
/// `screenshotClipboardFormat` (png/jpeg/tiff) oder als Datei – gespeicherte Datei, sonst eine Temp-Datei mit Namen
/// gemäß Vorlage (`QuickAccessDragFileStore`). Videos/GIFs immer als Datei.
fn copy_item(item: &QuickItem) -> bool {
    if item.kind != "image" {
        return core::clipboard::copy_file(&item.path);
    }
    let s = crate::settings::get();
    if s.screenshot_clipboard_content == "filePath" {
        core::clipboard::copy_file(&drag_file(item))
    } else {
        crate::capture::post::copy_image_as(&item.path, &s.screenshot_clipboard_format)
    }
}

/// Großansicht (Doppelklick auf ein Bild), 82 % des Bildschirms wie `QuickAccessPreviewWindow`.
fn open_preview(app: &AppHandle, item: &QuickItem) {
    let (iw, ih) = core::image_size(&item.path).unwrap_or((1200, 800));
    let m = core::cursor_monitor(app);
    let scale = m.as_ref().map(|m| m.scale_factor()).unwrap_or(2.0);
    let (vw, vh) = m.map(|m| (m.work_area().size.width as f64 / scale, m.work_area().size.height as f64 / scale)).unwrap_or((1440.0, 900.0));
    let (w, h) = fit_size((iw as f64 / scale, ih as f64 / scale), (vw * 0.82, vh * 0.82 - 28.0));
    let title = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut o = Win::app(&title, w.max(240.0), h.max(160.0) + 28.0);
    o.min_size = Some((240.0, 160.0));
    core::window(app, "quick-preview", "quick-preview", o, json!({"path": item.path, "kind": item.kind}));
}

/// Seitenverhältnis-treu in `max` einpassen (nie vergrößern).
pub fn fit_size(size: (f64, f64), max: (f64, f64)) -> (f64, f64) {
    if size.0 <= 0.0 || size.1 <= 0.0 {
        return max;
    }
    let s = (max.0 / size.0).min(max.1 / size.1).min(1.0);
    ((size.0 * s).floor(), (size.1 * s).floor())
}

fn run_action(app: &AppHandle, act: &str, item: &QuickItem) -> Result<Value, String> {
    let p = &item.path;
    match act {
        "copy" => Ok(json!(copy_item(item))),
        "save" => Ok(json!(save_item(item)?)),
        "saveAs" => Ok(json!(save_as(item)?)),
        "annotate" => {
            if item.kind == "image" {
                crate::editor::open(app, p);
            } else {
                crate::receditor::open(app, p);
            }
            Ok(Value::Null)
        }
        "ocr" => {
            // Capso: OCR aus Quick Access öffnet Visual OCR.
            crate::ocr::visual(app, p);
            Ok(Value::Null)
        }
        "translate" => {
            crate::translate::translate_image(app, p, None);
            Ok(Value::Null)
        }
        "pin" => {
            crate::pin::pin_image(app, p, None);
            Ok(Value::Null)
        }
        // Karte zeigt den Status selbst (Capso `PanelUploadState`): blockierend hochladen, Link kopieren.
        "upload" => {
            let url = crate::share::upload(app, p)?;
            core::clipboard::copy_text(&url);
            Ok(json!(url))
        }
        "reveal" => {
            core::reveal(p);
            Ok(Value::Null)
        }
        // Doppelklick: Bilder in der Großansicht, Videos im Aufnahme-Editor, GIFs mit der Standard-App.
        "open" => {
            match item.kind.as_str() {
                "video" => crate::receditor::open(app, p),
                "gif" => core::open(&p.to_string_lossy()),
                _ => open_preview(app, item),
            }
            Ok(Value::Null)
        }
        _ => Err(format!("quick: unbekannte Aktion {act}")),
    }
}

// ───────────────────────────── Drag ─────────────────────────────

/// Datei, die beim Ziehen übergeben wird: gespeicherte Datei, sonst Kopie mit Namen gemäß Vorlage
/// (Capso `QuickAccessDragFileStore`) – so landet in der Ziel-App ein sinnvoll benannter Screenshot.
fn drag_file(item: &QuickItem) -> PathBuf {
    if item.saved {
        return item.path.clone();
    }
    static CACHE: Mutex<Option<HashMap<PathBuf, PathBuf>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(p) = cache.get(&item.path).filter(|p| p.exists()) {
        return p.clone();
    }
    let dir = core::temp_dir().join("drag");
    let _ = std::fs::create_dir_all(&dir);
    prune_stale(&dir, Duration::from_secs(24 * 3600), &cache.values().cloned().collect::<Vec<_>>());
    let dst = core::unique_path(&dir, &target_name(item));
    if std::fs::hard_link(&item.path, &dst).is_err() && std::fs::copy(&item.path, &dst).is_err() {
        return item.path.clone();
    }
    cache.insert(item.path.clone(), dst.clone());
    dst
}

/// Alte Drag-Dateien entfernen (außer den noch verwendeten).
fn prune_stale(dir: &Path, age: Duration, keep: &[PathBuf]) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d > age);
        if old && !keep.contains(&p) {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn file_url(path: &str) -> String {
    let enc: String = path
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("file://{enc}")
}

/// Nativer Datei-Drag (wie aus dem Finder). Meldet `dropped` = (Kartenpfad, abgelegt?) an das Fenster.
fn drag(app: &AppHandle, window: &WebviewWindow, item: QuickItem, thumb: Option<PathBuf>) {
    let file = drag_file(&item);
    let path = file.to_string_lossy().into_owned();
    let card_path = item.path.to_string_lossy().into_owned();
    let src = thumb.filter(|t| t.exists()).unwrap_or_else(|| item.path.clone());
    let img = core::temp_dir().join("thumbs").join(format!("drag-{}.png", src.file_name().unwrap_or_default().to_string_lossy()));
    if !img.exists() {
        let _ = std::fs::create_dir_all(img.parent().unwrap());
        core::sh("sips", &["-s", "format", "png", "-Z", "268", &src.to_string_lossy(), "--out", &img.to_string_lossy()]);
    }
    let url = file_url(&path);
    let xml = path.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><array><string>{xml}</string></array></plist>"#
    );
    let (w, app) = (window.clone(), app.clone());
    let _ = window.run_on_main_thread(move || {
        let item = drag::DragItem::Data {
            provider: Box::new(move |t| match t {
                "public.file-url" => Some(url.clone().into_bytes()),
                "NSFilenamesPboardType" => Some(plist.clone().into_bytes()),
                _ => None,
            }),
            types: vec!["public.file-url".into(), "NSFilenamesPboardType".into()],
        };
        let img = if img.exists() { img } else { PathBuf::from(&path) };
        let label = w.label().to_owned();
        let _ = drag::start_drag(
            &w,
            item,
            drag::Image::File(img),
            move |res, _| {
                let _ = app.emit_to(label.as_str(), "dropped", (card_path.clone(), matches!(res, drag::DragResult::Dropped)));
            },
            drag::Options::default(),
        );
    });
}

// ───────────────────────────── Toast ─────────────────────────────

const TOAST_SHOW: Duration = Duration::from_millis(1800);
const TOAST_FADE: Duration = Duration::from_millis(300);

static TOAST_TX: OnceLock<Mutex<mpsc::Sender<String>>> = OnceLock::new();

/// Kurzer Hinweis unten in der Bildschirmmitte (Glas-Kapsel). Mehrere Meldungen werden nacheinander gezeigt.
pub fn toast(app: &AppHandle, msg: &str) {
    let tx = TOAST_TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<String>();
        let app = app.clone();
        std::thread::spawn(move || toast_worker(app, rx));
        Mutex::new(tx)
    });
    let _ = tx.lock().unwrap().send(msg.to_owned());
}

/// Warnsymbol statt Häkchen bei Fehlermeldungen.
pub fn toast_is_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    ["fehl", "error", "failed", "kein", "keine", "nicht", "no ", "not ", "unable", "unavailable"].iter().any(|k| m.contains(k))
}

/// Geschätzte Kapselbreite (das Frontend korrigiert per `toastFit`).
pub fn toast_width(msg: &str) -> f64 {
    (msg.chars().count() as f64 * 7.4 + 24.0 + 32.0 + 24.0).clamp(140.0, 640.0)
}

fn toast_place(app: &AppHandle, w: &WebviewWindow, width: f64, height: f64) {
    let Some(m) = core::cursor_monitor(app) else { return };
    let (s, wa) = (m.scale_factor(), m.work_area());
    let (vx, vy, vw, vh) = (wa.position.x as f64 / s, wa.position.y as f64 / s, wa.size.width as f64 / s, wa.size.height as f64 / s);
    let _ = w.set_size(LogicalSize::new(width, height));
    let _ = w.set_position(LogicalPosition::new(vx + (vw - width) / 2.0, vy + vh - 50.0 - height));
}

fn toast_worker(app: AppHandle, rx: mpsc::Receiver<String>) {
    let mut queue: VecDeque<String> = VecDeque::new();
    loop {
        let msg = match queue.pop_front() {
            Some(m) => m,
            None => match rx.recv() {
                Ok(m) => m,
                Err(_) => return,
            },
        };
        let h = 64.0;
        let data = json!({"message": msg, "error": toast_is_error(&msg), "id": now_ms()});
        let Some(w) = core::window(&app, "toast", "toast", Win { w: toast_width(&msg), h, visible: false, level: Some(core::LEVEL_POPUP), ..Default::default() }, data.clone()) else {
            continue;
        };
        let _ = w.set_ignore_cursor_events(true);
        core::exclude_from_capture(&w, true);
        toast_place(&app, &w, toast_width(&msg), h);
        let _ = w.emit_to("toast", "toast-show", data);
        let _ = w.show();
        std::thread::sleep(TOAST_SHOW);
        queue.extend(rx.try_iter());
        let _ = w.emit_to("toast", "toast-hide", ());
        std::thread::sleep(TOAST_FADE);
        queue.extend(rx.try_iter());
        if queue.is_empty() {
            let _ = w.hide();
        }
    }
}

// ───────────────────────────── IPC ─────────────────────────────

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    match cmd {
        "pending" => Ok(Value::Array(PENDING.lock().unwrap().as_mut().and_then(|m| m.remove(win.label())).unwrap_or_default())),
        "hover" => {
            let hover = a["hover"].as_bool().unwrap_or(false);
            with_stacks(|m| m.entry(win.label().to_owned()).or_default().hover = hover);
            Ok(Value::Null)
        }
        "focus" => {
            make_key(win);
            Ok(Value::Null)
        }
        "layout" => {
            layout(app, win, a["n"].as_u64().unwrap_or(0) as usize);
            Ok(Value::Null)
        }
        "actions" => {
            let s = crate::settings::get();
            let (visible, overflow) = action_layout(&s.quick_access_actions, s.is_cloud_share_configured());
            Ok(json!({"visible": visible, "overflow": overflow, "maxStack": MAX_STACK}))
        }
        "action" => run_action(app, a["action"].as_str().unwrap_or(""), &item_from(&a["item"])?),
        "closed" => {
            *LAST_CLOSED.lock().unwrap() = Some(a["item"].clone());
            Ok(Value::Null)
        }
        "drag" => {
            let item = item_from(&a["item"])?;
            let thumb = a["item"]["thumb"].as_str().filter(|t| !t.is_empty()).map(PathBuf::from);
            drag(app, win, item, thumb);
            Ok(Value::Null)
        }
        "toastFit" => {
            let (w, h) = (a["w"].as_f64().unwrap_or(200.0), a["h"].as_f64().unwrap_or(64.0));
            toast_place(app, win, w.clamp(80.0, 900.0), h.clamp(32.0, 200.0));
            Ok(Value::Null)
        }
        _ => Err(format!("quick: unbekannter Befehl {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIS: [f64; 4] = [0.0, 25.0, 1440.0, 800.0];

    #[test]
    fn stack_bottom_right_and_left() {
        let [x, y, w, h] = stack_frame("bottomRight", VIS, 1);
        assert_eq!((x + PAD, y + PAD), (1440.0 - CARD_W - INSET, 825.0 - INSET - CARD_H));
        assert_eq!((w, h), (CARD_W + 2.0 * PAD, CARD_H + 2.0 * PAD));
        let [x, y, _, h] = stack_frame("bottomLeft", VIS, 3);
        assert_eq!(x + PAD, INSET);
        assert_eq!(h - 2.0 * PAD, 3.0 * CARD_H + 2.0 * SPACING);
        assert_eq!(y + PAD + h - 2.0 * PAD, 825.0 - INSET);
    }

    #[test]
    fn stack_center_is_bounded() {
        let [x, y, _, _] = stack_frame("centerScreen", VIS, 1);
        assert_eq!(x + PAD, 720.0 - CARD_W / 2.0);
        assert_eq!(y + PAD, 25.0 + 400.0 - CARD_H / 2.0 + 48.0);
        // Hoher Stapel passt nicht unter die Mitte → am unteren Rand begrenzt, oben mindestens INSET.
        let [_, y, _, h] = stack_frame("centerScreen", VIS, 5);
        assert!(y + PAD >= 25.0 + INSET - 0.001);
        assert!(y + h - PAD <= 825.0 - INSET + 0.001 || y + PAD == 25.0 + INSET);
    }

    #[test]
    fn default_action_layout_matches_capso() {
        let (v, o) = action_layout(&Value::Null, false);
        assert_eq!(v, ["drag", "copy", "save", "annotate", "pin"]);
        assert_eq!(o, ["ocr", "translate"]);
        let (v, o) = action_layout(&Value::Null, true);
        assert_eq!(v, ["drag", "copy", "save", "upload", "annotate", "pin"]);
        assert_eq!(o, ["ocr", "translate"]);
    }

    #[test]
    fn custom_action_layout() {
        let (v, o) = action_layout(&json!(["copy", "ocr", "copy", "bogus", "upload"]), false);
        assert_eq!(v, ["copy", "ocr"]);
        assert_eq!(o, ["drag", "save", "annotate", "translate", "pin"]);
        let (v, o) = action_layout(&json!({"visible": ["save"], "overflow": ["save", "pin"]}), true);
        assert_eq!(v, ["save"]);
        assert_eq!(o, ["pin"]);
    }

    #[test]
    fn fit_never_upscales() {
        assert_eq!(fit_size((100.0, 50.0), (1000.0, 1000.0)), (100.0, 50.0));
        assert_eq!(fit_size((2000.0, 1000.0), (1000.0, 1000.0)), (1000.0, 500.0));
        assert_eq!(fit_size((1000.0, 2000.0), (1000.0, 500.0)), (250.0, 500.0));
    }

    #[test]
    fn toast_helpers() {
        assert!(toast_is_error("Speichern fehlgeschlagen"));
        assert!(toast_is_error("Upload failed"));
        assert!(!toast_is_error("Text kopiert"));
        assert!(toast_width("x") >= 140.0);
        assert!(toast_width(&"x".repeat(500)) <= 640.0);
    }

    #[test]
    fn fly_in_helpers() {
        assert_eq!(aspect_fit_frame((288.0, 200.0), [0.0, 0.0, 576.0, 800.0]), [0.0, 200.0, 576.0, 400.0]);
        assert_eq!(aspect_fit_frame((0.0, 1.0), [1.0, 2.0, 3.0, 4.0]), [1.0, 2.0, 3.0, 4.0]);
        assert!(bezier_ease(0.0, 0.22, 0.75, 0.2, 1.0).abs() < 1e-6);
        assert!((bezier_ease(1.0, 0.22, 0.75, 0.2, 1.0) - 1.0).abs() < 1e-6);
        // Ease-out: zur Hälfte der Zeit deutlich mehr als die Hälfte des Weges.
        assert!(bezier_ease(0.5, 0.22, 0.75, 0.2, 1.0) > 0.8);
    }

    #[test]
    fn screen_lookup() {
        let sc = |id, x| core::Screen { id, x, y: 0.0, width: 1000.0, height: 800.0, scale: 2.0, work: [x, 25.0, 1000.0, 775.0] };
        let screens = [sc(1, 0.0), sc(2, 1000.0)];
        assert_eq!(screen_index_at(&screens, (1500.0, 400.0)), Some(1));
        assert_eq!(screen_index_at(&screens, (-5.0, 400.0)), None);
        assert_eq!(stack_label(2), "quick-s2");
    }

    #[test]
    fn file_url_encoding() {
        assert_eq!(file_url("/tmp/a b&c.png"), "file:///tmp/a%20b%26c.png");
    }
}

// added by prefs agent
/// Sind Quick-Access-Karten offen? (Menü: „Alle sichern“/„Schließen“ nur dann zeigen, Capso `hasQuickAccess`)
pub fn has_items(app: &AppHandle) -> bool {
    PENDING.lock().unwrap().as_ref().is_some_and(|m| m.values().any(|v| !v.is_empty()))
        || stack_windows(app).iter().any(|w| w.is_visible().unwrap_or(false))
}
