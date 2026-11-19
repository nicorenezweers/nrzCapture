//! Zentrale Aktionen (Menüleiste, globale Kürzel, Automations-URLs) → Module.
use std::{collections::HashMap, sync::Mutex};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIcon,
    AppHandle, Wry,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::{capture::Intent, core::t, settings::Settings};

/// Alle Aktions-IDs (identisch zu Capsos Kürzel-Namen, damit Automations-URLs passen).
pub const ACTIONS: &[&str] = &[
    "captureArea",
    "captureWindow",
    "captureFullscreen",
    // nur Menü / URL
    "preferences",
    "about",
    "quit",
];

/// Aktionen mit globalem Kürzel (nrzCapture ist auf reine Screenshots beschränkt).
pub const SHORTCUT_ACTIONS: &[&str] = &["captureArea", "captureWindow", "captureFullscreen"];

/// Anzeigename einer Aktion (Menü, Einstellungen).
pub fn label(id: &str) -> String {
    match id {
        "captureAllInOne" => t("All-in-One", "All-in-One"),
        "captureArea" => t("Auswahl aufnehmen", "Capture Selection"),
        "captureFullscreen" => t("Ganzen Monitor aufnehmen", "Capture Entire Screen"),
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
        "captureArea" => crate::capture::area(app, Intent::Save),
        "captureFullscreen" => crate::capture::fullscreen(app),
        "captureWindow" => crate::capture::window(app),
        "preferences" => crate::prefs::open(app, None),
        "about" => crate::prefs::about(app),
        "quit" => app.exit(0),
        // Alle übrigen Funktionen (OCR, Übersetzung, Aufnahme, Pins …) sind deaktiviert.
        other => eprintln!("Aktion deaktiviert/unbekannt: {other}"),
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
    if let Some(a) = action.as_deref() {
        run(app, a);
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
    let mut wanted: Vec<(String, String)> = s.shortcuts.iter().filter(|(k, a)| !a.trim().is_empty() && SHORTCUT_ACTIONS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    if s.replace_system_shortcuts {
        // ⌘⇧3 / ⌘⇧4 / ⌘⇧5 → Vollbild / Bereich / Fenster (System-Kürzel werden in prefs deaktiviert).
        for (accel, action) in [("Cmd+Shift+3", "captureFullscreen"), ("Cmd+Shift+4", "captureArea"), ("Cmd+Shift+5", "captureWindow")] {
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

    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<Wry>>> = vec![];
    if !crate::core::can_record() {
        items.push(Box::new(custom("perm", t("⚠︎ Bildschirmaufnahme erlauben …", "⚠︎ Allow Screen Recording…"), None)?));
        items.push(Box::new(sep()?));
    }
    if let Some(u) = crate::UPDATE_LABEL.lock().unwrap().clone() {
        items.push(Box::new(custom("upd", &u, None)?));
        items.push(Box::new(sep()?));
    }
    for id in ["captureArea", "captureWindow", "captureFullscreen"] {
        items.push(Box::new(item(id)?));
    }
    items.push(Box::new(sep()?));
    items.push(Box::new(MenuItem::with_id(app, "preferences", label("preferences"), true, Some("Cmd+,"))?));
    items.push(Box::new(item("about")?));
    items.push(Box::new(sep()?));
    items.push(Box::new(MenuItem::with_id(app, "quit", label("quit"), true, Some("Cmd+Q"))?));

    let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

pub fn refresh_menu(app: &AppHandle) {
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
