//! Bildschirmaufnahme (MP4/GIF, Ton, Mikrofon, Pause, Neustart, Countdown, Steuerleiste).
//! Port von Capso `RecordingCoordinator` + `RecordingKit` + `EffectsKit` + `ExportKit`.
//!
//! Ablauf wie Capso: Bereich/Fenster/Vollbild wählen (`capture::select_region`) → Aufnahme-Leiste
//! (Video/GIF, Mikrofon + Gerät, System-Audio, Kamera, Cursor, Klicks, Tasten) mit Auswahlrahmen →
//! Countdown 3-2-1 → Aufnahme (roter Rahmen, Abdunkeln außerhalb, schwebende Steuerung, Klick-/Tasten-Overlays,
//! Cursor-Telemetrie) → Stopp → Export (MP4-Preset / GIF) → Auto-Kopieren, Verlauf, Quick Access oder Aufnahme-Editor.
//!
//! Untermodule: `sck` (ScreenCaptureKit), `export` (Zusammenfügen/MP4/GIF), `input` (Maus/Tasten/Telemetrie),
//! `keys` (KeystrokeFormatter).
//!
//! Fenster (Label = Route): `rec-toolbar`, `rec-frame`, `rec-countdown`, `rec-controls`, `rec-warning`,
//! `rec-clicks`, `rec-keys`. Events an diese Fenster: `rec-frame` {mode, rect, dim}, `rec-countdown` {n},
//! `rec-state` {elapsed, paused}, `rec-click` {x, y}, `rec-key` {text, command}.
//!
//! Tastenkürzel: Esc bricht Leiste/Countdown ab; während der Aufnahme stoppt ⌃⌘⎋ (wie macOS) oder erneut das
//! Aufnahme-Kürzel; Menüpunkt „Aufzeichnung beenden“.
mod export;
mod input;
mod keys;
mod sck;

use crate::capture::{Purpose, Region};
use crate::core::{self, t, Win};
use input::{Clock, MouseMonitor, TelemetryData};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const W_TOOLBAR: &str = "rec-toolbar";
const W_FRAME: &str = "rec-frame";
const W_COUNTDOWN: &str = "rec-countdown";
const W_CONTROLS: &str = "rec-controls";
const W_WARNING: &str = "rec-warning";
const W_CLICKS: &str = "rec-clicks";
const W_KEYS: &str = "rec-keys";
const STOP_ACCEL: &str = "Ctrl+Cmd+Escape";

/// Freier Speicher, unter dem vor dem Start gewarnt wird (Capso `DiskSpace`).
const DISK_WARN: u64 = 3_000_000_000;
/// Darunter wird eine laufende Aufnahme beendet.
const DISK_STOP: u64 = 1_000_000_000;
/// Mikrofon gilt als stumm, wenn der Spitzenpegel 8 s lang unter −80 dBFS bleibt.
const MIC_SILENCE_PEAK: f32 = 1e-4;
const MIC_SILENCE_SECS: f64 = 8.0;
/// Mikrofon/System-Audio gilt als ausgefallen, wenn 5 s lang kein Puffer kam (Capso `AudioActivityMonitor.stallTimeout`).
const AUDIO_STALL_MS: u64 = 5_000;
/// Merker der laufenden Aufnahme (Segment-Ordner) für die Wiederherstellung nach Absturz/Beenden
/// (Capso `InterruptedRecording`, UserDefaults-Schlüssel `inProgressRecordingPath`).
const MARKER: &str = "inProgressRecording";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Selecting,
    Toolbar,
    Countdown,
    Recording,
    Finishing,
}

/// Optionen einer Aufnahme (aus der Leiste + Einstellungen).
#[derive(Clone, Debug)]
struct Opts {
    gif: bool,
    mic: bool,
    mic_device: Option<String>,
    system_audio: bool,
    show_cursor: bool,
    clicks: bool,
    keys: bool,
}

struct Session {
    rec: sck::Recorder,
    clock: Arc<Clock>,
    mouse: Option<MouseMonitor>,
    data: Arc<Mutex<TelemetryData>>,
    dir: PathBuf,
    shows_cursor: bool,
    /// Bereits gemeldete Audio-Probleme (jedes höchstens einmal pro Aufnahme).
    warned: Vec<&'static str>,
    started_ms: u64,
}

struct Flow {
    phase: Phase,
    region: Option<Region>,
    opts: Option<Opts>,
    session: Option<Session>,
    /// Kamera wurde von uns für die Aufnahme geöffnet → danach wieder schließen.
    camera_by_us: bool,
    icons_by_us: bool,
    /// System-Audio in der Leiste (pro Sitzung, Capso `RecordingToolbarState.systemAudioEnabled`).
    sys_audio: bool,
}

static FLOW: Mutex<Flow> =
    Mutex::new(Flow { phase: Phase::Idle, region: None, opts: None, session: None, camera_by_us: false, icons_by_us: false, sys_audio: true });
/// Trefferfläche der Tasten-Blasen (lokal im Tasten-Overlay, Punkte) – nur dort nimmt das Overlay Mausereignisse an.
static KEYS_HIT: Mutex<Option<[f64; 4]>> = Mutex::new(None);
static KEYS_MOVE_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static RECORDING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

fn phase() -> Phase {
    FLOW.lock().unwrap().phase
}

fn set_phase(p: Phase) {
    FLOW.lock().unwrap().phase = p;
}

// ───────────────────────────── Öffentliche API ─────────────────────────────

/// Aufnahme starten: Bereich wählen (oder `fullscreen`). Läuft bereits eine Aufnahme, wird sie beendet.
pub fn start(app: &AppHandle, fullscreen: bool) {
    if is_recording() {
        return stop(app);
    }
    {
        let mut f = FLOW.lock().unwrap();
        if f.phase != Phase::Idle {
            return;
        }
        f.phase = Phase::Selecting;
    }
    // „Letzten Aufnahmebereich merken“: gemerkten Bereich direkt verwenden (Capso `showRememberedRecordingArea`).
    let remembered = if fullscreen { None } else { remembered_region(app) };
    let region = if fullscreen {
        fullscreen_region(app)
    } else if remembered.is_some() {
        remembered
    } else {
        crate::capture::select_region(app, Purpose::Recording)
    };
    let Some(region) = region.filter(|r| r.width >= 2.0 && r.height >= 2.0) else {
        set_phase(Phase::Idle);
        return;
    };
    remember_area(app, &region);
    show_toolbar(app, region);
}

/// Direkt zur Aufnahme-Leiste für einen schon bekannten Bereich (All-in-One „Aufnehmen“, Overlay-Modus „Letzter Bereich“).
pub fn start_region(app: &AppHandle, region: Region) {
    if is_recording() || region.width < 2.0 || region.height < 2.0 {
        return;
    }
    {
        let mut f = FLOW.lock().unwrap();
        if f.phase != Phase::Idle {
            return;
        }
        f.phase = Phase::Selecting;
    }
    remember_area(app, &region);
    show_toolbar(app, region);
}

/// Gemerkter Bereich, falls „Letzten Aufnahmebereich merken“ an ist, es ein Rechteck (> 5 pt) ist und sein
/// Bildschirm noch existiert; in den Bildschirm verschoben (Capso `selectLastRecordingArea`).
fn remembered_region(app: &AppHandle) -> Option<Region> {
    let s = crate::settings::get();
    if !s.remember_last_recording_area {
        return None;
    }
    let Some(crate::settings::StoredSelection::Area { x, y, width, height, screen }) = s.last_recording_area else { return None };
    if width <= 5.0 || height <= 5.0 {
        return None;
    }
    let sc = core::screens(app).into_iter().find(|sc| sc.id == screen)?;
    let r = fit_into([x, y, width, height], [sc.x, sc.y, sc.width, sc.height]);
    Some(Region { x: r[0], y: r[1], width: r[2], height: r[3], screen, window_id: None, fullscreen: false })
}

/// Rechteck (Größe höchstens die des Bereichs) so verschieben, dass es ganz im Bereich liegt.
pub fn fit_into(r: [f64; 4], area: [f64; 4]) -> [f64; 4] {
    let (w, h) = (r[2].min(area[2]), r[3].min(area[3]));
    let x = r[0].clamp(area[0], area[0] + area[2] - w);
    let y = r[1].clamp(area[1], area[1] + area[3] - h);
    [x, y, w, h]
}

fn remember_area(app: &AppHandle, region: &Region) {
    let s = crate::settings::get();
    if s.remember_last_recording_area && region.window_id.is_none() {
        let sel = if region.fullscreen {
            crate::settings::StoredSelection::Fullscreen { screen: region.screen }
        } else {
            crate::settings::StoredSelection::Area { x: region.x, y: region.y, width: region.width, height: region.height, screen: region.screen }
        };
        crate::settings::update(app, |s| s.last_recording_area = Some(sel));
    }
}

/// Aufnahme beenden und exportieren (blockiert bis zum Ende des Exports).
pub fn stop(app: &AppHandle) {
    let Some((sess, opts, rect)) = end_session(app, false) else {
        // Noch in Leiste/Countdown → abbrechen.
        if matches!(phase(), Phase::Toolbar | Phase::Countdown) {
            cancel(app);
        }
        return;
    };
    let (segments, data) = sess;
    finish(app, segments, data, opts, rect);
    clear_marker(app);
    set_phase(Phase::Idle);
}

pub fn is_recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// Beim Beenden der App aufräumen (Aufnahme abschließen, ohne zu exportieren). Der Merker bleibt stehen –
/// `recover_interrupted` sichert die Segmente beim nächsten Start.
pub fn shutdown(app: &AppHandle) {
    input::stop_key_tap();
    if let Some(((segments, _), _, _)) = end_session(app, true) {
        if let Some(first) = segments.first() {
            eprintln!("Aufnahme beim Beenden abgebrochen (wird beim nächsten Start wiederhergestellt): {}", first.display());
        }
    }
}

fn set_marker(app: &AppHandle, dir: &Path) {
    crate::settings::set_extra(app, MARKER, json!({ "dir": dir }));
}

fn clear_marker(app: &AppHandle) {
    if crate::settings::get_extra(MARKER).is_some_and(|v| !v.is_null()) {
        crate::settings::set_extra(app, MARKER, Value::Null);
    }
}

/// Index aus `segment-<n>.mp4`.
pub fn segment_index(name: &str) -> Option<usize> {
    name.strip_prefix("segment-")?.strip_suffix(".mp4")?.parse().ok()
}

/// Verwertbare Dateien einer unterbrochenen Aufnahme: fertiges `raw.mp4` (Absturz beim Export) oder die Segmente
/// in Reihenfolge; unlesbare (nicht abgeschlossene) Dateien fallen weg.
fn recoverable_files(dir: &Path) -> Vec<PathBuf> {
    let raw = dir.join("raw.mp4");
    if raw.is_file() && export::duration(&raw) > 0.0 {
        return vec![raw];
    }
    let mut segs: Vec<(usize, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter_map(|e| Some((segment_index(e.file_name().to_str()?)?, e.path()))).collect())
        .unwrap_or_default();
    segs.sort_by_key(|(i, _)| *i);
    segs.into_iter().map(|(_, p)| p).filter(|p| export::duration(p) > 0.0).collect()
}

/// Mitteilung der Mitteilungszentrale (osascript).
fn notify(title: &str, body: &str) {
    let script = format!("display notification \"{}\" with title \"{}\"", esc_as(body), esc_as(title));
    let _ = core::sh("/usr/bin/osascript", &["-e", &script]);
}

/// Einmal beim Start: Reste einer durch Absturz/Beenden unterbrochenen Aufnahme als Video sichern, in den Verlauf
/// aufnehmen und melden (Capso `RecordingCoordinator.recoverInterruptedRecording`).
pub fn recover_interrupted(app: &AppHandle) {
    let Some(dir) = crate::settings::get_extra(MARKER).and_then(|v| v["dir"].as_str().map(PathBuf::from)) else { return };
    clear_marker(app);
    if !dir.is_dir() || is_recording() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let files = recoverable_files(&dir);
        if files.is_empty() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let joined = dir.join("recovered.mp4");
        let s = crate::settings::get();
        let out_dir = core::export_dir();
        let out = core::unique_path(&out_dir, &core::file_name(&s.recording_filename_template, "", "mp4"));
        let res = export::concat(&files, &joined).and_then(|_| export::mp4(&joined, &s.export_quality, &out));
        if let Err(e) = res {
            eprintln!("Wiederherstellung der Aufnahme fehlgeschlagen ({}): {e}", dir.display());
            return;
        }
        let _ = std::fs::remove_dir_all(&dir);
        crate::history::add(&app, &out, "recording");
        let body = format!(
            "{} {}",
            t(
                "nrzCapture wurde während einer Aufnahme unerwartet beendet. Das Aufgenommene wurde gesichert in",
                "nrzCapture quit unexpectedly during a recording. What was captured has been saved to"
            ),
            out_dir.display()
        );
        notify(t("Aufnahme wiederhergestellt", "Recording recovered"), &body);
    });
}

/// Pause/Fortsetzen umschalten.
pub fn toggle_pause(app: &AppHandle) {
    let mut f = FLOW.lock().unwrap();
    let Some(s) = f.session.as_mut() else { return };
    if s.rec.is_paused() {
        if let Err(e) = s.rec.resume() {
            eprintln!("Fortsetzen: {e}");
            return;
        }
        s.clock.start();
    } else {
        s.clock.pause();
        s.rec.pause();
    }
    let st = json!({ "elapsed": s.clock.elapsed(), "paused": s.rec.is_paused() });
    drop(f);
    let _ = app.emit_to(W_CONTROLS, "rec-state", st);
}

/// Aufnahme verwerfen und mit denselben Einstellungen neu starten (Kamera-Platzierung bleibt erhalten).
pub fn restart(app: &AppHandle) {
    let Some(((segments, _), mut opts, _)) = end_session(app, true) else { return };
    discard(&segments);
    clear_marker(app);
    let app = app.clone();
    std::thread::spawn(move || {
        set_phase(Phase::Countdown);
        if countdown_and_checks(&app, &mut opts) {
            begin(&app, opts);
        } else {
            cancel(&app);
        }
    });
}

/// Nach Rückfrage verwerfen.
pub fn delete(app: &AppHandle) {
    let ok = alert(
        t("Diese Aufnahme löschen?", "Delete this recording?"),
        t("Die aktuelle Aufnahme wird endgültig verworfen.", "The current recording will be permanently discarded."),
        &[t("Abbrechen", "Cancel"), t("Löschen", "Delete")],
    );
    if ok.as_deref() != Some(t("Löschen", "Delete")) {
        return;
    }
    if let Some(((segments, _), _, _)) = end_session(app, false) {
        discard(&segments);
    }
    clear_marker(app);
    set_phase(Phase::Idle);
}

/// Mikrofone für die Geräteauswahl.
pub fn microphones() -> Vec<(String, String)> {
    sck::microphones()
}

// ───────────────────────────── Leiste ─────────────────────────────

fn screen_of(app: &AppHandle, r: &Region) -> core::Screen {
    let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
    let all = core::screens(app);
    all.iter()
        .find(|s| cx >= s.x && cy >= s.y && cx < s.x + s.width && cy < s.y + s.height)
        .or_else(|| all.iter().find(|s| s.id == r.screen))
        .or(all.first())
        .cloned()
        .unwrap_or(core::Screen { id: 0, x: 0.0, y: 0.0, width: 1440.0, height: 900.0, scale: 2.0, work: [0.0, 0.0, 1440.0, 900.0] })
}

fn fullscreen_region(app: &AppHandle) -> Option<Region> {
    let m = core::cursor_monitor(app)?;
    let s = m.scale_factor();
    let p = m.position();
    let probe = Region { x: p.x as f64 / s + 1.0, y: p.y as f64 / s + 1.0, width: 1.0, height: 1.0, ..Default::default() };
    let sc = screen_of(app, &probe);
    Some(Region { x: sc.x, y: sc.y, width: sc.width, height: sc.height, screen: sc.id, window_id: None, fullscreen: true })
}

/// Rechteck `[x, y, w, h]` an einer Stelle zentriert/geklemmt in den Bereich.
pub fn clamp_rect(x: f64, y: f64, w: f64, h: f64, area: [f64; 4], margin: f64) -> (f64, f64) {
    let [ax, ay, aw, ah] = area;
    let cx = x.max(ax + margin).min(ax + aw - w - margin).max(ax);
    let cy = y.max(ay + margin).min(ay + ah - h - margin).max(ay);
    (cx, cy)
}

/// Position der Steuerleiste: unter dem Bereich (18 pt Abstand), sonst darüber, sonst im Bereich unten.
pub fn controls_pos(r: [f64; 4], work: [f64; 4], w: f64, h: f64) -> (f64, f64) {
    let [rx, ry, rw, rh] = r;
    let x = rx + rw / 2.0 - w / 2.0;
    let mut y = ry + rh + 18.0;
    if y + h > work[1] + work[3] - 8.0 {
        y = ry - h - 18.0;
        if y < work[1] + 8.0 {
            y = ry + rh - h - 40.0;
        }
    }
    clamp_rect(x, y, w, h, work, 8.0)
}

fn frame_window(app: &AppHandle, sc: &core::Screen, payload: Value) {
    let label = W_FRAME;
    let w = core::window(
        app,
        label,
        label,
        Win { w: sc.width, h: sc.height, pos: Some((sc.x, sc.y)), level: Some(core::LEVEL_FLOATING + 1), ..Default::default() },
        payload.clone(),
    );
    if let Some(w) = w {
        let _ = w.set_position(tauri::LogicalPosition::new(sc.x, sc.y));
        let _ = w.set_size(tauri::LogicalSize::new(sc.width, sc.height));
        let _ = w.set_ignore_cursor_events(true);
        core::exclude_from_capture(&w, true);
        let _ = app.emit_to(label, "rec-frame", payload);
    }
}

fn local_rect(r: &Region, sc: &core::Screen) -> [f64; 4] {
    [r.x - sc.x, r.y - sc.y, r.width, r.height]
}

fn show_toolbar(app: &AppHandle, region: Region) {
    let sc = screen_of(app, &region);
    {
        let mut f = FLOW.lock().unwrap();
        f.phase = Phase::Toolbar;
        f.region = Some(region.clone());
        f.sys_audio = crate::settings::get().recording_system_audio;
    }
    CANCEL.store(false, Ordering::Relaxed);
    if !region.fullscreen {
        frame_window(app, &sc, json!({ "mode": "select", "rect": local_rect(&region, &sc), "dim": false }));
    }
    let (w, h) = (260.0, 420.0);
    let (x, y) = clamp_rect(region.x + region.width / 2.0 - w / 2.0, region.y + region.height / 2.0 - 120.0, w, h, sc.work, 8.0);
    let data = json!({
        "width": (region.width * sc.scale).round(),
        "height": (region.height * sc.scale).round(),
        "window": region.window_id.is_some(),
    });
    if let Some(win) = core::window(app, W_TOOLBAR, W_TOOLBAR, Win { w, h, pos: Some((x, y)), focus: true, level: Some(core::LEVEL_POPUP), ..Default::default() }, data) {
        let _ = win.set_position(tauri::LogicalPosition::new(x, y));
        core::exclude_from_capture(&win, true);
        let _ = win.set_focus();
    }
    let a = app.clone();
    crate::actions::register_temp(app, "Escape", move |_| {
        let a = a.clone();
        std::thread::spawn(move || escape(&a));
    });
}

/// Esc in Leiste/Countdown: zuerst den Kamera-Präsentationsmodus verlassen, sonst abbrechen
/// (Capso `cancelPendingRecordingFlow`).
fn escape(app: &AppHandle) {
    if crate::camera::is_open() && crate::camera::presentation_active() {
        crate::camera::toggle_presentation(app);
    } else {
        cancel(app);
    }
}

fn close(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.close();
    }
}

/// Leiste/Countdown abbrechen.
pub fn cancel(app: &AppHandle) {
    CANCEL.store(true, Ordering::Relaxed);
    crate::actions::unregister_temp(app, "Escape");
    for l in [W_TOOLBAR, W_FRAME, W_COUNTDOWN] {
        close(app, l);
    }
    let cam = {
        let mut f = FLOW.lock().unwrap();
        if f.session.is_some() {
            return;
        }
        f.phase = Phase::Idle;
        std::mem::take(&mut f.camera_by_us)
    };
    crate::camera::set_recording_frame(app, None);
    if cam && crate::camera::is_open() {
        crate::camera::close(app);
    }
    // Nach abgebrochenem Neustart: Schreibtisch-Symbole / Quick Access wiederherstellen.
    restore_after(app);
}

fn opts_from_settings(gif: bool, window: bool) -> Opts {
    let s = crate::settings::get();
    let sys_audio = FLOW.lock().unwrap().sys_audio;
    Opts {
        gif,
        mic: s.recording_mic_enabled,
        mic_device: s.recording_mic_device_id.clone(),
        system_audio: sys_audio,
        // Mit Editor zeichnet der Editor den Cursor aus der Telemetrie (wie Capso) – nicht ins Video brennen.
        show_cursor: if s.open_editor_after_recording { false } else { s.show_cursor },
        clicks: s.highlight_clicks && !window,
        keys: s.show_key_presses && !window,
    }
}

/// „Video“/„GIF“ in der Leiste gedrückt.
fn record_pressed(app: &AppHandle, gif: bool) {
    let window = FLOW.lock().unwrap().region.as_ref().is_some_and(|r| r.window_id.is_some());
    if phase() != Phase::Toolbar {
        return;
    }
    set_phase(Phase::Countdown);
    close(app, W_TOOLBAR);
    let mut opts = opts_from_settings(gif, window);
    let app = app.clone();
    std::thread::spawn(move || {
        if !countdown_and_checks(&app, &mut opts) {
            return cancel(&app);
        }
        begin(&app, opts);
    });
}

// ───────────────────────────── Countdown & Prüfungen ─────────────────────────────

fn free_bytes(dir: &Path) -> Option<u64> {
    let c = std::ffi::CString::new(dir.to_string_lossy().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

fn human_bytes(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1e9)
}

fn esc_as(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Modaler Hinweis (osascript). Letzter Knopf = Standard. Liefert den gedrückten Knopf.
fn alert(title: &str, msg: &str, buttons: &[&str]) -> Option<String> {
    let b = buttons.iter().map(|b| format!("\"{}\"", esc_as(b))).collect::<Vec<_>>().join(", ");
    let script = format!(
        "display alert \"{}\" message \"{}\" as warning buttons {{{b}}} default button {}",
        esc_as(title),
        esc_as(msg),
        buttons.len()
    );
    core::sh("/usr/bin/osascript", &["-e", &script]).and_then(|o| o.split_once("button returned:").map(|(_, b)| b.trim().to_owned()))
}

/// Speicherplatz-Warnung, Tasten-Overlay-Berechtigung, Countdown. false = abgebrochen.
fn countdown_and_checks(app: &AppHandle, opts: &mut Opts) -> bool {
    CANCEL.store(false, Ordering::Relaxed);
    let s = crate::settings::get();
    if s.warn_about_low_disk_space {
        if let Some(free) = free_bytes(&core::temp_dir()).filter(|f| *f < DISK_WARN) {
            let msg = format!(
                "{} {} {}",
                t("Nur", "Only"),
                human_bytes(free),
                t("sind frei. Die Aufnahme wird automatisch beendet, wenn der Platz ausgeht.", "is free. The recording will be stopped automatically if space runs out.")
            );
            let (cancel_b, never, ok) = (t("Abbrechen", "Cancel"), t("Nicht mehr warnen", "Don't Warn Again"), t("Trotzdem aufnehmen", "Record Anyway"));
            match alert(t("Dein Speicher ist fast voll", "Your disk is almost full"), &msg, &[cancel_b, never, ok]).as_deref() {
                Some(b) if b == ok => {}
                Some(b) if b == never => crate::settings::update(app, |s| s.warn_about_low_disk_space = false),
                _ => return false,
            }
        }
    }
    // Tasten-Overlay braucht „Eingabeüberwachung“ (Capso `KeyPressOverlayPermissionDecision`).
    if opts.keys && !input::can_listen_keys() && !input::request_listen_keys() {
        let (open_b, without, cancel_b) = (t("Einstellungen öffnen", "Open Settings"), t("Ohne Overlay aufnehmen", "Record Without Overlay"), t("Abbrechen", "Cancel"));
        match alert(
            t("Eingabeüberwachung erforderlich", "Input Monitoring Required"),
            t(
                "Um Tastendrücke anzuzeigen, braucht nrzCapture die Berechtigung „Eingabeüberwachung“. Erlaube sie in den Systemeinstellungen und starte die Aufnahme erneut.",
                "To show key presses, nrzCapture needs Input Monitoring access. Allow it in System Settings and start the recording again.",
            ),
            &[cancel_b, without, open_b],
        )
        .as_deref()
        {
            Some(b) if b == without => opts.keys = false,
            Some(b) if b == open_b => {
                core::open_privacy("Privacy_ListenEvent");
                return false;
            }
            _ => return false,
        }
    }
    if s.show_countdown {
        let region = FLOW.lock().unwrap().region.clone().unwrap_or_default();
        let sz = 120.0;
        let pos = (region.x + region.width / 2.0 - sz / 2.0, region.y + region.height / 2.0 - sz / 2.0);
        if let Some(w) = core::window(app, W_COUNTDOWN, W_COUNTDOWN, Win { w: sz, h: sz, pos: Some(pos), level: Some(core::LEVEL_POPUP), ..Default::default() }, json!({ "n": 3 })) {
            let _ = w.set_ignore_cursor_events(true);
            core::exclude_from_capture(&w, true);
        }
        for n in [3, 2, 1] {
            let _ = app.emit_to(W_COUNTDOWN, "rec-countdown", json!({ "n": n }));
            for _ in 0..10 {
                if CANCEL.load(Ordering::Relaxed) {
                    close(app, W_COUNTDOWN);
                    return false;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        close(app, W_COUNTDOWN);
    }
    crate::actions::unregister_temp(app, "Escape");
    !CANCEL.load(Ordering::Relaxed)
}

// ───────────────────────────── Aufnahme ─────────────────────────────

/// CGWindowID (NSWindow.windowNumber) eines Tauri-Fensters.
fn window_number(w: &WebviewWindow) -> Option<u32> {
    let (tx, rx) = std::sync::mpsc::channel();
    let w2 = w.clone();
    w.run_on_main_thread(move || {
        let n = w2.ns_window().ok().map(|p| unsafe { (*(p as *const objc2_app_kit::NSWindow)).windowNumber() });
        let _ = tx.send(n);
    })
    .ok()?;
    rx.recv_timeout(Duration::from_secs(2)).ok().flatten().filter(|n| *n > 0).map(|n| n as u32)
}

/// Overlay-Fenster über den ganzen Bildschirm, klick-durchlässig, wird mit aufgenommen.
fn overlay_window(app: &AppHandle, label: &str, pos: (f64, f64), size: (f64, f64), data: Value) -> Option<WebviewWindow> {
    let w = core::window(app, label, label, Win { w: size.0, h: size.1, pos: Some(pos), level: Some(core::LEVEL_POPUP + 1), ..Default::default() }, data)?;
    let _ = w.set_position(tauri::LogicalPosition::new(pos.0, pos.1));
    let _ = w.set_size(tauri::LogicalSize::new(size.0, size.1));
    let _ = w.set_ignore_cursor_events(true);
    Some(w)
}

/// Position des Tasten-Overlays: unten links im Aufnahmebereich (24 pt Rand) bzw. gespeicherter Versatz.
/// Port von Capso `KeyPressOverlayPlacement` (dort AppKit-Koordinaten mit Ursprung unten links; der Versatz
/// `keyPressOverlayOffset` ist daher (x ab links, y ab unten)).
pub fn keys_overlay_frame(r: [f64; 4], offset: Option<(f64, f64)>) -> [f64; 4] {
    let margin = 24.0;
    let [rx, ry, rw, rh] = r;
    let w = (rw - 2.0 * margin).clamp(80.0, 720.0).min(rw);
    let h = (rh - 2.0 * margin).clamp(40.0, 320.0).min(rh);
    let (ox, oy) = offset.unwrap_or((margin, margin));
    let hm = margin.min(((rw - w) / 2.0).max(0.0));
    let vm = margin.min(((rh - h) / 2.0).max(0.0));
    let x = (rx + ox).clamp(rx + hm, (rx + rw - w - hm).max(rx + hm));
    let bottom = (oy).clamp(vm, (rh - h - vm).max(vm));
    [x, ry + rh - bottom - h, w, h]
}

fn begin(app: &AppHandle, opts: Opts) {
    let Some(region) = FLOW.lock().unwrap().region.clone() else { return set_phase(Phase::Idle) };
    let sc = screen_of(app, &region);
    let s = crate::settings::get();
    crate::quick::hide_for_capture(app, true);

    // Schreibtisch-Symbole für die Dauer der Aufnahme abdecken (unabhängig vom manuellen Umschalter)
    let icons_by_us = s.hide_desktop_icons_while_recording;
    if icons_by_us {
        crate::prefs::set_desktop_icons_hidden_for_recording(app, true);
        FLOW.lock().unwrap().icons_by_us = true;
    }
    // Kamera-PiP (Präsentationsmodus/Standardplatz beziehen sich auf den Aufnahmebereich). Die Kamera wird nur
    // über die Leiste geöffnet (pro Sitzung aus, Capso `RecordingToolbarState.cameraEnabled = false`).
    crate::camera::set_recording_frame(app, Some([region.x, region.y, region.width, region.height]));

    // Rahmen + Abdunkeln
    if !region.fullscreen {
        frame_window(app, &sc, json!({ "mode": "recording", "rect": local_rect(&region, &sc), "dim": s.dim_screen_while_recording }));
    } else {
        close(app, W_FRAME);
    }
    let mut except = vec![];
    if opts.clicks {
        if let Some(w) = overlay_window(app, W_CLICKS, (sc.x, sc.y), (sc.width, sc.height), Value::Null) {
            except.push(w);
        }
    }
    let rect = [region.x, region.y, region.width, region.height];
    let keys_frame = keys_overlay_frame(rect, s.key_press_overlay_offset);
    if opts.keys {
        if let Some(w) = overlay_window(app, W_KEYS, (keys_frame[0], keys_frame[1]), (keys_frame[2], keys_frame[3]), Value::Null) {
            // Ziehbar (Capso `isMovableByWindowBackground`): nach dem Verschieben in den Bereich klemmen und merken.
            *KEYS_HIT.lock().unwrap() = None;
            let a = app.clone();
            w.on_window_event(move |e| {
                if let tauri::WindowEvent::Moved(_) = e {
                    keys_moved(&a, rect);
                }
            });
            except.push(w);
        }
    }
    std::thread::sleep(Duration::from_millis(200));
    if let Some(w) = app.get_webview_window("camera") {
        except.push(w);
    }
    let except_ids: Vec<u32> = except.iter().filter_map(window_number).collect();

    let dir = core::temp_dir().join(format!("rec-{}", core::unique_id()));
    let _ = std::fs::create_dir_all(&dir);
    let started_ms = sck::now_ms();
    let cfg = sck::Config {
        rect,
        window_id: region.window_id,
        fullscreen: region.fullscreen,
        fps: 30,
        show_cursor: opts.show_cursor,
        system_audio: opts.system_audio,
        mic: opts.mic,
        mic_device: opts.mic_device.clone(),
        except_windows: except_ids,
    };
    let rec = match sck::Recorder::start(&cfg, &dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Aufnahme-Start fehlgeschlagen: {e}");
            core::toast(app, &format!("{}: {e}", t("Aufnahme konnte nicht gestartet werden", "Recording failed to start")));
            if !core::can_record() {
                core::ask_screen_permission();
            }
            hide_ui(app, true);
            restore_after(app);
            set_phase(Phase::Idle);
            return;
        }
    };
    set_marker(app, &dir);
    let clock = Arc::new(Clock::new());
    clock.start();
    let data = Arc::new(Mutex::new(TelemetryData {
        version: 1,
        width: region.width,
        height: region.height,
        shows_cursor: opts.show_cursor,
        user_shows_cursor: s.show_cursor,
        output_format: if opts.gif { "gif" } else { "mp4" }.into(),
        ..Default::default()
    }));
    let (a, sx, sy, clicks) = (app.clone(), sc.x, sc.y, opts.clicks);
    let mouse = MouseMonitor::start(rect, clock.clone(), data.clone(), move |x, y| {
        if clicks {
            let _ = a.emit_to(W_CLICKS, "rec-click", json!({ "x": x - sx, "y": y - sy }));
        }
    });
    // Tastendrücke für die Telemetrie, sobald „Eingabeüberwachung“ erlaubt ist; Overlay-Ereignisse nur mit
    // Tasten-Overlay (Capso `CursorTelemetry`: keyDown-Maske bei `CGPreflightListenEventAccess`).
    if opts.keys || input::can_listen_keys() {
        let (a, d, c, show) = (app.clone(), data.clone(), clock.clone(), opts.keys);
        input::start_key_tap(move |text, command| {
            if c.running() {
                d.lock().unwrap().keys.push(input::KeyEvent { t: c.elapsed() });
                if let (true, Some(text)) = (show, text) {
                    let _ = a.emit_to(W_KEYS, "rec-key", json!({ "text": text, "command": command }));
                }
            }
        });
    }

    // Steuerleiste
    let (cw, ch) = (250.0, 52.0);
    let (cx, cy) = controls_pos(rect, sc.work, cw, ch);
    if let Some(w) = core::window(app, W_CONTROLS, W_CONTROLS, Win { w: cw, h: ch, pos: Some((cx, cy)), level: Some(core::LEVEL_POPUP), ..Default::default() }, Value::Null) {
        let _ = w.set_position(tauri::LogicalPosition::new(cx, cy));
        core::exclude_from_capture(&w, true);
    }
    {
        let mut f = FLOW.lock().unwrap();
        f.session = Some(Session { rec, clock, mouse: Some(mouse), data, dir, shows_cursor: opts.show_cursor, warned: vec![], started_ms });
        f.opts = Some(opts);
        f.phase = Phase::Recording;
    }
    RECORDING.store(true, Ordering::Relaxed);
    let a = app.clone();
    crate::actions::register_temp(app, STOP_ACCEL, move |_| {
        let a = a.clone();
        std::thread::spawn(move || stop(&a));
    });
    crate::actions::refresh_menu(app);
    let a = app.clone();
    std::thread::spawn(move || monitor(&a));
    if FLOW.lock().unwrap().opts.as_ref().is_some_and(|o| o.keys) {
        let a = app.clone();
        std::thread::spawn(move || keys_hover(&a));
    }
}

/// Tasten-Overlay nimmt nur über den Blasen Mausereignisse an (sonst klick-durchlässig), damit man es ziehen kann,
/// ohne den Aufnahmebereich zu blockieren.
fn keys_hover(app: &AppHandle) {
    let mut accepting = false;
    while is_recording() {
        std::thread::sleep(Duration::from_millis(50));
        let Some(w) = app.get_webview_window(W_KEYS) else { break };
        let hit = *KEYS_HIT.lock().unwrap();
        let inside = hit.is_some_and(|h| {
            let sf = w.scale_factor().unwrap_or(2.0);
            let Ok(p) = w.outer_position().map(|p| p.to_logical::<f64>(sf)) else { return false };
            let (cx, cy) = input::cursor_pos();
            cx >= p.x + h[0] && cy >= p.y + h[1] && cx <= p.x + h[0] + h[2] && cy <= p.y + h[1] + h[3]
        });
        if inside != accepting {
            accepting = inside;
            let _ = w.set_ignore_cursor_events(!inside);
        }
    }
}

/// Versatz des Tasten-Overlays (x ab links, y ab unten) aus seinem Rahmen – Umkehrung von `keys_overlay_frame`
/// (Capso `KeyPressOverlayPlacement.offset`).
pub fn keys_offset_of(r: [f64; 4], frame: [f64; 4]) -> (f64, f64) {
    (frame[0] - r[0], (r[1] + r[3]) - (frame[1] + frame[3]))
}

/// Tasten-Overlay wurde verschoben: nach kurzer Ruhe in den Aufnahmebereich klemmen und Versatz speichern
/// (Capso `KeyPressOverlayWindow.handleMoved`).
fn keys_moved(app: &AppHandle, rect: [f64; 4]) {
    let gen = KEYS_MOVE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        if KEYS_MOVE_GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let Some(w) = app.get_webview_window(W_KEYS) else { return };
        let sf = w.scale_factor().unwrap_or(2.0);
        let (Ok(p), Ok(sz)) = (w.outer_position(), w.outer_size()) else { return };
        let (p, sz) = (p.to_logical::<f64>(sf), sz.to_logical::<f64>(sf));
        let cur = [p.x, p.y, sz.width, sz.height];
        let clamped = keys_overlay_frame(rect, Some(keys_offset_of(rect, cur)));
        if (clamped[0] - cur[0]).abs() > 0.5 || (clamped[1] - cur[1]).abs() > 0.5 {
            let _ = w.set_position(tauri::LogicalPosition::new(clamped[0], clamped[1]));
        }
        let off = keys_offset_of(rect, clamped);
        if crate::settings::get().key_press_overlay_offset != Some(off) {
            crate::settings::update(&app, |s| s.key_press_overlay_offset = Some(off));
        }
    });
}

/// Zeit an die Steuerleiste, Speicherplatz, stummes Mikrofon, vom System beendeter Stream.
fn monitor(app: &AppHandle) {
    let mut tick = 0u64;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        tick += 1;
        let mut f = FLOW.lock().unwrap();
        let mic = f.opts.as_ref().is_some_and(|o| o.mic);
        let sys = f.opts.as_ref().is_some_and(|o| o.system_audio);
        let Some(s) = f.session.as_mut() else { return };
        let elapsed = s.clock.elapsed();
        let paused = s.rec.is_paused();
        let _ = app.emit_to(W_CONTROLS, "rec-state", json!({ "elapsed": elapsed, "paused": paused }));
        if s.rec.shared.stream_stopped.load(Ordering::Relaxed) || s.rec.shared.failed.load(Ordering::Relaxed) {
            drop(f);
            let a = app.clone();
            std::thread::spawn(move || stop(&a));
            return;
        }
        let mut warn = None;
        let sh = &s.rec.shared;
        let now = sck::now_ms();
        let since_start = now.saturating_sub(s.started_ms) as f64 / 1000.0;
        let last = |a: &std::sync::atomic::AtomicU64| match a.load(Ordering::Relaxed) {
            0 => s.started_ms,
            v => v,
        };
        let issues = audio_issues(
            mic.then(|| now.saturating_sub(last(&sh.mic_last_ms))),
            sh.mic_peak() > MIC_SILENCE_PEAK,
            sys.then(|| now.saturating_sub(last(&sh.sys_last_ms))),
            since_start,
        );
        if let Some(kind) = issues.into_iter().find(|k| !s.warned.contains(k)) {
            s.warned.push(kind);
            warn = match kind {
                "micStopped" => Some((kind, t("Das Mikrofon sendet keinen Ton mehr. Es wurde eventuell getrennt.", "The microphone stopped sending audio. It may have been disconnected."))),
                "micSilent" if crate::settings::get().warn_about_muted_microphone => Some((
                    kind,
                    t(
                        "Dein Mikrofon ist möglicherweise stummgeschaltet. Seit Beginn der Aufnahme wurde kein Ton aufgenommen.",
                        "Your microphone might be muted. No sound has been picked up since the recording started.",
                    ),
                )),
                "systemAudioStopped" => Some((
                    kind,
                    t(
                        "Der Systemton wird nicht mehr aufgenommen. Der Rest der Aufnahme hat eventuell keinen Ton.",
                        "System audio stopped being captured. The rest of the recording may have no sound.",
                    ),
                )),
                _ => None,
            };
        }
        drop(f);
        if let Some((kind, msg)) = warn {
            show_warning(app, kind, msg);
        }
        if tick % 20 == 0 {
            if let Some(free) = free_bytes(&core::temp_dir()).filter(|b| *b < DISK_STOP) {
                eprintln!("Wenig Speicher ({free} B) – Aufnahme wird beendet");
                stop(app);
                alert(
                    t("Aufnahme beendet, weil der Speicher fast voll ist", "Recording stopped because your disk is almost full"),
                    t(
                        "Die Aufnahme wurde beendet und gesichert, um Datenverlust zu vermeiden. Gib Speicherplatz frei, bevor du erneut aufnimmst.",
                        "The recording was stopped and saved to prevent data loss. Free up disk space before recording again.",
                    ),
                    &["OK"],
                );
                return;
            }
        }
    }
}

/// Audio-Probleme (Capso `AudioActivityMonitor.newIssues`): `mic_gap_ms`/`sys_gap_ms` = Zeit seit dem letzten
/// Puffer (None = nicht aufgenommen), `heard` = Mikrofon war je lauter als −80 dBFS, `since_start` in s.
pub fn audio_issues(mic_gap_ms: Option<u64>, heard: bool, sys_gap_ms: Option<u64>, since_start: f64) -> Vec<&'static str> {
    let mut v = vec![];
    if let Some(gap) = mic_gap_ms {
        if gap > AUDIO_STALL_MS {
            v.push("micStopped");
        } else if !heard && since_start >= MIC_SILENCE_SECS {
            v.push("micSilent");
        }
    }
    if sys_gap_ms.is_some_and(|g| g > AUDIO_STALL_MS) {
        v.push("systemAudioStopped");
    }
    v
}

fn show_warning(app: &AppHandle, kind: &str, msg: &str) {
    let Some(ctl) = app.get_webview_window(W_CONTROLS) else { return };
    let sf = ctl.scale_factor().unwrap_or(2.0);
    let p = ctl.outer_position().map(|p| p.to_logical::<f64>(sf)).unwrap_or(tauri::LogicalPosition::new(100.0, 100.0));
    let (w, h) = (380.0, 112.0);
    let x = p.x + 125.0 - w / 2.0;
    let y = if p.y - h - 10.0 > 30.0 { p.y - h - 10.0 } else { p.y + 62.0 };
    if let Some(win) = core::window(app, W_WARNING, W_WARNING, Win { w, h, pos: Some((x, y)), level: Some(core::LEVEL_POPUP), ..Default::default() }, json!({ "kind": kind, "message": msg })) {
        core::exclude_from_capture(&win, true);
    }
}

/// Alle Aufnahme-Fenster schließen. `camera`: auch die von uns geöffnete Kamera schließen.
fn hide_ui(app: &AppHandle, camera: bool) {
    for l in [W_TOOLBAR, W_FRAME, W_COUNTDOWN, W_CONTROLS, W_WARNING, W_CLICKS, W_KEYS] {
        close(app, l);
    }
    if camera {
        crate::camera::set_recording_frame(app, None);
        let cam = std::mem::take(&mut FLOW.lock().unwrap().camera_by_us);
        if cam && crate::camera::is_open() {
            crate::camera::close(app);
        }
    }
}

/// Schreibtisch-Symbole und Quick Access wiederherstellen.
fn restore_after(app: &AppHandle) {
    let icons = std::mem::take(&mut FLOW.lock().unwrap().icons_by_us);
    if icons {
        crate::prefs::set_desktop_icons_hidden_for_recording(app, false);
    }
    crate::quick::hide_for_capture(app, false);
}

type Ended = ((Vec<PathBuf>, TelemetryData), Opts, [f64; 4]);

/// Laufende Sitzung beenden. `keep_camera`: Kamera offen lassen (Neustart).
fn end_session(app: &AppHandle, keep_camera: bool) -> Option<Ended> {
    let (sess, opts, region) = {
        let mut f = FLOW.lock().unwrap();
        let sess = f.session.take()?;
        f.phase = Phase::Finishing;
        (sess, f.opts.clone()?, f.region.clone().unwrap_or_default())
    };
    RECORDING.store(false, Ordering::Relaxed);
    crate::actions::unregister_temp(app, STOP_ACCEL);
    input::stop_key_tap();
    let Session { rec, clock, mut mouse, data: _, dir: _, shows_cursor, .. } = sess;
    clock.pause();
    let mut data = mouse.take().map(|m| m.finish()).unwrap_or_default();
    data.duration = clock.elapsed();
    data.shows_cursor = shows_cursor;
    let segments = rec.stop();
    hide_ui(app, !keep_camera);
    if !keep_camera {
        restore_after(app);
    }
    crate::actions::refresh_menu(app);
    Some(((segments, data), opts, [region.x, region.y, region.width, region.height]))
}

fn discard(segments: &[PathBuf]) {
    if let Some(dir) = segments.first().and_then(|p| p.parent()) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Pfad der Cursor-Telemetrie zu einem Video (`foo.mp4` → `foo.cursor.json`).
pub fn telemetry_path(video: &Path) -> PathBuf {
    video.with_extension("cursor.json")
}

fn finish(app: &AppHandle, segments: Vec<PathBuf>, data: TelemetryData, opts: Opts, _rect: [f64; 4]) {
    let Some(dir) = segments.first().and_then(|p| p.parent()).map(Path::to_path_buf) else {
        core::toast(app, t("Aufnahme enthält keine Daten", "Recording contains no data"));
        return;
    };
    let s = crate::settings::get();
    let raw = dir.join("raw.mp4");
    if let Err(e) = export::concat(&segments, &raw) {
        core::toast(app, &format!("{}: {e}", t("Aufnahme konnte nicht gesichert werden", "Recording could not be saved")));
        return;
    }
    let write_telemetry = |video: &Path| {
        let _ = std::fs::write(telemetry_path(video), serde_json::to_vec(&data).unwrap_or_default());
    };

    // Editor-Ablauf: Rohvideo + Telemetrie an den Aufnahme-Editor (der exportiert MP4/GIF selbst).
    let auto_upload = s.cloud_share_auto_upload && s.is_cloud_share_configured();
    if s.open_editor_after_recording {
        let video = core::unique_path(&dir, &core::file_name(&s.recording_filename_template, "", "mp4"));
        let _ = std::fs::rename(&raw, &video);
        write_telemetry(&video);
        crate::history::add(app, &video, if opts.gif { "gif" } else { "recording" });
        crate::receditor::open(app, &video);
        if opts.gif && (s.recording_auto_copy || auto_upload) {
            // GIF-Aufnahme: GIF in den Temp-Ordner exportieren, dann kopieren/hochladen (Capso `exportRecordingToClipboard`).
            let (a, quality) = (app.clone(), s.export_quality.clone());
            std::thread::spawn(move || {
                let gif = core::temp_file("gif");
                match export::gif(&video, &quality, &gif, |_| {}) {
                    Ok(()) => {
                        if s.recording_auto_copy {
                            core::clipboard::copy_file(&gif);
                        }
                        if auto_upload {
                            crate::share::upload_and_copy(&a, &gif);
                        }
                    }
                    Err(e) => core::toast(&a, &format!("{}: {e}", t("GIF konnte nicht kopiert werden", "GIF could not be copied"))),
                }
            });
        } else {
            if s.recording_auto_copy {
                core::clipboard::copy_file(&video);
            }
            if auto_upload {
                upload_later(app, video);
            }
        }
        return;
    }

    let (ext, kind, hist) = if opts.gif { ("gif", "gif", "gif") } else { ("mp4", "video", "recording") };
    let out = core::unique_path(&dir, &core::file_name(&s.recording_filename_template, "", ext));
    let res = if opts.gif {
        core::toast(app, t("GIF wird erstellt …", "Creating GIF…"));
        export::gif(&raw, &s.export_quality, &out, |_| {})
    } else {
        export::mp4(&raw, &s.export_quality, &out)
    };
    if let Err(e) = res {
        core::toast(app, &format!("{}: {e}", t("Export fehlgeschlagen", "Export failed")));
        return;
    }
    if !opts.gif {
        write_telemetry(&out);
    }
    let _ = std::fs::remove_file(&raw);
    if s.recording_auto_copy {
        core::clipboard::copy_file(&out);
    }
    crate::history::add(app, &out, hist);
    if auto_upload {
        upload_later(app, out.clone());
    }
    crate::quick::show(app, crate::quick::QuickItem { path: out, kind: kind.into(), saved: false });
}

/// Automatisch hochladen (Einstellung `cloudShareAutoUpload`, Capso `startAutomaticUploadIfNeeded`).
fn upload_later(app: &AppHandle, path: PathBuf) {
    let a = app.clone();
    std::thread::spawn(move || crate::share::upload_and_copy(&a, &path));
}

// ───────────────────────────── IPC ─────────────────────────────

#[derive(Deserialize)]
struct WarnAction {
    action: String,
    #[serde(default)]
    kind: String,
}

pub fn dispatch(app: &AppHandle, _win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    let spawn = |f: fn(&AppHandle)| {
        let a = app.clone();
        std::thread::spawn(move || f(&a));
    };
    match cmd {
        // Aufnahme-Leiste
        "toolbarInfo" => {
            let mics: Vec<Value> = microphones().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
            let sys = FLOW.lock().unwrap().sys_audio;
            Ok(json!({ "data": core::window_data(W_TOOLBAR), "mics": mics, "cameraOpen": crate::camera::is_open(), "systemAudio": sys }))
        }
        "setSystemAudio" => {
            FLOW.lock().unwrap().sys_audio = a["on"].as_bool().unwrap_or(true);
            Ok(Value::Null)
        }
        "escape" => {
            spawn(escape);
            Ok(Value::Null)
        }
        // Kamera-Gerät in der Leiste gewählt → Kamera mit diesem Gerät einblenden.
        "selectCamera" => {
            crate::camera::set_device(app, a["id"].as_str().filter(|s| !s.is_empty()).map(str::to_owned));
            if !crate::camera::is_open() {
                crate::camera::open(app);
            }
            let open = crate::camera::is_open();
            FLOW.lock().unwrap().camera_by_us = open;
            Ok(json!(open))
        }
        // Tasten-Overlay: Trefferfläche der Blasen (lokal, Punkte) bzw. Ziehen starten.
        "keysHit" => {
            let r = a["rect"].as_array().filter(|v| v.len() == 4).map(|v| {
                let n = |i: usize| v[i].as_f64().unwrap_or(0.0);
                [n(0), n(1), n(2), n(3)]
            });
            *KEYS_HIT.lock().unwrap() = r.filter(|r| r[2] > 0.0 && r[3] > 0.0);
            Ok(Value::Null)
        }
        "keysDrag" => {
            if let Some(w) = app.get_webview_window(W_KEYS) {
                let w2 = w.clone();
                let _ = w.run_on_main_thread(move || {
                    let _ = w2.start_dragging();
                });
            }
            Ok(Value::Null)
        }
        "record" => {
            record_pressed(app, a["format"].as_str() == Some("gif"));
            Ok(Value::Null)
        }
        "changeArea" => {
            cancel(app);
            let a = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                start(&a, false)
            });
            Ok(Value::Null)
        }
        "cancel" => {
            spawn(cancel);
            Ok(Value::Null)
        }
        "toggleCamera" => {
            crate::camera::toggle(app);
            let open = crate::camera::is_open();
            FLOW.lock().unwrap().camera_by_us = open;
            Ok(json!(open))
        }
        // Steuerleiste
        "togglePause" => {
            toggle_pause(app);
            Ok(Value::Null)
        }
        "stop" => {
            spawn(stop);
            Ok(Value::Null)
        }
        "restart" => {
            spawn(restart);
            Ok(Value::Null)
        }
        "delete" => {
            spawn(delete);
            Ok(Value::Null)
        }
        "state" => {
            let f = FLOW.lock().unwrap();
            Ok(match f.session.as_ref() {
                Some(s) => json!({ "elapsed": s.clock.elapsed(), "paused": s.rec.is_paused(), "recording": true }),
                None => json!({ "elapsed": 0.0, "paused": false, "recording": false }),
            })
        }
        // Warnhinweis
        "warningAction" => {
            let w: WarnAction = serde_json::from_value(a).map_err(|e| e.to_string())?;
            close(app, W_WARNING);
            match w.action.as_str() {
                "stop" => spawn(stop),
                "dontWarn" if w.kind == "micSilent" => crate::settings::update(app, |s| s.warn_about_muted_microphone = false),
                _ => {}
            }
            Ok(Value::Null)
        }
        "microphones" => Ok(json!(microphones().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>())),
        "isRecording" => Ok(json!(is_recording())),
        _ => Err(format!("record: unbekannter Befehl {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_below_above_inside() {
        let work = [0.0, 25.0, 1440.0, 875.0];
        // genug Platz unten
        assert_eq!(controls_pos([100.0, 100.0, 400.0, 300.0], work, 250.0, 52.0), (175.0, 418.0));
        // unten kein Platz → darüber
        let (_, y) = controls_pos([100.0, 600.0, 400.0, 280.0], work, 250.0, 52.0);
        assert_eq!(y, 600.0 - 52.0 - 18.0);
        // Vollbild → im Bereich unten
        let (x, y) = controls_pos([0.0, 0.0, 1440.0, 900.0], work, 250.0, 52.0);
        assert_eq!(x, 595.0);
        assert!(y + 52.0 <= 900.0 - 8.0 && y > 700.0);
    }

    #[test]
    fn clamp_into_area() {
        assert_eq!(clamp_rect(-50.0, -50.0, 100.0, 100.0, [0.0, 0.0, 500.0, 500.0], 8.0), (8.0, 8.0));
        assert_eq!(clamp_rect(480.0, 480.0, 100.0, 100.0, [0.0, 0.0, 500.0, 500.0], 8.0), (392.0, 392.0));
    }

    #[test]
    fn keys_overlay_default_and_offset() {
        let r = [100.0, 100.0, 800.0, 600.0];
        let f = keys_overlay_frame(r, None);
        assert_eq!(f[0], 124.0, "24 pt vom linken Rand");
        assert_eq!(f[1] + f[3], 100.0 + 600.0 - 24.0, "24 pt vom unteren Rand");
        let g = keys_overlay_frame(r, Some((10_000.0, -5.0)));
        assert!(g[0] + g[2] <= 900.0 && g[1] + g[3] <= 700.0);
        // winziger Bereich: Overlay bleibt im Bereich
        let s = keys_overlay_frame([0.0, 0.0, 60.0, 30.0], None);
        assert!(s[2] <= 60.0 && s[3] <= 30.0 && s[0] >= 0.0 && s[1] >= 0.0);
    }

    #[test]
    fn keys_offset_roundtrip() {
        let r = [100.0, 100.0, 800.0, 600.0];
        let f = keys_overlay_frame(r, Some((40.0, 60.0)));
        assert_eq!(keys_offset_of(r, f), (40.0, 60.0));
        // aus dem Bereich gezogen → geklemmt
        let far = [5000.0, -300.0, f[2], f[3]];
        let c = keys_overlay_frame(r, Some(keys_offset_of(r, far)));
        assert!(c[0] >= r[0] && c[0] + c[2] <= r[0] + r[2] && c[1] >= r[1] && c[1] + c[3] <= r[1] + r[3]);
    }

    #[test]
    fn audio_issue_detection() {
        // alles gut
        assert!(audio_issues(Some(100), true, Some(100), 20.0).is_empty());
        // Mikrofon > 5 s ohne Puffer – auch nach Beginn, nicht nur am Anfang
        assert_eq!(audio_issues(Some(5_001), true, None, 120.0), vec!["micStopped"]);
        // stumm nach 8 s
        assert_eq!(audio_issues(Some(50), false, None, 8.0), vec!["micSilent"]);
        assert!(audio_issues(Some(50), false, None, 7.9).is_empty());
        // System-Audio ausgefallen
        assert_eq!(audio_issues(None, false, Some(6_000), 30.0), vec!["systemAudioStopped"]);
    }

    #[test]
    fn remembered_area_moved_into_screen() {
        let sc = [0.0, 0.0, 1440.0, 900.0];
        assert_eq!(fit_into([100.0, 100.0, 400.0, 300.0], sc), [100.0, 100.0, 400.0, 300.0]);
        assert_eq!(fit_into([1300.0, 800.0, 400.0, 300.0], sc), [1040.0, 600.0, 400.0, 300.0]);
        assert_eq!(fit_into([-50.0, 0.0, 2000.0, 300.0], sc), [0.0, 0.0, 1440.0, 300.0]);
    }

    #[test]
    fn segment_names() {
        assert_eq!(segment_index("segment-0.mp4"), Some(0));
        assert_eq!(segment_index("segment-12.mp4"), Some(12));
        assert_eq!(segment_index("raw.mp4"), None);
        assert_eq!(segment_index("segment-x.mp4"), None);
    }

    #[test]
    fn telemetry_next_to_video() {
        assert_eq!(telemetry_path(Path::new("/tmp/a b.mp4")), PathBuf::from("/tmp/a b.cursor.json"));
    }

    #[test]
    fn applescript_escaping() {
        assert_eq!(esc_as(r#"a "b" \c"#), r#"a \"b\" \\c"#);
    }
}
