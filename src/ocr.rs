//! Frontend des Moduls `ocr`: Visual-OCR-Fenster (Capso `OCROverlayView`) und OCR-Onboarding (`OCROnboardingView`).
use crate::{api, ui::*};
use leptos::{ev, prelude::*, task::spawn_local};
use serde_json::{json, Value};
use std::time::Duration;
use wasm_bindgen::JsCast;

pub fn route(r: &str) -> Option<AnyView> {
    match r {
        "ocr-visual" => Some(view! { <Visual /> }.into_any()),
        "ocr-onboarding" => Some(view! { <Onboarding /> }.into_any()),
        _ => None,
    }
}

pub const LINK: &str = r#"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#;
pub const ARROW_RIGHT: &str = r#"<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>"#;
pub const CROSSHAIR: &str = r#"<circle cx="12" cy="12" r="10"/><path d="M22 12h-4"/><path d="M6 12H2"/><path d="M12 6V2"/><path d="M12 22v-4"/>"#;
pub const CLIPBOARD_CHECK: &str = r#"<rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="m9 14 2 2 4-4"/>"#;

/// Fenster per Maus verschieben (Backend `start_dragging`), außer beim Klick auf Knöpfe/Eingaben.
pub fn drag_unless_control(module: &'static str, e: &ev::MouseEvent) {
    if e.button() != 0 {
        return;
    }
    let on_control = e
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.closest("button, a, input, textarea, select, [data-no-drag], .select-text").ok().flatten())
        .is_some();
    if !on_control {
        api::fire(module, "drag", Value::Null);
    }
}

#[derive(Clone)]
struct Block {
    text: String,
    bbox: [f64; 4],
    links: Vec<String>,
}

#[component]
fn Visual() -> impl IntoView {
    let path = RwSignal::new(String::new());
    let size = RwSignal::new((1.0f64, 1.0f64));
    let blocks: RwSignal<Vec<Block>> = RwSignal::new(vec![]);
    let selected = RwSignal::new(None::<usize>);
    let copied = RwSignal::new(None::<usize>);
    let panel = NodeRef::<leptos::html::Div>::new();
    let panel_size = RwSignal::new((400.0f64, 400.0f64));

    let apply = move |v: Value| {
        if v.is_null() {
            return;
        }
        path.set(v["path"].as_str().unwrap_or("").to_owned());
        size.set((v["width"].as_f64().unwrap_or(1.0), v["height"].as_f64().unwrap_or(1.0)));
        selected.set(None);
        copied.set(None);
        blocks.set(
            v["blocks"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|b| {
                            let bb = |i: usize| b["bbox"][i].as_f64().unwrap_or(0.0);
                            Block {
                                text: b["text"].as_str().unwrap_or("").to_owned(),
                                bbox: [bb(0), bb(1), bb(2), bb(3)],
                                links: b["links"].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        );
    };
    spawn_local(async move { apply(api::window_data().await) });
    api::on("window-data", apply);

    let measure = move || {
        if let Some(el) = panel.get_untracked() {
            panel_size.set((el.client_width() as f64, el.client_height() as f64));
        }
    };
    Effect::new(move |_| {
        if panel.get().is_some() {
            set_timeout(measure, Duration::from_millis(0));
        }
    });
    let _ = window_event_listener(ev::resize, move |_| measure());

    let select_and_copy = move |i: usize| {
        let Some(text) = blocks.with_untracked(|b| b.get(i).map(|b| b.text.clone())) else { return };
        selected.set(Some(i));
        copied.set(Some(i));
        api::fire("ocr", "copy", json!({ "text": text }));
        set_timeout(
            move || {
                if copied.get_untracked() == Some(i) {
                    copied.set(None)
                }
            },
            Duration::from_secs(2),
        );
    };
    let copy_all = move || {
        let all = blocks.with_untracked(|b| b.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n"));
        spawn_local(async move {
            let _ = api::rpc("ocr", "copy", json!({ "text": all })).await;
            api::close_window();
        });
    };
    let _ = window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| {
        if e.key() == "Escape" {
            api::close_window();
        } else if e.meta_key() && e.key().eq_ignore_ascii_case("c") && !has_dom_selection() {
            e.prevent_default();
            copy_all();
        }
    });

    let scale = move || {
        let ((w, h), (pw, ph)) = (size.get(), panel_size.get());
        ((pw - 32.0) / w).min((ph - 32.0) / h).clamp(0.01, 1.0)
    };
    let items = move || blocks.get().into_iter().enumerate().collect::<Vec<_>>();
    let total_chars = move || blocks.with(|b| b.iter().map(|b| b.text.chars().count()).sum::<usize>());

    view! {
        <div class="app-window flex h-screen flex-col overflow-hidden">
            <div class="flex h-7 shrink-0 items-center justify-center text-[13px] font-semibold text-black/70 dark:text-white/70"
                on:mousedown=move |e| drag_unless_control("ocr", &e)>
                {api::t("OCR — Texterkennung", "OCR — Text Recognition")}
            </div>
            <div class="flex min-h-0 flex-1 border-t border-black/10 dark:border-white/10">
                // Bild mit hervorgehobenen Textblöcken
                <div node_ref=panel class="relative flex min-w-[400px] flex-1 items-center justify-center bg-[#1f1f1f]">
                    <div class="relative" style=move || { let s = scale(); let (w, h) = size.get(); format!("width:{}px;height:{}px", w * s, h * s) }>
                        <img src=move || api::asset_url(&path.get()) class="absolute inset-0 size-full select-none" draggable="false" />
                        <For each=items key=|(i, b)| (*i, b.text.clone()) let:item>
                            {
                                let (i, b) = item;
                                let active = move || selected.get() == Some(i) || copied.get() == Some(i);
                                view! {
                                    <div
                                        title=b.text.clone()
                                        class=move || format!(
                                            "absolute cursor-pointer rounded-[3px] border-[1.5px] transition-colors {}",
                                            if active() { "border-green-500/60 bg-green-500/15 shadow-[0_0_6px_rgba(34,197,94,0.3)]" } else { "border-blue-500/40 bg-blue-500/10 hover:bg-blue-500/20" }
                                        )
                                        style=format!("left:{}%;top:{}%;width:{}%;height:{}%", b.bbox[0] * 100.0, b.bbox[1] * 100.0, b.bbox[2] * 100.0, b.bbox[3] * 100.0)
                                        on:click=move |_| select_and_copy(i)
                                    />
                                }
                            }
                        </For>
                    </div>
                </div>
                // Textliste
                <div class="flex w-[300px] shrink-0 flex-col border-l border-black/10 bg-white/60 dark:border-white/10 dark:bg-white/5">
                    <div class="flex items-center justify-between px-4 py-2.5">
                        <span class="text-[13px] font-semibold">{api::t("Erkannter Text", "Recognized Text")}</span>
                        <span class="font-mono text-[11px] text-black/40 dark:text-white/40">
                            {move || format!("{} {} · {} {}", blocks.with(Vec::len), api::t("Blöcke", "regions"), total_chars(), api::t("Zeichen", "chars"))}
                        </span>
                    </div>
                    <div class="min-h-0 flex-1 space-y-1.5 overflow-y-auto border-y border-black/10 p-3 dark:border-white/10">
                        <For each=items key=|(i, b)| (*i, b.text.clone()) let:item>
                            {
                                let (i, b) = item;
                                let active = move || selected.get() == Some(i) || copied.get() == Some(i);
                                let links = b.links.clone();
                                view! {
                                    <div
                                        class=move || format!(
                                            "cursor-pointer rounded-lg border p-2.5 transition-colors {}",
                                            if active() { "border-green-500/25 bg-green-500/[0.06]" } else { "border-transparent bg-black/[0.03] hover:bg-black/[0.06] dark:bg-white/[0.04] dark:hover:bg-white/[0.08]" }
                                        )
                                        on:click=move |_| select_and_copy(i)
                                    >
                                        <div class="flex items-start gap-2">
                                            <div class="min-w-0 flex-1 whitespace-pre-wrap break-words text-[13px]">{b.text.clone()}</div>
                                            <Show when=move || copied.get() == Some(i)>
                                                <span class="shrink-0 font-mono text-[9px] font-medium text-green-500">{api::t("KOPIERT ✓", "COPIED ✓")}</span>
                                            </Show>
                                        </div>
                                        {(!links.is_empty()).then(|| view! {
                                            <div class="mt-1.5 flex flex-wrap gap-1">
                                                {links.into_iter().map(|url| {
                                                    let u = url.clone();
                                                    view! {
                                                        <button
                                                            title=api::t("Link öffnen", "Open link")
                                                            class="inline-flex max-w-full items-center gap-1 truncate rounded bg-blue-500/10 px-1.5 py-0.5 text-[11px] text-blue-600 hover:bg-blue-500/20 dark:text-blue-400 [&_svg]:size-3 [&_svg]:shrink-0"
                                                            on:click=move |e| { e.stop_propagation(); api::fire("ocr", "openLink", json!({ "url": u.clone() })) }
                                                        >
                                                            <Icon d=LINK />
                                                            <span class="truncate">{url}</span>
                                                        </button>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        })}
                                    </div>
                                }
                            }
                        </For>
                    </div>
                    <div class="flex justify-end gap-2 p-3">
                        <TextButton variant="primary" title="⌘C" on_click=copy_all>{api::t("Alles kopieren", "Copy All")}</TextButton>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Hat der Benutzer Text im Fenster markiert? (dann normales ⌘C)
pub fn has_dom_selection() -> bool {
    js_sys::Function::new_no_args("return String(window.getSelection ? window.getSelection() : '')")
        .call0(&wasm_bindgen::JsValue::NULL)
        .ok()
        .and_then(|v| v.as_string())
        .is_some_and(|s| !s.is_empty())
}

/// Gemeinsames Onboarding-Layout (OCR und Übersetzung): Titel, Untertitel, drei Schritte, „Verstanden!“.
#[component]
pub fn OnboardingCard(module: &'static str, title: &'static str, subtitle: &'static str, steps: Vec<(&'static str, &'static str, &'static str)>) -> impl IntoView {
    let _ = window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| match e.key().as_str() {
        "Escape" => api::fire(module, "onboardingCancel", Value::Null),
        "Enter" => api::fire(module, "onboardingDone", Value::Null),
        _ => {}
    });
    let n = steps.len();
    view! {
        <div class="flex h-screen w-screen p-1">
            <div class="flex flex-1 flex-col items-center gap-6 rounded-2xl border border-black/10 bg-[#f5f5f7]/95 px-8 py-7 text-[#1d1d1f] shadow-xl dark:border-white/10 dark:bg-[#1e1e20]/95 dark:text-[#f5f5f7]"
                on:mousedown=move |e| drag_unless_control(module, &e)>
                <div class="text-center">
                    <h1 class="text-[24px] font-bold">{title}</h1>
                    <p class="mt-1 text-[14px] text-black/50 dark:text-white/50">{subtitle}</p>
                </div>
                <div class="flex w-full flex-1 items-stretch gap-3">
                    {steps.into_iter().enumerate().map(|(i, (icon, t, d))| view! {
                        <div class="flex flex-1 flex-col items-center gap-3 text-center">
                            <div class="flex h-[140px] w-full items-center justify-center rounded-xl border border-black/5 bg-white/70 text-blue-500 dark:border-white/10 dark:bg-white/5 [&_svg]:size-12 [&_svg]:stroke-[1.25]">
                                <Icon d=icon />
                            </div>
                            <div class="font-mono text-[10px] font-medium text-black/40 dark:text-white/40">{format!("STEP {:02}", i + 1)}</div>
                            <div class="text-[15px] font-semibold">{t}</div>
                            <div class="text-[11px] leading-snug text-black/55 dark:text-white/55">{d}</div>
                        </div>
                        {(i + 1 < n).then(|| view! { <div class="flex items-center pb-16 text-black/25 dark:text-white/25 [&_svg]:size-4"><Icon d=ARROW_RIGHT /></div> })}
                    }).collect_view()}
                </div>
                <button class="h-8 rounded-lg bg-blue-500 px-6 text-[13px] font-semibold text-white shadow-sm hover:bg-blue-600"
                    on:click=move |_| api::fire(module, "onboardingDone", Value::Null)>
                    {api::t("Verstanden!", "Got it!")}
                </button>
            </div>
        </div>
    }
}

#[component]
fn Onboarding() -> impl IntoView {
    view! {
        <OnboardingCard
            module="ocr"
            title=api::t("So funktioniert die Texterkennung", "How to use Text Recognition")
            subtitle=api::t("Nicht markierbaren Text aus Bildern, Videos und Webseiten kopieren", "Copy non-selectable text from images, videos, and webpages")
            steps=vec![
                (CROSSHAIR, api::t("Bereich wählen", "Select Area"), api::t("Mit dem Fadenkreuz den Bereich mit Text aufziehen", "Drag crosshair to select the region containing text")),
                (SCAN_TEXT, api::t("Scannen & erkennen", "Scan & Recognize"), api::t("Text wird automatisch gefunden und erkannt", "Text is automatically detected and recognized")),
                (CLIPBOARD_CHECK, api::t("Automatisch kopiert", "Auto-Copy"), api::t("Der erkannte Text liegt sofort in der Zwischenablage", "Recognized text is instantly copied to your clipboard")),
            ]
        />
    }
}
