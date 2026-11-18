//! Eingaben während der Aufnahme: Cursor-Telemetrie, Klick-Erkennung (für Hervorhebung + Telemetrie)
//! und globaler Tasten-Tap (für das Tasten-Overlay). Port von Capso `CursorTelemetry`, `ClickMonitor`, `KeyPressMonitor`.
//!
//! Maus: Abfrage mit 60 Hz über `CGEventCreate`/`CGEventSourceButtonState` – braucht keine Berechtigung.
//! Tasten: `CGEventTap` (nur lesen) auf eigenem Thread – braucht „Eingabeüberwachung“ (`CGPreflightListenEventAccess`).
//!
//! # Format `<video>.cursor.json` (wird vom Aufnahme-Editor gelesen)
//! Liegt neben dem Video: `foo.mp4` → `foo.cursor.json` (`Path::with_extension("cursor.json")`).
//! ```json
//! {
//!   "version": 1,
//!   "width": 1280.0, "height": 720.0,      // Aufnahmebereich in Punkten
//!   "duration": 12.34,                     // Sekunden (ohne Pausen)
//!   "showsCursor": false,                  // Mauszeiger ins Video eingebrannt?
//!   "samples": [{"t": 0.016, "x": 0.5, "y": 0.25}],   // Cursorposition, x/y normiert 0…1, Ursprung oben links
//!   "clicks":  [{"t": 1.2, "x": 0.4, "y": 0.6, "button": "left"}],  // Maustaste gedrückt ("left" | "right")
//!   "ups":     [{"t": 1.3, "x": 0.4, "y": 0.6}],  // Maustaste losgelassen (Capso `CursorEventType.mouseUp`)
//!   "keys":    [{"t": 2.0}],               // Tastendrücke (ohne Tastenidentität), sobald „Eingabeüberwachung“ erlaubt ist
//!   "userShowsCursor": true,               // Einstellung „Mauszeiger anzeigen“ (Editor zeichnet den Cursor nur dann)
//!   "outputFormat": "mp4"                  // "mp4" | "gif" – Standard-Ausgabeformat des Aufnahme-Editors
//! }
//! ```
//! `t` ist die Videozeit in Sekunden (Pausen herausgerechnet). Abtastpunkte werden nur aufgezeichnet,
//! solange der Cursor im Aufnahmebereich ist, und nur bei Bewegung (spätestens alle 0,5 s ein Punkt).
use serde::Serialize;
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

// ───────────────────────────── Uhr mit Pause ─────────────────────────────

/// Aufnahmeuhr: zählt nur, solange nicht pausiert.
pub struct Clock {
    inner: Mutex<(Option<Instant>, Duration)>, // (läuft seit, bisher angesammelt)
}

impl Clock {
    pub fn new() -> Self {
        Self { inner: Mutex::new((None, Duration::ZERO)) }
    }
    pub fn start(&self) {
        let mut g = self.inner.lock().unwrap();
        if g.0.is_none() {
            g.0 = Some(Instant::now());
        }
    }
    pub fn pause(&self) {
        let mut g = self.inner.lock().unwrap();
        if let Some(s) = g.0.take() {
            g.1 += s.elapsed();
        }
    }
    pub fn running(&self) -> bool {
        self.inner.lock().unwrap().0.is_some()
    }
    pub fn elapsed(&self) -> f64 {
        let g = self.inner.lock().unwrap();
        (g.1 + g.0.map(|s| s.elapsed()).unwrap_or_default()).as_secs_f64()
    }
}

// ───────────────────────────── Telemetrie-Daten ─────────────────────────────

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct Sample {
    pub t: f64,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Click {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    pub button: &'static str,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct KeyEvent {
    pub t: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryData {
    pub version: u32,
    pub width: f64,
    pub height: f64,
    pub duration: f64,
    pub shows_cursor: bool,
    pub samples: Vec<Sample>,
    pub clicks: Vec<Click>,
    pub ups: Vec<Sample>,
    pub keys: Vec<KeyEvent>,
    pub user_shows_cursor: bool,
    pub output_format: String,
}

/// Globaler Punkt → normiert auf den Bereich `[x, y, w, h]` (None, falls außerhalb).
pub fn normalize(rect: [f64; 4], px: f64, py: f64) -> Option<(f64, f64)> {
    let [x, y, w, h] = rect;
    if w <= 0.0 || h <= 0.0 || px < x || py < y || px > x + w || py > y + h {
        return None;
    }
    Some((((px - x) / w).clamp(0.0, 1.0), ((py - y) / h).clamp(0.0, 1.0)))
}

impl TelemetryData {
    /// Abtastpunkt aufnehmen, falls bewegt oder ≥ 0,5 s seit dem letzten Punkt.
    pub fn push_sample(&mut self, s: Sample) {
        if let Some(l) = self.samples.last() {
            let moved = (l.x - s.x).abs() > 1e-4 || (l.y - s.y).abs() > 1e-4;
            if !moved && s.t - l.t < 0.5 {
                return;
            }
        }
        self.samples.push(s);
    }
}

// ───────────────────────────── CoreGraphics-FFI ─────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

type TapCallback = extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreate(src: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(e: *const c_void) -> CGPoint;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, cb: TapCallback, info: *mut c_void) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetIntegerValueField(e: *const c_void, field: u32) -> i64;
    fn CGEventGetFlags(e: *const c_void) -> u64;
    fn CGEventKeyboardGetUnicodeString(e: *const c_void, max: usize, actual: *mut usize, buf: *mut u16);
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(p: *const c_void);
    fn CFMachPortCreateRunLoopSource(alloc: *const c_void, port: *mut c_void, order: isize) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, src: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: *mut c_void);
    static kCFRunLoopCommonModes: *const c_void;
}

/// Aktuelle Cursorposition (global, Punkte, Ursprung oben links des Hauptbildschirms).
pub fn cursor_pos() -> (f64, f64) {
    unsafe {
        let e = CGEventCreate(std::ptr::null());
        if e.is_null() {
            return (0.0, 0.0);
        }
        let p = CGEventGetLocation(e);
        CFRelease(e);
        (p.x, p.y)
    }
}

/// Darf die App globale Tastendrücke lesen („Eingabeüberwachung“)?
pub fn can_listen_keys() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

/// Fragt „Eingabeüberwachung“ an (Systemdialog beim ersten Mal).
pub fn request_listen_keys() -> bool {
    unsafe { CGRequestListenEventAccess() }
}

// ───────────────────────────── Maus-Monitor ─────────────────────────────

/// Läuft bis `stop` gesetzt wird. `on_click(x, y)` in globalen Punkten (für die Klick-Hervorhebung).
pub struct MouseMonitor {
    stop: Arc<AtomicBool>,
    data: Arc<Mutex<TelemetryData>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MouseMonitor {
    pub fn start(rect: [f64; 4], clock: Arc<Clock>, data: Arc<Mutex<TelemetryData>>, on_click: impl Fn(f64, f64) + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, data2) = (stop.clone(), data.clone());
        let thread = std::thread::spawn(move || {
            let mut prev = [false, false];
            while !stop2.load(Ordering::Relaxed) {
                let (px, py) = cursor_pos();
                // kCGEventSourceStateCombinedSessionState = 0; Tasten 0 = links, 1 = rechts
                let now = unsafe { [CGEventSourceButtonState(0, 0), CGEventSourceButtonState(0, 1)] };
                let rec = clock.running();
                let t = clock.elapsed();
                let n = normalize(rect, px, py);
                for (i, name) in [(0usize, "left"), (1, "right")] {
                    if now[i] && !prev[i] && rec {
                        on_click(px, py);
                        if let Some((x, y)) = n {
                            data2.lock().unwrap().clicks.push(Click { t, x, y, button: name });
                        }
                    }
                    // Loslassen (Capso `.mouseUp`, nur innerhalb des Bereichs)
                    if !now[i] && prev[i] && rec {
                        if let Some((x, y)) = n {
                            data2.lock().unwrap().ups.push(Sample { t, x, y });
                        }
                    }
                }
                prev = now;
                if rec {
                    if let Some((x, y)) = n {
                        data2.lock().unwrap().push_sample(Sample { t, x, y });
                    }
                }
                std::thread::sleep(Duration::from_millis(16));
            }
        });
        Self { stop, data, thread: Some(thread) }
    }

    /// Anhalten und gesammelte Daten liefern.
    pub fn finish(mut self) -> TelemetryData {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let d = self.data.lock().unwrap().clone();
        d
    }
}

// ───────────────────────────── Tasten-Tap ─────────────────────────────

type KeyHandler = Box<dyn Fn(Option<String>, bool) + Send>;
static KEY_HANDLER: Mutex<Option<KeyHandler>> = Mutex::new(None);
static KEY_RUNLOOP: AtomicUsize = AtomicUsize::new(0);
static KEY_TAP: AtomicUsize = AtomicUsize::new(0);

extern "C" fn key_tap_cb(_proxy: *mut c_void, ty: u32, event: *mut c_void, _info: *mut c_void) -> *mut c_void {
    // kCGEventTapDisabledByTimeout / ByUserInput → wieder einschalten
    if ty == 0xFFFF_FFFE || ty == 0xFFFF_FFFF {
        let tap = KEY_TAP.load(Ordering::Relaxed);
        if tap != 0 {
            unsafe { CGEventTapEnable(tap as *mut c_void, true) };
        }
        return event;
    }
    if ty != 10 {
        return event; // nur kCGEventKeyDown
    }
    unsafe {
        let code = CGEventGetIntegerValueField(event, 9) as u16; // kCGKeyboardEventKeycode
        let repeat = CGEventGetIntegerValueField(event, 8) != 0; // kCGKeyboardEventAutorepeat
        let flags = CGEventGetFlags(event);
        let mut buf = [0u16; 8];
        let mut len = 0usize;
        CGEventKeyboardGetUnicodeString(event, 8, &mut len, buf.as_mut_ptr());
        let chars = String::from_utf16_lossy(&buf[..len.min(8)]);
        let ks = super::keys::Keystroke { key_code: code, characters: chars.clone(), characters_ignoring_modifiers: chars, flags };
        let text = super::keys::format(&ks, repeat, super::keys::translate_key_code);
        if let Some(h) = KEY_HANDLER.lock().unwrap().as_ref() {
            h(text, ks.is_command());
        }
    }
    event
}

/// Globalen Tasten-Tap starten. `on_key(text, is_command)` bei jedem keyDown (`text` = Overlay-Text, falls
/// darstellbar). Liefert false ohne Berechtigung.
pub fn start_key_tap(on_key: impl Fn(Option<String>, bool) + Send + 'static) -> bool {
    stop_key_tap();
    *KEY_HANDLER.lock().unwrap() = Some(Box::new(on_key));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe {
        // kCGSessionEventTap = 1, kCGHeadInsertEventTap = 0, kCGEventTapOptionListenOnly = 1, Maske keyDown (1 << 10)
        let tap = CGEventTapCreate(1, 0, 1, 1 << 10, key_tap_cb, std::ptr::null_mut());
        if tap.is_null() {
            let _ = tx.send(false);
            return;
        }
        let src = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        let rl = CFRunLoopGetCurrent();
        CFRunLoopAddSource(rl, src, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        KEY_TAP.store(tap as usize, Ordering::Relaxed);
        KEY_RUNLOOP.store(rl as usize, Ordering::Relaxed);
        let _ = tx.send(true);
        CFRunLoopRun();
        CGEventTapEnable(tap, false);
        KEY_TAP.store(0, Ordering::Relaxed);
        CFRelease(src);
        CFRelease(tap);
    });
    rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
}

pub fn stop_key_tap() {
    let rl = KEY_RUNLOOP.swap(0, Ordering::Relaxed);
    if rl != 0 {
        unsafe { CFRunLoopStop(rl as *mut c_void) };
    }
    *KEY_HANDLER.lock().unwrap() = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_inside_and_outside() {
        let r = [100.0, 50.0, 200.0, 100.0];
        assert_eq!(normalize(r, 200.0, 100.0), Some((0.5, 0.5)));
        assert_eq!(normalize(r, 100.0, 50.0), Some((0.0, 0.0)));
        assert_eq!(normalize(r, 300.0, 150.0), Some((1.0, 1.0)));
        assert_eq!(normalize(r, 99.0, 100.0), None);
        assert_eq!(normalize([0.0, 0.0, 0.0, 10.0], 0.0, 0.0), None);
    }

    #[test]
    fn samples_are_deduplicated() {
        let mut d = TelemetryData::default();
        d.push_sample(Sample { t: 0.0, x: 0.1, y: 0.1 });
        d.push_sample(Sample { t: 0.1, x: 0.1, y: 0.1 }); // unverändert → verworfen
        d.push_sample(Sample { t: 0.2, x: 0.2, y: 0.1 }); // bewegt
        d.push_sample(Sample { t: 0.8, x: 0.2, y: 0.1 }); // ≥ 0,5 s → Haltepunkt
        assert_eq!(d.samples.len(), 3);
    }

    #[test]
    fn clock_pauses() {
        let c = Clock::new();
        assert_eq!(c.elapsed(), 0.0);
        c.start();
        std::thread::sleep(Duration::from_millis(20));
        c.pause();
        let a = c.elapsed();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(a, c.elapsed());
        assert!(a >= 0.02 && !c.running());
    }

    #[test]
    fn telemetry_json_shape() {
        let d = TelemetryData { version: 1, width: 10.0, height: 5.0, samples: vec![Sample { t: 0.0, x: 0.5, y: 0.5 }], ..Default::default() };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["samples"][0]["x"], 0.5);
        assert!(v.get("showsCursor").is_some() && v["clicks"].is_array() && v["keys"].is_array() && v["ups"].is_array());
        assert!(v.get("userShowsCursor").is_some() && v.get("outputFormat").is_some());
    }
}
