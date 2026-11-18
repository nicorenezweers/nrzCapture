//! Zentrale Aktionen (Menüleiste, globale Kürzel, Automations-URLs) → Module.
use std::{collections::HashMap, sync::Mutex};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIcon,
    AppHandle, Wry,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::{capture::Intent, core::t, settings::Settings};

/// Alle Aktions-IDs (identisch zu Capsos Kürzel-Namen, damit Automations-URLs passen).
pub const ACTIONS: &[&str] = &[
    "captureAllInOne",
    "captureArea",
    "captureFullscreen",
    "captureWindow",
    "captureText",
    "recordScreen",
    "recordFullscreen",
    "captureScrolling",
    "captureAreaToClipboard",
    "captureAreaAndShare",
    "captureAreaAndAnnotate",
    "captureAreaAndPin",
    "pinFromClipboard",
    "editClipboardImage",
    "screenshotHistory",
    "captureAndTranslate",
    "translateSelectedText",
    "translateTypedText",
    "selfTimerCapture",
    "captureLastArea",
    "togglePins",
    // nur Menü / URL
    "saveAllQuickAccess",
    "closeQuickAccess",
    "reopenQuickAccess",
    "openImageFile",
    "toggleCameraPiP",
    "toggleDesktopIcons",
    "preferences",
    "about",
    "quit",
];

/// Anzeigename einer Aktion (Menü, Einstellungen).
pub fn label(id: &str) -> String {
    match id {
        "captureAllInOne" => t("All-in-One", "All-in-One"),
        "captureArea" => t("Bereich aufnehmen", "Capture Area"),
        "captureFullscreen" => t("Vollbild aufnehmen", "Capture Fullscreen"),
        "captureWindow" => t("Fenster aufnehmen", "Capture Window"),
        "captureText" => t("Text erkennen (OCR)", "Capture Text (OCR)"),
        "recordScreen" => t("Bildschirm aufzeichnen", "Record Screen"),
        "recordFullscreen" => t("Vollbild aufzeichnen", "Record Fullscreen"),
        "captureScrolling" => t("Scroll-Aufnahme", "Scrolling Capture"),
        "captureAreaToClipboard" => t("Bereich in Zwischenablage", "Capture Area to Clipboard"),
        "captureAreaAndShare" => t("Bereich aufnehmen & teilen", "Capture Area & Share"),
        "captureAreaAndAnnotate" => t("Bereich aufnehmen & bearbeiten", "Capture Area & Annotate"),
        "captureAreaAndPin" => t("Bereich aufnehmen & anheften", "Capture Area & Pin"),
        "pinFromClipboard" => t("Aus Zwischenablage anheften", "Pin from Clipboard"),
        "editClipboardImage" => t("Bild aus Zwischenablage bearbeiten", "Edit Clipboard Image"),
        "screenshotHistory" => t("Verlauf", "Screenshot History"),
        "captureAndTranslate" => t("Aufnehmen & übersetzen", "Capture & Translate"),
        "translateSelectedText" => t("Markierten Text übersetzen", "Translate Selected Text"),
        "translateTypedText" => t("Text eingeben & übersetzen", "Translate Typed Text"),
        "selfTimerCapture" => t("Selbstauslöser", "Self-Timer"),
        "captureLastArea" => t("Letzten Bereich aufnehmen", "Capture Previous Area"),
        "togglePins" => t("Alle Pins aus-/einblenden", "Hide/Show All Pins"),
        "saveAllQuickAccess" => t("Alle in Quick Access sichern", "Save All in Quick Access"),
        "closeQuickAccess" => t("Quick Access schließen", "Close Quick Access"),
        "reopenQuickAccess" => t("Zuletzt geschlossenen Screenshot öffnen", "Reopen Last Closed Screenshot"),
        "openImageFile" => t("Bild öffnen …", "Open Image..."),
        "toggleCameraPiP" => t("Kamera-PiP", "Camera PiP"),
        "toggleDesktopIcons" => t("Schreibtisch-Symbole ausblenden", "Hide Desktop Icons"),
        "preferences" => t("Einstellungen …", "Preferences..."),
        "about" => t("Über nrzCapture", "About nrzCapture"),
        "quit" => t("nrzCapture beenden", "Quit nrzCapture"),
        _ => id,
    }
    .to_owned()
}

/// Aktion ausführen. Läuft auf einem eigenen Thread, damit Module blockieren dürfen.
pub fn run(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_owned();
    std::thread::spawn(move || run_now(&app, &id));
}

fn needs_screen(id: &str) -> bool {
    id.starts_with("capture") || id.starts_with("record") || id == "selfTimerCapture"
}

fn run_now(app: &AppHandle, id: &str) {
    if needs_screen(id) && !crate::core::can_record() {
        crate::core::ask_screen_permission();
        return;
    }
    match id {
        "captureAllInOne" => crate::capture::all_in_one(app),
        "captureArea" => crate::capture::area(app, Intent::Save),
        "captureFullscreen" => crate::capture::fullscreen(app),
        "captureWindow" => crate::capture::window(app),
        "captureText" => crate::ocr::capture_text(app),
        "recordScreen" => crate::record::start(app, false),
        "recordFullscreen" => crate::record::start(app, true),
        "captureScrolling" => crate::scroll::start(app),
        "captureAreaToClipboard" => crate::capture::area(app, Intent::Clipboard),
        "captureAreaAndShare" => crate::capture::area(app, Intent::Share),
        "captureAreaAndAnnotate" => crate::capture::area(app, Intent::Annotate),
        "captureAreaAndPin" => crate::capture::area(app, Intent::Pin),
        "pinFromClipboard" => crate::pin::pin_clipboard(app),
        "editClipboardImage" => crate::editor::open_clipboard(app),
        "screenshotHistory" => crate::history::open(app),
        // Capso: Übersetzt zuerst die fokussierte/gehoverte Quick-Access-Karte, sonst neue Aufnahme.
        "captureAndTranslate" => {
            if !crate::quick::translate_focused(app) {
                crate::translate::capture_and_translate(app)
            }
        }
        "translateSelectedText" => crate::translate::selected_text(app),
        "translateTypedText" => crate::translate::typed_text(app),
        "selfTimerCapture" => crate::capture::self_timer(app),
        "captureLastArea" => crate::capture::last_area(app),
        "togglePins" => crate::pin::toggle_all(app),
        "saveAllQuickAccess" => crate::quick::save_all(app),
        "closeQuickAccess" => crate::quick::close_all(app),
        "reopenQuickAccess" => crate::quick::reopen_last(app),
        "openImageFile" => crate::editor::open_file_dialog(app),
        "toggleCameraPiP" => crate::camera::toggle(app),
        "toggleDesktopIcons" => crate::prefs::toggle_desktop_icons(app),
        "preferences" => crate::prefs::open(app, None),
        "about" => crate::prefs::about(app),
        "quit" => app.exit(0),
        other => eprintln!("Unbekannte Aktion: {other}"),
    }
    refresh_menu(app);
}

// ───────────────────────────── Globale Kürzel ─────────────────────────────

static SHORTCUTS: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);
/// Zusätzliche, von Modulen temporär registrierte Kürzel (z. B. Esc während einer Aufnahme).
static TEMP: Mutex<Option<HashMap<u32, Box<dyn Fn(&AppHandle) + Send>>>> = Mutex::new(None);

/// Aufruf aus dem Global-Shortcut-Plugin.
pub fn on_shortcut(app: &AppHandle, sc: &Shortcut) {
    let id = sc.id();
    if let Some(f) = TEMP.lock().unwrap().as_ref().and_then(|m| m.get(&id)) {
        return f(app);
    }
    let action = SHORTCUTS.lock().unwrap().as_ref().and_then(|m| m.get(&id).cloned());
    match action.as_deref() {
        // Verlaufs-Kürzel schaltet das Fenster um (Capso `HistoryCoordinator.toggleWindow`).
        Some("screenshotHistory") => {
            let app = app.clone();
            std::thread::spawn(move || crate::history::toggle(&app));
        }
        Some(a) => run(app, a),
        None => {}
    }
}

/// Temporäres globales Kürzel (z. B. "Escape", "Cmd+Shift+Escape"). Rückgabe: ob registriert.
pub fn register_temp(app: &AppHandle, accel: &str, f: impl Fn(&AppHandle) + Send + 'static) -> bool {
    let Ok(sc) = accel.parse::<Shortcut>() else { return false };
    TEMP.lock().unwrap().get_or_insert_with(HashMap::new).insert(sc.id(), Box::new(f));
    app.global_shortcut().register(sc).is_ok()
}

pub fn unregister_temp(app: &AppHandle, accel: &str) {
    if let Ok(sc) = accel.parse::<Shortcut>() {
        TEMP.lock().unwrap().get_or_insert_with(HashMap::new).remove(&sc.id());
        let _ = app.global_shortcut().unregister(sc);
    }
}

/// Alle Aktions-Kürzel aus den Einstellungen (neu) registrieren. Liefert Fehlermeldungen (Konflikte).
pub fn register_shortcuts(app: &AppHandle, s: &Settings) -> Vec<String> {
    let gs = app.global_shortcut();
    let mut map = SHORTCUTS.lock().unwrap();
    let mut active = ACTIVE.lock().unwrap();
    for sc in active.drain(..) {
        let _ = gs.unregister(sc);
    }
    let m = map.get_or_insert_with(HashMap::new);
    m.clear();
    let mut wanted: Vec<(String, String)> = s.shortcuts.iter().filter(|(_, a)| !a.trim().is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
    if s.replace_system_shortcuts {
        // ⌘⇧3 / ⌘⇧4 / ⌘⇧5 → Vollbild / Bereich / All-in-One (System-Kürzel werden in prefs deaktiviert).
        for (accel, action) in [("Cmd+Shift+3", "captureFullscreen"), ("Cmd+Shift+4", "captureArea"), ("Cmd+Shift+5", "captureAllInOne")] {
            wanted.push((action.into(), accel.into()));
        }
    }
    let mut errors = vec![];
    for (action, accel) in wanted {
        match accel.parse::<Shortcut>() {
            Ok(sc) if m.contains_key(&sc.id()) => errors.push(format!("{accel}: doppelt ({})", label(&action))),
            Ok(sc) => match gs.register(sc) {
                Ok(()) => {
                    m.insert(sc.id(), action);
                    active.push(sc);
                }
                Err(e) => errors.push(format!("{accel} ({}): {e}", label(&action))),
            },
            Err(e) => errors.push(format!("{accel}: {e}")),
        }
    }
    *LAST_ERRORS.lock().unwrap() = errors.clone();
    errors
}

static ACTIVE: Mutex<Vec<Shortcut>> = Mutex::new(Vec::new());
/// Letzte Registrierungsfehler (für die Einstellungen).
pub static LAST_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

// ───────────────────────────── Menüleiste ─────────────────────────────

static TRAY: Mutex<Option<TrayIcon>> = Mutex::new(None);

pub fn set_tray(tray: TrayIcon) {
    *TRAY.lock().unwrap() = Some(tray);
}

/// Menü-Accelerator-Anzeige: "Alt+Shift+1" passt direkt zum Tauri-Format.
fn accel(s: &Settings, id: &str) -> Option<String> {
    s.shortcuts.get(id).filter(|a| !a.is_empty()).cloned()
}

pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let s = crate::settings::get();
    let item = |id: &str| MenuItem::with_id(app, id, label(id), true, accel(&s, id));
    let custom = |id: &str, text: &str, acc: Option<&str>| MenuItem::with_id(app, id, text, true, acc);
    let sep = || PredefinedMenuItem::separator(app);

    let pins_label = if crate::pin::all_hidden() { t("Alle Pins einblenden", "Show All Pins") } else { t("Alle Pins ausblenden", "Hide All Pins") };
    let timer_label = format!("{} ({} s)", label("selfTimerCapture"), s.self_timer_seconds);
    let icons_label = if crate::prefs::desktop_icons_hidden() { t("Schreibtisch-Symbole einblenden", "Show Desktop Icons") } else { t("Schreibtisch-Symbole ausblenden", "Hide Desktop Icons") };

    let cam_on = crate::camera::is_open();
    let cam = Submenu::with_items(
        app,
        label("toggleCameraPiP"),
        true,
        &[
            &CheckMenuItem::with_id(app, "toggleCameraPiP", t("Kamera anzeigen", "Show Camera"), true, cam_on, None::<&str>)?,
            &CheckMenuItem::with_id(app, "camFade", t("Bei Hover ausblenden", "Fade on Hover"), true, s.camera_pip_fade_on_hover, None::<&str>)?,
            &CheckMenuItem::with_id(app, "camClickThrough", t("Klicks durchlassen", "Click Through PiP"), true, s.camera_pip_click_through, None::<&str>)?,
        ],
    )?;

    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<Wry>>> = vec![];
    if !crate::core::can_record() {
        items.push(Box::new(custom("perm", t("⚠︎ Bildschirmaufnahme erlauben …", "⚠︎ Allow Screen Recording…"), None)?));
        items.push(Box::new(sep()?));
    }
    if let Some(u) = crate::UPDATE_LABEL.lock().unwrap().clone() {
        items.push(Box::new(custom("upd", &u, None)?));
        items.push(Box::new(sep()?));
    }
    items.push(Box::new(item("captureAllInOne")?));
    items.push(Box::new(sep()?));
    for id in ["captureArea", "captureFullscreen", "captureWindow", "pinFromClipboard"] {
        items.push(Box::new(item(id)?));
    }
    // Wie Capso `menuWillOpen`: Pins-/Quick-Access-Einträge nur zeigen, wenn es etwas zu tun gibt.
    let st = menu_state(app);
    if st.has_pins {
        items.push(Box::new(MenuItem::with_id(app, "togglePins", pins_label, true, accel(&s, "togglePins"))?));
    }
    if st.has_quick {
        items.push(Box::new(item("saveAllQuickAccess")?));
        items.push(Box::new(item("closeQuickAccess")?));
    }
    if st.can_reopen {
        items.push(Box::new(item("reopenQuickAccess")?));
    }
    items.push(Box::new(item("editClipboardImage")?));
    items.push(Box::new(MenuItem::with_id(app, "openImageFile", label("openImageFile"), true, Some("Cmd+O"))?));
    items.push(Box::new(sep()?));
    for id in ["captureText", "captureAndTranslate", "translateSelectedText", "translateTypedText", "captureScrolling"] {
        items.push(Box::new(item(id)?));
    }
    items.push(Box::new(MenuItem::with_id(app, "selfTimerCapture", timer_label, true, accel(&s, "selfTimerCapture"))?));
    items.push(Box::new(sep()?));
    if crate::record::is_recording() {
        items.push(Box::new(custom("stopRecording", t("Aufzeichnung beenden", "Stop Recording"), None)?));
    } else {
        items.push(Box::new(item("recordScreen")?));
        items.push(Box::new(item("recordFullscreen")?));
    }
    items.push(Box::new(cam));
    items.push(Box::new(sep()?));
    items.push(Box::new(item("screenshotHistory")?));
    items.push(Box::new(MenuItem::with_id(app, "toggleDesktopIcons", icons_label, true, None::<&str>)?));
    items.push(Box::new(sep()?));
    items.push(Box::new(MenuItem::with_id(app, "preferences", label("preferences"), true, Some("Cmd+,"))?));
    items.push(Box::new(item("about")?));
    items.push(Box::new(sep()?));
    items.push(Box::new(MenuItem::with_id(app, "quit", label("quit"), true, Some("Cmd+Q"))?));

    let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

/// Zustand, von dem die Sichtbarkeit einzelner Menüpunkte abhängt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct MenuState {
    has_pins: bool,
    pins_hidden: bool,
    has_quick: bool,
    can_reopen: bool,
}

fn menu_state(app: &AppHandle) -> MenuState {
    MenuState { has_pins: crate::pin::count() > 0, pins_hidden: crate::pin::all_hidden(), has_quick: crate::quick::has_items(app), can_reopen: crate::quick::can_reopen() }
}

/// Menü neu aufbauen, sobald sich Pins/Quick Access ändern (Tauri hat kein `menuWillOpen`).
fn watch_menu_state(app: &AppHandle) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let app = app.clone();
        std::thread::spawn(move || {
            let mut last = menu_state(&app);
            loop {
                std::thread::sleep(std::time::Duration::from_millis(400));
                let now = menu_state(&app);
                if now != last {
                    last = now;
                    refresh_menu(&app);
                }
            }
        });
    });
}

pub fn refresh_menu(app: &AppHandle) {
    watch_menu_state(app);
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let (Some(tray), Ok(menu)) = (TRAY.lock().unwrap().as_ref(), build_menu(&app2)) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

pub fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "perm" => crate::core::ask_screen_permission(),
        "upd" => crate::update(app.clone(), true),
        "stopRecording" => {
            let a = app.clone();
            std::thread::spawn(move || crate::record::stop(&a));
        }
        "camFade" => crate::settings::update(app, |s| s.camera_pip_fade_on_hover = !s.camera_pip_fade_on_hover),
        "camClickThrough" => crate::settings::update(app, |s| s.camera_pip_click_through = !s.camera_pip_click_through),
        other => run(app, other),
    }
}

/// Wird nach jeder Einstellungsänderung aufgerufen.
pub fn settings_changed(app: &AppHandle, s: &Settings) {
    static LAST: Mutex<Option<(std::collections::BTreeMap<String, String>, bool, String, bool)>> = Mutex::new(None);
    let key = (s.shortcuts.clone(), s.replace_system_shortcuts, s.language.clone(), s.show_menu_bar_icon);
    let changed = LAST.lock().unwrap().as_ref() != Some(&key);
    if changed {
        *LAST.lock().unwrap() = Some(key);
        register_shortcuts(app, s);
        crate::prefs::apply_system_shortcuts(s.replace_system_shortcuts);
        if let Some(t) = TRAY.lock().unwrap().as_ref() {
            let _ = t.set_visible(s.show_menu_bar_icon);
        }
    }
    refresh_menu(app);
}
