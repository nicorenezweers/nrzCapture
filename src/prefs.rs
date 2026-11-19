//! Einstellungsfenster (Route `prefs`, Fensterdaten `{tab}`) – Port von Capso `PreferencesView` mit allen Tabs.
use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;

use crate::{api, api::t, ui};

mod cloud;
mod general;
mod keys;
mod media;
mod shortcuts;
mod widgets;

/// (ID, Icon) – Reihenfolge wie Capso, zusätzlich „Über“.
const TABS: &[(&str, &str)] = &[
    ("general", ui::SETTINGS),
    ("permissions", ui::SHIELD),
    ("screenshot", ui::CAMERA),
    ("export", ui::FOLDER),
    ("shortcuts", ui::KEYBOARD),
    ("about", ui::INFO),
];

fn tab_title(id: &str) -> &'static str {
    match id {
        "general" => t("Allgemein", "General"),
        "permissions" => t("Berechtigungen", "Permissions"),
        "screenshot" => t("Screenshot", "Screenshots"),
        "recording" => t("Aufnahme", "Recording"),
        "quickAccess" => t("Quick Access", "Quick Access"),
        "cloud" => t("Cloud-Freigabe", "Cloud Share"),
        "export" => t("Export", "Export"),
        "ocr" => t("OCR & Übersetzung", "Text & Translation"),
        "shortcuts" => t("Kürzel", "Shortcuts"),
        _ => t("Über", "About"),
    }
}

/// Tab-Namen aus `prefs::open(app, Some(tab))` auf die Seitenleiste abbilden.
fn normalize_tab(tab: &str) -> &'static str {
    match tab {
        "general" => "general",
        "permissions" => "permissions",
        "screenshot" | "screenshots" => "screenshot",
        "recording" => "recording",
        "quickAccess" => "quickAccess",
        "cloud" | "cloudShare" => "cloud",
        "export" => "export",
        "ocr" | "translation" => "ocr",
        "shortcuts" => "shortcuts",
        "about" => "about",
        _ => "general",
    }
}

pub fn route(r: &str) -> Option<AnyView> {
    (r == "prefs").then(|| view! { <Prefs/> }.into_any())
}

#[component]
fn Prefs() -> impl IntoView {
    let tab = RwSignal::new("general");
    let apply = move |v: Value| {
        if let Some(s) = v["tab"].as_str() {
            tab.set(normalize_tab(s));
        }
    };
    spawn_local(async move { apply(api::window_data().await) });
    api::on("window-data", apply);

    let content = move || match tab.get() {
        "general" => general::general().into_any(),
        "permissions" => general::permissions().into_any(),
        "screenshot" => media::screenshot().into_any(),
        "recording" => media::recording().into_any(),
        "quickAccess" => media::quick_access().into_any(),
        "cloud" => view! { <cloud::CloudTab/> }.into_any(),
        "export" => media::export().into_any(),
        "ocr" => media::ocr().into_any(),
        "shortcuts" => view! { <shortcuts::ShortcutsTab/> }.into_any(),
        _ => general::about().into_any(),
    };

    view! {
        <div class="app-window flex h-screen overflow-hidden text-[13px]">
            <div class="fixed inset-x-0 top-0 h-7 z-10" data-tauri-drag-region></div>
            <aside class="flex w-[190px] shrink-0 flex-col gap-0.5 border-r border-black/5 bg-black/[0.03] px-2 pt-10 pb-3 dark:border-white/10 dark:bg-white/[0.04]">
                {TABS.iter().map(|(id, icon)| {
                    let id: &'static str = id;
                    let icon: &'static str = icon;
                    view! {
                        <button
                            class=move || format!(
                                "flex items-center gap-2.5 rounded-lg px-2.5 py-[6px] text-left text-[13px] transition-colors [&_svg]:size-4 [&_svg]:shrink-0 {}",
                                if tab.get() == id { "bg-blue-500 text-white shadow-sm" } else { "text-black/75 hover:bg-black/5 dark:text-white/75 dark:hover:bg-white/10" }
                            )
                            on:click=move |_| tab.set(id)
                        >
                            <ui::Icon d=icon/>
                            <span class="truncate">{tab_title(id)}</span>
                        </button>
                    }
                }).collect_view()}
            </aside>
            <main class="min-w-0 flex-1 overflow-y-auto px-7 pt-10 pb-8">
                <h1 class="mb-4 text-[20px] font-bold">{move || tab_title(tab.get())}</h1>
                {content}
            </main>
        </div>
    }
}
