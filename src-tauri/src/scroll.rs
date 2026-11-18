//! Scroll-Aufnahme (lange Seiten zusammensetzen). Port von Capso `ScrollCaptureController` + `ScrollCaptureHUD`.
//! Ablauf: Bereich wählen → HUD (Start / Fertig / Abbrechen, Live-Vorschau) → während der Nutzer scrollt
//! (oder automatisch per CGEvent gescrollt wird) wiederholt aufnehmen → Versatz suchen → zusammensetzen
//! → `capture::deliver(…, " (Scrolling)")`.
use image::RgbaImage;
use serde_json::{json, Value};
use std::{
    ffi::c_void,
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, WebviewWindow};

use crate::capture::{self, geom::Rect, post, stitch, Intent, Purpose, Region};
use crate::core::{self, t, Win};

/// Höchstens so hoch wird die Seite (Pixel), wie in Capso.
const MAX_HEIGHT: u32 = 30_000;
/// Größe des HUD (Steuerung oben, Vorschau darunter).
const HUD: (f64, f64) = (300.0, 420.0);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cmd {
    Start,
    Done,
    Cancel,
    Auto(bool),
}

static CMD: Mutex<Option<mpsc::Sender<Cmd>>> = Mutex::new(None);

const ENTER_KEYS: [&str; 2] = ["Enter", "NumpadEnter"];

fn send(c: Cmd) {
    if let Some(tx) = CMD.lock().unwrap().as_ref() {
        let _ = tx.send(c);
    }
}

pub fn start(app: &AppHandle) {
    start_region(app, None);
}

/// Mit vorgegebenem Bereich starten (All-in-One „Scrollen“), sonst erst auswählen.
pub fn start_region(app: &AppHandle, region: Option<Region>) {
    if CMD.lock().unwrap().is_some() {
        return;
    }
    capture::remember_source_app();
    let region = match region.filter(|r| r.width >= 20.0 && r.height >= 20.0) {
        Some(r) => r,
        None => match capture::select_region(app, Purpose::Scroll) {
            Some(r) => {
                // Capso: Auswahl der Scroll-Aufnahme wird als „letzter Bereich“ gemerkt.
                capture::remember(app, &r);
                r
            }
            None => return,
        },
    };
    // Fenster gewählt → dessen Rahmen als Bereich (auf den Bildschirm begrenzt).
    let region = Region { window_id: None, fullscreen: false, ..region };
    let region = clamp_to_work_area(app, region);
    run(app, region);
}

/// Dock / Menüleiste nie mit aufnehmen (Port von `ScrollCaptureGeometry.clampedScreenLocalSelection`).
fn clamp_to_work_area(app: &AppHandle, r: Region) -> Region {
    let sel = Rect::new(r.x, r.y, r.width, r.height);
    for s in core::screens(app) {
        let work = Rect::new(s.work[0], s.work[1], s.work[2], s.work[3]);
        if Rect::new(s.x, s.y, s.width, s.height).contains(sel.mid()) {
            if let Some(i) = sel.intersect(&work) {
                return Region { x: i.x.round(), y: i.y.round(), width: i.w.round(), height: i.h.round(), screen: s.id, ..r };
            }
        }
    }
    r
}

/// HUD-Position: rechts neben der Auswahl, sonst links, sonst innen rechts.
pub fn hud_origin(sel: &Rect, work: &Rect, size: (f64, f64)) -> (f64, f64) {
    let gap = 12.0;
    let y = (sel.y).max(work.y + 8.0).min(work.max_y() - size.1 - 8.0);
    if sel.max_x() + gap + size.0 <= work.max_x() {
        (sel.max_x() + gap, y)
    } else if sel.x - gap - size.0 >= work.x {
        (sel.x - gap - size.0, y)
    } else {
        (sel.max_x() - size.0 - gap, y)
    }
}

fn emit(app: &AppHandle, v: Value) {
    let _ = app.emit_to("scroll-hud", "scroll-progress", v);
}

fn grab_rgba(r: &Region) -> Option<RgbaImage> {
    let p = capture::grab(r, false)?;
    let img = post::load(&p);
    let _ = std::fs::remove_file(&p);
    img
}

fn status(frames: u32, height: u32) -> String {
    format!("{frames} {} · {height} px", t("Bilder", "frames"))
}

fn run(app: &AppHandle, region: Region) {
    let (tx, rx) = mpsc::channel();
    *CMD.lock().unwrap() = Some(tx);
    let screens = core::screens(app);
    let work = screens
        .iter()
        .find(|s| s.id == region.screen)
        .map(|s| Rect::new(s.work[0], s.work[1], s.work[2], s.work[3]))
        .unwrap_or(Rect::new(0.0, 0.0, 1440.0, 900.0));
    let sel = Rect::new(region.x, region.y, region.width, region.height);
    let pos = hud_origin(&sel, &work, HUD);

    capture::show_border(app, &region, "scroll");
    let hud = core::window(
        app,
        "scroll-hud",
        "scroll-hud",
        Win { w: HUD.0, h: HUD.1, pos: Some(pos), level: Some(core::LEVEL_POPUP), shadow: false, ..Default::default() },
        json!({ "status": t("Klicke auf „Start“ und scrolle dann langsam nach unten.", "Click Start, then scroll down slowly.") }),
    );
    if let Some(w) = &hud {
        let _ = w.set_position(LogicalPosition::new(pos.0, pos.1));
        core::exclude_from_capture(w, true);
        let _ = w.show();
    }
    // Capso `ScrollCaptureHUD`: Esc bricht von überall ab, Return/Enter startet von überall (globale Tasten).
    crate::actions::register_temp(app, "Escape", |_| send(Cmd::Cancel));
    for k in ENTER_KEYS {
        crate::actions::register_temp(app, k, |_| send(Cmd::Start));
    }

    let result = capture_loop(app, &region, &rx);

    crate::actions::unregister_temp(app, "Escape");
    for k in ENTER_KEYS {
        crate::actions::unregister_temp(app, k);
    }
    *CMD.lock().unwrap() = None;
    if let Some(w) = app.get_webview_window("scroll-hud") {
        let _ = w.hide();
    }
    capture::hide_border(app);

    if let Some(img) = result {
        let p = core::temp_file("png");
        if post::save_png(&img, &p) {
            capture::deliver(app, p, Intent::Save, " (Scrolling)", Some(region));
        }
    }
}

fn capture_loop(app: &AppHandle, region: &Region, rx: &mpsc::Receiver<Cmd>) -> Option<RgbaImage> {
    let mut auto = false;
    // Auf „Start“ warten.
    loop {
        match rx.recv().ok()? {
            Cmd::Start => break,
            Cmd::Cancel => return None,
            Cmd::Done => return None,
            Cmd::Auto(on) => auto = on,
        }
    }
    let first = grab_rgba(region)?;
    let mut st = stitch::Stitcher::new();
    st.set_initial(first.clone());
    let mut prev = first;
    let mut fixed: Option<stitch::FixedRegions> = None;
    let mut frames = 1u32;
    let mut unchanged = 0u32;
    let mut last_preview = Instant::now() - Duration::from_secs(1);
    send_preview(app, &st, frames, true);
    if auto {
        start_auto(region);
    }
    loop {
        // Befehle abarbeiten.
        while let Ok(c) = rx.try_recv() {
            match c {
                Cmd::Done => return st.merged(),
                Cmd::Cancel => return None,
                Cmd::Auto(on) => {
                    auto = on;
                    if on {
                        start_auto(region);
                    }
                }
                Cmd::Start => {}
            }
        }
        if st.total_height() >= MAX_HEIGHT {
            emit(app, json!({ "status": t("Maximale Höhe erreicht", "Maximum height reached"), "limit": true }));
            return st.merged();
        }
        if auto {
            scroll_by(-((region.height * 0.6).max(40.0)) as i32);
            std::thread::sleep(Duration::from_millis(260));
        } else {
            std::thread::sleep(Duration::from_millis(120));
        }
        let Some(next) = grab_rgba(region) else { continue };
        if next.dimensions() != prev.dimensions() || next.as_raw() == prev.as_raw() {
            unchanged += 1;
            // Automatisches Scrollen: Seitenende erreicht → fertig.
            if auto && unchanged >= 3 {
                return st.merged();
            }
            continue;
        }
        unchanged = 0;
        let f = *fixed.get_or_insert_with(|| {
            let f = stitch::FixedRegions::detect(&prev, &next);
            st.set_footer(f.footer);
            f
        });
        let Some(d) = stitch::vertical_offset(&prev, &next, f).filter(|d| *d >= 3) else { continue };
        if let stitch::StitchResult::Stitched(_) = st.stitch(next.clone(), d) {
            prev = next;
            frames += 1;
            let throttle = last_preview.elapsed() >= Duration::from_millis(350);
            send_preview(app, &st, frames, throttle);
            if throttle {
                last_preview = Instant::now();
            }
        }
    }
}

/// Fortschritt + (gedrosselt) verkleinerte Vorschau des bisherigen Ergebnisses.
fn send_preview(app: &AppHandle, st: &stitch::Stitcher, frames: u32, with_image: bool) {
    let height = st.total_height();
    let mut v = json!({ "status": status(frames, height), "frames": frames, "height": height, "capturing": true });
    if with_image {
        if let Some(m) = st.merged() {
            let w = 240u32.min(m.width());
            let h = ((m.height() as u64 * w as u64) / m.width().max(1) as u64).max(1) as u32;
            let thumb = image::imageops::thumbnail(&m, w, h);
            let p = core::temp_file("png");
            if post::save_png(&thumb, &p) {
                v["preview"] = json!(p.to_string_lossy());
            }
        }
    }
    emit(app, v);
}

// ───────────────────────────── Automatisches Scrollen (CGEvent) ─────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreateScrollWheelEvent(source: *const c_void, units: u32, wheel_count: u32, wheel1: i32, ...) -> *mut c_void;
    fn CGEventPost(tap: u32, event: *const c_void);
    fn CGWarpMouseCursorPosition(p: CGPoint) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(p: *const c_void);
}

/// Mauszeiger in die Mitte des Bereichs (Scroll-Ereignisse gehen an das Fenster darunter).
fn start_auto(r: &Region) {
    if !core::has_accessibility() {
        core::open_privacy("Privacy_Accessibility");
        return;
    }
    unsafe {
        CGWarpMouseCursorPosition(CGPoint { x: r.x + r.width / 2.0, y: r.y + r.height / 2.0 });
    }
}

/// Um `pixels` scrollen (negativ = nach unten).
fn scroll_by(pixels: i32) {
    unsafe {
        let e = CGEventCreateScrollWheelEvent(std::ptr::null(), 0, 1, pixels);
        if !e.is_null() {
            CGEventPost(0, e);
            CFRelease(e);
        }
    }
}

pub fn dispatch(_app: &AppHandle, _win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    match cmd {
        "start" => send(Cmd::Start),
        "done" => send(Cmd::Done),
        "cancel" => send(Cmd::Cancel),
        "auto" => send(Cmd::Auto(a["on"].as_bool().unwrap_or(false))),
        _ => return Err(format!("scroll: unbekannter Befehl {cmd}")),
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_placement() {
        let work = Rect::new(0.0, 25.0, 1440.0, 850.0);
        assert_eq!(hud_origin(&Rect::new(100.0, 100.0, 400.0, 500.0), &work, (300.0, 420.0)), (512.0, 100.0));
        assert_eq!(hud_origin(&Rect::new(1000.0, 100.0, 400.0, 500.0), &work, (300.0, 420.0)), (688.0, 100.0));
        let (x, y) = hud_origin(&Rect::new(10.0, 30.0, 1420.0, 840.0), &work, (300.0, 420.0));
        assert_eq!((x, y), (1118.0, 33.0));
    }
}
