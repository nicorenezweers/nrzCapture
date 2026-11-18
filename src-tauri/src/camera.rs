//! Kamera-Bild-im-Bild (4 Formen, 3 Größen + Drag-Resize, Ecken-Einrasten, Spiegeln, Präsentationsmodus,
//! Fade-on-Hover, Click-Through). Port von Capso `CameraPiPWindow` + `CameraPiPView` + `CameraPiPPlacement`
//! + `CameraMenuBuilder`.
//!
//! Aufbau: Fenster `camera` (Route `camera`), randlos/transparent, Level „schwebend“, auf allen Spaces.
//! Das Webcam-Bild kommt per `getUserMedia` direkt im WebView (`src/camera.rs`) – kein AVFoundation nötig.
//! Die Kamera ist absichtlich NICHT von der Bildschirmaufnahme ausgeschlossen (sie soll im Video erscheinen).
//!
//! WKWebView-Kamerafreigabe (geprüft in wry 0.57, `wkwebview/class/wry_web_view_ui_delegate.rs`):
//! wry implementiert `webView:requestMediaCapturePermissionForOrigin:…:decisionHandler:` und antwortet ohne
//! eigenen Permission-Handler (Tauri setzt keinen) mit `WKPermissionDecision::Grant`. Es gibt also keinen
//! zusätzlichen WebKit-Dialog; macOS fragt nur einmal über TCC nach dem Kamerazugriff. Dafür muss
//! `NSCameraUsageDescription` (und für Mikrofon `NSMicrophoneUsageDescription`) in `Info.plist` stehen –
//! ist ergänzt. Bei Hardened Runtime zusätzlich die Entitlements `com.apple.security.device.camera` /
//! `com.apple.security.device.audio-input` setzen, sonst liefert `getUserMedia` `NotAllowedError`.
//!
//! Koordinaten: global logisch, Ursprung oben links (wie Tauri). Capsos Cocoa-Logik (Ursprung unten links)
//! ist entsprechend gespiegelt.
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow, WindowEvent,
};

use crate::core::{self, t, Win};

const LABEL: &str = "camera";
const PRESENTATION_LEVEL: isize = 26; // NSStatusWindowLevel (25) + 1, wie Capso

// ───────────────────────────── Geometrie (reine Logik, Port von CameraPiPPlacement) ─────────────────────────────

/// Rechteck `[x, y, w, h]` in globalen logischen Punkten, Ursprung oben links.
pub type Rect = [f64; 4];

pub mod geo {
    use super::Rect;

    pub const DEFAULT_SCREEN_MARGIN: f64 = 32.0;
    pub const RECORDING_BOTTOM_OFFSET: f64 = 20.0;
    pub const VISIBLE_MARGIN: f64 = 8.0;
    pub const FADE_HOVER_ALPHA: f64 = 0.18;
    pub const FADE_FULL_ALPHA: f64 = 1.0;
    pub const SNAP_RADIUS: f64 = 60.0;
    pub const SNAP_MARGIN: f64 = 16.0;
    pub const MIN_SHORTER: f64 = 80.0;
    pub const MAX_SHORTER: f64 = 400.0;

    /// Seitenverhältnis Breite/Höhe der Form.
    pub fn aspect(shape: &str) -> f64 {
        match shape {
            "landscape" => 16.0 / 9.0,
            "portrait" => 9.0 / 16.0,
            _ => 1.0, // circle, square
        }
    }

    /// Kürzere Kante der Größenstufe in Punkten.
    pub fn shorter(size: &str) -> f64 {
        match size {
            "small" => 100.0,
            "large" => 220.0,
            _ => 150.0,
        }
    }

    /// Fenstergröße (w, h) für Form + Größe; `custom_pt > 0` überschreibt die Größenstufe (Drag-Resize).
    pub fn window_size(shape: &str, size: &str, custom_pt: f64) -> (f64, f64) {
        let s = if custom_pt > 0.0 { custom_pt } else { shorter(size) };
        let a = aspect(shape);
        if a >= 1.0 {
            (s * a, s)
        } else {
            (s, s / a)
        }
    }

    /// Neue kürzere Kante beim Ziehen am Griff unten rechts: größere der beiden Verschiebungen zählt.
    pub fn resized_shorter(start_shorter: f64, dx: f64, dy: f64) -> f64 {
        (start_shorter + dx.max(dy)).clamp(MIN_SHORTER, MAX_SHORTER)
    }

    fn clamp_origin(v: f64, lo: f64, hi: f64) -> f64 {
        if lo > hi {
            lo
        } else {
            v.clamp(lo, hi)
        }
    }

    /// Rahmen mit `margin` Abstand in den sichtbaren Bereich schieben (Größe bleibt).
    pub fn clamped(f: Rect, vis: Rect, margin: f64) -> Rect {
        [
            clamp_origin(f[0], vis[0] + margin, vis[0] + vis[2] - f[2] - margin),
            clamp_origin(f[1], vis[1] + margin, vis[1] + vis[3] - f[3] - margin),
            f[2],
            f[3],
        ]
    }

    /// Standardplatz: mittig 20 pt über der Unterkante des Aufnahmebereichs, sonst unten rechts im Arbeitsbereich.
    pub fn default_frame(size: (f64, f64), recording: Option<Rect>, vis: Rect) -> Rect {
        let (w, h) = size;
        match recording {
            Some(r) => [r[0] + r[2] / 2.0 - w / 2.0, r[1] + r[3] - RECORDING_BOTTOM_OFFSET - h, w, h],
            None => [vis[0] + vis[2] - w - DEFAULT_SCREEN_MARGIN, vis[1] + vis[3] - h - DEFAULT_SCREEN_MARGIN, w, h],
        }
    }

    /// Startrahmen: gemerkter Rahmen (geklemmt) oder Standardplatz.
    pub fn frame(restored: Option<Rect>, size: (f64, f64), recording: Option<Rect>, vis: Rect) -> Rect {
        clamped(restored.unwrap_or_else(|| default_frame(size, recording, vis)), vis, VISIBLE_MARGIN)
    }

    /// Ecken-Einrasten (Capso `handleWindowMoved`): liegt eine Fensterecke innerhalb von 60 pt der passenden
    /// Ecke des Arbeitsbereichs, wird das Fenster mit 16 pt Abstand in diese Ecke gesetzt.
    pub fn snapped(f: Rect, vis: Rect) -> Option<Rect> {
        let (l, t, r, b) = (vis[0], vis[1], vis[0] + vis[2], vis[1] + vis[3]);
        let (w, h) = (f[2], f[3]);
        let d = |ax: f64, ay: f64, bx: f64, by: f64| ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
        let cands = [
            (d(f[0], f[1], l, t), [l + SNAP_MARGIN, t + SNAP_MARGIN]),
            (d(f[0] + w, f[1], r, t), [r - w - SNAP_MARGIN, t + SNAP_MARGIN]),
            (d(f[0], f[1] + h, l, b), [l + SNAP_MARGIN, b - h - SNAP_MARGIN]),
            (d(f[0] + w, f[1] + h, r, b), [r - w - SNAP_MARGIN, b - h - SNAP_MARGIN]),
        ];
        let (dist, o) = cands.iter().copied().min_by(|a, b| a.0.total_cmp(&b.0))?;
        (dist <= SNAP_RADIUS).then_some([o[0], o[1], w, h])
    }

    /// Größe ändern und die obere linke Ecke festhalten.
    pub fn resized_keep_top_left(f: Rect, size: (f64, f64)) -> Rect {
        [f[0], f[1], size.0, size.1]
    }

    /// Ziel-Deckkraft bei Fade-on-Hover (Präsentationsmodus immer voll).
    pub fn fade_alpha(enabled: bool, presentation: bool, inside: bool) -> f64 {
        if enabled && !presentation && inside {
            FADE_HOVER_ALPHA
        } else {
            FADE_FULL_ALPHA
        }
    }

    /// Klicks durchlassen nur, wenn aktiviert, Zeiger über dem PiP und nicht im Präsentationsmodus.
    pub fn should_click_through(enabled: bool, presentation: bool, inside: bool) -> bool {
        enabled && !presentation && inside
    }

    /// Rahmen, zu dem nach einem Neustart zurückgekehrt wird (im Präsentationsmodus der gemerkte PiP-Rahmen).
    pub fn restoration_frame(current: Rect, stored: Option<Rect>, presentation: bool) -> Rect {
        if presentation {
            stored.unwrap_or(current)
        } else {
            current
        }
    }

    pub fn contains(f: Rect, x: f64, y: f64) -> bool {
        x >= f[0] && y >= f[1] && x < f[0] + f[2] && y < f[1] + f[3]
    }

    pub fn intersection_area(a: Rect, b: Rect) -> f64 {
        let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
        let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
        if w > 0.0 && h > 0.0 {
            w * h
        } else {
            0.0
        }
    }

    /// Ease-in-out-Interpolation zwischen zwei Rahmen (`p` in 0…1).
    pub fn lerp(a: Rect, b: Rect, p: f64) -> Rect {
        let e = if p < 0.5 { 2.0 * p * p } else { 1.0 - (-2.0 * p + 2.0).powi(2) / 2.0 };
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * e)
    }
}

// ───────────────────────────── Zustand ─────────────────────────────

#[derive(Default)]
struct Pip {
    presentation: bool,
    /// PiP-Rahmen vor dem Präsentationsmodus.
    stored: Option<Rect>,
    /// Aktueller Aufnahmebereich (von `record` gesetzt) – Ziel des Präsentationsmodus und Standardplatz.
    recording: Option<Rect>,
    /// Präsentationsmodus beim nächsten Öffnen wiederherstellen (Neustart der Aufnahme).
    restore_presentation: bool,
    inside: bool,
    click_through: bool,
    alpha: f64,
}

static OPEN: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<Pip>> = Mutex::new(None);
/// Bis zu diesem Zeitpunkt werden `Moved`-Events ignoriert (eigene Verschiebungen).
static SUPPRESS_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
static MOVE_GEN: AtomicU64 = AtomicU64::new(0);
/// ⌘ war beim letzten Verschieben gedrückt → nicht an Ecken einrasten (Capso `CameraPiPWindow.mouseUp`).
static CMD_HELD: AtomicBool = AtomicBool::new(false);
/// Kameras aus dem WebView (`MediaDeviceInfo.deviceId`, Name) für das Kontextmenü.
static DEVICES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceFlagsState(state: i32) -> u64;
}

/// Ist ⌘ gerade gedrückt? (kCGEventSourceStateCombinedSessionState, kCGEventFlagMaskCommand)
fn command_down() -> bool {
    unsafe { CGEventSourceFlagsState(0) & (1 << 20) != 0 }
}

fn st<R>(f: impl FnOnce(&mut Pip) -> R) -> R {
    let mut g = STATE.lock().unwrap();
    f(g.get_or_insert_with(|| Pip { alpha: 1.0, ..Default::default() }))
}

fn suppress_moves(ms: u64) {
    *SUPPRESS_UNTIL.lock().unwrap() = Some(Instant::now() + Duration::from_millis(ms));
}

fn moves_suppressed() -> bool {
    SUPPRESS_UNTIL.lock().unwrap().is_some_and(|t| Instant::now() < t)
}

fn current_size() -> (f64, f64) {
    let s = crate::settings::get();
    geo::window_size(&s.camera_shape, &s.camera_size, s.camera_custom_size_pt)
}

/// Gemerkter PiP-Rahmen (prozessweit + `settings.extra.cameraPiPPlacement`).
fn saved_frame() -> Option<Rect> {
    crate::settings::get_extra("cameraPiPPlacement").and_then(|v| serde_json::from_value::<Rect>(v).ok())
}

fn save_frame(app: &AppHandle, f: Rect) {
    if saved_frame() != Some(f) {
        crate::settings::set_extra(app, "cameraPiPPlacement", json!(f));
    }
}

/// Arbeitsbereich des Bildschirms, der den Rahmen am meisten überdeckt (sonst Bildschirm unter dem Zeiger).
fn work_area_for(app: &AppHandle, f: Option<Rect>) -> Rect {
    let screens = core::screens(app);
    if let Some(f) = f {
        if let Some(s) = screens
            .iter()
            .map(|s| (geo::intersection_area(f, s.work), s))
            .filter(|(a, _)| *a > 0.0)
            .max_by(|a, b| a.0.total_cmp(&b.0))
        {
            return s.1.work;
        }
    }
    if let Some(m) = core::cursor_monitor(app) {
        let (sc, p) = (m.scale_factor(), m.position());
        if let Some(s) = screens.iter().find(|s| (s.x - p.x as f64 / sc).abs() < 1.0 && (s.y - p.y as f64 / sc).abs() < 1.0) {
            return s.work;
        }
    }
    screens.first().map(|s| s.work).unwrap_or([0.0, 25.0, 1440.0, 875.0])
}

/// Ganzer Bildschirm (inkl. Menüleiste), auf dem das Fenster liegt.
fn screen_frame_for(app: &AppHandle, f: Rect) -> Rect {
    core::screens(app)
        .iter()
        .map(|s| ([s.x, s.y, s.width, s.height], geo::intersection_area(f, [s.x, s.y, s.width, s.height])))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(r, _)| r)
        .unwrap_or(f)
}

fn win_frame(w: &WebviewWindow) -> Option<Rect> {
    let s = w.scale_factor().ok()?;
    let p = w.outer_position().ok()?;
    let z = w.inner_size().ok()?;
    Some([p.x as f64 / s, p.y as f64 / s, z.width as f64 / s, z.height as f64 / s])
}

fn set_frame(w: &WebviewWindow, f: Rect) {
    suppress_moves(400);
    let _ = w.set_size(LogicalSize::new(f[2], f[3]));
    let _ = w.set_position(LogicalPosition::new(f[0], f[1]));
}

/// Rahmen animiert setzen (≈ 0,28 s, ease-in-out – wie Capsos NSAnimationContext).
fn animate_frame(w: &WebviewWindow, from: Rect, to: Rect) {
    const STEPS: u32 = 14;
    for i in 1..=STEPS {
        set_frame(w, geo::lerp(from, to, i as f64 / STEPS as f64));
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn state_json() -> Value {
    let s = crate::settings::get();
    st(|p| {
        json!({
            "presentation": p.presentation,
            "alpha": p.alpha,
            "shape": s.camera_shape,
            "size": s.camera_size,
            "mirror": s.camera_mirror,
            "deviceId": s.camera_device_id,
            "fadeOnHover": s.camera_pip_fade_on_hover,
            "clickThrough": s.camera_pip_click_through,
        })
    })
}

fn emit_state(app: &AppHandle) {
    let _ = app.emit_to(LABEL, "camera-state", state_json());
}

// ───────────────────────────── Öffentliche API ─────────────────────────────

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::SeqCst)
}

pub fn toggle(app: &AppHandle) {
    if is_open() {
        close(app);
    } else {
        open(app);
    }
}

/// Kamera-PiP öffnen (am gemerkten Platz bzw. Standardplatz; Präsentationsmodus nach Neustart wiederherstellen).
pub fn open(app: &AppHandle) {
    if is_open() {
        if let Some(w) = window(app) {
            let _ = w.show();
        }
        return;
    }
    let size = current_size();
    let (recording, restore) = st(|p| (p.recording, p.restore_presentation));
    let restored = saved_frame().map(|f| [f[0], f[1], size.0, size.1]);
    let vis = work_area_for(app, restored.or(recording));
    let pip = geo::frame(restored, size, recording, vis);
    let presentation = restore && recording.is_some();
    let initial = if presentation { recording.unwrap() } else { pip };
    st(|p| {
        p.presentation = presentation;
        p.stored = presentation.then_some(pip);
        p.inside = false;
        p.click_through = false;
        p.alpha = 1.0;
    });
    let Some(w) = core::window(
        app,
        LABEL,
        LABEL,
        Win {
            title: "nrzCapture Camera".into(),
            w: initial[2],
            h: initial[3],
            pos: Some((initial[0], initial[1])),
            focus: false,
            level: Some(if presentation { PRESENTATION_LEVEL } else { core::LEVEL_FLOATING }),
            ..Default::default()
        },
        state_json(),
    ) else {
        return;
    };
    OPEN.store(true, Ordering::SeqCst);
    suppress_moves(600);
    let app2 = app.clone();
    w.on_window_event(move |e| match e {
        WindowEvent::Moved(_) => on_moved(&app2),
        WindowEvent::Destroyed => {
            OPEN.store(false, Ordering::SeqCst);
            crate::actions::refresh_menu(&app2);
        }
        _ => {}
    });
    w.on_menu_event(|w, e| on_menu(w.app_handle(), e.id().as_ref()));
    start_hover_thread(app.clone());
    crate::actions::refresh_menu(app);
}

/// Kamera-PiP schließen (Platz wird gemerkt; der WebView-Stream endet mit dem Fenster).
pub fn close(app: &AppHandle) {
    if let Some(w) = window(app) {
        let (presentation, stored) = st(|p| (p.presentation, p.stored));
        if let Some(cur) = win_frame(&w) {
            save_frame(app, geo::restoration_frame(cur, stored, presentation));
        }
        st(|p| {
            p.restore_presentation = presentation;
            p.presentation = false;
            p.stored = None;
        });
        let _ = w.destroy();
    }
    OPEN.store(false, Ordering::SeqCst);
    crate::actions::refresh_menu(app);
}

/// Vom Aufnahme-Modul: aktueller Aufnahmebereich (Ziel des Präsentationsmodus, Standardplatz des PiP).
/// `None` = Aufnahme beendet (Präsentationsmodus wird dann nicht mehr wiederhergestellt).
pub fn set_recording_frame(app: &AppHandle, frame: Option<Rect>) {
    let was_presentation = st(|p| {
        p.recording = frame;
        if frame.is_none() {
            p.restore_presentation = false;
        }
        p.presentation
    });
    if was_presentation && frame.is_none() {
        toggle_presentation(app);
    }
}

pub fn presentation_active() -> bool {
    st(|p| p.presentation)
}

/// Klick auf das PiP: Präsentationsmodus (Aufnahmebereich bzw. ganzer Bildschirm) ein/aus.
pub fn toggle_presentation(app: &AppHandle) {
    let Some(w) = window(app) else { return };
    let Some(cur) = win_frame(&w) else { return };
    let (presentation, stored, recording) = st(|p| (p.presentation, p.stored, p.recording));
    if presentation {
        let target = stored.unwrap_or_else(|| {
            let size = current_size();
            geo::frame(None, size, recording, work_area_for(app, Some(cur)))
        });
        st(|p| {
            p.presentation = false;
            p.stored = None;
        });
        emit_state(app);
        animate_frame(&w, cur, target);
        core::set_level(&w, core::LEVEL_FLOATING);
    } else {
        let target = recording.unwrap_or_else(|| screen_frame_for(app, cur));
        st(|p| {
            p.presentation = true;
            p.stored = Some(cur);
            p.alpha = 1.0;
        });
        core::set_level(&w, PRESENTATION_LEVEL);
        let _ = w.set_ignore_cursor_events(false);
        emit_state(app);
        animate_frame(&w, cur, target);
    }
}

/// Fenstergröße an Form/Größe der Einstellungen anpassen (obere linke Ecke bleibt).
pub fn relayout(app: &AppHandle) {
    let Some(w) = window(app) else { return };
    if !presentation_active() {
        if let Some(cur) = win_frame(&w) {
            let size = current_size();
            if (cur[2] - size.0).abs() > 0.5 || (cur[3] - size.1).abs() > 0.5 {
                let f = geo::clamped(geo::resized_keep_top_left(cur, size), work_area_for(app, Some(cur)), geo::VISIBLE_MARGIN);
                set_frame(&w, f);
            }
        }
    }
    emit_state(app);
}

// ───────────────────────────── Ereignisse ─────────────────────────────

/// Nach dem Ziehen (keine `Moved`-Events mehr für 250 ms) an die nächste Ecke einrasten und Platz merken.
fn on_moved(app: &AppHandle) {
    if moves_suppressed() || presentation_active() {
        return;
    }
    let gen = MOVE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    CMD_HELD.store(command_down(), Ordering::SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        if MOVE_GEN.load(Ordering::SeqCst) != gen || moves_suppressed() || presentation_active() {
            return;
        }
        let Some(w) = window(&app) else { return };
        let Some(cur) = win_frame(&w) else { return };
        let vis = work_area_for(&app, Some(cur));
        // Mit ⌘ gezogen → frei platzieren (nur im sichtbaren Bereich halten).
        let f = if CMD_HELD.load(Ordering::SeqCst) || command_down() {
            geo::clamped(cur, vis, geo::VISIBLE_MARGIN)
        } else {
            geo::snapped(cur, vis).unwrap_or(cur)
        };
        if f != cur {
            set_frame(&w, f);
        }
        save_frame(&app, f);
    });
}

/// Zeigerposition abfragen (Fade-on-Hover + Click-Through; bei ignorierten Mausereignissen liefert das
/// WebView selbst kein Hover mehr – daher wie Capso über die globale Mausposition).
fn start_hover_thread(app: AppHandle) {
    std::thread::spawn(move || {
        while is_open() {
            std::thread::sleep(Duration::from_millis(100));
            let s = crate::settings::get();
            let Some(w) = window(&app) else { break };
            let (presentation, was_inside, was_ct, was_alpha) = st(|p| (p.presentation, p.inside, p.click_through, p.alpha));
            let inside = if s.camera_pip_fade_on_hover || s.camera_pip_click_through || was_inside {
                match (app.cursor_position(), w.outer_position(), w.outer_size()) {
                    (Ok(c), Ok(p), Ok(z)) => geo::contains([p.x as f64, p.y as f64, z.width as f64, z.height as f64], c.x, c.y),
                    _ => false,
                }
            } else {
                false
            };
            let ct = geo::should_click_through(s.camera_pip_click_through, presentation, inside);
            let alpha = geo::fade_alpha(s.camera_pip_fade_on_hover, presentation, inside);
            if ct != was_ct {
                let _ = w.set_ignore_cursor_events(ct);
            }
            if inside != was_inside || ct != was_ct || (alpha - was_alpha).abs() > 0.001 {
                st(|p| {
                    p.inside = inside;
                    p.click_through = ct;
                    p.alpha = alpha;
                });
                let _ = app.emit_to(LABEL, "camera-hover", json!({ "alpha": alpha, "inside": inside }));
            }
        }
    });
}

/// Kontextmenü (Capso `CameraMenuBuilder`: Kamera-Geräte, Form, Größe, Optionen).
fn show_menu(app: &AppHandle, w: &WebviewWindow) -> tauri::Result<()> {
    let s = crate::settings::get();
    let custom = s.camera_custom_size_pt > 0.0;
    let check = |id: &str, text: &str, on: bool| CheckMenuItem::with_id(app, id, text, true, on, None::<&str>);
    let header = |text: &str| MenuItem::with_id(app, format!("campip:h:{text}"), text.to_uppercase(), false, None::<&str>);
    let sep = || PredefinedMenuItem::separator(app);
    let shapes = [
        ("circle", t("Kreis", "Circle")),
        ("square", t("Quadrat", "Square")),
        ("landscape", t("Quer (16:9)", "Landscape (16:9)")),
        ("portrait", t("Hoch (9:16)", "Portrait (9:16)")),
    ];
    let sizes = [("small", t("Klein", "Small")), ("medium", t("Mittel", "Medium")), ("large", t("Groß", "Large"))];
    let menu = Menu::new(app)?;
    let devices = DEVICES.lock().unwrap().clone();
    if !devices.is_empty() {
        menu.append(&header(t("Kamera", "Camera"))?)?;
        let cur = s.camera_device_id.clone().filter(|d| !d.is_empty());
        let cur = cur.filter(|c| devices.iter().any(|(id, _)| id == c)).or_else(|| devices.first().map(|d| d.0.clone()));
        for (i, (id, name)) in devices.iter().enumerate() {
            menu.append(&check(&format!("campip:device:{i}"), name, cur.as_deref() == Some(id.as_str()))?)?;
        }
        menu.append(&sep()?)?;
    }
    menu.append(&header(t("Form", "Shape"))?)?;
    for (id, text) in shapes {
        menu.append(&check(&format!("campip:shape:{id}"), text, s.camera_shape == id)?)?;
    }
    menu.append(&sep()?)?;
    menu.append(&header(t("Größe", "Size"))?)?;
    for (id, text) in sizes {
        menu.append(&check(&format!("campip:size:{id}"), text, !custom && s.camera_size == id)?)?;
    }
    menu.append(&sep()?)?;
    menu.append(&check("campip:mirror", t("Spiegeln", "Mirror"), s.camera_mirror)?)?;
    menu.append(&check("campip:fade", t("Bei Hover ausblenden", "Fade on Hover"), s.camera_pip_fade_on_hover)?)?;
    menu.append(&check("campip:clickThrough", t("Klicks durchlassen", "Click Through PiP"), s.camera_pip_click_through)?)?;
    menu.append(&sep()?)?;
    menu.append(&MenuItem::with_id(app, "campip:close", t("Kamera schließen", "Close Camera"), true, None::<&str>)?)?;
    w.popup_menu(&menu)
}

/// Kamera-Gerät wählen (`None` = Standardkamera); das PiP startet den Stream daraufhin neu.
pub fn set_device(app: &AppHandle, id: Option<String>) {
    crate::settings::update(app, |s| s.camera_device_id = id);
    emit_state(app);
}

fn set_shape(app: &AppHandle, shape: &str) {
    let shape = shape.to_owned();
    crate::settings::update(app, |s| s.camera_shape = shape);
    relayout(app);
}

/// Größenstufe wählen (setzt die Drag-Resize-Größe zurück).
fn set_size(app: &AppHandle, size: &str) {
    let size = size.to_owned();
    crate::settings::update(app, |s| {
        s.camera_size = size;
        s.camera_custom_size_pt = 0.0;
    });
    relayout(app);
}

fn on_menu(app: &AppHandle, id: &str) {
    let Some(rest) = id.strip_prefix("campip:") else { return };
    let app = app.clone();
    let rest = rest.to_owned();
    // Nicht auf dem Haupt-Thread blockieren.
    std::thread::spawn(move || match rest.split_once(':') {
        Some(("shape", v)) => set_shape(&app, v),
        Some(("size", v)) => set_size(&app, v),
        Some(("device", v)) => {
            let id = v.parse::<usize>().ok().and_then(|i| DEVICES.lock().unwrap().get(i).map(|d| d.0.clone()));
            if let Some(id) = id {
                set_device(&app, Some(id));
            }
        }
        _ => match rest.as_str() {
            "mirror" => {
                crate::settings::update(&app, |s| s.camera_mirror = !s.camera_mirror);
                emit_state(&app);
            }
            "fade" => crate::settings::update(&app, |s| s.camera_pip_fade_on_hover = !s.camera_pip_fade_on_hover),
            "clickThrough" => crate::settings::update(&app, |s| s.camera_pip_click_through = !s.camera_pip_click_through),
            "close" => close(&app),
            _ => {}
        },
    });
}

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    match cmd {
        "state" => return Ok(state_json()),
        "toggle" => toggle(app),
        "open" => open(app),
        "close" => close(app),
        "isOpen" => return Ok(json!(is_open())),
        "togglePresentation" => toggle_presentation(app),
        "relayout" => relayout(app),
        // Natives Fenster-Ziehen starten (Ende → `Moved`-Entprellung → Einrasten).
        "startDrag" => {
            if !presentation_active() {
                let w = win.clone();
                let _ = win.run_on_main_thread(move || {
                    let _ = w.start_dragging();
                });
            }
        }
        // Drag-Resize: neue kürzere Kante in pt (80…400) → `cameraCustomSizePt`.
        "resize" => {
            let size = a["size"].as_f64().ok_or("size fehlt")?.clamp(geo::MIN_SHORTER, geo::MAX_SHORTER).round();
            if presentation_active() {
                return Ok(Value::Null);
            }
            if let Some(w) = window(app) {
                if let Some(cur) = win_frame(&w) {
                    let s = crate::settings::get();
                    let f = geo::resized_keep_top_left(cur, geo::window_size(&s.camera_shape, &s.camera_size, size));
                    set_frame(&w, f);
                    if a["done"].as_bool().unwrap_or(false) {
                        crate::settings::update(app, |s| s.camera_custom_size_pt = size);
                        save_frame(app, f);
                    }
                }
            }
        }
        "setShape" => set_shape(app, a["shape"].as_str().ok_or("shape fehlt")?),
        "setSize" => set_size(app, a["size"].as_str().ok_or("size fehlt")?),
        "toggleMirror" => {
            crate::settings::update(app, |s| s.camera_mirror = !s.camera_mirror);
            emit_state(app);
        }
        "setDevice" => set_device(app, a["id"].as_str().filter(|s| !s.is_empty()).map(str::to_owned)),
        "menu" => {
            if let Some(list) = a["devices"].as_array() {
                *DEVICES.lock().unwrap() = list
                    .iter()
                    .filter_map(|d| Some((d["id"].as_str()?.to_owned(), d["name"].as_str().unwrap_or("?").to_owned())))
                    .collect();
            }
            if let Some(w) = window(app) {
                show_menu(app, &w).map_err(|e| e.to_string())?;
            }
        }
        _ => return Err(format!("camera: unbekannter Befehl {cmd}")),
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::geo::*;

    // Tests portiert aus Capso `CameraPiPPlacementTests` (y-Achse auf „oben links“ gespiegelt).
    const VIS: [f64; 4] = [0.0, 0.0, 1000.0, 800.0];
    const SIZE: (f64, f64) = (160.0, 120.0);

    #[test]
    fn restored_frame_takes_precedence() {
        let restored = [100.0, 200.0, 220.0, 180.0];
        assert_eq!(frame(Some(restored), SIZE, Some([200.0, 100.0, 400.0, 300.0]), VIS), restored);
    }

    #[test]
    fn restored_dimensions_preserved_origin_clamped() {
        // Capso (unten links): (950,760) → (772,612). Oben links: y gespiegelt, Ergebnis y = 800-612-180 = 8.
        let f = frame(Some([950.0, -140.0, 220.0, 180.0]), SIZE, None, VIS);
        assert_eq!(f, [772.0, 8.0, 220.0, 180.0]);
    }

    #[test]
    fn default_uses_recording_placement() {
        // Aufnahmebereich (100,50,500,300) oben links → Unterkante 350; 20 pt darüber.
        let f = frame(None, SIZE, Some([100.0, 50.0, 500.0, 300.0]), VIS);
        assert_eq!(f, [270.0, 350.0 - 20.0 - 120.0, 160.0, 120.0]);
    }

    #[test]
    fn default_uses_screen_placement() {
        assert_eq!(frame(None, SIZE, None, VIS), [808.0, 800.0 - 32.0 - 120.0, 160.0, 120.0]);
    }

    #[test]
    fn frames_clamped_inside_visible_bounds() {
        assert_eq!(frame(Some([-40.0, -30.0, 160.0, 120.0]), SIZE, None, VIS), [8.0, 8.0, 160.0, 120.0]);
    }

    #[test]
    fn restoration_keeps_stored_frame_in_presentation() {
        let cur = [0.0, 0.0, 1000.0, 800.0];
        let stored = [10.0, 10.0, 150.0, 150.0];
        assert_eq!(restoration_frame(cur, Some(stored), true), stored);
        assert_eq!(restoration_frame(cur, Some(stored), false), cur);
        assert_eq!(restoration_frame(cur, None, true), cur);
    }

    #[test]
    fn fade_and_click_through() {
        assert_eq!(fade_alpha(true, false, true), FADE_HOVER_ALPHA);
        assert_eq!(fade_alpha(true, false, false), FADE_FULL_ALPHA);
        assert_eq!(fade_alpha(true, true, true), FADE_FULL_ALPHA);
        assert_eq!(fade_alpha(false, false, true), FADE_FULL_ALPHA);
        assert!(should_click_through(true, false, true));
        assert!(!should_click_through(true, true, true));
        assert!(!should_click_through(true, false, false));
        assert!(!should_click_through(false, false, true));
    }

    #[test]
    fn sizes_and_shapes() {
        assert_eq!(window_size("circle", "medium", 0.0), (150.0, 150.0));
        assert_eq!(window_size("square", "small", 0.0), (100.0, 100.0));
        assert_eq!(window_size("landscape", "large", 0.0), (220.0 * 16.0 / 9.0, 220.0));
        assert_eq!(window_size("portrait", "small", 0.0), (100.0, 100.0 * 16.0 / 9.0));
        assert_eq!(window_size("circle", "small", 250.0), (250.0, 250.0));
        assert_eq!(resized_shorter(150.0, 30.0, 10.0), 180.0);
        assert_eq!(resized_shorter(150.0, -200.0, -100.0), MIN_SHORTER);
        assert_eq!(resized_shorter(350.0, 10.0, 90.0), MAX_SHORTER);
    }

    #[test]
    fn snaps_to_nearest_corner() {
        // nahe unten rechts
        let f = [1000.0 - 160.0 - 30.0, 800.0 - 120.0 - 20.0, 160.0, 120.0];
        assert_eq!(snapped(f, VIS), Some([1000.0 - 160.0 - 16.0, 800.0 - 120.0 - 16.0, 160.0, 120.0]));
        // nahe oben links
        assert_eq!(snapped([20.0, 30.0, 160.0, 120.0], VIS), Some([16.0, 16.0, 160.0, 120.0]));
        // Mitte: kein Einrasten
        assert_eq!(snapped([400.0, 300.0, 160.0, 120.0], VIS), None);
    }

    #[test]
    fn lerp_endpoints() {
        let a = [0.0, 0.0, 100.0, 100.0];
        let b = [100.0, 50.0, 200.0, 300.0];
        assert_eq!(lerp(a, b, 0.0), a);
        assert_eq!(lerp(a, b, 1.0), b);
        assert_eq!(lerp(a, b, 0.5), [50.0, 25.0, 150.0, 200.0]);
    }
}
