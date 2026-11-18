//! Kleine Bausteine für die Einstellungen: Zeilen, die direkt an einen Einstellungsschlüssel gebunden sind.
use leptos::prelude::*;
use serde_json::{json, Value};

use crate::{api, ui};

/// Reaktiver Bool-Wert.
pub fn sb(key: &'static str) -> Signal<bool> {
    Signal::derive(move || api::setting_bool(key))
}

/// Reaktiver Wert als Text (Zahlen werden formatiert, `null` = leer).
pub fn ss(key: &'static str) -> Signal<String> {
    Signal::derive(move || value_str(&api::setting(key)))
}

pub fn value_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Number(n) => n.as_f64().map(|f| if f.fract() == 0.0 { format!("{}", f as i64) } else { f.to_string() }).unwrap_or_default(),
        other => other.to_string(),
    }
}

pub fn opts(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect()
}

pub fn toggle_row(label: &'static str, hint: &'static str, key: &'static str) -> impl IntoView {
    view! {
        <ui::Row label=label hint=hint>
            <ui::Toggle checked=sb(key) on_change=move |v| api::set_setting(key, json!(v))/>
        </ui::Row>
    }
}

/// Auswahl mit Text-Werten. Leerer Wert wird als `null` gespeichert, wenn `nullable`.
pub fn select_row(label: &'static str, hint: &'static str, key: &'static str, options: Vec<(String, String)>) -> impl IntoView {
    select_row_ext(label, hint, key, options, false)
}

pub fn select_row_ext(label: &'static str, hint: &'static str, key: &'static str, options: Vec<(String, String)>, nullable: bool) -> impl IntoView {
    view! {
        <ui::Row label=label hint=hint>
            <ui::Select value=ss(key) options=options on_change=move |v: String| {
                api::set_setting(key, if nullable && v.is_empty() { Value::Null } else { json!(v) })
            }/>
        </ui::Row>
    }
}

/// Auswahl mit Zahlen-Werten.
pub fn num_select_row(label: &'static str, hint: &'static str, key: &'static str, options: &[(i64, &str)]) -> impl IntoView {
    let o: Vec<(String, String)> = options.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
    view! {
        <ui::Row label=label hint=hint>
            <ui::Select value=ss(key) options=o on_change=move |v: String| {
                if let Ok(n) = v.parse::<i64>() { api::set_setting(key, json!(n)) }
            }/>
        </ui::Row>
    }
}

/// Textfeld; bei `nullable` wird ein leerer Wert als `null` gespeichert (für `Option<String>`-Felder).
pub fn text_row(label: &'static str, hint: &'static str, key: &'static str, placeholder: &'static str, password: bool, nullable: bool) -> impl IntoView {
    view! {
        <ui::Row label=label hint=hint>
            <div class="w-60">
                <ui::TextInput value=ss(key) placeholder=placeholder password=password on_change=move |v: String| {
                    let v = v.trim().to_owned();
                    api::set_setting(key, if nullable && v.is_empty() { Value::Null } else { json!(v) })
                }/>
            </div>
        </ui::Row>
    }
}

/// Schieberegler mit Wertanzeige.
pub fn slider_row(label: &'static str, hint: &'static str, key: &'static str, min: f64, max: f64, step: f64, unit: &'static str, int: bool) -> impl IntoView {
    let val = Signal::derive(move || api::setting_f64(key));
    view! {
        <ui::Row label=label hint=hint>
            <ui::Slider value=val min=min max=max step=step on_change=move |v: f64| {
                api::set_setting(key, if int { json!(v.round() as i64) } else { json!(v) })
            }/>
            <span class="w-14 text-right text-[12px] tabular-nums text-black/50 dark:text-white/50">
                {move || format!("{}{unit}", if int { format!("{}", val.get().round() as i64) } else { format!("{:.1}", val.get()) })}
            </span>
        </ui::Row>
    }
}

/// Hinweis-Kasten (blau/gelb/rot).
pub fn banner(kind: &'static str, text: String) -> impl IntoView {
    let c = match kind {
        "warn" => "bg-amber-500/10 border-amber-500/30 text-amber-800 dark:text-amber-200",
        "error" => "bg-red-500/10 border-red-500/30 text-red-700 dark:text-red-300",
        "ok" => "bg-green-500/10 border-green-500/30 text-green-800 dark:text-green-300",
        _ => "bg-blue-500/[0.08] border-blue-500/20 text-black/70 dark:text-white/70",
    };
    view! {
        <div class=format!("mb-4 flex items-start gap-2 rounded-lg border px-3 py-2.5 text-[12px] [&_svg]:mt-px [&_svg]:size-4 [&_svg]:shrink-0 {c}")>
            <ui::Icon d=ui::INFO/>
            <span class="whitespace-pre-line">{text}</span>
        </div>
    }
}

/// Modales Blatt (wie SwiftUI `.sheet`).
#[component]
pub fn Sheet(open: RwSignal<bool>, #[prop(optional)] wide: bool, children: ChildrenFn) -> impl IntoView {
    let children = StoredValue::new(children);
    view! {
        <Show when=move || open.get()>
            <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-6" on:click=move |_| open.set(false)>
                <div
                    class=format!("max-h-full w-full {} overflow-y-auto rounded-2xl border border-black/10 bg-[#f5f5f7] p-5 shadow-2xl dark:border-white/10 dark:bg-[#2a2a2c]", if wide { "max-w-[640px]" } else { "max-w-[460px]" })
                    on:click=|e| e.stop_propagation()
                >
                    {children.read_value()()}
                </div>
            </div>
        </Show>
    }
}

/// Externer Link als kleiner Knopf.
pub fn link_button(label: &'static str, url: &'static str) -> impl IntoView {
    view! {
        <ui::TextButton on_click=move || api::fire("core", "open", json!({"target": url}))>
            {label}<ui::Icon d=ui::EXTERNAL/>
        </ui::TextButton>
    }
}

/// Zahl mit −/+ (wie SwiftUI `Stepper`), begrenzt auf `min…max`.
pub fn stepper_row(label: &'static str, hint: &'static str, key: &'static str, min: i64, max: i64, unit: &'static str) -> impl IntoView {
    let val = Signal::derive(move || (api::setting_f64(key).round() as i64).clamp(min, max));
    let set = move |v: i64| api::set_setting(key, json!(v.clamp(min, max)));
    let btn = "flex size-6 items-center justify-center rounded-md border border-black/10 text-[14px] leading-none disabled:opacity-40 dark:border-white/10";
    view! {
        <ui::Row label=label hint=hint>
            <span class="w-12 text-right text-[12px] tabular-nums text-black/70 dark:text-white/70">{move || format!("{}{unit}", val.get())}</span>
            <button class=btn disabled=move || val.get() <= min on:click=move |_| set(val.get_untracked() - 1)>"−"</button>
            <button class=btn disabled=move || val.get() >= max on:click=move |_| set(val.get_untracked() + 1)>"+"</button>
        </ui::Row>
    }
}
