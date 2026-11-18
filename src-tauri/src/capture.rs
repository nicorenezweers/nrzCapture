//! Screenshots: Bereich / Vollbild / Fenster / All-in-One / letzter Bereich / Selbstauslöser
//! und die gemeinsame Nachbearbeitung (`deliver`). Port von Capso `CaptureCoordinator` + `CaptureKit`.
//!
//! Ablauf einer Auswahl: alle Bildschirme werden eingefroren (`screencapture -R` je Bildschirm), dann zeigt
//! je Bildschirm ein Overlay-Fenster (`select-<i>`, Route `select`) das Standbild mit Abdunklung, Fadenkreuz,
//! Fenstererkennung, Presets und – im All-in-One-Modus – Werkzeugleiste und Inline-Annotation.
//! Das Frontend meldet das Ergebnis mit `rpc("capture","overlayResult",…)`.
pub mod geom;
pub mod post;
pub mod stitch;
pub mod windows;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow};

use crate::core::{self, Screen, Win};

/// Was nach der Aufnahme passieren soll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Intent {
    /// Standard: Quick Access / Auto-Kopieren / Auto-Speichern gemäß Einstellungen.
    Save,
    Clipboard,
    Annotate,
    Pin,
    Share,
    Ocr,
    Translate,
}

/// Wofür ein Bereich ausgewählt wird (steuert Overlay-Optionen).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Purpose {
    Screenshot,
    Recording,
    Ocr,
    Translate,
    Scroll,
}

/// Ausgewählter Bereich in globalen logischen Koordinaten (Punkte, Ursprung oben links des Hauptbildschirms).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub screen: u32,
    /// Gesetzt, wenn ein Fenster gewählt wurde.
    pub window_id: Option<u32>,
    /// Ganzer Bildschirm gewählt.
    pub fullscreen: bool,
}

// ───────────────────────────── Overlay-Steuerung ─────────────────────────────

/// Ergebnis eines Overlays (vom Frontend). Koordinaten lokal zum Bildschirm `screen`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OverlayResult {
    pub session: u64,
    /// "area" | "window" | "fullscreen" | "cancel"
    pub kind: String,
    pub screen: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub window_id: Option<u32>,
    /// All-in-One: "copy" | "save" | "pin" | "annotate" | "ocr" | "share" | "translate" | "scroll" | "timer" | "record" | "ocrMode" …
    pub action: Option<String>,
    /// Inline-Annotation: gerendertes Ergebnis als PNG-Data-URL.
    pub data_url: Option<String>,
    /// Mehrfenster-Auswahl (⇧-Klick, kind "windows"): Fenster-IDs von vorne nach hinten.
    pub window_ids: Vec<u32>,
}

struct Frozen {
    screen: Screen,
    path: Option<PathBuf>,
}

struct OverlayState {
    session: u64,
    tx: mpsc::Sender<OverlayResult>,
    labels: Vec<String>,
    focus_label: String,
}

static SESSION: AtomicU64 = AtomicU64::new(1);
static OVERLAY: Mutex<Option<OverlayState>> = Mutex::new(None);
static BUSY: Mutex<bool> = Mutex::new(false);
static TIMER_CANCEL: AtomicBool = AtomicBool::new(false);
/// Fensterliste der letzten Auswahl (für Mehrfenster, Menüs und Quell-App/-Titel).
static OVERLAY_WINS: Mutex<Vec<windows::WindowInfo>> = Mutex::new(Vec::new());
/// Vorderste App beim Start der Aufnahme (Capso `rememberSourceApplication`).
static PENDING_SOURCE: Mutex<Option<String>> = Mutex::new(None);

/// Vorderste App (ohne nrzCapture selbst) für {source}/{app} und den Verlauf merken.
/// Am Anfang jedes Aufnahme-Ablaufs aufrufen (vor dem Overlay).
pub fn remember_source_app() {
    let own = std::process::id() as i32;
    let ws = objc2_app_kit::NSWorkspace::sharedWorkspace();
    let name = ws
        .frontmostApplication()
        .filter(|a| a.processIdentifier() != own)
        .and_then(|a| a.localizedName())
        .map(|n| n.to_string().trim().to_string())
        .filter(|n| !n.is_empty());
    *PENDING_SOURCE.lock().unwrap() = name;
}

/// Quell-App und Fenstertitel einer Aufnahme: bei Fenstern aus der Fensterliste, sonst die gemerkte vorderste App.
fn take_source(region: Option<&Region>) -> (Option<String>, Option<String>) {
    let pending = PENDING_SOURCE.lock().unwrap().take();
    if let Some(id) = region.and_then(|r| r.window_id) {
        if let Some(w) = OVERLAY_WINS.lock().unwrap().iter().find(|w| w.id == id) {
            let nz = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
            return (nz(&w.app).or(pending), nz(&w.title));
        }
    }
    (pending, None)
}

fn guard() -> bool {
    let mut b = BUSY.lock().unwrap();
    if *b {
        return false;
    }
    *b = true;
    true
}

fn release() {
    *BUSY.lock().unwrap() = false;
}

/// Index des Bildschirms unter dem Mauszeiger.
fn cursor_screen(app: &AppHandle, screens: &[Screen]) -> usize {
    core::cursor_monitor(app)
        .and_then(|m| {
            let s = m.scale_factor();
            let (x, y) = (m.position().x as f64 / s, m.position().y as f64 / s);
            screens.iter().position(|sc| (sc.x - x).abs() < 1.0 && (sc.y - y).abs() < 1.0)
        })
        .unwrap_or(0)
}

/// Mauszeiger global (logisch, oben links) über `NSEvent.mouseLocation` (Cocoa: unten links des Hauptbildschirms).
fn cursor_global(screens: &[Screen]) -> Option<(f64, f64)> {
    let main_h = screens.iter().find(|s| s.x == 0.0 && s.y == 0.0)?.height;
    let p = objc2_app_kit::NSEvent::mouseLocation();
    Some((p.x, main_h - p.y))
}

/// Bildschirm vollständig als PNG (Standbild fürs Overlay).
fn freeze(screen: &Screen, cursor: bool) -> Option<PathBuf> {
    let r = Region { x: screen.x, y: screen.y, width: screen.width, height: screen.height, screen: screen.id, fullscreen: true, window_id: None };
    grab(&r, cursor)
}

fn presets_json(s: &crate::settings::Settings) -> Value {
    let free = core::t("Frei", "Freeform");
    let list: Vec<Value> = geom::available_presets(&s.custom_capture_presets, &s.hidden_builtin_presets)
        .iter()
        .map(|p| json!({ "id": p.id(), "name": p.display_name(free) }))
        .collect();
    Value::Array(list)
}

/// Overlay auf allen Bildschirmen zeigen und blockierend auf das Ergebnis warten.
/// `mode`: "area" | "window" | "allInOne". `freeze_screen`: Standbild als Hintergrund.
fn run_overlay(app: &AppHandle, purpose: Purpose, mode: &str, freeze_screen: bool) -> Option<(OverlayResult, Vec<Frozen>)> {
    let screens = core::screens(app);
    if screens.is_empty() {
        return None;
    }
    let s = crate::settings::get();
    let focus = cursor_screen(app, &screens);
    let session = SESSION.fetch_add(1, Ordering::Relaxed);

    // 1. Einfrieren (parallel je Bildschirm).
    let frozen: Vec<Frozen> = if freeze_screen && core::can_record() {
        let cursor = s.screenshot_shows_cursor && purpose == Purpose::Screenshot;
        let handles: Vec<_> = screens.iter().cloned().map(|sc| std::thread::spawn(move || Frozen { path: freeze(&sc, cursor), screen: sc })).collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    } else {
        screens.iter().cloned().map(|screen| Frozen { screen, path: None }).collect()
    };

    // 2. Fensterliste (nur wo Fenster gewählt werden können).
    let wins = windows::list();
    *OVERLAY_WINS.lock().unwrap() = wins.clone();

    // Optionen je Zweck (Capso: Presets nur bei Screenshots, Anpassen-Phase bei Aufnahme/Scrollen/All-in-One).
    let presets_on = s.capture_presets_enabled && purpose == Purpose::Screenshot;
    // Capso: Scroll-Aufnahme startet direkt nach dem Auswählen (kein zusätzlicher Bestätigungsschritt).
    let adjust = mode == "allInOne" || purpose == Purpose::Recording;
    let can_last_rec = matches!(s.last_recording_area, Some(crate::settings::StoredSelection::Area { width, height, screen, .. })
        if width > 5.0 && height > 5.0 && screens.iter().any(|x| x.id == screen));
    let confirm = match purpose {
        Purpose::Recording => core::t("Aufnehmen", "Record"),
        Purpose::Scroll => core::t("Scroll-Aufnahme starten", "Start Scrolling Capture"),
        _ => core::t("Aufnehmen", "Capture"),
    };

    let cursor = cursor_global(&screens);
    let (tx, rx) = mpsc::channel();
    let mut labels = vec![];
    for (i, f) in frozen.iter().enumerate() {
        let sc = &f.screen;
        let label = format!("select-{i}");
        let local_wins: Vec<Value> = wins
            .iter()
            .filter_map(|w| {
                let r = geom::Rect::new(w.x, w.y, w.width, w.height).intersect(&geom::Rect::new(sc.x, sc.y, sc.width, sc.height))?;
                Some(json!({ "id": w.id, "x": w.x - sc.x, "y": w.y - sc.y, "width": w.width, "height": w.height,
                    "visible": [r.x - sc.x, r.y - sc.y, r.w, r.h], "app": w.app, "title": w.title, "layer": w.layer }))
            })
            .collect();
        let data = json!({
            "session": session,
            "mode": mode,
            "purpose": purpose,
            "screen": sc,
            "frozen": f.path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "windows": local_wins,
            "presetsEnabled": presets_on,
            "presets": presets_json(&s),
            "preset": s.capture_preset,
            "squareKey": s.square_center_lock_key,
            "magnifier": s.show_magnifier,
            "adjust": adjust,
            "confirmLabel": confirm,
            "focused": i == focus,
            // ⇧-Klick sammelt mehrere Fenster (nicht bei Aufnahme-Auswahl).
            "multiWindow": purpose == Purpose::Screenshot,
            "canLastArea": can_last_rec,
            "cursor": cursor.filter(|c| geom::Rect::new(sc.x, sc.y, sc.width, sc.height).contains(*c)).map(|c| (c.0 - sc.x, c.1 - sc.y)),
        });
        let Some(w) = core::window(
            app,
            &label,
            "select",
            Win { visible: false, focus: false, w: sc.width, h: sc.height, pos: Some((sc.x, sc.y)), level: Some(core::LEVEL_OVERLAY), ..Default::default() },
            data,
        ) else {
            continue;
        };
        let _ = w.set_position(LogicalPosition::new(sc.x, sc.y));
        let _ = w.set_size(LogicalSize::new(sc.width, sc.height));
        labels.push(label);
    }
    if labels.is_empty() {
        return None;
    }
    let focus_label = format!("select-{focus}");
    *OVERLAY.lock().unwrap() = Some(OverlayState { session, tx, labels: labels.clone(), focus_label: focus_label.clone() });

    // Rückfall, falls das Frontend „bereit“ nicht meldet (z. B. erstes Laden dauert).
    {
        let app = app.clone();
        let labels = labels.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(900));
            if OVERLAY.lock().unwrap().as_ref().is_some_and(|o| o.session == session) {
                for l in &labels {
                    if let Some(w) = app.get_webview_window(l) {
                        if !w.is_visible().unwrap_or(false) {
                            let _ = w.show();
                            if *l == focus_label {
                                let _ = w.set_focus();
                            }
                        }
                    }
                }
            }
        });
    }

    let res = rx.recv().ok();
    *OVERLAY.lock().unwrap() = None;
    for l in &labels {
        if let Some(w) = app.get_webview_window(l) {
            let _ = w.hide();
        }
    }
    let res = res.filter(|r| r.kind != "cancel" && !r.kind.is_empty())?;
    Some((res, frozen))
}

/// Overlay-Ergebnis → globaler Bereich.
fn to_region(r: &OverlayResult, frozen: &[Frozen]) -> Option<Region> {
    let sc = &frozen.get(r.screen as usize)?.screen;
    if r.kind == "windows" {
        let wins = OVERLAY_WINS.lock().unwrap();
        let u = r.window_ids.iter().filter_map(|id| wins.iter().find(|w| w.id == *id)).map(|w| geom::Rect::new(w.x, w.y, w.width, w.height)).reduce(|a, b| {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            geom::Rect::new(x, y, a.max_x().max(b.max_x()) - x, a.max_y().max(b.max_y()) - y)
        })?;
        return Some(Region { x: u.x, y: u.y, width: u.w, height: u.h, screen: sc.id, window_id: None, fullscreen: false });
    }
    // Aufnahme-Modusleiste „Letzter Bereich“ (R).
    if r.kind == "lastArea" {
        let crate::settings::StoredSelection::Area { x, y, width, height, screen } = crate::settings::get().last_recording_area? else { return None };
        return Some(Region { x, y, width, height, screen, ..Default::default() });
    }
    Some(match r.kind.as_str() {
        "fullscreen" => Region { x: sc.x, y: sc.y, width: sc.width, height: sc.height, screen: sc.id, fullscreen: true, window_id: None },
        "window" => Region { x: r.x + sc.x, y: r.y + sc.y, width: r.width, height: r.height, screen: sc.id, window_id: r.window_id, fullscreen: false },
        _ => Region { x: (r.x + sc.x).round(), y: (r.y + sc.y).round(), width: r.width.round().max(1.0), height: r.height.round().max(1.0), screen: sc.id, ..Default::default() },
    })
}

/// Bild zum Ergebnis: Bereich/Vollbild aus dem Standbild ausschneiden, Fenster live aufnehmen.
fn image_for(r: &OverlayResult, region: &Region, frozen: &[Frozen]) -> Option<PathBuf> {
    if let Some(data) = r.data_url.as_deref() {
        if let Some(p) = save_data_url(data) {
            return Some(p);
        }
    }
    let cursor = crate::settings::get().screenshot_shows_cursor;
    if r.kind == "windows" {
        if let Some(p) = multi_window_image(&r.window_ids, cursor) {
            return Some(p);
        }
    }
    if let Some(id) = region.window_id {
        // Menüs, Popover und die Menüleiste (Ebene > 0) – oder inzwischen geschlossene Fenster – aus dem Standbild.
        let layer = OVERLAY_WINS.lock().unwrap().iter().find(|w| w.id == id).map(|w| w.layer);
        let live = layer == Some(0) && windows::bounds(id).is_some();
        if live {
            return window_image(region, frozen, cursor);
        }
        let sc = frozen.iter().find(|f| f.path.is_some() && geom::Rect::new(f.screen.x, f.screen.y, f.screen.width, f.screen.height).contains(geom::Rect::new(region.x, region.y, region.width, region.height).mid()));
        if let Some(p) = sc.and_then(|f| crop_frozen(f, region)) {
            return Some(p);
        }
        return grab(region, cursor);
    }
    let f = frozen.get(r.screen as usize);
    if let Some((f, src)) = f.and_then(|f| f.path.as_ref().map(|p| (f, p))) {
        if region.fullscreen {
            let p = core::temp_file("png");
            if std::fs::copy(src, &p).is_ok() {
                return Some(p);
            }
        }
        let (pw, _) = image::image_dimensions(src).unwrap_or(((f.screen.width * f.screen.scale) as u32, 0));
        let k = pw as f64 / f.screen.width;
        let (x, y) = (region.x - f.screen.x, region.y - f.screen.y);
        if let Some(p) = post::crop_file(src, (x * k).round() as u32, (y * k).round() as u32, (region.width * k).round() as u32, (region.height * k).round() as u32) {
            return Some(p);
        }
    }
    std::thread::sleep(Duration::from_millis(60));
    grab(region, cursor)
}

/// Globalen Bereich (auf den Bildschirm begrenzt) aus einem Standbild ausschneiden.
fn crop_frozen(f: &Frozen, region: &Region) -> Option<PathBuf> {
    let src = f.path.as_ref()?;
    let sc = geom::Rect::new(f.screen.x, f.screen.y, f.screen.width, f.screen.height);
    let r = geom::Rect::new(region.x, region.y, region.width, region.height).intersect(&sc)?;
    let (pw, _) = image::image_dimensions(src).ok()?;
    let k = pw as f64 / f.screen.width;
    post::crop_file(src, ((r.x - sc.x) * k).round() as u32, ((r.y - sc.y) * k).round() as u32, (r.w * k).round() as u32, (r.h * k).round() as u32)
}

/// Fensteraufnahme immer ohne Systemschatten; mit „Fensterschatten“ wie Capso auf Milchglas
/// (weichgezeichneter, gesättigter Schreibtisch dahinter, ~4 % Rand, runde Ecken, weicher Schatten).
fn window_image(region: &Region, frozen: &[Frozen], cursor: bool) -> Option<PathBuf> {
    let p = grab(region, cursor)?;
    if !crate::settings::get().capture_window_shadow {
        return Some(p);
    }
    let Some(win) = post::load(&p) else { return Some(p) };
    let k = (win.width() as f64 / region.width.max(1.0)).max(0.5);
    let pad_px = post::frosted_padding(win.width(), win.height()).round();
    let (ow, oh) = (win.width() + 2 * pad_px as u32, win.height() + 2 * pad_px as u32);
    let pad = pad_px / k;
    let outer = geom::Rect::new(region.x - pad, region.y - pad, region.width + 2.0 * pad, region.height + 2.0 * pad);
    // Schreibtisch dahinter: aus dem Standbild (Ränder außerhalb des Bildschirms werden fortgesetzt), sonst live.
    let from_frozen = frozen.iter().find(|f| f.path.is_some() && geom::Rect::new(f.screen.x, f.screen.y, f.screen.width, f.screen.height).contains(outer.mid())).and_then(|f| {
        let img = post::load(f.path.as_ref()?)?;
        let fk = img.width() as f64 / f.screen.width;
        let mut bg = image::RgbaImage::new(ow, oh);
        for (x, y, px) in bg.enumerate_pixels_mut() {
            let gx = outer.x + x as f64 / k;
            let gy = outer.y + y as f64 / k;
            let sx = (((gx - f.screen.x) * fk) as i64).clamp(0, img.width() as i64 - 1) as u32;
            let sy = (((gy - f.screen.y) * fk) as i64).clamp(0, img.height() as i64 - 1) as u32;
            *px = *img.get_pixel(sx, sy);
        }
        Some(bg)
    });
    let backdrop = from_frozen.or_else(|| {
        let r = Region { x: outer.x.round(), y: outer.y.round(), width: outer.w.round(), height: outer.h.round(), ..Default::default() };
        let live = post::load(&grab(&r, false)?)?;
        Some(image::imageops::resize(&live, ow, oh, image::imageops::FilterType::Triangle))
    });
    let Some(backdrop) = backdrop else { return Some(p) };
    let out = post::frosted_window(&win, &backdrop);
    let dst = core::temp_file("png");
    if post::save_png(&out, &dst) {
        Some(dst)
    } else {
        Some(p)
    }
}

/// Mehrere Fenster (⇧-Klick) an ihren Bildschirmpositionen zu einem Bild zusammensetzen
/// (Capso `MultiWindowCompositor`): transparente Fläche = Vereinigung der Rahmen, hinten zuerst.
fn multi_window_image(ids: &[u32], cursor: bool) -> Option<PathBuf> {
    let infos: Vec<windows::WindowInfo> = {
        let wins = OVERLAY_WINS.lock().unwrap();
        ids.iter().filter_map(|id| wins.iter().find(|w| w.id == *id).cloned()).collect()
    };
    if infos.is_empty() {
        return None;
    }
    let shots: Vec<(windows::WindowInfo, image::RgbaImage)> = infos
        .into_iter()
        .filter_map(|w| {
            let r = Region { x: w.x, y: w.y, width: w.width, height: w.height, window_id: Some(w.id), ..Default::default() };
            let p = grab(&r, cursor)?;
            let img = post::load(&p)?;
            Some((w, img))
        })
        .collect();
    let (x0, y0) = shots.iter().fold((f64::MAX, f64::MAX), |a, (w, _)| (a.0.min(w.x), a.1.min(w.y)));
    let (x1, y1) = shots.iter().fold((f64::MIN, f64::MIN), |a, (w, _)| (a.0.max(w.x + w.width), a.1.max(w.y + w.height)));
    let k = shots.iter().map(|(w, img)| img.width() as f64 / w.width.max(1.0)).fold(1.0, f64::max);
    let mut canvas = image::RgbaImage::new(((x1 - x0) * k).round().max(1.0) as u32, ((y1 - y0) * k).round().max(1.0) as u32);
    for (w, img) in shots.iter().rev() {
        let (tw, th) = ((w.width * k).round() as u32, (w.height * k).round() as u32);
        let scaled = if (img.width(), img.height()) != (tw, th) { image::imageops::resize(img, tw.max(1), th.max(1), image::imageops::FilterType::Triangle) } else { img.clone() };
        image::imageops::overlay(&mut canvas, &scaled, ((w.x - x0) * k).round() as i64, ((w.y - y0) * k).round() as i64);
    }
    let dst = core::temp_file("png");
    post::save_png(&canvas, &dst).then_some(dst)
}

/// PNG-Data-URL (`data:image/png;base64,…`) als Temp-Datei.
pub fn save_data_url(data: &str) -> Option<PathBuf> {
    let b64 = data.split_once(',').map(|(_, b)| b).unwrap_or(data);
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let p = core::temp_file("png");
    std::fs::write(&p, bytes).ok()?;
    Some(p)
}

fn overlay_options(purpose: Purpose) -> (&'static str, bool) {
    let s = crate::settings::get();
    match purpose {
        Purpose::Screenshot | Purpose::Ocr | Purpose::Translate => ("area", true),
        Purpose::Recording => (if s.recording_selects_window { "window" } else { "area" }, false),
        Purpose::Scroll => ("area", false),
    }
}

/// Auswahl-Overlay zeigen und blockierend auf das Ergebnis warten (None = abgebrochen).
pub fn select_region(app: &AppHandle, purpose: Purpose) -> Option<Region> {
    let (mode, freeze) = overlay_options(purpose);
    let (r, frozen) = run_overlay(app, purpose, mode, freeze)?;
    to_region(&r, &frozen)
}

/// Auswahl + Bild (aus dem Standbild, d. h. genau das, was beim Auswählen zu sehen war). Für OCR/Übersetzen.
pub fn select_and_grab(app: &AppHandle, purpose: Purpose) -> Option<(Region, PathBuf)> {
    let (mode, freeze) = overlay_options(purpose);
    let (r, frozen) = run_overlay(app, purpose, mode, freeze)?;
    let region = to_region(&r, &frozen)?;
    let p = image_for(&r, &region, &frozen)?;
    Some((region, p))
}

fn finish_overlay(r: OverlayResult) {
    let g = OVERLAY.lock().unwrap();
    if let Some(o) = g.as_ref() {
        if r.session == 0 || r.session == o.session {
            let _ = o.tx.send(r);
        }
    }
}

/// Bereich als PNG in den Temp-Ordner aufnehmen (ohne Ton). `cursor`: Mauszeiger mit aufnehmen.
pub fn grab(r: &Region, cursor: bool) -> Option<PathBuf> {
    let path = core::temp_file("png");
    let mut c = Command::new("/usr/sbin/screencapture");
    c.arg("-x");
    if cursor {
        c.arg("-C");
    }
    if let Some(id) = r.window_id {
        // Immer ohne Systemschatten (Capso erzeugt den Hintergrund selbst, siehe `window_image`).
        c.arg(format!("-l{id}"));
        c.arg("-o");
    } else if r.fullscreen && r.width <= 0.0 {
        c.arg(format!("-D{}", r.screen + 1));
    } else {
        c.arg(format!("-R{},{},{},{}", r.x, r.y, r.width.max(1.0), r.height.max(1.0)));
    }
    let ok = c.arg(&path).output().map(|o| o.status.success()).unwrap_or(false);
    (ok && path.exists()).then_some(path)
}

fn source_for(r: &Region) -> &'static str {
    if r.window_id.is_some() {
        " (Window)"
    } else if r.fullscreen {
        " (Fullscreen)"
    } else {
        " (Area)"
    }
}

// ───────────────────────────── Aktionen ─────────────────────────────

/// Bereich wählen → aufnehmen → `deliver`.
pub fn area(app: &AppHandle, intent: Intent) {
    capture_with_overlay(app, "area", intent);
}

/// Fenster wählen (Overlay im Fenstermodus) → aufnehmen → `deliver`.
pub fn window(app: &AppHandle) {
    capture_with_overlay(app, "window", Intent::Save);
}

fn capture_with_overlay(app: &AppHandle, mode: &str, intent: Intent) {
    // Capso `captureAreaAndShare`: ohne Cloud-Konfiguration gleich die Cloud-Einstellungen öffnen.
    if intent == Intent::Share && !crate::share::is_configured() {
        crate::prefs::open(app, Some("cloud"));
        return;
    }
    if !guard() {
        return;
    }
    remember_source_app();
    crate::quick::hide_for_capture(app, true);
    let res = run_overlay(app, Purpose::Screenshot, mode, true);
    crate::quick::hide_for_capture(app, false);
    release();
    let Some((r, frozen)) = res else { return };
    let Some(region) = to_region(&r, &frozen) else { return };
    if let Some(p) = image_for(&r, &region, &frozen) {
        if r.kind != "windows" {
            remember(app, &region);
        }
        let src = if r.kind == "windows" { " (Window)" } else { source_for(&region) };
        deliver(app, p, intent, src, Some(region));
    }
}

fn cursor_screen_region(app: &AppHandle) -> Region {
    let screens = core::screens(app);
    let i = cursor_screen(app, &screens);
    match screens.get(i) {
        Some(s) => Region { x: s.x, y: s.y, width: s.width, height: s.height, screen: s.id, fullscreen: true, window_id: None },
        None => Region { fullscreen: true, ..Default::default() },
    }
}

pub fn fullscreen(app: &AppHandle) {
    if !guard() {
        return;
    }
    remember_source_app();
    crate::quick::hide_for_capture(app, true);
    let r = cursor_screen_region(app);
    let p = grab(&r, crate::settings::get().screenshot_shows_cursor);
    crate::quick::hide_for_capture(app, false);
    release();
    if let Some(p) = p {
        remember(app, &r);
        deliver(app, p, Intent::Save, " (Fullscreen)", Some(r));
    }
}

/// All-in-One (CleanShot-Stil): eingefrorener Bildschirm, Auswahl mit Griffen, schwebende Werkzeugleiste
/// mit Modi (Bereich, Vollbild, Fenster, Scrollen, Timer, OCR, Aufnahme) und Aktionen.
pub fn all_in_one(app: &AppHandle) {
    if !guard() {
        return;
    }
    remember_source_app();
    crate::quick::hide_for_capture(app, true);
    let res = run_overlay(app, Purpose::Screenshot, "allInOne", true);
    crate::quick::hide_for_capture(app, false);
    release();
    let Some((r, frozen)) = res else { return };
    // kind "none" = Modus-Knopf ohne Auswahl (z. B. Timer/Scrollen direkt aus der Leiste).
    let region = if r.kind == "none" { None } else { to_region(&r, &frozen) };
    // Capso: die Auswahl im All-in-One wird sofort als „letzter Bereich“ gemerkt (nie Fenster).
    if let Some(region) = region.as_ref().filter(|_| r.kind != "windows") {
        remember(app, region);
    }
    let action = r.action.clone().unwrap_or_else(|| "copy".into());
    match action.as_str() {
        "scroll" => return crate::scroll::start_region(app, region),
        // Capso `startRecordingFlow(withSelectedArea:)`: bereits gewählten Bereich direkt aufnehmen.
        "record" => {
            return match region {
                Some(reg) if !reg.fullscreen && reg.window_id.is_none() => crate::record::start_region(app, reg),
                _ => crate::record::start(app, r.kind == "fullscreen"),
            }
        }
        "ocrMode" => return crate::ocr::capture_text(app),
        "timer" => {
            if let Some(region) = region {
                run_timer(app, region);
            } else {
                self_timer(app);
            }
            return;
        }
        _ => {}
    }
    let Some(region) = region else { return };
    let Some(p) = image_for(&r, &region, &frozen) else { return };
    let (intent, force) = match action.as_str() {
        "copy" => (Intent::Clipboard, false),
        "save" => (Intent::Save, true),
        "pin" => (Intent::Pin, false),
        "ocr" => (Intent::Ocr, false),
        "share" => (Intent::Share, false),
        "translate" => (Intent::Translate, false),
        "annotate" | "editor" => (Intent::Annotate, false),
        _ => (Intent::Save, false),
    };
    let src = if r.kind == "windows" { " (Window)" } else { source_for(&region) };
    deliver_with(app, p, intent, src, Some(region), force);
}

/// Capso `replayLastCapture`: letzten Bereich / Vollbild ohne Overlay erneut aufnehmen.
/// Ohne gespeicherten Bereich oder wenn der Bildschirm fehlt: nichts tun.
pub fn last_area(app: &AppHandle) {
    let Some(sel) = crate::settings::get().last_capture_selection else { return };
    let screens = core::screens(app);
    let r = match sel {
        crate::settings::StoredSelection::Area { x, y, width, height, screen } => {
            if !screens.iter().any(|s| s.id == screen) {
                return;
            }
            Region { x, y, width, height, screen, ..Default::default() }
        }
        crate::settings::StoredSelection::Fullscreen { screen } => match screens.iter().find(|s| s.id == screen) {
            Some(s) => Region { x: s.x, y: s.y, width: s.width, height: s.height, screen, fullscreen: true, window_id: None },
            None => return,
        },
        // Fensteraufnahmen werden nie als „letzter Bereich“ gemerkt (Altdaten ignorieren).
        crate::settings::StoredSelection::Window { .. } => return,
    };
    if !guard() {
        return;
    }
    remember_source_app();
    crate::quick::hide_for_capture(app, true);
    let p = grab(&r, crate::settings::get().screenshot_shows_cursor);
    crate::quick::hide_for_capture(app, false);
    release();
    if let Some(p) = p {
        deliver(app, p, Intent::Save, source_for(&r), Some(r));
    }
}

/// Selbstauslöser: Bereich wählen, dann Countdown-HUD (1–60 s), danach live aufnehmen.
pub fn self_timer(app: &AppHandle) {
    if !guard() {
        return;
    }
    remember_source_app();
    crate::quick::hide_for_capture(app, true);
    let res = run_overlay(app, Purpose::Screenshot, "area", true);
    crate::quick::hide_for_capture(app, false);
    release();
    if let Some(region) = res.and_then(|(r, f)| to_region(&r, &f)) {
        remember(app, &region);
        run_timer(app, region);
    }
}

/// Rahmen um einen Bereich (klick-durchlässig, nicht im Bild), z. B. während Countdown / Scroll-Aufnahme.
pub fn show_border(app: &AppHandle, r: &Region, style: &str) {
    let pad = 4.0;
    let Some(w) = core::window(
        app,
        "capture-border",
        "capture-border",
        Win { visible: true, w: r.width + 2.0 * pad, h: r.height + 2.0 * pad, pos: Some((r.x - pad, r.y - pad)), level: Some(core::LEVEL_POPUP), ..Default::default() },
        json!({ "style": style }),
    ) else {
        return;
    };
    let _ = w.set_position(LogicalPosition::new(r.x - pad, r.y - pad));
    let _ = w.set_size(LogicalSize::new(r.width + 2.0 * pad, r.height + 2.0 * pad));
    let _ = w.set_ignore_cursor_events(true);
    core::exclude_from_capture(&w, true);
    let _ = w.show();
}

pub fn hide_border(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("capture-border") {
        let _ = w.hide();
    }
}

const TIMER_HUD: (f64, f64) = (120.0, 120.0);
const TINK: &str = "/System/Library/Sounds/Tink.aiff";

/// Countdown-HUD zeigen und danach den Bereich aufnehmen.
pub fn run_timer(app: &AppHandle, region: Region) {
    if !guard() {
        return;
    }
    let s = crate::settings::get();
    // Capso `selfTimerDurationSeconds`: beliebiger Wert 1–60 s.
    let total: u32 = timer_seconds(s.self_timer_seconds);
    TIMER_CANCEL.store(false, Ordering::SeqCst);
    let screens = core::screens(app);
    let sc = screens.iter().find(|x| x.id == region.screen).or(screens.first()).cloned();
    // Position: gemerkt (auf sichtbare Fläche begrenzt) oder oben mittig im Arbeitsbereich.
    let pos = match (s.self_timer_hud_position, &sc) {
        (Some((x, y)), _) if screens.iter().any(|s| geom::Rect::new(s.work[0], s.work[1], s.work[2], s.work[3]).contains((x + 10.0, y + 10.0))) => (x, y),
        (_, Some(sc)) => (sc.work[0] + (sc.work[2] - TIMER_HUD.0) / 2.0, sc.work[1] + 80.0),
        _ => (200.0, 200.0),
    };
    if !region.fullscreen && region.window_id.is_none() {
        show_border(app, &region, "timer");
    }
    let hud = core::window(
        app,
        "selftimer",
        "selftimer",
        Win { w: TIMER_HUD.0, h: TIMER_HUD.1, pos: Some(pos), focus: true, level: Some(core::LEVEL_POPUP), ..Default::default() },
        json!({ "total": total, "remaining": total }),
    );
    if let Some(w) = &hud {
        let _ = w.set_position(LogicalPosition::new(pos.0, pos.1));
        core::exclude_from_capture(w, true);
        let _ = w.show();
        let _ = w.set_focus();
    }
    crate::actions::register_temp(app, "Escape", |_| TIMER_CANCEL.store(true, Ordering::SeqCst));
    let started = std::time::Instant::now();
    let mut remaining = total;
    let _ = app.emit_to("selftimer", "timer-tick", json!({ "remaining": remaining, "total": total }));
    while remaining > 0 && !TIMER_CANCEL.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(40));
        let left = total.saturating_sub(started.elapsed().as_secs() as u32);
        if left != remaining {
            remaining = left;
            let _ = app.emit_to("selftimer", "timer-tick", json!({ "remaining": remaining, "total": total }));
            if remaining > 0 && s.self_timer_tick_sound {
                core::play_sound(TINK);
            }
        }
    }
    crate::actions::unregister_temp(app, "Escape");
    if let Some(w) = &hud {
        let _ = w.hide();
    }
    hide_border(app);
    release();
    if TIMER_CANCEL.load(Ordering::SeqCst) {
        return;
    }
    std::thread::sleep(Duration::from_millis(120));
    crate::quick::hide_for_capture(app, true);
    let p = grab(&region, s.screenshot_shows_cursor);
    crate::quick::hide_for_capture(app, false);
    if let Some(p) = p {
        deliver(app, p, Intent::Save, source_for(&region), Some(region));
    }
}

/// Selbstauslöser-Dauer auf 1–60 s begrenzen.
pub fn timer_seconds(v: u32) -> u32 {
    v.clamp(1, 60)
}

/// Bereich/Vollbild als „letzten Bereich“ merken (Capso `lastCaptureSelection`); Fenster nie.
pub fn remember(app: &AppHandle, r: &Region) {
    let sel = if r.window_id.is_some() {
        return;
    } else if r.fullscreen {
        crate::settings::StoredSelection::Fullscreen { screen: r.screen }
    } else {
        crate::settings::StoredSelection::Area { x: r.x, y: r.y, width: r.width, height: r.height, screen: r.screen }
    };
    crate::settings::update(app, |s| s.last_capture_selection = Some(sel));
}

// ───────────────────────────── Nachbearbeitung ─────────────────────────────

/// Zeitstempel und Auto-Beautify anwenden (falls aktiv). Liefert ggf. einen neuen PNG-Pfad.
fn postprocess(app: &AppHandle, path: &Path, s: &crate::settings::Settings, region: Option<&Region>) -> PathBuf {
    if !s.screenshot_timestamp_enabled && !s.auto_beautify_screenshots {
        return path.to_path_buf();
    }
    let Some(mut img) = post::load(path) else { return path.to_path_buf() };
    let scale = region
        .filter(|r| r.width > 0.0 && r.window_id.is_none())
        .map(|r| img.width() as f64 / r.width)
        .or_else(|| core::screens(app).first().map(|s| s.scale))
        .unwrap_or(1.0)
        .clamp(0.5, 4.0);
    if s.screenshot_timestamp_enabled {
        post::watermark(&mut img, s, scale);
    }
    if s.auto_beautify_screenshots {
        img = post::beautify(&img, &post::Beautify::from_value(&s.beautify_style), scale);
    }
    let out = core::temp_file("png");
    if post::save_png(&img, &out) {
        out
    } else {
        path.to_path_buf()
    }
}

/// Zwischenablage gemäß Einstellungen (Capso `ScreenshotClipboard.copy`): Bild im Zwischenablage-Format
/// oder Dateipfad – dann die gesicherte Datei bzw. eine benannte Temp-Datei (nie der Zielordner).
fn copy_per_settings(image: &Path, converted: &Path, saved: Option<&Path>, name: &str, s: &crate::settings::Settings) {
    if s.screenshot_clipboard_content == "filePath" {
        let file = match saved.filter(|p| p.is_file()) {
            Some(p) => p.to_path_buf(),
            None => {
                let dir = core::temp_dir().join("clipboard").join(format!("{}", chrono::Local::now().timestamp_millis()));
                let _ = std::fs::create_dir_all(&dir);
                let dst = dir.join(name);
                if std::fs::copy(converted, &dst).is_ok() {
                    dst
                } else {
                    converted.to_path_buf()
                }
            }
        };
        core::clipboard::copy_file(&file);
    } else {
        post::copy_image_as(image, &s.screenshot_clipboard_format);
    }
}

/// Aufnahmeart für den Verlauf aus dem `source`-Etikett (" (Area)", " (Window)", " (Fullscreen)", " (Scrolling)").
fn mode_for(source: &str, region: Option<&Region>) -> &'static str {
    let l = source.to_lowercase();
    if l.contains("scroll") {
        "scroll"
    } else if l.contains("window") || region.is_some_and(|r| r.window_id.is_some()) {
        "window"
    } else if l.contains("full") || region.is_some_and(|r| r.fullscreen) {
        "fullscreen"
    } else {
        "area"
    }
}

/// Gemeinsame Nachbearbeitung jeder Bildaufnahme (Screenshot, Scroll-Aufnahme, …).
/// `path` liegt im Temp-Ordner. `source` = Art der Aufnahme (" (Area)", " (Window)", " (Fullscreen)", " (Scrolling)");
/// der Dateiname nutzt die Quell-App ({source} = " - App", {app}, {window}), siehe `remember_source_app`.
/// Spielt den Auslöser-Ton (außer beim Annotieren).
pub fn deliver(app: &AppHandle, path: PathBuf, intent: Intent, source: &str, region: Option<Region>) {
    deliver_with(app, path, intent, source, region, false);
}

/// Wie `deliver`; `force_save` = ausdrücklich „Sichern“ (Datei wird immer in den Zielordner geschrieben).
pub fn deliver_with(app: &AppHandle, path: PathBuf, intent: Intent, source: &str, region: Option<Region>, force_save: bool) {
    let s = crate::settings::get();
    let mode = mode_for(source, region.as_ref());
    let (src_app, src_win) = take_source(region.as_ref());
    let captured_at = chrono::Local::now().timestamp_millis();
    // Capso `playsShutterSound`: nicht beim direkten Annotieren.
    if intent != Intent::Annotate {
        core::shutter();
    }
    let processed = match intent {
        Intent::Ocr | Intent::Translate | Intent::Annotate => path.clone(),
        _ => postprocess(app, &path, &s, region.as_ref()),
    };
    let name = core::file_name_with(&s.screenshot_filename_template, src_app.as_deref(), src_win.as_deref(), post::ext_for(&s.screenshot_format));
    // Capso `savesOriginalCaptureToHistory`: nicht bei Annotieren und Anheften.
    let hist = |p: &Path| crate::history::add_meta(app, p, "screenshot", mode, src_app.as_deref(), src_win.as_deref());
    let auto_upload = s.cloud_share_auto_upload && crate::share::is_configured();
    match intent {
        Intent::Clipboard => {
            // Nur Kopieren: kein Hinweis, nichts im Zielordner.
            let out = post::convert(&processed, &s.screenshot_format, s.screenshot_quality);
            copy_per_settings(&processed, &out, None, &name, &s);
            hist(&out);
        }
        Intent::Annotate => {
            let meta = json!({ "sourceApp": src_app, "windowTitle": src_win, "capturedAt": captured_at });
            crate::editor::open_with_meta(app, &path, meta);
        }
        Intent::Pin => crate::pin::pin_image(app, &processed, region.map(|r| [r.x, r.y, r.width, r.height])),
        Intent::Share => {
            hist(&processed);
            crate::share::upload_and_copy(app, &processed);
        }
        // Capso: OCR-Aktion öffnet Visual OCR; das Original kommt in den Verlauf.
        Intent::Ocr => {
            hist(&path);
            crate::ocr::visual(app, &path);
        }
        Intent::Translate => crate::translate::translate_image(app, &path, region),
        Intent::Save => {
            let out = post::convert(&processed, &s.screenshot_format, s.screenshot_quality);
            let defaults = !force_save;
            let to_clip_image = defaults && s.screenshot_auto_copy && s.screenshot_clipboard_content != "filePath";
            let to_clip_path = defaults && s.screenshot_auto_copy && s.screenshot_clipboard_content == "filePath";
            if to_clip_image {
                copy_per_settings(&processed, &out, None, &name, &s);
            }
            let saved_path = if s.screenshot_auto_save || force_save {
                let dst = core::unique_path(&core::export_dir(), &name);
                std::fs::copy(&out, &dst).is_ok().then_some(dst)
            } else {
                None
            };
            if to_clip_path {
                copy_per_settings(&processed, &out, saved_path.as_deref(), &name, &s);
            }
            let final_path = saved_path.clone().unwrap_or(out);
            hist(&final_path);
            if force_save {
                core::toast(app, core::t("Gesichert", "Saved"));
            } else if s.screenshot_show_preview {
                // Stapel auf dem Bildschirm der Aufnahme, Einflug vom Bereich; Auto-Upload übernimmt die Karte.
                let item = crate::quick::QuickItem { path: final_path.clone(), kind: "image".into(), saved: saved_path.is_some() };
                crate::quick::show_from(app, item, region.as_ref().map(|r| [r.x, r.y, r.width, r.height]));
            } else if auto_upload {
                // Ohne Vorschau lädt Capso direkt hoch (mit Vorschau übernimmt das die Quick-Access-Karte).
                crate::share::upload_and_copy(app, &final_path);
            }
            // Weder Vorschau noch Auto-Sichern: Capso schreibt nichts in den Zielordner.
        }
    }
}

fn parse_intent(v: &Value) -> Intent {
    serde_json::from_value(v.clone()).unwrap_or(Intent::Save)
}

fn spawn(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    std::thread::spawn(move || f(&app));
}

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    match cmd {
        // Overlay hat das Standbild geladen → Fenster zeigen (und fokussieren, falls Mauszeiger dort).
        "overlayReady" => {
            let session = a["session"].as_u64().unwrap_or(0);
            let focus = OVERLAY.lock().unwrap().as_ref().filter(|o| o.session == session).map(|o| o.focus_label == win.label());
            if let Some(focus) = focus {
                let _ = win.show();
                if focus {
                    let _ = win.set_focus();
                }
            }
            Ok(Value::Null)
        }
        // Mauszeiger ist auf diesen Bildschirm gewechselt → Tastatur hierher.
        "overlayFocus" => {
            if OVERLAY.lock().unwrap().as_ref().is_some_and(|o| o.labels.iter().any(|l| l == win.label())) {
                let _ = win.set_focus();
            }
            Ok(Value::Null)
        }
        "overlayResult" => {
            let r: OverlayResult = serde_json::from_value(a).map_err(|e| e.to_string())?;
            finish_overlay(r);
            Ok(Value::Null)
        }
        // Alte Schnittstelle der Grundversion: {region: Region|null}
        "selectDone" => {
            let r = match serde_json::from_value::<Region>(a["region"].clone()) {
                Ok(r) => OverlayResult { kind: "area".into(), x: r.x, y: r.y, width: r.width, height: r.height, ..Default::default() },
                Err(_) => OverlayResult { kind: "cancel".into(), ..Default::default() },
            };
            finish_overlay(r);
            Ok(Value::Null)
        }
        "area" => {
            let intent = parse_intent(&a["intent"]);
            spawn(app, move |app| area(app, intent));
            Ok(Value::Null)
        }
        "fullscreen" => {
            spawn(app, fullscreen);
            Ok(Value::Null)
        }
        "window" => {
            spawn(app, window);
            Ok(Value::Null)
        }
        "allInOne" => {
            spawn(app, all_in_one);
            Ok(Value::Null)
        }
        "lastArea" => {
            spawn(app, last_area);
            Ok(Value::Null)
        }
        "selfTimer" => {
            spawn(app, self_timer);
            Ok(Value::Null)
        }
        "timerCancel" => {
            TIMER_CANCEL.store(true, Ordering::SeqCst);
            Ok(Value::Null)
        }
        // HUD verschieben (Ziehen im Frontend): {x, y} global logisch; {save:true} merkt die Position.
        "hudMove" => {
            let (x, y) = (a["x"].as_f64().unwrap_or(0.0), a["y"].as_f64().unwrap_or(0.0));
            let _ = win.set_position(LogicalPosition::new(x, y));
            if a["save"].as_bool().unwrap_or(false) && win.label() == "selftimer" {
                crate::settings::update(app, |s| s.self_timer_hud_position = Some((x, y)));
            }
            Ok(Value::Null)
        }
        "windows" => Ok(serde_json::to_value(windows::list()).unwrap_or_default()),
        // Preset-Menü „Presets verwalten…“: Overlay ist schon abgebrochen → Einstellungen „Bildschirmfoto“.
        "managePresets" => {
            spawn(app, |app| {
                std::thread::sleep(Duration::from_millis(250));
                crate::prefs::open(app, Some("screenshot"));
            });
            Ok(Value::Null)
        }
        // Ausschnitt aus einem Standbild (Inline-Annotation): {path, x, y, width, height} in Pixeln → Temp-PNG
        "cropFrozen" => {
            let path = a["path"].as_str().ok_or("path fehlt")?;
            let n = |k: &str| a[k].as_f64().unwrap_or(0.0).max(0.0).round() as u32;
            let p = post::crop_file(Path::new(path), n("x"), n("y"), n("width"), n("height")).ok_or("Zuschneiden fehlgeschlagen")?;
            Ok(json!(p.to_string_lossy()))
        }
        // Fertiges Bild aus einem Frontend nachbearbeiten:
        // {path | dataUrl, intent, source?, region?, forceSave?}
        "deliver" => {
            let path = match (a["path"].as_str(), a["dataUrl"].as_str()) {
                (Some(p), _) => PathBuf::from(p),
                (None, Some(d)) => save_data_url(d).ok_or("Bilddaten ungültig")?,
                _ => return Err("path oder dataUrl fehlt".into()),
            };
            let intent = parse_intent(&a["intent"]);
            let source = a["source"].as_str().unwrap_or("").to_owned();
            let region: Option<Region> = serde_json::from_value(a["region"].clone()).ok();
            let force = a["forceSave"].as_bool().unwrap_or(false);
            spawn(app, move |app| deliver_with(app, path, intent, &source, region, force));
            Ok(Value::Null)
        }
        _ => Err(format!("capture: unbekannter Befehl {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_range() {
        assert_eq!(timer_seconds(0), 1);
        assert_eq!(timer_seconds(7), 7);
        assert_eq!(timer_seconds(90), 60);
    }

    #[test]
    fn history_mode() {
        assert_eq!(mode_for(" (Scrolling)", None), "scroll");
        assert_eq!(mode_for(" (Window)", None), "window");
        assert_eq!(mode_for(" (Fullscreen)", None), "fullscreen");
        assert_eq!(mode_for(" (Area)", None), "area");
        assert_eq!(mode_for("", Some(&Region { window_id: Some(3), ..Default::default() })), "window");
    }

    #[test]
    fn frosted_size() {
        let win = image::RgbaImage::from_pixel(200, 100, image::Rgba([255, 255, 255, 255]));
        let pad = post::frosted_padding(200, 100).round() as u32;
        let bg = image::RgbaImage::from_pixel(200 + 2 * pad, 100 + 2 * pad, image::Rgba([0, 80, 200, 255]));
        let out = post::frosted_window(&win, &bg);
        assert_eq!(out.dimensions(), (200 + 2 * pad, 100 + 2 * pad));
        // Mitte = Fenster, Ecke = (bläulicher) Hintergrund, Loch wird nicht vom Fenster eingefärbt.
        assert_eq!(out.get_pixel(pad + 100, pad + 50).0, [255, 255, 255, 255]);
        let c = out.get_pixel(2, 2).0;
        assert!(c[2] > c[0] && c[3] == 255, "{c:?}");
    }
}
