//! Einstellungsfenster, Über, Schreibtisch-Symbole, System-Kürzel, Automations-URLs, Dock-Präsenz.
//! Port von Capso `Preferences`, `MenuBar`, `DesktopIconsCover`, `DockPresenceController`, `AutomationURLAction`.
//!
//! Integration: `prefs::init(&handle)` in `main.rs` → `setup` (optional – die Dock-Beobachter werden sonst
//! beim ersten Menüaufbau auf dem Main-Thread automatisch installiert).
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::{AppHandle, WebviewWindow};

use crate::core::{self, t, Win};

// ───────────────────────────── Fenster ─────────────────────────────

/// Einstellungen öffnen (optional bestimmter Tab: "general" | "screenshot" | "quickAccess" | "recording" | "shortcuts" | "ocr" | "translation" | "cloud" | "export" | "permissions" | "about").
pub fn open(app: &AppHandle, tab: Option<&str>) {
    init(app);
    core::window(app, "prefs", "prefs", Win { min_size: Some((720.0, 480.0)), ..Win::app(t("Einstellungen", "Preferences"), 820.0, 600.0) }, json!({ "tab": tab }));
    update_dock_presence(app);
}

pub fn about(app: &AppHandle) {
    open(app, Some("about"));
}

// ───────────────────────────── Dock-Präsenz ─────────────────────────────

/// Fensterzustand für die Dock-Entscheidung (reine Logik, testbar).
#[derive(Clone, Copy, Debug, Default)]
pub struct DockWin {
    /// Normales App-Fenster mit Titelleiste (Einstellungen, Editor, Verlauf, Aufnahme-Editor) – keine Overlays/Panels.
    pub app_window: bool,
    pub visible: bool,
    pub miniaturized: bool,
    pub on_active_space: bool,
}

/// Wie Capso `DockPresenceController.policy`: `Some(true)` = Regular (Dock + ⌘Tab), `Some(false)` = Accessory, `None` = unverändert.
pub fn dock_policy(wins: &[DockWin], app_hidden: bool) -> Option<bool> {
    if app_hidden {
        return None; // Ausgeblendet: Dock-Symbol behalten, damit man zurückkommt.
    }
    let tagged: Vec<&DockWin> = wins.iter().filter(|w| w.app_window && (w.visible || w.miniaturized)).collect();
    if tagged.is_empty() {
        return Some(false);
    }
    if tagged.iter().any(|w| w.miniaturized || w.on_active_space) {
        Some(true)
    } else {
        None // Fenster nur auf anderem Space: nicht dorthin zurückziehen.
    }
}

static DOCK_INIT: AtomicBool = AtomicBool::new(false);

/// Installiert die Fenster-Beobachter für die Dock-Präsenz (idempotent). Darf von jedem Thread gerufen werden.
pub fn init(app: &AppHandle) {
    if DOCK_INIT.load(Ordering::Relaxed) {
        return;
    }
    let _ = app.run_on_main_thread(native::install_dock_observers);
}

/// Dock-Symbol zeigen, solange ein App-Fenster offen ist (sonst nur Menüleiste).
pub fn update_dock_presence(app: &AppHandle) {
    let _ = app.run_on_main_thread(native::schedule_dock_update);
}

// ───────────────────────────── Schreibtisch-Symbole ─────────────────────────────

static ICONS_MANUAL: AtomicBool = AtomicBool::new(false);
static ICONS_RECORDING: AtomicBool = AtomicBool::new(false);

/// Menü / URL: Schreibtisch-Symbole mit dem Hintergrundbild abdecken bzw. wieder zeigen.
pub fn toggle_desktop_icons(app: &AppHandle) {
    set_desktop_icons_hidden(app, !ICONS_MANUAL.load(Ordering::Relaxed));
}

pub fn set_desktop_icons_hidden(app: &AppHandle, hidden: bool) {
    ICONS_MANUAL.store(hidden, Ordering::Relaxed);
    let _ = app.run_on_main_thread(native::update_cover);
    crate::actions::refresh_menu(app);
}

/// Für die Dauer einer Aufnahme (Einstellung `hideDesktopIconsWhileRecording`) – vom Aufnahme-Modul gerufen.
pub fn set_desktop_icons_hidden_for_recording(app: &AppHandle, hidden: bool) {
    ICONS_RECORDING.store(hidden, Ordering::Relaxed);
    let _ = app.run_on_main_thread(native::update_cover);
}

/// Manuell ausgeblendet (für die Menübeschriftung).
pub fn desktop_icons_hidden() -> bool {
    // Wird beim Menüaufbau (Main-Thread) gerufen → guter Zeitpunkt, die Dock-Beobachter ohne main.rs-Änderung zu installieren.
    if objc2::MainThreadMarker::new().is_some() {
        native::install_dock_observers();
    }
    ICONS_MANUAL.load(Ordering::Relaxed)
}

/// Beim Beenden: Abdeckung entfernen (die Fenster verschwinden mit dem Prozess ohnehin – keine Finder-Änderung nötig).
pub fn restore_desktop_icons() {
    ICONS_MANUAL.store(false, Ordering::Relaxed);
    ICONS_RECORDING.store(false, Ordering::Relaxed);
    if objc2::MainThreadMarker::new().is_some() {
        native::update_cover();
    }
}

// ───────────────────────────── macOS-Kürzel ⌘⇧3/4/5 ─────────────────────────────

/// Symbolic-Hotkey-IDs: 28 = ⌘⇧3, 30 = ⌘⇧4, 184 = ⌘⇧5 (mit Standard-Parametern: Zeichen, Tastencode, Modifikatoren).
const SYS_KEYS: [(u32, [i64; 3]); 3] = [(28, [51, 20, 1179648]), (30, [52, 21, 1179648]), (184, [53, 23, 1179648])];

fn sys_backup_file() -> PathBuf {
    core::data_dir().join("system-shortcuts-backup.json")
}

/// `defaults`-Wert für einen Symbolic-Hotkey-Eintrag.
pub fn hotkey_entry(enabled: bool, params: [i64; 3]) -> String {
    format!("{{enabled = {}; value = {{ parameters = ({}, {}, {}); type = standard; }}; }}", enabled as u8, params[0], params[1], params[2])
}

/// Aktueller Zustand der drei System-Kürzel: (id, aktiv, Parameter). Fehlender Eintrag = macOS-Standard (aktiv).
fn read_sys_states() -> Vec<(u32, bool, [i64; 3])> {
    let tmp = core::temp_file("plist");
    let tmp_s = tmp.to_string_lossy().to_string();
    let exported = core::sh("defaults", &["export", "com.apple.symbolichotkeys", &tmp_s]).is_some();
    let res = SYS_KEYS
        .iter()
        .map(|(id, def)| {
            if !exported {
                return (*id, true, *def);
            }
            let enabled = core::sh("plutil", &["-extract", &format!("AppleSymbolicHotKeys.{id}.enabled"), "raw", "-o", "-", &tmp_s])
                .map(|v| matches!(v.trim(), "true" | "1" | "YES"))
                .unwrap_or(true);
            let params = core::sh("plutil", &["-extract", &format!("AppleSymbolicHotKeys.{id}.value.parameters"), "json", "-o", "-", &tmp_s])
                .and_then(|j| serde_json::from_str::<Vec<i64>>(&j).ok())
                .filter(|v| v.len() == 3)
                .map(|v| [v[0], v[1], v[2]])
                .unwrap_or(*def);
            (*id, enabled, params)
        })
        .collect();
    let _ = std::fs::remove_file(&tmp);
    res
}

fn write_sys_states(states: &[(u32, bool, [i64; 3])]) {
    for (id, enabled, params) in states {
        core::sh("defaults", &["write", "com.apple.symbolichotkeys", "AppleSymbolicHotKeys", "-dict-add", &id.to_string(), &hotkey_entry(*enabled, *params)]);
    }
    core::sh("/System/Library/PrivateFrameworks/SystemAdministration.framework/Resources/activateSettings", &["-u"]);
}

/// Ob macOS ⌘⇧3/4/5 noch selbst belegt (mindestens eins aktiv).
pub fn system_shortcuts_enabled() -> bool {
    read_sys_states().iter().any(|(_, e, _)| *e)
}

/// macOS-eigene Kürzel ⌘⇧3/4/5 aus- (`replace = true`) bzw. wieder auf den gesicherten Zustand setzen.
/// Der Originalzustand wird in einer Datei gesichert – so wird er auch nach einem Absturz beim nächsten Start wiederhergestellt.
pub fn apply_system_shortcuts(replace: bool) {
    static STATE: Mutex<Option<bool>> = Mutex::new(None);
    let mut st = STATE.lock().unwrap();
    if *st == Some(replace) {
        return;
    }
    *st = Some(replace);
    let backup = sys_backup_file();
    if replace {
        if !backup.exists() {
            let orig: Vec<Value> = read_sys_states().into_iter().map(|(id, e, p)| json!({"id": id, "enabled": e, "params": p})).collect();
            let _ = std::fs::write(&backup, serde_json::to_vec(&orig).unwrap_or_default());
        }
        write_sys_states(&SYS_KEYS.map(|(id, p)| (id, false, p)));
    } else if let Some(orig) = std::fs::read(&backup).ok().and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok()) {
        let states: Vec<(u32, bool, [i64; 3])> = SYS_KEYS
            .iter()
            .map(|(id, def)| {
                let o = orig.iter().find(|v| v["id"].as_u64() == Some(*id as u64));
                let enabled = o.and_then(|v| v["enabled"].as_bool()).unwrap_or(true);
                let params = o
                    .and_then(|v| serde_json::from_value::<Vec<i64>>(v["params"].clone()).ok())
                    .filter(|v| v.len() == 3)
                    .map(|v| [v[0], v[1], v[2]])
                    .unwrap_or(*def);
                (*id, enabled, params)
            })
            .collect();
        write_sys_states(&states);
        let _ = std::fs::remove_file(&backup);
    }
}

// ───────────────────────────── Automations-URLs ─────────────────────────────

/// Ergebnis einer Automations-URL.
#[derive(Clone, Debug, PartialEq)]
pub enum UrlAction {
    /// Aktions-ID aus `actions::ACTIONS`.
    Run(&'static str),
    HideIcons,
    ShowIcons,
    Preferences(Option<String>),
}

/// Capso-Pfade (`capso://grab/<pfad>`) → Aktion. Bei nrzCapture: `nrzcapture://grab/<pfad>` oder `nrzcapture://<pfad>`.
const URL_PATHS: &[(&str, &str)] = &[
    ("area", "captureArea"),
    ("fullscreen", "captureFullscreen"),
    ("window", "captureWindow"),
    ("all-in-one", "captureAllInOne"),
    ("scrolling", "captureScrolling"),
    ("self-timer", "selfTimerCapture"),
    ("last-area", "captureLastArea"),
    ("clipboard", "captureAreaToClipboard"),
    ("area-to-clipboard", "captureAreaToClipboard"),
    ("share", "captureAreaAndShare"),
    ("area-and-share", "captureAreaAndShare"),
    ("annotate", "captureAreaAndAnnotate"),
    ("area-and-annotate", "captureAreaAndAnnotate"),
    ("area-and-pin", "captureAreaAndPin"),
    ("pin", "pinFromClipboard"),
    ("pin-from-clipboard", "pinFromClipboard"),
    ("edit-clipboard", "editClipboardImage"),
    ("edit-clipboard-image", "editClipboardImage"),
    ("open-image", "openImageFile"),
    ("open-image-file", "openImageFile"),
    ("ocr", "captureText"),
    ("text", "captureText"),
    ("capture-text", "captureText"),
    ("translate", "captureAndTranslate"),
    ("capture-and-translate", "captureAndTranslate"),
    ("translate-selection", "translateSelectedText"),
    ("translate-selected-text", "translateSelectedText"),
    ("translate-text", "translateTypedText"),
    ("translate-typed-text", "translateTypedText"),
    ("record", "recordScreen"),
    ("record-area", "recordScreen"),
    ("record-screen", "recordScreen"),
    ("record-fullscreen", "recordFullscreen"),
    ("record-full-screen", "recordFullscreen"),
    ("history", "screenshotHistory"),
    ("screenshot-history", "screenshotHistory"),
    ("toggle-desktop-icons", "toggleDesktopIcons"),
    ("toggle-pins", "togglePins"),
    ("camera", "toggleCameraPiP"),
];

/// URL wie Capso `AutomationURLAction(url:)` prüfen: Schema `nrzcapture`, kein Benutzer/Passwort/Port/Fragment,
/// Query nur `tab=` bei `preferences`. Akzeptiert `nrzcapture://grab/area`, `nrzcapture://area` und `nrzcapture://captureArea`.
pub fn parse_url(url: &str) -> Option<UrlAction> {
    let (scheme, rest) = url.trim().split_once("://")?;
    if !scheme.eq_ignore_ascii_case("nrzcapture") || rest.contains('#') {
        return None;
    }
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    let (host, sub) = match path.split_once('/') {
        Some((h, s)) => (h, s.trim_end_matches('/')),
        None => (path, ""),
    };
    if host.contains(['@', ':']) {
        return None;
    }
    let name = if host.eq_ignore_ascii_case("grab") {
        if sub.is_empty() || sub.contains('/') {
            return None;
        }
        sub
    } else {
        if !sub.is_empty() {
            return None;
        }
        host
    };
    let lower = name.to_ascii_lowercase();
    let tab = query.and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("tab=")).map(str::to_owned));
    if matches!(lower.as_str(), "preferences" | "settings") {
        if query.is_some() && tab.is_none() {
            return None;
        }
        return Some(UrlAction::Preferences(tab));
    }
    if query.is_some() {
        return None;
    }
    match lower.as_str() {
        "hide-desktop-icons" => return Some(UrlAction::HideIcons),
        "show-desktop-icons" => return Some(UrlAction::ShowIcons),
        _ => {}
    }
    if let Some((_, id)) = URL_PATHS.iter().find(|(p, _)| *p == lower) {
        return Some(UrlAction::Run(id));
    }
    // Direkte Aktions-ID (ohne „quit“, damit fremde Apps nrzCapture nicht beenden können).
    crate::actions::ACTIONS.iter().find(|a| a.eq_ignore_ascii_case(name) && **a != "quit" && **a != "preferences").map(|a| UrlAction::Run(a))
}

/// Frist für den Hinweis „Bildschirmfoto von einer anderen App“ (Capso `automatedCaptureDeadline`, 10 s).
static AUTOMATED_DEADLINE: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Nächste Aufnahme als von einer anderen App ausgelöst markieren (Capso `markNextCaptureAsAutomated`).
pub fn mark_next_capture_automated() {
    *AUTOMATED_DEADLINE.lock().unwrap() = Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
}

/// Für die Aufnahme-Verarbeitung: `true`, wenn die aktuelle Aufnahme innerhalb von 10 s nach einer
/// Automations-URL entstand (dann Hinweis zeigen). Setzt die Markierung immer zurück.
pub fn take_automated_flag() -> bool {
    AUTOMATED_DEADLINE.lock().unwrap().take().is_some_and(|d| std::time::Instant::now() < d)
}

/// Hinweis „Bildschirmfoto von einer anderen App“ (Capso-Benachrichtigung) als Toast.
pub fn notify_automated_capture(app: &AppHandle) {
    core::toast(
        app,
        &format!(
            "{} – {}",
            t("Bildschirmfoto von einer anderen App", "Screenshot taken by another app"),
            t("Eine andere App hat mit nrzCapture deinen Bildschirm aufgenommen.", "Another app used nrzCapture to capture your screen.")
        ),
    );
}

/// Wartet bis zu 10 s auf einen neuen Verlaufseintrag und zeigt dann den Automations-Hinweis,
/// sofern die Aufnahme-Verarbeitung die Markierung nicht schon selbst verbraucht hat.
fn watch_automated_capture(app: &AppHandle) {
    let before = crate::history::latest_created_at().unwrap_or(0);
    let app = app.clone();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if AUTOMATED_DEADLINE.lock().unwrap().is_none() {
                return; // bereits von der Aufnahme verarbeitet
            }
            if crate::history::latest_created_at().unwrap_or(0) > before {
                if take_automated_flag() {
                    notify_automated_capture(&app);
                }
                return;
            }
        }
        AUTOMATED_DEADLINE.lock().unwrap().take();
    });
}

fn run_url_action(app: &AppHandle, a: UrlAction) {
    core::diag("AutomationURL", &format!("Performed action {a:?}"));
    match a {
        UrlAction::Run(id @ ("captureFullscreen" | "captureLastArea")) => {
            mark_next_capture_automated();
            watch_automated_capture(app);
            crate::actions::run(app, id)
        }
        UrlAction::Run(id) => crate::actions::run(app, id),
        UrlAction::HideIcons => set_desktop_icons_hidden(app, true),
        UrlAction::ShowIcons => set_desktop_icons_hidden(app, false),
        UrlAction::Preferences(tab) => open(app, tab.as_deref()),
    }
}

fn applescript_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Einmalige Nachfrage (wie Capso `askToAllowAutomationURLs`). Blockiert bis zur Antwort.
fn ask_allow_automation(url: &str) -> bool {
    let msg = format!(
        "{}\n\n{}\n{url}\n\n{}",
        t("Eine andere App möchte nrzCapture steuern.", "Another app wants to control nrzCapture."),
        t("Sie hat diesen Befehl gesendet:", "It sent this command:"),
        t(
            "Anderen Apps erlauben, Aufnahmen mit nrzCapture zu starten? Das lässt sich später unter Einstellungen → Allgemein ändern.",
            "Allow other apps to start captures and recordings with nrzCapture? You can change this later in Settings → General."
        )
    );
    let (allow, deny) = (t("Erlauben", "Allow"), t("Nicht erlauben", "Don't Allow"));
    let script = format!(
        "display dialog {} buttons {{{}, {}}} default button {} cancel button {} with icon caution with title \"nrzCapture\"",
        applescript_str(&msg),
        applescript_str(deny),
        applescript_str(allow),
        applescript_str(allow),
        applescript_str(deny)
    );
    core::sh("osascript", &["-e", &script]).is_some_and(|o| o.contains(allow))
}

/// Automations-URL `nrzcapture://<aktion>` ausführen. Beim ersten Aufruf (noch nicht gefragt) wird nachgefragt.
pub fn handle_url(app: &AppHandle, url: &str) {
    let Some(action) = parse_url(url) else {
        eprintln!("Automations-URL ignoriert: {url}");
        core::diag("AutomationURL", "Ignored unsupported request");
        return;
    };
    let s = crate::settings::get();
    if s.automation_urls_enabled {
        return run_url_action(app, action);
    }
    if s.automation_urls_asked {
        core::diag("AutomationURL", "Ignored request because Automation URLs are disabled");
        return;
    }
    let app = app.clone();
    let url = url.to_owned();
    std::thread::spawn(move || {
        crate::settings::update(&app, |s| s.automation_urls_asked = true);
        let ok = ask_allow_automation(&url);
        crate::settings::update(&app, |s| s.automation_urls_enabled = ok);
        if ok {
            run_url_action(&app, action);
        } else {
            core::diag("AutomationURL", "Ignored request because Automation URLs are disabled");
        }
    });
}

/// Liste für das Automations-URLs-Blatt (Gruppen wie Capso).
fn automation_url_list() -> Value {
    let u = |p: &str| format!("nrzcapture://grab/{p}");
    let item = |name: String, p: &str| json!({"name": name, "url": u(p)});
    let l = crate::actions::label;
    json!([
        {"title": t("Bildschirmfotos", "Screenshots"), "items": [
            item(l("captureAllInOne"), "all-in-one"), item(l("captureArea"), "area"), item(l("captureFullscreen"), "fullscreen"),
            item(l("captureWindow"), "window"), item(l("captureScrolling"), "scrolling"), item(l("selfTimerCapture"), "self-timer"),
            item(l("captureLastArea"), "last-area"), item(l("captureAreaToClipboard"), "clipboard"), item(l("captureAreaAndShare"), "share"),
            item(l("captureAreaAndAnnotate"), "annotate"), item(l("captureAreaAndPin"), "area-and-pin"),
        ]},
        {"title": t("Zwischenablage", "Clipboard"), "items": [
            item(l("pinFromClipboard"), "pin"), item(l("editClipboardImage"), "edit-clipboard"), item(t("Bilddatei öffnen", "Open Image File").into(), "open-image"),
        ]},
        {"title": t("Text & Übersetzung", "Text & Translation"), "items": [
            item(l("captureText"), "ocr"), item(l("captureAndTranslate"), "translate"),
            item(l("translateSelectedText"), "translate-selection"), item(l("translateTypedText"), "translate-text"),
        ]},
        {"title": t("Aufnahme", "Recording"), "items": [
            item(l("recordScreen"), "record"), item(l("recordFullscreen"), "record-fullscreen"),
        ]},
        {"title": t("Schreibtisch", "Desktop"), "items": [
            item(t("Schreibtisch-Symbole ausblenden", "Hide Desktop Icons").into(), "hide-desktop-icons"),
            item(t("Schreibtisch-Symbole einblenden", "Show Desktop Icons").into(), "show-desktop-icons"),
            item(t("Schreibtisch-Symbole umschalten", "Toggle Desktop Icons").into(), "toggle-desktop-icons"),
        ]},
        {"title": t("Verlauf & Einstellungen", "History & Settings"), "items": [
            item(l("screenshotHistory"), "history"), item(t("Einstellungen", "Preferences").into(), "preferences"),
        ]},
    ])
}

// ───────────────────────────── Geräte & Berechtigungen ─────────────────────────────

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
}
const HID_LISTEN_EVENT: u32 = 1;

fn av_status(video: bool) -> &'static str {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio, AVMediaTypeVideo};
    let Some(mt) = (unsafe { if video { AVMediaTypeVideo } else { AVMediaTypeAudio } }) else { return "notDetermined" };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(mt) } {
        AVAuthorizationStatus::Authorized => "granted",
        AVAuthorizationStatus::Denied => "denied",
        AVAuthorizationStatus::Restricted => "restricted",
        _ => "notDetermined",
    }
}

fn devices(video: bool) -> Vec<Value> {
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio, AVMediaTypeVideo};
    let Some(mt) = (unsafe { if video { AVMediaTypeVideo } else { AVMediaTypeAudio } }) else { return vec![] };
    #[allow(deprecated)]
    let list = unsafe { AVCaptureDevice::devicesWithMediaType(mt) };
    list.iter().map(|d| json!({"id": unsafe { d.uniqueID() }.to_string(), "name": unsafe { d.localizedName() }.to_string()})).collect()
}

fn has_usage_description(key: &str) -> bool {
    objc2_foundation::NSBundle::mainBundle().objectForInfoDictionaryKey(&objc2_foundation::NSString::from_str(key)).is_some()
}

fn permissions() -> Value {
    let hid = match unsafe { IOHIDCheckAccess(HID_LISTEN_EVENT) } {
        0 => "granted",
        1 => "denied",
        _ => "notDetermined",
    };
    let b = |v: bool| if v { "granted" } else { "denied" };
    json!({
        "screen": b(core::can_record()),
        "accessibility": b(core::has_accessibility()),
        "inputMonitoring": hid,
        "camera": av_status(true),
        "microphone": av_status(false),
    })
}

fn request_permission(kind: &str) {
    match kind {
        "screen" => core::ask_screen_permission(),
        "accessibility" => core::open_privacy("Privacy_Accessibility"),
        "inputMonitoring" => {
            if unsafe { IOHIDCheckAccess(HID_LISTEN_EVENT) } != 2 || !unsafe { IOHIDRequestAccess(HID_LISTEN_EVENT) } {
                core::open_privacy("Privacy_ListenEvent");
            }
        }
        "camera" | "microphone" => {
            let video = kind == "camera";
            let key = if video { "NSCameraUsageDescription" } else { "NSMicrophoneUsageDescription" };
            // Ohne Usage-Description würde macOS die App beenden → dann direkt die Systemeinstellungen öffnen.
            if av_status(video) == "notDetermined" && has_usage_description(key) {
                use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio, AVMediaTypeVideo};
                if let Some(mt) = unsafe { if video { AVMediaTypeVideo } else { AVMediaTypeAudio } } {
                    let block = block2::RcBlock::new(|_ok: objc2::runtime::Bool| {});
                    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(mt, &block) };
                    return;
                }
            }
            core::open_privacy(if video { "Privacy_Camera" } else { "Privacy_Microphone" });
        }
        _ => {}
    }
}

// ───────────────────────────── Sonstige Helfer ─────────────────────────────

/// Ordner wählen (AppleScript-Dialog, blockiert). `None` bei Abbruch.
fn choose_folder(current: &str) -> Option<String> {
    let mut script = format!("POSIX path of (choose folder with prompt {}", applescript_str(t("Exportordner wählen", "Choose export folder")));
    let cur = current.replacen('~', &core::home(), 1);
    if !cur.is_empty() && std::path::Path::new(&cur).is_dir() {
        script.push_str(&format!(" default location (POSIX file {})", applescript_str(&cur)));
    }
    script.push(')');
    core::sh("osascript", &["-e", "tell application \"System Events\" to activate", "-e", &script])
        .map(|p| p.trim_end_matches('/').to_owned())
        .filter(|p| !p.is_empty())
}

fn logs_dir() -> PathBuf {
    let d = core::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Kürzel-Konflikte in den aktuellen Einstellungen (normalisierte Accelerator-Strings).
pub fn normalize_accel(accel: &str) -> String {
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
    let order = ["Ctrl", "Alt", "Shift", "Cmd"];
    let mut out: Vec<String> = order.iter().filter(|m| mods.contains(m)).map(|m| m.to_string()).collect();
    out.push(key);
    out.join("+")
}

/// Welche andere Aktion (oder macOS) belegt `accel` schon? `None` = frei.
pub fn shortcut_conflict(shortcuts: &std::collections::BTreeMap<String, String>, action: &str, accel: &str, replace_system: bool) -> Option<String> {
    let n = normalize_accel(accel);
    if let Some((a, _)) = shortcuts.iter().find(|(a, v)| a.as_str() != action && !v.trim().is_empty() && normalize_accel(v) == n) {
        return Some(a.clone());
    }
    let sys = ["Shift+Cmd+3", "Shift+Cmd+4", "Shift+Cmd+5"];
    if sys.contains(&n.as_str()) {
        return Some(if replace_system { "replaceSystemShortcuts".into() } else { "macOS".into() });
    }
    ["Cmd+Q", "Cmd+W", "Cmd+TAB", "Cmd+SPACE", "Ctrl+SPACE", "Cmd+H", "Cmd+M"].contains(&n.as_str()).then(|| "macOS".into())
}

pub fn dispatch(app: &AppHandle, _win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    let null = Ok(Value::Null);
    match cmd {
        "open" => {
            open(app, a["tab"].as_str());
            null
        }
        "actions" => Ok(json!(crate::actions::ACTIONS.iter().map(|id| json!({"id": id, "label": crate::actions::label(id)})).collect::<Vec<_>>())),
        "shortcutErrors" => Ok(json!(*crate::actions::LAST_ERRORS.lock().unwrap())),
        // {action, accel} – leer = Kürzel entfernen. Rückgabe {conflict, errors}.
        "setShortcut" => {
            let action = a["action"].as_str().ok_or("action fehlt")?.to_owned();
            let accel = a["accel"].as_str().unwrap_or("").trim().to_owned();
            let s = crate::settings::get();
            if !accel.is_empty() {
                if accel.parse::<tauri_plugin_global_shortcut::Shortcut>().is_err() {
                    return Err(format!("{}: {accel}", t("Ungültiges Kürzel", "Invalid shortcut")));
                }
                if let Some(c) = shortcut_conflict(&s.shortcuts, &action, &accel, s.replace_system_shortcuts) {
                    return Ok(json!({"conflict": c}));
                }
            }
            crate::settings::update(app, |s| {
                s.shortcuts.insert(action, accel);
            });
            Ok(json!({"conflict": null, "errors": *crate::actions::LAST_ERRORS.lock().unwrap()}))
        }
        "resetShortcuts" => {
            crate::settings::update(app, |s| s.shortcuts = crate::settings::default_shortcuts());
            Ok(json!(*crate::actions::LAST_ERRORS.lock().unwrap()))
        }
        // Während der Kürzel-Aufnahme alle globalen Kürzel aussetzen, damit der Tastendruck im Fenster ankommt.
        "pauseShortcuts" => {
            if a["paused"].as_bool().unwrap_or(false) {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                let _ = app.global_shortcut().unregister_all();
            } else {
                crate::actions::register_shortcuts(app, &crate::settings::get());
            }
            null
        }
        "systemShortcutsEnabled" => Ok(json!(system_shortcuts_enabled())),
        "openKeyboardSettings" => {
            core::open("x-apple.systempreferences:com.apple.Keyboard-Settings.extension");
            null
        }
        "autostart" => {
            use tauri_plugin_autostart::ManagerExt;
            Ok(json!(app.autolaunch().is_enabled().unwrap_or(crate::settings::get().start_at_login)))
        }
        "setAutostart" => {
            use tauri_plugin_autostart::ManagerExt;
            let on = a["enabled"].as_bool().unwrap_or(false);
            let r = if on { app.autolaunch().enable() } else { app.autolaunch().disable() };
            crate::settings::update(app, |s| s.start_at_login = on);
            r.map(|_| json!(on)).map_err(|e| e.to_string())
        }
        "chooseFolder" => Ok(json!(choose_folder(a["current"].as_str().unwrap_or("")))),
        "exportDir" => Ok(json!(core::export_dir().to_string_lossy())),
        "revealExportDir" => {
            core::open(&core::export_dir().to_string_lossy());
            null
        }
        // {template, kind: "screenshot"|"recording"} → Beispiel-Dateiname
        "filenamePreview" => {
            let s = crate::settings::get();
            let rec = a["kind"].as_str() == Some("recording");
            let tpl = a["template"].as_str().map(str::to_owned).unwrap_or_else(|| if rec { s.recording_filename_template.clone() } else { s.screenshot_filename_template.clone() });
            let ext = if rec { s.recording_format.clone() } else { s.screenshot_format.clone() };
            Ok(json!(core::file_name(&tpl, if rec { "" } else { " (Area)" }, &ext)))
        }
        "microphones" => Ok(json!(devices(false))),
        "cameras" => Ok(json!(devices(true))),
        "permissions" => Ok(permissions()),
        "requestPermission" => {
            request_permission(a["kind"].as_str().unwrap_or(""));
            null
        }
        "checkUpdates" => {
            crate::update(app.clone(), true);
            null
        }
        "openLogs" => {
            core::open(&logs_dir().to_string_lossy());
            null
        }
        "openCrashReports" => {
            core::open(&format!("{}/Library/Logs/DiagnosticReports", core::home()));
            null
        }
        "desktopIconsHidden" => Ok(json!(ICONS_MANUAL.load(Ordering::Relaxed))),
        "toggleDesktopIcons" => {
            toggle_desktop_icons(app);
            Ok(json!(ICONS_MANUAL.load(Ordering::Relaxed)))
        }
        "automationUrls" => Ok(automation_url_list()),
        "updateDock" => {
            update_dock_presence(app);
            null
        }
        _ => Err(format!("prefs: unbekannter Befehl {cmd}")),
    }
}

// ───────────────────────────── Native (Main-Thread) ─────────────────────────────

mod native {
    use super::{dock_policy, DockWin, DOCK_INIT, ICONS_MANUAL, ICONS_RECORDING};
    use block2::RcBlock;
    use objc2::{rc::Retained, runtime::AnyObject, AllocAnyThread, ClassType, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSApplicationDidChangeScreenParametersNotification, NSApplicationDidUnhideNotification, NSBackingStoreType, NSColor, NSImage, NSPanel, NSScreen, NSView,
        NSWindow, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowDidBecomeKeyNotification, NSWindowDidChangeOcclusionStateNotification, NSWindowDidDeminiaturizeNotification,
        NSWindowDidMiniaturizeNotification, NSWindowStyleMask, NSWindowWillCloseNotification, NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName, NSObjectProtocol, NSOperationQueue, NSPoint, NSRect, NSString};
    use std::{cell::RefCell, ptr::NonNull, sync::atomic::Ordering};

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowLevelForKey(key: i32) -> i32;
    }
    const DESKTOP_ICON_LEVEL_KEY: i32 = 18;

    type Token = Retained<objc2::runtime::ProtocolObject<dyn NSObjectProtocol>>;

    thread_local! {
        static DOCK_SCHEDULED: RefCell<bool> = const { RefCell::new(false) };
        static COVER: RefCell<Vec<Retained<NSWindow>>> = const { RefCell::new(Vec::new()) };
        static COVER_OBS: RefCell<Vec<(bool, Token)>> = const { RefCell::new(Vec::new()) };
    }

    fn observe(center: &NSNotificationCenter, name: &NSNotificationName, f: impl Fn() + 'static) -> Token {
        let block = RcBlock::new(move |_n: NonNull<NSNotification>| f());
        unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, Some(&NSOperationQueue::mainQueue()), &block) }
    }

    // ── Dock ──

    pub fn install_dock_observers() {
        if MainThreadMarker::new().is_none() || DOCK_INIT.swap(true, Ordering::Relaxed) {
            return;
        }
        let nc = NSNotificationCenter::defaultCenter();
        let names = unsafe {
            [
                NSWindowDidBecomeKeyNotification,
                NSWindowWillCloseNotification,
                NSWindowDidChangeOcclusionStateNotification,
                NSWindowDidMiniaturizeNotification,
                NSWindowDidDeminiaturizeNotification,
                NSApplicationDidUnhideNotification,
            ]
        };
        for n in names {
            std::mem::forget(observe(&nc, n, schedule_dock_update));
        }
        let ws = NSWorkspace::sharedWorkspace().notificationCenter();
        std::mem::forget(observe(&ws, unsafe { NSWorkspaceActiveSpaceDidChangeNotification }, schedule_dock_update));
        schedule_dock_update();
    }

    /// Auf den nächsten Runloop-Durchlauf zusammenfassen (`willClose` meldet das Fenster noch als sichtbar).
    pub fn schedule_dock_update() {
        if MainThreadMarker::new().is_none() || DOCK_SCHEDULED.with(|s| s.replace(true)) {
            return;
        }
        let block = RcBlock::new(|| {
            DOCK_SCHEDULED.with(|s| *s.borrow_mut() = false);
            apply_dock_policy();
        });
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
    }

    fn apply_dock_policy() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        let wins: Vec<DockWin> = app
            .windows()
            .iter()
            .map(|w| {
                let titled = w.styleMask().contains(NSWindowStyleMask::Titled);
                let panel = w.isKindOfClass(NSPanel::class());
                DockWin { app_window: titled && !panel, visible: w.isVisible(), miniaturized: w.isMiniaturized(), on_active_space: w.isOnActiveSpace() }
            })
            .collect();
        let Some(regular) = dock_policy(&wins, app.isHidden()) else { return };
        let want = if regular { NSApplicationActivationPolicy::Regular } else { NSApplicationActivationPolicy::Accessory };
        if app.activationPolicy() != want {
            app.setActivationPolicy(want);
            if regular {
                #[allow(deprecated)]
                app.activateIgnoringOtherApps(true);
            }
        }
    }

    // ── Schreibtisch-Abdeckung ──

    pub fn update_cover() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let want = ICONS_MANUAL.load(Ordering::Relaxed) || ICONS_RECORDING.load(Ordering::Relaxed);
        let empty = COVER.with(|c| c.borrow().is_empty());
        if want && empty {
            rebuild(mtm);
            let nc = NSNotificationCenter::defaultCenter();
            let ws = NSWorkspace::sharedWorkspace().notificationCenter();
            let rb = || {
                if let Some(m) = MainThreadMarker::new() {
                    rebuild(m)
                }
            };
            let t1 = observe(&ws, unsafe { NSWorkspaceActiveSpaceDidChangeNotification }, rb);
            let t2 = observe(&nc, unsafe { NSApplicationDidChangeScreenParametersNotification }, rb);
            COVER_OBS.with(|o| *o.borrow_mut() = vec![(true, t1), (false, t2)]);
        } else if !want {
            let ws = NSWorkspace::sharedWorkspace().notificationCenter();
            let nc = NSNotificationCenter::defaultCenter();
            for (is_ws, tok) in COVER_OBS.with(|o| std::mem::take(&mut *o.borrow_mut())) {
                let obj: &AnyObject = tok.as_ref();
                unsafe { if is_ws { ws.removeObserver(obj) } else { nc.removeObserver(obj) } };
            }
            for w in COVER.with(|c| std::mem::take(&mut *c.borrow_mut())) {
                w.orderOut(None);
            }
        }
    }

    fn rebuild(mtm: MainThreadMarker) {
        for w in COVER.with(|c| std::mem::take(&mut *c.borrow_mut())) {
            w.orderOut(None);
        }
        let wins: Vec<Retained<NSWindow>> = NSScreen::screens(mtm).iter().map(|s| make_window(mtm, &s)).collect();
        COVER.with(|c| *c.borrow_mut() = wins);
    }

    fn make_window(mtm: MainThreadMarker, screen: &NSScreen) -> Retained<NSWindow> {
        let frame = screen.frame();
        let w = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, NSWindowStyleMask::Borderless, NSBackingStoreType::Buffered, false) };
        unsafe { w.setReleasedWhenClosed(false) };
        w.setLevel((unsafe { CGWindowLevelForKey(DESKTOP_ICON_LEVEL_KEY) } + 1) as isize);
        w.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::IgnoresCycle);
        w.setIgnoresMouseEvents(true);
        w.setOpaque(true);
        w.setHasShadow(false);
        w.setAnimationBehavior(NSWindowAnimationBehavior::None);
        w.setBackgroundColor(Some(&NSColor::colorWithCalibratedRed_green_blue_alpha(0.12, 0.13, 0.15, 1.0)));

        let view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
        view.setWantsLayer(true);
        // Hintergrundbild des Bildschirms (NSWorkspace) als Layer-Inhalt, „aspect fill“.
        if let Some(url) = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(screen) {
            if let Some(img) = NSImage::initWithContentsOfURL(NSImage::alloc(), &url) {
                unsafe {
                    let layer: Option<Retained<AnyObject>> = objc2::msg_send![&*view, layer];
                    if let Some(layer) = layer {
                        let _: () = objc2::msg_send![&*layer, setContents: &*img];
                        let _: () = objc2::msg_send![&*layer, setContentsGravity: &*NSString::from_str("resizeAspectFill")];
                    }
                }
            }
        }
        w.setContentView(Some(&view));
        w.setFrame_display(frame, false);
        w.orderFrontRegardless();
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parsing() {
        assert_eq!(parse_url("nrzcapture://grab/area"), Some(UrlAction::Run("captureArea")));
        assert_eq!(parse_url("NRZCAPTURE://GRAB/all-in-one"), Some(UrlAction::Run("captureAllInOne")));
        assert_eq!(parse_url("nrzcapture://captureWindow"), Some(UrlAction::Run("captureWindow")));
        assert_eq!(parse_url("nrzcapture://record-fullscreen/"), Some(UrlAction::Run("recordFullscreen")));
        assert_eq!(parse_url("nrzcapture://grab/hide-desktop-icons"), Some(UrlAction::HideIcons));
        assert_eq!(parse_url("nrzcapture://show-desktop-icons"), Some(UrlAction::ShowIcons));
        assert_eq!(parse_url("nrzcapture://grab/settings"), Some(UrlAction::Preferences(None)));
        assert_eq!(parse_url("nrzcapture://preferences?tab=shortcuts"), Some(UrlAction::Preferences(Some("shortcuts".into()))));
        // Abgelehnt wie bei Capso
        assert_eq!(parse_url("nrzcapture://grab/area?x=1"), None);
        assert_eq!(parse_url("nrzcapture://grab/area#frag"), None);
        assert_eq!(parse_url("nrzcapture://user@grab/area"), None);
        assert_eq!(parse_url("nrzcapture://grab:80/area"), None);
        assert_eq!(parse_url("capso://grab/area"), None);
        assert_eq!(parse_url("nrzcapture://grab/unknown"), None);
        assert_eq!(parse_url("nrzcapture://grab/area/extra"), None);
        assert_eq!(parse_url("nrzcapture://quit"), None);
    }

    #[test]
    fn dock_logic() {
        let w = |app_window, visible, miniaturized, on_active_space| DockWin { app_window, visible, miniaturized, on_active_space };
        assert_eq!(dock_policy(&[], false), Some(false));
        assert_eq!(dock_policy(&[w(false, true, false, true)], false), Some(false)); // nur Overlay
        assert_eq!(dock_policy(&[w(true, true, false, true)], false), Some(true));
        assert_eq!(dock_policy(&[w(true, false, true, false)], false), Some(true)); // minimiert zählt
        assert_eq!(dock_policy(&[w(true, false, false, true)], false), Some(false)); // geschlossen/versteckt
        assert_eq!(dock_policy(&[w(true, true, false, false)], false), None); // anderer Space
        assert_eq!(dock_policy(&[w(true, true, false, true)], true), None); // App ausgeblendet
    }

    #[test]
    fn shortcut_normalize_and_conflicts() {
        assert_eq!(normalize_accel("Shift+Alt+1"), "Alt+Shift+1");
        assert_eq!(normalize_accel("cmd+shift+KeyA"), "Shift+Cmd+A");
        assert_eq!(normalize_accel("Option+Digit5"), "Alt+5");
        let mut m = std::collections::BTreeMap::new();
        m.insert("captureArea".to_string(), "Alt+Shift+1".to_string());
        m.insert("captureWindow".to_string(), String::new());
        assert_eq!(shortcut_conflict(&m, "captureFullscreen", "Shift+Alt+1", false), Some("captureArea".into()));
        assert_eq!(shortcut_conflict(&m, "captureArea", "Alt+Shift+1", false), None);
        assert_eq!(shortcut_conflict(&m, "captureWindow", "Cmd+Shift+4", false), Some("macOS".into()));
        assert_eq!(shortcut_conflict(&m, "captureWindow", "Cmd+Shift+4", true), Some("replaceSystemShortcuts".into()));
        assert_eq!(shortcut_conflict(&m, "captureWindow", "Cmd+Q", false), Some("macOS".into()));
        assert_eq!(shortcut_conflict(&m, "captureWindow", "Ctrl+Alt+K", false), None);
    }

    #[test]
    fn hotkey_entry_format() {
        assert_eq!(hotkey_entry(false, [51, 20, 1179648]), "{enabled = 0; value = { parameters = (51, 20, 1179648); type = standard; }; }");
    }
}
