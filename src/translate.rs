//! Frontend des Moduls `translate`: schwebende Ergebniskarte (Capso `TranslationResultView` + Sprachwähler),
//! Eingabefenster (`TypedTranslationInputWindow`) und Onboarding (`TranslationOnboardingView`).
use crate::{
    api,
    ocr::{drag_unless_control, has_dom_selection, OnboardingCard, ARROW_RIGHT, CROSSHAIR},
    ui::*,
};
use leptos::{ev, prelude::*, task::spawn_local};
use serde_json::{json, Value};
use std::time::Duration;

pub fn route(r: &str) -> Option<AnyView> {
    match r {
        "translate-card" => Some(view! { <ResultCard /> }.into_any()),
        "translate-input" => Some(view! { <TypedInput /> }.into_any()),
        "translate-onboarding" => Some(view! { <Onboarding /> }.into_any()),
        _ => None,
    }
}

const CHEVRON_RIGHT: &str = r#"<path d="m9 18 6-6-6-6"/>"#;
const ALERT: &str = r#"<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>"#;
const BUBBLE: &str = r#"<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/><path d="M8 12h.01"/><path d="M12 12h.01"/><path d="M16 12h.01"/>"#;
const PIN_OFF: &str = r#"<path d="M12 17v5"/><path d="M15 9.34V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H7.89"/><path d="m2 2 20 20"/><path d="M9 9v1.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h11"/>"#;

#[derive(Clone, PartialEq)]
enum Phase {
    Recognizing,
    Loading,
    Streaming(String),
    Done(String, Value),
    Failed(String),
}

fn spinner(size: &'static str) -> impl IntoView {
    view! { <span class=format!("inline-block {size} animate-spin rounded-full border-2 border-black/15 border-t-black/60 dark:border-white/20 dark:border-t-white/70") /> }
}

/// Absätze, Aufzählungen und nummerierte Listen (vom Backend zerlegt) darstellen.
fn render_blocks(blocks: Value, body: &'static str, marker: &'static str) -> impl IntoView {
    let items = blocks.as_array().cloned().unwrap_or_default();
    view! {
        <div class="flex flex-col gap-2.5">
            {items.into_iter().map(|b| {
                match b["kind"].as_str().unwrap_or("") {
                    "bulletList" => {
                        let rows = b["items"].as_array().cloned().unwrap_or_default();
                        view! {
                            <div class="flex flex-col gap-1.5">
                                {rows.into_iter().map(|it| view! {
                                    <div class="flex gap-2.5">
                                        <span class=format!("w-2.5 shrink-0 text-center {body} {marker}")>"•"</span>
                                        <span class=format!("select-text min-w-0 flex-1 whitespace-pre-wrap break-words leading-relaxed {body}")>{it.as_str().unwrap_or("").to_owned()}</span>
                                    </div>
                                }).collect_view()}
                            </div>
                        }.into_any()
                    }
                    "numberedList" => {
                        let rows = b["items"].as_array().cloned().unwrap_or_default();
                        view! {
                            <div class="flex flex-col gap-1.5">
                                {rows.into_iter().map(|it| view! {
                                    <div class="flex items-baseline gap-2.5">
                                        <span class=format!("w-[22px] shrink-0 text-right text-[13px] font-semibold {marker}")>{format!("{}.", it[0].as_u64().unwrap_or(0))}</span>
                                        <span class=format!("select-text min-w-0 flex-1 whitespace-pre-wrap break-words leading-relaxed {body}")>{it[1].as_str().unwrap_or("").to_owned()}</span>
                                    </div>
                                }).collect_view()}
                            </div>
                        }.into_any()
                    }
                    _ => view! {
                        <p class=format!("select-text whitespace-pre-wrap break-words leading-relaxed {body}")>{b["items"].as_str().unwrap_or("").to_owned()}</p>
                    }.into_any(),
                }
            }).collect_view()}
        </div>
    }
}

fn section_label(text: &'static str) -> impl IntoView {
    view! { <div class="px-4 pb-1.5 pt-2.5 text-[10px] font-semibold tracking-[0.1em] text-black/35 dark:text-white/35">{text}</div> }
}

#[component]
fn ResultCard() -> impl IntoView {
    let phase = RwSignal::new(Phase::Recognizing);
    let text = RwSignal::new(String::new());
    let original_blocks = RwSignal::new(Value::Null);
    let target = RwSignal::new(String::new());
    let target_name = RwSignal::new(String::new());
    let detected = RwSignal::new(None::<String>);
    let provider = RwSignal::new(String::new());
    let auto_copy = RwSignal::new(true);
    let show_original = RwSignal::new(true);
    let auto_dismiss = RwSignal::new(String::from("manual"));
    let dismiss_delay = RwSignal::new(10.0f64);
    let pinned = RwSignal::new(false);
    let expanded = RwSignal::new(false);
    let manual_copied = RwSignal::new(false);
    let picker = RwSignal::new(false);
    let languages = RwSignal::new(Vec::<(String, String)>::new());
    let req = StoredValue::new(0u64);
    // Auto-Schließen: Token invalidiert alte Timer; `click_outside` = beim Fokusverlust schließen.
    let dismiss_token = StoredValue::new(0u64);
    let click_outside = StoredValue::new(false);

    let close = move || api::fire("translate", "close", Value::Null);

    let disarm = move || {
        dismiss_token.update_value(|t| *t += 1);
        click_outside.set_value(false);
    };
    let arm = move || {
        disarm();
        if pinned.get_untracked() || !matches!(phase.get_untracked(), Phase::Done(..)) {
            return;
        }
        match auto_dismiss.get_untracked().as_str() {
            "afterDelay" => {
                let token = dismiss_token.get_value();
                set_timeout(
                    move || {
                        if dismiss_token.get_value() == token && !pinned.get_untracked() {
                            close();
                        }
                    },
                    Duration::from_secs_f64(dismiss_delay.get_untracked().max(1.0)),
                );
            }
            "clickOutside" => click_outside.set_value(true),
            _ => {}
        }
    };

    let start = move || {
        let id = req.get_value() + 1;
        req.set_value(id);
        disarm();
        phase.set(Phase::Loading);
        expanded.set(false);
        manual_copied.set(false);
        picker.set(false);
        let (t, tg) = (text.get_untracked(), target.get_untracked());
        spawn_local(async move {
            let res = api::rpc("translate", "run", json!({ "text": t, "target": tg, "req": id })).await;
            if req.get_value() != id {
                return;
            }
            match res {
                Ok(v) => {
                    if let Some(d) = v["detected"].as_str() {
                        detected.set(Some(d.to_owned()));
                    }
                    phase.set(Phase::Done(v["text"].as_str().unwrap_or("").to_owned(), v["blocks"].clone()));
                    arm();
                }
                Err(e) if e.is_empty() => {}
                Err(e) => phase.set(Phase::Failed(e)),
            }
        });
    };

    let apply = move |v: Value| {
        if v.is_null() {
            return;
        }
        if v["phase"] == "recognizing" {
            req.update_value(|r| *r += 1);
            disarm();
            detected.set(None);
            phase.set(Phase::Recognizing);
            return;
        }
        text.set(v["text"].as_str().unwrap_or("").to_owned());
        original_blocks.set(v["originalBlocks"].clone());
        target.set(v["target"].as_str().unwrap_or("en").to_owned());
        target_name.set(v["targetName"].as_str().unwrap_or("").to_owned());
        provider.set(v["provider"].as_str().unwrap_or("").to_owned());
        auto_copy.set(v["autoCopy"].as_bool().unwrap_or(true));
        show_original.set(v["showOriginal"].as_bool().unwrap_or(true));
        auto_dismiss.set(v["autoDismiss"].as_str().unwrap_or("manual").to_owned());
        dismiss_delay.set(v["autoDismissDelay"].as_f64().unwrap_or(10.0));
        detected.set(None);
        start();
    };
    spawn_local(async move { apply(api::window_data().await) });
    api::on("window-data", apply);
    api::on("translate-update", move |v: Value| {
        if v["req"].as_u64() != Some(req.get_value()) || matches!(phase.get_untracked(), Phase::Done(..) | Phase::Failed(_)) {
            return;
        }
        if let Some(d) = v["detected"].as_str() {
            detected.set(Some(d.to_uppercase()));
        }
        phase.set(Phase::Streaming(v["text"].as_str().unwrap_or("").to_owned()));
    });
    spawn_local(async move {
        if let Ok(v) = api::rpc("translate", "languages", Value::Null).await {
            languages.set(
                v.as_array()
                    .map(|a| a.iter().map(|l| (l["code"].as_str().unwrap_or("").to_owned(), l["name"].as_str().unwrap_or("").to_owned())).collect())
                    .unwrap_or_default(),
            );
        }
    });

    let current_translation = move || match phase.get_untracked() {
        Phase::Streaming(t) | Phase::Done(t, _) => Some(t),
        _ => None,
    };
    let copy_text = move |t: String| {
        api::fire("translate", "copy", json!({ "text": t }));
        manual_copied.set(true);
        set_timeout(move || manual_copied.set(false), Duration::from_millis(2500));
    };
    let copy_current = move || {
        if let Some(t) = current_translation() {
            copy_text(t);
        }
    };
    let toggle_pin = move || {
        let p = !pinned.get_untracked();
        pinned.set(p);
        api::fire("translate", "pin", json!({ "pinned": p }));
        if p {
            disarm();
        } else {
            arm();
        }
    };

    let _ = window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| {
        if e.key() == "Escape" {
            if picker.get_untracked() {
                picker.set(false);
            } else {
                close();
            }
        } else if e.meta_key() && e.key().eq_ignore_ascii_case("c") && !has_dom_selection() {
            e.prevent_default();
            copy_current();
        }
    });
    let _ = window_event_listener(ev::blur, move |_| {
        if click_outside.get_value() && !pinned.get_untracked() {
            close();
        }
    });

    let lang_pair = move || format!("{} → {}", detected.get().unwrap_or_else(|| "—".into()), target_name.get());
    let copied_visible = move || manual_copied.get() || (auto_copy.get() && matches!(phase.get(), Phase::Done(..)));

    let header = move || {
        let recognizing = phase.get() == Phase::Recognizing;
        view! {
            <div class="relative flex shrink-0 items-center gap-2.5 border-b border-black/[0.06] px-3 py-2.5 dark:border-white/[0.08]"
                on:mousedown=move |e| drag_unless_control("translate", &e)>
                <button title=api::t("Schließen", "Close") class="size-[11px] rounded-full border-[0.5px] border-black/20 bg-red-500/90 hover:bg-red-500"
                    on:click=move |_| close() />
                <div class="flex-1" />
                {(!recognizing).then(|| view! {
                    <button class="rounded px-1.5 py-0.5 text-[11px] text-black/55 hover:bg-black/5 dark:text-white/55 dark:hover:bg-white/10"
                        title=api::t("Zielsprache ändern", "Change target language")
                        on:click=move |_| picker.update(|p| *p = !*p)>
                        {lang_pair}
                    </button>
                })}
                <div class="flex-1" />
                {(!recognizing).then(|| view! {
                    <div class="flex items-center gap-2.5 [&_button]:text-black/70 dark:[&_button]:text-white/70 [&_svg]:size-[15px]">
                        <button title=move || if pinned.get() { api::t("Lösen", "Unpin") } else { api::t("Anheften", "Pin to screen") }
                            class=move || if pinned.get() { "text-blue-500!" } else { "hover:text-black! dark:hover:text-white!" }
                            on:click=move |_| toggle_pin()>
                            <Icon d=Signal::derive(move || if pinned.get() { PIN } else { PIN_OFF }) />
                        </button>
                        <button title=api::t("Übersetzung kopieren (⌘C)", "Copy translation (⌘C)") class="hover:text-black! dark:hover:text-white!"
                            on:click=move |_| copy_current()>
                            <Icon d=COPY />
                        </button>
                    </div>
                })}
            </div>
        }
    };

    let body = move || match phase.get() {
        Phase::Recognizing => view! {
            <div class="flex flex-1 items-center justify-center gap-3 pb-6 text-[13px] text-black/55 dark:text-white/55">
                {spinner("size-4")}{api::t("Text wird erkannt…", "Recognizing text…")}
            </div>
        }
        .into_any(),
        Phase::Loading => view! {
            <div class="flex flex-col items-center gap-3 py-9 text-[13px] text-black/55 dark:text-white/55">
                {spinner("size-4")}{api::t("Übersetze…", "Translating…")}
            </div>
        }
        .into_any(),
        Phase::Streaming(t) => view! {
            <div>
                {section_label(api::t("ÜBERSETZUNG", "TRANSLATION"))}
                <p class="select-text whitespace-pre-wrap break-words px-4 pb-3.5 text-[15px] font-medium leading-relaxed">{t}</p>
                <div class="flex items-center gap-1.5 px-4 pb-3 text-[10px] text-black/35 dark:text-white/35">
                    {spinner("size-2.5")}{api::t("Übersetze…", "Translating…")}
                </div>
            </div>
        }
        .into_any(),
        Phase::Done(_, blocks) => view! {
            <div>
                {section_label(api::t("ÜBERSETZUNG", "TRANSLATION"))}
                <div class="px-4 pb-3.5">{render_blocks(blocks, "text-[15px] font-medium", "text-black/35 dark:text-white/35")}</div>
                <Show when=move || show_original.get()>
                    <div class="mx-3 border-t border-black/[0.08] dark:border-white/10" />
                    <div class="flex items-center gap-2 px-4 py-2.5">
                        <button class="flex flex-1 items-center gap-2 text-left" on:click=move |_| expanded.update(|e| *e = !*e)>
                            <span class="text-[10px] font-semibold tracking-[0.1em] text-black/35 dark:text-white/35">{api::t("ORIGINAL", "ORIGINAL")}</span>
                            <span class="flex-1" />
                            <span class=move || format!("text-black/35 transition-transform dark:text-white/35 [&_svg]:size-3 {}", if expanded.get() { "rotate-90" } else { "" })>
                                <Icon d=CHEVRON_RIGHT />
                            </span>
                        </button>
                        <button title=api::t("Original kopieren", "Copy original") class="text-black/35 hover:text-black/70 dark:text-white/35 dark:hover:text-white/70 [&_svg]:size-3.5"
                            on:click=move |_| copy_text(text.get_untracked())>
                            <Icon d=COPY />
                        </button>
                    </div>
                    <Show when=move || expanded.get()>
                        <div class="px-4 pb-3.5">{move || render_blocks(original_blocks.get(), "text-[13px] text-black/60 dark:text-white/60", "text-black/35 dark:text-white/35")}</div>
                    </Show>
                </Show>
            </div>
        }
        .into_any(),
        Phase::Failed(msg) => view! {
            <div class="flex flex-col gap-3.5 px-4 py-5">
                <div class="flex items-start gap-2.5">
                    <span class="mt-0.5 shrink-0 text-orange-500 [&_svg]:size-4"><Icon d=ALERT /></span>
                    <span class="select-text flex-1 text-[13px]">{msg}</span>
                </div>
                <div class="flex gap-2">
                    <TextButton variant="primary" on_click=move || start()>{api::t("Erneut versuchen", "Retry")}</TextButton>
                    <TextButton on_click=move || api::fire("translate", "openSettings", Value::Null)>{api::t("Einstellungen öffnen", "Open Settings")}</TextButton>
                </div>
            </div>
        }
        .into_any(),
    };

    let footer = move || {
        (phase.get() != Phase::Recognizing).then(|| {
            view! {
                <div class="flex shrink-0 items-center gap-1.5 border-t border-black/[0.06] bg-black/[0.02] px-3 py-2 text-[10px] text-black/35 dark:border-white/[0.08] dark:text-white/35">
                    <span>{move || provider.get()}</span>
                    <Show when=copied_visible>
                        <span class="size-1 rounded-full bg-green-500" />
                        <span>{api::t("Kopiert", "Copied")}</span>
                    </Show>
                    <span class="flex-1" />
                    <span class="font-mono">{move || if auto_copy.get() { "Esc" } else { "⌘C  Esc" }}</span>
                </div>
            }
        })
    };

    let pick = move |code: String, name: String| {
        picker.set(false);
        target.set(code);
        target_name.set(name);
        start();
    };

    view! {
        <div class="h-screen w-screen p-px">
            <div class="relative flex size-full flex-col overflow-hidden rounded-[14px] border-[0.5px] border-black/10 bg-[#f6f6f8]/95 text-[#1d1d1f] backdrop-blur-xl dark:border-white/10 dark:bg-[#232326]/95 dark:text-[#f5f5f7]">
                {header}
                <div class="min-h-0 flex-1 overflow-y-auto">{body}</div>
                {footer}
                // Sprachwähler (Capso `TranslationLanguagePickerPopover`)
                <Show when=move || picker.get()>
                    <div class="absolute inset-0 z-10" on:click=move |_| picker.set(false) />
                    <div class="absolute left-1/2 top-10 z-20 flex max-h-[min(320px,calc(100vh-56px))] w-[260px] -translate-x-1/2 flex-col overflow-hidden rounded-xl border border-black/10 bg-white shadow-xl dark:border-white/10 dark:bg-[#2c2c2f]">
                        <div class="px-3 pb-1 pt-2.5 text-[10px] font-semibold tracking-[0.1em] text-black/40 dark:text-white/40">{api::t("ÜBERSETZEN NACH", "TRANSLATE TO")}</div>
                        <div class="min-h-0 flex-1 overflow-y-auto px-1.5">
                            <For each=move || languages.get() key=|l| l.0.clone() let:lang>
                                {
                                    let (code, name) = lang;
                                    let (c2, n2, c3) = (code.clone(), name.clone(), code.clone());
                                    view! {
                                        <button class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left hover:bg-blue-500/10"
                                            on:click=move |_| pick(c2.clone(), n2.clone())>
                                            <span class=move || format!("flex-1 text-[13px] {}", if target.get() == c3 { "font-medium" } else { "" })>{name.clone()}</span>
                                            <span class="font-mono text-[10px] text-black/40 dark:text-white/40">{code.clone()}</span>
                                            <span class="w-3 text-blue-500 [&_svg]:size-3">
                                                {let c4 = code.clone(); move || (target.get() == c4).then(|| view! { <Icon d=CHECK /> })}
                                            </span>
                                        </button>
                                    }
                                }
                            </For>
                        </div>
                        <div class="border-t border-black/[0.06] px-3 py-2 text-[10px] text-black/40 dark:border-white/[0.08] dark:text-white/40">
                            {api::t("Standard in den Einstellungen ändern", "Change default in Preferences")}
                        </div>
                    </div>
                </Show>
            </div>
        </div>
    }
}

#[component]
fn TypedInput() -> impl IntoView {
    let value = RwSignal::new(String::new());
    let area = NodeRef::<leptos::html::Textarea>::new();
    let can_submit = move || !value.get().trim().is_empty();
    let submit = move || {
        let t = value.get_untracked();
        if t.trim().is_empty() {
            return;
        }
        api::fire("translate", "submitTyped", json!({ "text": t }));
    };
    Effect::new(move |_| {
        if let Some(a) = area.get() {
            set_timeout(move || drop(a.focus()), Duration::from_millis(50));
        }
    });
    api::on("window-data", move |_| {
        value.set(String::new());
        if let Some(a) = area.get_untracked() {
            let _ = a.focus();
        }
    });
    let _ = window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| {
        if e.key() == "Escape" {
            api::close_window();
        } else if e.key() == "Enter" && e.meta_key() {
            e.prevent_default();
            submit();
        }
    });
    view! {
        <div class="app-window flex h-screen flex-col gap-3 px-5 pb-4 pt-8" on:mousedown=move |e| drag_unless_control("translate", &e)>
            <div class="flex items-center gap-2">
                <span class="text-blue-500 [&_svg]:size-[18px]"><Icon d=BUBBLE /></span>
                <span class="text-[18px] font-semibold">{api::t("Text übersetzen", "Translate Text")}</span>
            </div>
            <div class="flex min-h-0 flex-1 flex-col gap-1.5">
                <span class="text-[11px] font-semibold text-black/50 dark:text-white/50">{api::t("Original", "Original")}</span>
                <textarea
                    node_ref=area
                    class="min-h-0 flex-1 resize-none rounded-lg border border-black/10 bg-white/80 p-2.5 text-[14px] outline-none focus:ring-2 focus:ring-blue-500/40 dark:border-white/10 dark:bg-white/10"
                    placeholder=api::t("Text zum Übersetzen eingeben oder einfügen", "Type or paste text to translate")
                    prop:value=move || value.get()
                    on:input=move |e| value.set(event_target_value(&e))
                />
            </div>
            <div class="flex items-center justify-end gap-2">
                <TextButton on_click=move || api::close_window()>{api::t("Abbrechen", "Cancel")}</TextButton>
                <TextButton variant="primary" title="⌘↩" disabled=Signal::derive(move || !can_submit()) on_click=submit>
                    <Icon d=ARROW_RIGHT />{api::t("Übersetzen", "Translate")}
                </TextButton>
            </div>
        </div>
    }
}

#[component]
fn Onboarding() -> impl IntoView {
    view! {
        <OnboardingCard
            module="translate"
            title=api::t("So funktioniert die Übersetzung", "How to use Translate")
            subtitle=api::t("Text aus jedem Screenshot übersetzen, ohne nrzCapture zu verlassen", "Translate text from any screenshot without leaving nrzCapture")
            steps=vec![
                (CROSSHAIR, api::t("Aufnehmen & übersetzen", "Capture & Translate"), api::t("Bereich mit Text wählen – oder markierten bzw. eingegebenen Text übersetzen", "Select an area with text – or translate selected or typed text")),
                (LANGUAGES, api::t("Dein Anbieter", "Your Provider"), api::t("Übersetzt mit dem in den Einstellungen gewählten Dienst, live gestreamt", "Translated by the service chosen in Settings, streamed live")),
                (PIN, api::t("Lesen oder anheften", "Read or Pin"), api::t("Ergebnis kopieren, Zielsprache wechseln oder die Karte anheften", "Copy the result, switch the target language or pin the card")),
            ]
        />
    }
}
