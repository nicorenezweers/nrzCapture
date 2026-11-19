//! Tab „Kürzel“: aufnehmbare globale Kürzel, Konflikte, Standard, macOS ⌘⇧3/4/5, Kontext-Kürzel.
use leptos::{html, prelude::*, task::spawn_local};
use serde_json::{json, Value};
use std::collections::HashMap;

use super::{keys, widgets::*};
use crate::{api, api::t, ui};

/// Reihenfolge wie Capso `ShortcutSettingsView`.
const ORDER: &[&str] = &["captureArea", "captureWindow", "captureFullscreen"];

#[derive(Clone, Copy)]
struct Ctx {
    labels: RwSignal<HashMap<String, String>>,
    defaults: RwSignal<HashMap<String, String>>,
    errors: RwSignal<Vec<String>>,
}

fn shortcuts_now() -> Vec<(String, String)> {
    api::settings().with_untracked(|s| s["shortcuts"].as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned())).collect()).unwrap_or_default())
}

fn load_errors(errors: RwSignal<Vec<String>>) {
    spawn_local(async move {
        if let Ok(Value::Array(a)) = api::rpc("prefs", "shortcutErrors", Value::Null).await {
            errors.set(a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect());
        }
    });
}

#[component]
pub fn ShortcutsTab() -> impl IntoView {
    let ctx = Ctx { labels: RwSignal::new(HashMap::new()), defaults: RwSignal::new(HashMap::new()), errors: RwSignal::new(vec![]) };
    spawn_local(async move {
        if let Ok(Value::Array(a)) = api::rpc("prefs", "actions", Value::Null).await {
            ctx.labels.set(a.iter().map(|v| (v["id"].as_str().unwrap_or("").to_owned(), v["label"].as_str().unwrap_or("").to_owned())).collect());
        }
        if let Ok(v) = api::rpc("settings", "defaults", Value::Null).await {
            if let Some(o) = v["shortcuts"].as_object() {
                ctx.defaults.set(o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned())).collect());
            }
        }
    });
    // Nach jeder Einstellungsänderung (neu registrierte Kürzel) Fehler neu laden; Effekt endet mit dem Tab.
    Effect::new(move |_| {
        api::settings().track();
        load_errors(ctx.errors);
    });
    provide_context(ctx);

    let reset_all = move || {
        spawn_local(async move {
            if let Ok(Value::Array(a)) = api::rpc("prefs", "resetShortcuts", Value::Null).await {
                ctx.errors.set(a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect());
            }
        })
    };

    let contextual: Vec<(&str, &str, &[&str])> = vec![
        (t("Auswahl", "Selection"), t("Ganzen Monitor statt Auswahl", "Entire screen instead"), &["F"]),
        (t("Auswahl", "Selection"), t("Abbrechen", "Cancel"), &["Esc"]),
        (t("Fensteraufnahme", "Window Capture"), t("Mehrere Fenster wählen", "Select multiple windows"), &["⇧ + Klick"]),
        (t("Fensteraufnahme", "Window Capture"), t("Gewählte Fenster aufnehmen", "Capture selected windows"), &["⇧ loslassen"]),
    ];

    view! {
        {banner("info", t(
            "Klicke auf ein Kürzel, um eine neue Kombination aufzunehmen. Esc bricht ab, ⌫ entfernt das Kürzel. Kontext-Kürzel sind fest und gelten nur, während das jeweilige Fenster aktiv ist.",
            "Click a customizable shortcut to record a new combination. Press Esc to cancel or Delete to remove. Contextual shortcuts are fixed and work only while that window is active.").to_owned())}

        {move || {
            let e = ctx.errors.get();
            (!e.is_empty()).then(|| banner("error", format!("{}\n{}", t("Diese Kürzel konnten nicht registriert werden (evtl. von einer anderen App belegt):", "These shortcuts could not be registered (maybe used by another app):"), e.join("\n"))))
        }}

        <SystemShortcutsCard/>

        <ui::Card title=t("Anpassbare Kürzel", "Customizable Shortcuts")>
            {ORDER.iter().map(|id| view! { <ShortcutRow action=id/> }).collect_view()}
        </ui::Card>
        <div class="-mt-3 mb-5 flex justify-end">
            <ui::TextButton on_click=reset_all><ui::Icon d=ui::RESTART/>{t("Alle auf Standard zurücksetzen", "Reset All to Defaults")}</ui::TextButton>
        </div>

        <ui::Card title=t("Kontext-Kürzel", "Contextual Shortcuts")>
            <CenterLockRow/>
            {contextual.into_iter().map(|(scope, action, keys)| view! {
                <ui::Row label=action hint=scope>
                    {keys.iter().map(|k| view! { <span class="rounded border border-black/10 bg-black/5 px-1.5 py-0.5 text-[11px] dark:border-white/15 dark:bg-white/10">{*k}</span> }).collect_view()}
                </ui::Row>
            }).collect_view()}
        </ui::Card>
    }
}

#[component]
fn ShortcutRow(action: &'static str) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let value = Signal::derive(move || api::settings().with(|s| s["shortcuts"][action].as_str().unwrap_or("").to_owned()));
    let recording = RwSignal::new(false);
    let msg = RwSignal::new(None::<String>);
    let btn = NodeRef::<html::Button>::new();
    let label = move || ctx.labels.with(|l| l.get(action).cloned().unwrap_or_else(|| action.to_owned()));
    let default = move || ctx.defaults.with(|d| d.get(action).cloned().unwrap_or_default());

    let pause = |p: bool| api::fire("prefs", "pauseShortcuts", json!({"paused": p}));
    let stop = move || {
        if recording.get_untracked() {
            recording.set(false);
            pause(false);
        }
    };
    let save = move |accel: String| {
        spawn_local(async move {
            match api::rpc("prefs", "setShortcut", json!({"action": action, "accel": accel})).await {
                Ok(v) if v["conflict"].is_string() => msg.set(Some(conflict_text(&ctx, v["conflict"].as_str().unwrap_or("")))),
                Ok(v) => {
                    msg.set(None);
                    if let Some(a) = v["errors"].as_array() {
                        ctx.errors.set(a.iter().filter_map(|e| e.as_str().map(str::to_owned)).collect());
                    }
                }
                Err(e) => msg.set(Some(e)),
            }
        });
    };
    let on_key = move |e: leptos::ev::KeyboardEvent| {
        if !recording.get_untracked() {
            return;
        }
        e.prevent_default();
        e.stop_propagation();
        match keys::record(&e.code(), e.ctrl_key(), e.alt_key(), e.shift_key(), e.meta_key()) {
            keys::Recorded::Cancel => stop(),
            keys::Recorded::Clear => {
                stop();
                save(String::new());
            }
            keys::Recorded::Pending => {}
            keys::Recorded::NeedsModifier => msg.set(Some(t("Bitte mit ⌘, ⌥ oder ⌃ kombinieren.", "Combine with ⌘, ⌥ or ⌃.").to_owned())),
            keys::Recorded::Accel(a) => {
                let list = shortcuts_now();
                let replace = api::setting_bool("replaceSystemShortcuts");
                if let Some(c) = keys::conflict(list.iter().map(|(k, v)| (k.as_str(), v.as_str())), action, &a, replace) {
                    msg.set(Some(format!("{} – {}", ui::shortcut_symbols(&a), conflict_text(&ctx, &c))));
                } else {
                    stop();
                    save(a);
                }
            }
        }
    };

    view! {
        <div class="flex min-h-[40px] items-center justify-between gap-4 px-3 py-1.5">
            <div class="min-w-0">
                <div class="text-[13px]">{label}</div>
                {move || msg.get().map(|m| view! { <div class="text-[11px] text-red-500">{m}</div> })}
            </div>
            <div class="flex shrink-0 items-center gap-1">
                <button
                    node_ref=btn
                    class=move || format!(
                        "flex h-7 min-w-[120px] items-center justify-center rounded-md border px-2 text-[12px] outline-none transition-colors {}",
                        if recording.get() { "border-blue-500 ring-2 ring-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-300" }
                        else { "border-black/10 bg-white/80 hover:bg-white dark:border-white/10 dark:bg-white/10 dark:hover:bg-white/15" }
                    )
                    on:click=move |_| {
                        msg.set(None);
                        if recording.get_untracked() { stop() } else {
                            recording.set(true);
                            pause(true);
                            if let Some(b) = btn.get() { let _ = b.focus(); }
                        }
                    }
                    on:keydown=on_key
                    on:blur=move |_| stop()
                >
                    {move || if recording.get() {
                        t("Kürzel drücken …", "Press shortcut…").into_any()
                    } else if value.get().is_empty() {
                        view! { <span class="text-black/40 dark:text-white/40">{t("Aufnehmen", "Record Shortcut")}</span> }.into_any()
                    } else {
                        view! { <span class="font-medium tracking-wide">{ui::shortcut_symbols(&value.get())}</span> }.into_any()
                    }}
                </button>
                <Show when=move || !value.get().is_empty()>
                    <ui::TextButton variant="ghost" title=t("Entfernen", "Remove").to_string() on_click=move || save(String::new())><ui::Icon d=ui::X/></ui::TextButton>
                </Show>
                <Show when=move || keys::normalize(&value.get()) != keys::normalize(&default())>
                    <ui::TextButton variant="ghost" title=t("Auf Standard zurücksetzen", "Reset to default").to_string()
                        on_click=move || save(default())><ui::Icon d=ui::UNDO/></ui::TextButton>
                </Show>
            </div>
        </div>
    }
}

fn conflict_text(ctx: &Ctx, c: &str) -> String {
    match c {
        "macOS" => t("Von macOS belegt", "Used by macOS").to_owned(),
        "replaceSystemShortcuts" => t("Belegt durch „macOS-Kürzel ersetzen“", "Used by “Replace macOS shortcuts”").to_owned(),
        other => format!("{} „{}“", t("Bereits belegt durch", "Already used by"), ctx.labels.with_untracked(|l| l.get(other).cloned().unwrap_or_else(|| other.to_owned()))),
    }
}

/// Capso `SystemScreenshotShortcutsCard`: ⌘⇧3/4/5 an nrzCapture umleiten.
#[component]
fn SystemShortcutsCard() -> impl IntoView {
    let sys_on = RwSignal::new(None::<bool>);
    let refresh = move || {
        spawn_local(async move {
            if let Ok(v) = api::rpc("prefs", "systemShortcutsEnabled", Value::Null).await {
                sys_on.set(v.as_bool());
            }
        })
    };
    refresh();
    let replace = sb("replaceSystemShortcuts");
    view! {
        <ui::Card title=t("macOS-Bildschirmfoto-Kürzel", "macOS Screenshot Shortcuts")>
            <ui::Row label=t("⇧⌘3 / ⇧⌘4 / ⇧⌘5 durch nrzCapture ersetzen", "Use ⇧⌘3 / ⇧⌘4 / ⇧⌘5 for nrzCapture")
                hint=t("⇧⌘3 Ganzer Monitor · ⇧⌘4 Auswahl · ⇧⌘5 Fenster. Die macOS-Kürzel werden beim Beenden wiederhergestellt.", "⇧⌘3 Entire Screen · ⇧⌘4 Selection · ⇧⌘5 Window. macOS shortcuts are restored on quit.")>
                <ui::Toggle checked=replace on_change=move |v| {
                    api::set_setting("replaceSystemShortcuts", json!(v));
                    set_timeout(refresh, std::time::Duration::from_millis(800));
                }/>
            </ui::Row>
            <ui::Row label=t("Status", "Status") hint="">
                <span class="max-w-[300px] text-right text-[11px] text-black/50 dark:text-white/50">{move || match (sys_on.get(), replace.get()) {
                    (Some(true), false) => t("macOS nutzt ⇧⌘3/4/5 für das eigene Bildschirmfoto-Werkzeug.", "macOS still uses ⇧⌘3/4/5 for its own screenshot tool."),
                    (Some(true), true) => t("macOS-Kürzel sind noch aktiv – ggf. in den Tastatureinstellungen deaktivieren.", "macOS shortcuts are still active — disable them in Keyboard Settings if needed."),
                    (Some(false), _) => t("macOS-Kürzel sind deaktiviert.", "macOS shortcuts are disabled."),
                    _ => "",
                }}</span>
                <ui::TextButton on_click=move || api::fire("prefs", "openKeyboardSettings", Value::Null)>{t("Tastatureinstellungen öffnen", "Open Keyboard Settings")}</ui::TextButton>
            </ui::Row>
        </ui::Card>
    }
}

/// Capso `SquareCenterLockKeyRecorder`: ⇧ + Zusatztaste, um 1:1 vom Klickpunkt aus aufzuziehen.
#[component]
fn CenterLockRow() -> impl IntoView {
    let recording = RwSignal::new(false);
    let key = ss("squareCenterLockKey");
    view! {
        <ui::Row label=t("1:1 vom Klickpunkt aus zentrieren", "Center 1:1 from click") hint=t("Auswahl – ⇧ halten für 1:1, dazu diese Taste zum Zentrieren", "Selection — hold ⇧ for 1:1, plus this key to grow from the center")>
            <span class="rounded border border-black/10 bg-black/5 px-1.5 py-0.5 text-[11px] dark:border-white/15 dark:bg-white/10">"⇧"</span>
            <span class="text-[11px] font-semibold text-black/40">"+"</span>
            <button
                class=move || format!("h-6 min-w-[32px] rounded-md border px-2 font-mono text-[11px] font-semibold outline-none {}",
                    if recording.get() { "border-blue-500 text-blue-500 bg-blue-500/10" } else { "border-black/10 bg-black/5 dark:border-white/10 dark:bg-white/10" })
                on:click=move |_| recording.update(|r| *r = !*r)
                on:blur=move |_| recording.set(false)
                on:keydown=move |e| {
                    if !recording.get_untracked() { return; }
                    e.prevent_default();
                    let k = e.key();
                    if k == "Escape" { recording.set(false); return; }
                    if k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) {
                        api::set_setting("squareCenterLockKey", json!(k.to_uppercase()));
                        recording.set(false);
                    }
                }
            >{move || if recording.get() { "…".to_owned() } else { key.get() }}</button>
        </ui::Row>
    }
}
