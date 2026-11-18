//! Tabs „Allgemein“, „Berechtigungen“, „Über“ + Automations-URLs-Blatt.
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

use super::widgets::*;
use crate::{api, api::t, ui};

pub fn general() -> impl IntoView {
    let autostart = RwSignal::new(api::setting_bool("startAtLogin"));
    spawn_local(async move {
        if let Ok(v) = api::rpc("prefs", "autostart", Value::Null).await {
            autostart.set(v.as_bool().unwrap_or(false));
        }
    });
    let confirm_hide = RwSignal::new(false);
    let show_urls = RwSignal::new(false);
    let icons_hidden = RwSignal::new(false);
    spawn_local(async move {
        if let Ok(v) = api::rpc("prefs", "desktopIconsHidden", Value::Null).await {
            icons_hidden.set(v.as_bool().unwrap_or(false));
        }
    });
    let version = RwSignal::new(String::new());
    spawn_local(async move {
        if let Ok(v) = api::rpc("core", "version", Value::Null).await {
            version.set(v.as_str().unwrap_or("").to_owned());
        }
    });
    let shortcut = |id: &str| {
        let s = api::settings().with_untracked(|s| s["shortcuts"][id].as_str().unwrap_or("").to_owned());
        if s.is_empty() { t("nicht belegt", "not set").to_owned() } else { ui::shortcut_symbols(&s) }
    };

    view! {
        <ui::Card title=t("Sprache", "Language")>
            <ui::Row label=t("Sprache der Oberfläche", "Interface Language") hint=t("Fenster werden neu geladen", "Windows reload")>
                <ui::Select value=ss("language")
                    options=opts(&[("system", t("Systemsprache", "System")), ("de", "Deutsch"), ("en", "English")])
                    on_change=move |v: String| {
                        api::set_setting("language", json!(v));
                        // `t()` liest die Sprache nicht reaktiv → neu laden.
                        set_timeout(|| { let _ = window().location().reload(); }, std::time::Duration::from_millis(250));
                    }/>
            </ui::Row>
        </ui::Card>

        <ui::Card title=t("Start", "Startup")>
            <ui::Row label=t("Bei Anmeldung starten", "Launch at Login") hint=t("nrzCapture beim Anmelden starten", "Start nrzCapture when you log in")>
                <ui::Toggle checked=autostart on_change=move |v| {
                    autostart.set(v);
                    spawn_local(async move {
                        if let Ok(r) = api::rpc("prefs", "setAutostart", json!({"enabled": v})).await { autostart.set(r.as_bool().unwrap_or(v)); }
                        else { autostart.set(!v); }
                    });
                }/>
            </ui::Row>
            <ui::Row label=t("Symbol in der Menüleiste", "Show Menu Bar Icon") hint=t("Ausblenden für eine aufgeräumte Menüleiste – Kürzel funktionieren weiter", "Hide for a minimalist menu bar — shortcuts still work")>
                <ui::Toggle checked=sb("showMenuBarIcon") on_change=move |v| {
                    if v { api::set_setting("showMenuBarIcon", json!(true)) } else { confirm_hide.set(true) }
                }/>
            </ui::Row>
        </ui::Card>

        <ui::Card title=t("Automation", "Automation")>
            <ui::Row label=t("Automations-URLs", "Automation URLs") hint=t("Alfred, Raycast, Kurzbefehlen und anderen Apps erlauben, Aufnahmen auszulösen", "Allow Alfred, Raycast, Shortcuts, and other apps to trigger captures")>
                <ui::Toggle checked=sb("automationUrlsEnabled") on_change=move |v| {
                    api::set_setting("automationUrlsEnabled", json!(v));
                    api::set_setting("automationUrlsAsked", json!(true));
                }/>
            </ui::Row>
            <ui::Row label=t("Unterstützte URLs", "Supported URLs") hint=t("Alle Befehle für Raycast, Alfred und Kurzbefehle", "View all commands for Raycast, Alfred, and Shortcuts")>
                <ui::TextButton on_click=move || show_urls.set(true)>{t("Befehle anzeigen …", "View Commands…")}</ui::TextButton>
            </ui::Row>
        </ui::Card>

        <ui::Card title=t("Ton", "Sound")>
            {toggle_row(t("Auslöserton", "Shutter Sound"), t("Ton nach der Aufnahme abspielen", "Play sound after capture"), "playShutterSound")}
        </ui::Card>

        <ui::Card title=t("Schreibtisch", "Desktop")>
            <ui::Row label=t("Schreibtisch-Symbole ausblenden", "Hide Desktop Icons") hint=t("Deckt die Symbole mit dem Hintergrundbild ab (ohne Finder-Neustart)", "Covers icons with the wallpaper (no Finder restart)")>
                <ui::Toggle checked=icons_hidden on_change=move |_| spawn_local(async move {
                    if let Ok(v) = api::rpc("prefs", "toggleDesktopIcons", Value::Null).await { icons_hidden.set(v.as_bool().unwrap_or(false)); }
                })/>
            </ui::Row>
        </ui::Card>

        <ui::Card title=t("Updates", "Updates")>
            {toggle_row(t("Automatisch nach Updates suchen", "Automatically Check for Updates"), t("Im Hintergrund nach neuen Versionen suchen", "Look for new versions in the background"), "updateAutoCheck")}
            // Abhängige Zeilen werden ausgegraut, die gespeicherten Werte bleiben erhalten (Capso `UpdatesSettingsSection`).
            <div class=move || if sb("updateAutoCheck").get() { "" } else { "pointer-events-none opacity-40" }>
                {select_row(t("Prüfintervall", "Check Frequency"), "", "updateCheck",
                    opts(&[("daily", t("Täglich", "Daily")), ("weekly", t("Wöchentlich", "Weekly")), ("monthly", t("Monatlich", "Monthly"))]))}
                {toggle_row(t("Updates automatisch installieren", "Automatically Install Updates"), t("Verfügbare Updates im Hintergrund installieren", "Install updates in the background when available"), "updateAutoInstall")}
            </div>
            <ui::Row label=t("Nach Updates suchen", "Check for Updates") hint=t("Jetzt nach einer neuen Version suchen", "Look for a new version now")>
                <span class="font-mono text-[12px] text-black/50 dark:text-white/50">{move || version.get()}</span>
                <ui::TextButton on_click=move || api::fire("prefs", "checkUpdates", Value::Null)>{t("Jetzt suchen", "Check Now")}</ui::TextButton>
            </ui::Row>
        </ui::Card>

        <ui::Card title=t("Verlauf", "History")>
            {toggle_row(t("Im Verlauf speichern", "Save to History"), t("Alle Aufnahmen automatisch sichern", "Automatically save all captures"), "historyEnabled")}
            {select_row(t("Verlauf behalten", "Keep History"), t("Ältere Aufnahmen automatisch löschen", "Auto-delete older captures"), "historyRetention",
                opts(&[("oneWeek", t("1 Woche", "1 Week")), ("twoWeeks", t("2 Wochen", "2 Weeks")), ("oneMonth", t("1 Monat", "1 Month")), ("unlimited", t("Unbegrenzt", "Unlimited"))]))}
        </ui::Card>

        <ui::Card title=t("Diagnose", "Diagnostics")>
            {toggle_row(t("Diagnose-Protokoll", "Diagnostic Logging"), t("Lokale Protokolle zur Fehlersuche schreiben", "Write local troubleshooting logs when enabled"), "diagnosticLoggingEnabled")}
            <ui::Row label=t("Protokolle", "Diagnostic Logs") hint=t("Lokalen Protokollordner öffnen", "Open nrzCapture's local log folder")>
                <ui::TextButton on_click=move || api::fire("prefs", "openLogs", Value::Null)><ui::Icon d=ui::FOLDER/>{t("Öffnen", "Open")}</ui::TextButton>
            </ui::Row>
            <ui::Row label=t("Absturzberichte", "Crash Reports") hint=t("macOS-DiagnosticReports öffnen", "Open macOS DiagnosticReports")>
                <ui::TextButton on_click=move || api::fire("prefs", "openCrashReports", Value::Null)><ui::Icon d=ui::FOLDER/>{t("Öffnen", "Open")}</ui::TextButton>
            </ui::Row>
        </ui::Card>

        <AutomationSheet open=show_urls/>

        <Sheet open=confirm_hide>
            <h2 class="mb-2 text-[15px] font-semibold">{t("Menüleistensymbol ausblenden?", "Hide menu bar icon?")}</h2>
            <p class="mb-4 whitespace-pre-line text-[12px] text-black/70 dark:text-white/70">
                {move || format!("{}\n\n• {}: {}\n• {}: {}\n• {}: {}\n\n{}",
                    t("nrzCapture läuft weiter, und die globalen Kürzel funktionieren:", "nrzCapture will keep running, and your global shortcuts still work:"),
                    t("Bereich aufnehmen", "Capture Area"), shortcut("captureArea"),
                    t("Bildschirm aufzeichnen", "Record Screen"), shortcut("recordScreen"),
                    t("Verlauf", "Screenshot History"), shortcut("screenshotHistory"),
                    t("Um die Einstellungen wieder zu öffnen, starte nrzCapture erneut über Spotlight, Launchpad oder den Finder – auch während es läuft.",
                      "To open Preferences again, launch nrzCapture from Spotlight, Launchpad, or Finder — even while it's still running."))}
            </p>
            <div class="flex justify-end gap-2">
                <ui::TextButton on_click=move || confirm_hide.set(false)>{t("Abbrechen", "Cancel")}</ui::TextButton>
                <ui::TextButton variant="danger" on_click=move || { api::set_setting("showMenuBarIcon", json!(false)); confirm_hide.set(false); }>
                    {t("Symbol ausblenden", "Hide Icon")}
                </ui::TextButton>
            </div>
        </Sheet>
    }
}

#[component]
fn AutomationSheet(open: RwSignal<bool>) -> impl IntoView {
    let groups = RwSignal::new(Value::Null);
    let copied = RwSignal::new(String::new());
    Effect::new(move |_| {
        if open.get() && groups.get_untracked().is_null() {
            spawn_local(async move {
                if let Ok(v) = api::rpc("prefs", "automationUrls", Value::Null).await {
                    groups.set(v);
                }
            });
        }
    });
    view! {
        <Sheet open=open wide=true>
            <div class="mb-3 flex items-start justify-between gap-3">
                <div>
                    <h2 class="text-[15px] font-semibold">{t("Automations-URLs", "Automation URLs")}</h2>
                    <p class="text-[12px] text-black/55 dark:text-white/55">{t("nrzCapture-Aktionen aus Raycast, Alfred, Kurzbefehlen oder Shell-Skripten auslösen.", "Trigger nrzCapture actions from Raycast, Alfred, Shortcuts, or shell scripts.")}</p>
                </div>
                <ui::TextButton variant="ghost" on_click=move || open.set(false)><ui::Icon d=ui::X/></ui::TextButton>
            </div>
            <ui::Card>
                <ui::Row label=t("Automations-URLs erlauben", "Allow Automation URLs") hint=t("Ohne Erlaubnis werden Befehle ignoriert", "Commands are ignored unless allowed")>
                    <ui::Toggle checked=sb("automationUrlsEnabled") on_change=move |v| {
                        api::set_setting("automationUrlsEnabled", json!(v));
                        api::set_setting("automationUrlsAsked", json!(true));
                    }/>
                </ui::Row>
            </ui::Card>
            {move || groups.get().as_array().cloned().unwrap_or_default().into_iter().map(|g| {
                let title = g["title"].as_str().unwrap_or("").to_owned();
                view! {
                    <ui::Card title=title>
                        {g["items"].as_array().cloned().unwrap_or_default().into_iter().map(|it| {
                            let url = it["url"].as_str().unwrap_or("").to_owned();
                            let (u1, u2, u3) = (url.clone(), url.clone(), url.clone());
                            view! {
                                <ui::Row label=it["name"].as_str().unwrap_or("").to_owned()>
                                    <code class="select-text text-[11px] text-black/60 dark:text-white/60">{u1}</code>
                                    <ui::TextButton variant="ghost" title=t("Kopieren", "Copy").to_string() on_click=move || {
                                        api::fire("core", "copyText", json!({"text": u2.clone()}));
                                        copied.set(u2.clone());
                                    }>
                                        <ui::Icon d=Signal::derive(move || if copied.get() == u3 { ui::CHECK } else { ui::COPY })/>
                                    </ui::TextButton>
                                </ui::Row>
                            }
                        }).collect_view()}
                    </ui::Card>
                }
            }).collect_view()}
            <p class="text-[11px] text-black/45 dark:text-white/45">
                {t("Kurzformen wie nrzcapture://area oder Aktions-IDs wie nrzcapture://captureArea funktionieren ebenfalls. Terminal: open \"nrzcapture://grab/area\"",
                   "Short forms like nrzcapture://area or action IDs like nrzcapture://captureArea work too. Terminal: open \"nrzcapture://grab/area\"")}
            </p>
        </Sheet>
    }
}

pub fn permissions() -> impl IntoView {
    let st = RwSignal::new(Value::Null);
    let refresh = move || {
        spawn_local(async move {
            if let Ok(v) = api::rpc("prefs", "permissions", Value::Null).await {
                st.set(v);
            }
        })
    };
    refresh();
    // Nach Rückkehr aus den Systemeinstellungen neu prüfen.
    let h = window_event_listener(leptos::ev::focus, move |_| refresh());
    on_cleanup(move || h.remove());

    let row = move |kind: &'static str, title: &'static str, sub: &'static str, anchor: &'static str| {
        let status = move || st.with(|s| s[kind].as_str().unwrap_or("").to_owned());
        view! {
            <ui::Row label=title hint=sub>
                {move || {
                    let s = status();
                    let (c, l) = match s.as_str() {
                        "granted" => ("bg-green-500/15 text-green-700 dark:text-green-300", t("Erlaubt", "Granted")),
                        "notDetermined" => ("bg-black/5 text-black/60 dark:bg-white/10 dark:text-white/60", t("Nicht gefragt", "Not Requested")),
                        "restricted" => ("bg-amber-500/15 text-amber-700 dark:text-amber-300", t("Eingeschränkt", "Restricted")),
                        "" => ("", ""),
                        _ => ("bg-red-500/15 text-red-700 dark:text-red-300", t("Nicht erlaubt", "Not Granted")),
                    };
                    let granted = s == "granted";
                    view! {
                        <span class=format!("rounded-full px-2 py-0.5 text-[11px] font-medium {c}")>{l}</span>
                        <Show when=move || !granted>
                            <ui::TextButton variant="primary" on_click=move || {
                                api::fire("prefs", "requestPermission", json!({"kind": kind}));
                                set_timeout(refresh, std::time::Duration::from_millis(1500));
                            }>{t("Erlauben", "Grant")}</ui::TextButton>
                        </Show>
                        <ui::TextButton variant="ghost" title=t("Systemeinstellungen öffnen", "Open System Settings").to_string()
                            on_click=move || api::fire("core", "openPrivacy", json!({"anchor": anchor}))>
                            <ui::Icon d=ui::EXTERNAL/>
                        </ui::TextButton>
                    }
                }}
            </ui::Row>
        }
    };

    view! {
        <div class="mb-3 flex justify-end">
            <ui::TextButton on_click=refresh><ui::Icon d=ui::RESTART/>{t("Aktualisieren", "Refresh")}</ui::TextButton>
        </div>
        <ui::Card title=t("Datenschutz & Sicherheit", "Privacy & Security")>
            {row("screen", t("Bildschirmaufnahme", "Screen Recording"), t("Für Screenshots und Aufnahmen nötig", "Required for screenshots and recording"), "Privacy_ScreenCapture")}
            {row("accessibility", t("Bedienungshilfen", "Accessibility"), t("Für die Übersetzung markierten Texts nötig", "Required for selected text translation"), "Privacy_Accessibility")}
            {row("inputMonitoring", t("Eingabeüberwachung", "Input Monitoring"), t("Zum Anzeigen von Tastendrücken während der Aufnahme", "Required to show key presses while recording"), "Privacy_ListenEvent")}
            {row("camera", t("Kamera", "Camera"), t("Für die Kamera-Einblendung nötig", "Required for camera overlay"), "Privacy_Camera")}
            {row("microphone", t("Mikrofon", "Microphone"), t("Für Mikrofon-Aufnahmen nötig", "Required for microphone recording"), "Privacy_Microphone")}
        </ui::Card>
        <p class="px-1 text-[11px] text-black/45 dark:text-white/45">
            {t("Nach dem Erteilen der Bildschirmaufnahme-Berechtigung muss nrzCapture eventuell neu gestartet werden.",
               "After granting Screen Recording, nrzCapture may need to be restarted.")}
        </p>
        <div class="mt-2">
            <ui::TextButton on_click=move || api::fire("core", "restart", Value::Null)>{t("nrzCapture neu starten", "Restart nrzCapture")}</ui::TextButton>
        </div>
    }
}

pub fn about() -> impl IntoView {
    let version = RwSignal::new(String::new());
    spawn_local(async move {
        if let Ok(v) = api::rpc("core", "version", Value::Null).await {
            version.set(v.as_str().unwrap_or("").to_owned());
        }
    });
    view! {
        <div class="mb-6 flex items-center gap-4">
            <div class="flex size-16 items-center justify-center rounded-2xl bg-gradient-to-br from-blue-500 to-indigo-600 text-white shadow-lg [&_svg]:size-8">
                <ui::Icon d=ui::CAMERA/>
            </div>
            <div>
                <div class="text-[22px] font-bold">"nrzCapture"</div>
                <div class="font-mono text-[12px] text-black/50 dark:text-white/50">{move || format!("{} {}", t("Version", "Version"), version.get())}</div>
            </div>
        </div>
        <ui::Card title=t("Lizenz", "License")>
            <div class="px-3 py-3 text-[12px] leading-relaxed text-black/70 dark:text-white/70">
                {t("nrzCapture ist ein Rust-Port (Tauri + Leptos) der macOS-App Capso von lzhgus. Capso steht unter der Business Source License 1.1 (BSL 1.1); dieser Port übernimmt Funktionsumfang und Verhalten und unterliegt denselben Lizenzbedingungen. Alle Rechte am Original liegen bei den Capso-Autoren.",
                   "nrzCapture is a Rust port (Tauri + Leptos) of the macOS app Capso by lzhgus. Capso is licensed under the Business Source License 1.1 (BSL 1.1); this port mirrors its features and behaviour and is subject to the same license terms. All rights to the original belong to the Capso authors.")}
            </div>
        </ui::Card>
        <ui::Card title=t("Links", "Links")>
            <ui::Row label=t("Original: Capso", "Original: Capso") hint="github.com/lzhgus/Capso">
                {link_button(t("Öffnen", "Open"), "https://github.com/lzhgus/Capso")}
            </ui::Row>
            <ui::Row label=t("Lizenz (BSL 1.1)", "License (BSL 1.1)") hint=t("Lizenztext des Originals", "License text of the original")>
                {link_button(t("Öffnen", "Open"), "https://github.com/lzhgus/Capso/blob/main/LICENSE")}
            </ui::Row>
            <ui::Row label=t("Fehler melden", "Report a Bug") hint=t("Etwas funktioniert nicht?", "Found something broken?")>
                {link_button(t("Melden", "Report"), "http://100.78.107.162:8390/nicorenezweers/nrzCapture/issues")}
            </ui::Row>
            <ui::Row label=t("Quelltext", "Source Code") hint="nrzCapture">
                {link_button(t("Öffnen", "Open"), "http://100.78.107.162:8390/nicorenezweers/nrzCapture")}
            </ui::Row>
        </ui::Card>
        <div class="mt-2 flex gap-2">
            <ui::TextButton on_click=move || api::fire("prefs", "checkUpdates", Value::Null)>{t("Nach Updates suchen", "Check for Updates")}</ui::TextButton>
            <ui::TextButton variant="danger" on_click=move || api::fire("core", "quit", Value::Null)>{t("nrzCapture beenden", "Quit nrzCapture")}</ui::TextButton>
        </div>
    }
}
