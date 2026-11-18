//! Fensterliste für die Fenstererkennung im Overlay (Port von Capso `WindowInfo`).
//! `CGWindowListCopyWindowInfo` liefert sichtbare Fenster von vorne nach hinten, Koordinaten in Punkten,
//! Ursprung oben links des Hauptbildschirms – dasselbe System wie `capture::Region`.
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use serde::Serialize;
use std::ffi::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut c_void;
}

const ON_SCREEN_ONLY: u32 = 1 << 0;
const EXCLUDE_DESKTOP: u32 = 1 << 4;

/// Systemprozesse, deren Fenster nicht auswählbar sein sollen (Capso: Dock, WindowManager, Wallpaper –
/// sie legen transparente bildschirmgroße Fenster über alles).
const IGNORED_OWNERS: &[&str] = &["Dock", "WindowManager", "Wallpaper", "Screenshot", "nrzCapture", "nrzcapture"];
/// `CGWindowLevelForKey(kCGMainMenuWindowLevelKey)` bzw. `kCGScreenSaverWindowLevelKey`.
const MAIN_MENU_LEVEL: i64 = 24;
const SCREEN_SAVER_LEVEL: i64 = 1000;

/// Capso `WindowInfo.isCaptureCandidate`: normale Fenster (Ebene 0), erhöhte App-Fenster (Menüs, Popover,
/// schwebende Paletten; 0 < Ebene < Bildschirmschoner) und die System-Menüleiste.
pub fn is_candidate(w: f64, h: f64, layer: i64, app: &str, title: &str) -> bool {
    let has_owner = !app.is_empty() && app != "Window Server";
    let menu_bar = app == "Window Server" && layer == MAIN_MENU_LEVEL;
    let elevated = has_owner && layer > 0 && layer < SCREEN_SAVER_LEVEL;
    let size_ok = w > 100.0 && (h > 50.0 || (menu_bar && h >= 20.0));
    let label_ok = !title.is_empty() || !app.is_empty();
    size_ok && (has_owner || menu_bar) && (label_ok || menu_bar) && (layer == 0 || elevated || menu_bar)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub app: String,
    pub title: String,
    pub pid: i64,
    /// Fensterebene (0 = normal; > 0 = Menü, Popover, Menüleiste …). Solche Fenster werden aus dem Standbild ausgeschnitten.
    pub layer: i64,
}

type Dict = NSDictionary<NSString, AnyObject>;

fn get(d: &Dict, k: &str) -> Option<Retained<AnyObject>> {
    d.objectForKey(&NSString::from_str(k))
}

fn num(d: &Dict, k: &str) -> Option<f64> {
    get(d, k)?.downcast::<NSNumber>().ok().map(|n| n.doubleValue())
}

fn string(d: &Dict, k: &str) -> String {
    get(d, k).and_then(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string()).unwrap_or_default()
}

fn core_menu_bar_name() -> String {
    crate::core::t("Menüleiste", "Menu Bar").to_string()
}

/// Auswählbare Fenster, von vorne nach hinten, ohne eigene Fenster und Systemleisten.
pub fn list() -> Vec<WindowInfo> {
    let own = std::process::id() as i64;
    let ptr = unsafe { CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0) };
    if ptr.is_null() {
        return vec![];
    }
    // SAFETY: CFArray ist gebührenfrei zu NSArray überbrückt; „Copy“-Regel → wir besitzen die Referenz.
    let Some(arr) = (unsafe { Retained::from_raw(ptr as *mut NSArray<NSDictionary<NSString, AnyObject>>) }) else { return vec![] };
    let mut out = Vec::new();
    for d in arr.iter() {
        let layer = num(&d, "kCGWindowLayer").unwrap_or(0.0) as i64;
        let alpha = num(&d, "kCGWindowAlpha").unwrap_or(1.0);
        let pid = num(&d, "kCGWindowOwnerPID").unwrap_or(0.0) as i64;
        let app = string(&d, "kCGWindowOwnerName");
        if alpha <= 0.01 || pid == own || IGNORED_OWNERS.contains(&app.as_str()) {
            continue;
        }
        let Some(b) = get(&d, "kCGWindowBounds").and_then(|o| o.downcast::<NSDictionary>().ok()) else { continue };
        // SAFETY: kCGWindowBounds ist ein Wörterbuch NSString → NSNumber.
        let b: Retained<Dict> = unsafe { Retained::cast_unchecked(b) };
        let (Some(x), Some(y), Some(w), Some(h)) = (num(&b, "X"), num(&b, "Y"), num(&b, "Width"), num(&b, "Height")) else { continue };
        let title = string(&d, "kCGWindowName");
        if !is_candidate(w, h, layer, &app, &title) {
            continue;
        }
        let Some(id) = num(&d, "kCGWindowNumber") else { continue };
        let app = if app == "Window Server" { core_menu_bar_name() } else { app };
        out.push(WindowInfo { id: id as u32, x, y, width: w, height: h, title, app, pid, layer });
    }
    out
}

/// Rahmen eines Fensters (für „letzter Bereich = Fenster“ und Aufnahmen per Fenster-ID).
pub fn bounds(id: u32) -> Option<WindowInfo> {
    list().into_iter().find(|w| w.id == id)
}

#[cfg(test)]
mod tests {
    use super::is_candidate;

    #[test]
    fn candidates() {
        assert!(is_candidate(800.0, 600.0, 0, "Safari", ""));
        assert!(is_candidate(220.0, 300.0, 101, "Finder", "")); // Kontextmenü
        assert!(is_candidate(1440.0, 24.0, 24, "Window Server", "Menubar"));
        assert!(!is_candidate(1440.0, 24.0, 0, "Safari", "")); // zu flach
        assert!(!is_candidate(800.0, 600.0, 1000, "Saver", "x"));
        assert!(!is_candidate(800.0, 600.0, 3, "Window Server", "x"));
        assert!(!is_candidate(90.0, 600.0, 0, "Safari", "x"));
    }
}
