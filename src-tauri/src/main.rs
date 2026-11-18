// nrzCapture – Screenshot- & Bildschirmaufnahme-App für die macOS-Menüleiste (Rust-Port von Capso).
#![allow(dead_code)]
mod actions;
mod camera;
mod capture;
mod core;
mod editor;
mod history;
mod ocr;
mod pin;
mod prefs;
mod quick;
mod receditor;
mod record;
mod scroll;
mod settings;
mod share;
mod translate;

use serde_json::Value;
use std::{sync::Mutex, time::Duration};
use tauri::{tray::TrayIconBuilder, AppHandle, RunEvent, WebviewWindow};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_plugin_updater::{Update, UpdaterExt};

static PENDING: Mutex<Option<Update>> = Mutex::new(None);
/// Update wurde im Hintergrund installiert; Menüpunkt startet neu.
static RESTART_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Text des Update-Menüpunkts (None = ausblenden).
pub static UPDATE_LABEL: Mutex<Option<String>> = Mutex::new(None);

/// Einziger IPC-Befehl: `invoke("rpc", {m: "<modul>", c: "<befehl>", a: {...}})`.
/// Jedes Modul hat `dispatch(app, window, cmd, args) -> Result<Value, String>` und darf blockieren.
#[tauri::command]
async fn rpc(app: AppHandle, window: WebviewWindow, m: String, c: String, a: Option<Value>) -> Result<Value, String> {
    let a = a.unwrap_or(Value::Null);
    tauri::async_runtime::spawn_blocking(move || {
        let w = &window;
        match m.as_str() {
            "core" => core::dispatch(&app, w, &c, a),
            "settings" => settings::dispatch(&app, &c, a),
            "action" => {
                actions::run(&app, &c);
                Ok(Value::Null)
            }
            "capture" => capture::dispatch(&app, w, &c, a),
            "scroll" => scroll::dispatch(&app, w, &c, a),
            "quick" => quick::dispatch(&app, w, &c, a),
            "pin" => pin::dispatch(&app, w, &c, a),
            "editor" => editor::dispatch(&app, w, &c, a),
            "ocr" => ocr::dispatch(&app, w, &c, a),
            "translate" => translate::dispatch(&app, w, &c, a),
            "record" => record::dispatch(&app, w, &c, a),
            "camera" => camera::dispatch(&app, w, &c, a),
            "receditor" => receditor::dispatch(&app, w, &c, a),
            "history" => history::dispatch(&app, w, &c, a),
            "share" => share::dispatch(&app, w, &c, a),
            "prefs" => prefs::dispatch(&app, w, &c, a),
            _ => Err(format!("unbekanntes Modul {m}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Update prüfen bzw. (zweiter Klick) installieren und neu starten.
pub fn update(app: AppHandle, manual: bool) {
    tauri::async_runtime::spawn(async move {
        if RESTART_PENDING.load(std::sync::atomic::Ordering::Relaxed) && manual {
            app.restart();
        }
        let pending = PENDING.lock().unwrap().take();
        if let Some(u) = pending {
            *UPDATE_LABEL.lock().unwrap() = Some(format!("{} {} …", core::t("Lade Update", "Downloading update"), u.version));
            actions::refresh_menu(&app);
            match u.download_and_install(|_, _| {}, || {}).await {
                Ok(()) => app.restart(),
                Err(e) => {
                    eprintln!("Update: {e}");
                    *UPDATE_LABEL.lock().unwrap() = Some(core::t("Update fehlgeschlagen – erneut versuchen", "Update failed – retry").into());
                    actions::refresh_menu(&app);
                }
            }
            return;
        }
        let res = match app.updater() {
            Ok(u) => u.check().await,
            Err(e) => Err(e),
        };
        let label = match res {
            // Automatisch installieren (Capso „Automatically Install Updates“): direkt laden und installieren,
            // die neue Version gilt ab dem nächsten Start.
            Ok(Some(u)) if !manual && settings::get().update_auto_install => {
                let version = u.version.clone();
                match u.download_and_install(|_, _| {}, || {}).await {
                    Ok(()) => {
                        core::diag("Update", &format!("installed version={version}"));
                        RESTART_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
                        Some(format!("⟳ {} {version}", core::t("Neu starten für Update", "Restart to finish update")))
                    }
                    Err(e) => {
                        core::diag("Update", &format!("auto install failed: {e}"));
                        None
                    }
                }
            }
            Ok(Some(u)) => {
                let t = format!("⬆︎ {} {}", core::t("Update installieren:", "Install update:"), u.version);
                *PENDING.lock().unwrap() = Some(u);
                Some(t)
            }
            Ok(None) if manual => {
                core::toast(&app, &format!("{} (v{})", core::t("nrzCapture ist aktuell", "nrzCapture is up to date"), app.package_info().version));
                None
            }
            Err(e) if manual => {
                core::toast(&app, &format!("{}: {e}", core::t("Update-Prüfung fehlgeschlagen", "Update check failed")));
                None
            }
            // Bereits automatisch installiert → Hinweis „Neu starten“ stehen lassen.
            _ if RESTART_PENDING.load(std::sync::atomic::Ordering::Relaxed) => UPDATE_LABEL.lock().unwrap().clone(),
            _ => None,
        };
        *UPDATE_LABEL.lock().unwrap() = label;
        actions::refresh_menu(&app);
    });
}

fn main() {
    std::env::set_var("HOME", core::home());
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        actions::on_shortcut(app, sc);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![rpc])
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let h = app.handle().clone();
            let s = settings::get();

            // Automations-URLs: nrzcapture://<aktion>
            let h2 = h.clone();
            app.deep_link().on_open_url(move |e| {
                for url in e.urls() {
                    // Bilddateien (Finder/Dock) kommen über RunEvent::Opened.
                    if url.scheme() != "file" {
                        prefs::handle_url(&h2, url.as_str());
                    }
                }
            });

            let tray = TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/tray.png"))
                .icon_as_template(true)
                .tooltip("nrzCapture")
                .menu(&actions::build_menu(&h)?)
                .on_menu_event(|app, e| actions::on_menu(app, e.id().as_ref()))
                .build(app)?;
            actions::set_tray(tray);
            actions::settings_changed(&h, &s);

            // Kein Bildschirmaufnahme-Dialog beim Start (Capso fragt erst bei der ersten Aufnahme);
            // der Menüpunkt „Bildschirmaufnahme erlauben …“ erscheint, solange das Recht fehlt.
            core::diag("App", &format!("App launched version={}", app.package_info().version));
            history::cleanup(&h);
            // Klartext-Schlüssel (Übersetzung, Cloud) einmalig in den Schlüsselbund verschieben.
            translate::migrate_settings(&h);
            share::migrate_keys(&h);
            prefs::init(&h);
            record::recover_interrupted(&h);
            ocr::prewarm();

            // Rauchtests/Automatisierung: NRZ_ACTIONS="preferences,screenshotHistory" führt Aktionen nach dem Start aus.
            if let Ok(list) = std::env::var("NRZ_ACTIONS") {
                let h4 = h.clone();
                std::thread::spawn(move || {
                    for a in list.split(',').map(str::trim).filter(|a| !a.is_empty()) {
                        std::thread::sleep(Duration::from_millis(1500));
                        // "editor:/pfad.png", "pin:/pfad.png", "receditor:/pfad.mp4", "quick:/pfad.png", "ocr:/pfad.png" oder Aktions-ID
                        match a.split_once(':') {
                            Some(("editor", p)) => editor::open(&h4, std::path::Path::new(p)),
                            Some(("pin", p)) => pin::pin_image(&h4, std::path::Path::new(p), None),
                            Some(("receditor", p)) => receditor::open(&h4, std::path::Path::new(p)),
                            Some(("ocr", p)) => ocr::visual(&h4, std::path::Path::new(p)),
                            Some(("quick", p)) => quick::show(&h4, quick::QuickItem { path: p.into(), kind: "image".into(), saved: false }),
                            _ => actions::run(&h4, a),
                        }
                    }
                });
            }

            // Automatische Update-Prüfung gemäß Einstellung.
            let h3 = h.clone();
            tauri::async_runtime::spawn(async move {
                // Wie Sparkle: geplante Prüfung im gewählten Abstand (Täglich/Wöchentlich/Monatlich),
                // Zeitpunkt der letzten Prüfung bleibt über Neustarts erhalten.
                loop {
                    let s = settings::get();
                    let every = update_interval(&s.update_check);
                    let last = s.extra.get("lastUpdateCheck").and_then(|v| v.as_i64()).unwrap_or(0);
                    let now = chrono::Utc::now().timestamp();
                    if s.update_auto_check && now - last >= every && PENDING.lock().unwrap().is_none() && !RESTART_PENDING.load(std::sync::atomic::Ordering::Relaxed) {
                        settings::set_extra(&h3, "lastUpdateCheck", serde_json::json!(now));
                        update(h3.clone(), false);
                    }
                    sleep(Duration::from_secs(3600)).await;
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Start fehlgeschlagen");

    app.run(|app, e| match e {
        // Erneutes Öffnen (Spotlight, Launchpad, Finder) zeigt immer die Einstellungen (Capso `handleReopen`).
        RunEvent::Reopen { .. } => prefs::open(app, None),
        // Bilder aus Finder/Dock („Öffnen mit“) → Editor; andere URLs übernimmt das Deep-Link-Plugin.
        RunEvent::Opened { urls } => {
            for url in urls {
                if url.scheme() != "file" {
                    continue;
                }
                match url.to_file_path() {
                    Ok(p) if is_image_file(&p) => {
                        let (a, p) = (app.clone(), p.clone());
                        std::thread::spawn(move || editor::open(&a, &p));
                    }
                    _ => core::diag("App", &format!("Ignored unsupported open request {url}")),
                }
            }
        }
        RunEvent::Exit => {
            prefs::apply_system_shortcuts(false); // macOS-Kürzel wiederherstellen
            prefs::restore_desktop_icons();
            record::shutdown(app);
        }
        // Letztes Fenster geschlossen → App läuft in der Menüleiste weiter.
        RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
        _ => {}
    });
}

/// Abstand der automatischen Update-Prüfung in Sekunden (Capso `UpdateCheckFrequency`).
pub fn update_interval(freq: &str) -> i64 {
    match freq {
        "weekly" => 604_800,
        "monthly" => 2_592_000,
        _ => 86_400,
    }
}

/// Von nrzCapture lesbare Bildformate (Capso `ImageFileOpenRequest`, Info.plist-Dokumenttypen).
pub fn is_image_file(p: &std::path::Path) -> bool {
    p.is_file()
        && p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "heic" | "heif" | "tif" | "tiff" | "gif")
        })
}

/// Schlafen ohne eigene tokio-Abhängigkeit.
pub async fn sleep(d: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

