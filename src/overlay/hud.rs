//! Kleine Capture-HUDs: Selbstauslöser-Countdown, Scroll-Aufnahme-Steuerung, Auswahlrahmen.
use crate::api::{self, t};
use crate::ui::{self, Icon};
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

/// Startdaten laden und bei erneutem Öffnen (`window-data`) neu setzen.
fn on_data(f: impl Fn(Value) + Clone + 'static) {
    let f2 = f.clone();
    spawn_local(async move { f2(api::window_data().await) });
    api::on("window-data", move |v| f(v));
}

// ───────────────────────────── Selbstauslöser ─────────────────────────────

/// Countdown-HUD (Capso `SelfTimerHUD`): abgerundetes Quadrat mit Fortschrittsring und Ziffer,
/// beim Überfahren ein ×-Knopf (Klick bricht ab), verschiebbar (Position wird gemerkt), Esc bricht ab.
#[component]
pub fn SelfTimer() -> impl IntoView {
    let total = RwSignal::new(5u32);
    let remaining = RwSignal::new(5u32);
    let hover = RwSignal::new(false);
    // Ziehen: (Bildschirm-Startpunkt, Fensterursprung, bewegt?)
    let drag = StoredValue::new(None::<((f64, f64), (f64, f64), bool)>);

    on_data(move |v| {
        let tt = v["total"].as_u64().unwrap_or(5) as u32;
        total.set(tt);
        remaining.set(v["remaining"].as_u64().map(|r| r as u32).unwrap_or(tt));
    });
    api::on("timer-tick", move |v| {
        if let Some(t) = v["total"].as_u64() {
            total.set(t as u32);
        }
        remaining.set(v["remaining"].as_u64().unwrap_or(0) as u32);
    });
    let cancel = || api::fire("capture", "timerCancel", Value::Null);
    let key = window_event_listener(leptos::ev::keydown, move |e| {
        if e.key() == "Escape" {
            cancel();
        }
    });
    on_cleanup(move || key.remove());

    let up = window_event_listener(leptos::ev::mouseup, move |e| {
        if let Some((s, o, moved)) = drag.get_value() {
            drag.set_value(None);
            if moved {
                let (x, y) = (o.0 + e.screen_x() as f64 - s.0, o.1 + e.screen_y() as f64 - s.1);
                api::fire("capture", "hudMove", json!({ "x": x, "y": y, "save": true }));
            } else {
                cancel();
            }
        }
    });
    let mv = window_event_listener(leptos::ev::mousemove, move |e| {
        if let Some((s, o, moved)) = drag.get_value() {
            let (dx, dy) = (e.screen_x() as f64 - s.0, e.screen_y() as f64 - s.1);
            if moved || dx.abs() + dy.abs() > 3.0 {
                drag.set_value(Some((s, o, true)));
                api::fire("capture", "hudMove", json!({ "x": o.0 + dx, "y": o.1 + dy }));
            }
        }
    });
    on_cleanup(move || {
        up.remove();
        mv.remove();
    });

    const R: f64 = 38.5;
    let circ = 2.0 * std::f64::consts::PI * R;
    let progress = move || {
        let t = total.get().max(1) as f64;
        // Ring läuft gleichmäßig: Zielwert = Stand am Ende der laufenden Sekunde.
        ((remaining.get() as f64 - 1.0).max(0.0) / t).clamp(0.0, 1.0)
    };

    view! {
        <div
            class="fixed inset-0 flex items-center justify-center rounded-[22px] border border-black/10 bg-white/80 text-zinc-900 shadow-xl backdrop-blur-xl dark:border-white/10 dark:bg-zinc-800/80 dark:text-white"
            on:mouseenter=move |_| hover.set(true)
            on:mouseleave=move |_| hover.set(false)
            on:mousedown=move |e| {
                let w = window();
                let o = (w.screen_x().ok().and_then(|v| v.as_f64()).unwrap_or(0.0), w.screen_y().ok().and_then(|v| v.as_f64()).unwrap_or(0.0));
                drag.set_value(Some(((e.screen_x() as f64, e.screen_y() as f64), o, false)));
            }
            title=move || t("Selbstauslöser – klicken zum Abbrechen", "Self-timer – click to cancel")
        >
            <svg width="80" height="80" viewBox="0 0 80 80" class="absolute -rotate-90">
                <circle cx="40" cy="40" r=R fill="none" stroke="currentColor" stroke-opacity="0.12" stroke-width="3" />
                <circle cx="40" cy="40" r=R fill="none" stroke="currentColor" stroke-opacity="0.85" stroke-width="3"
                    stroke-linecap="round" stroke-dasharray=circ
                    style="transition: stroke-dashoffset 1s linear"
                    stroke-dashoffset=move || circ * (1.0 - progress()) />
            </svg>
            <Show
                when=move || hover.get()
                fallback=move || view! { <span class="text-[44px] font-semibold tabular-nums leading-none">{move || remaining.get()}</span> }
            >
                <span class="opacity-60 [&_svg]:size-7"><Icon d=ui::X /></span>
            </Show>
        </div>
    }
}

// ───────────────────────────── Scroll-Aufnahme ─────────────────────────────

/// Steuerung (Abbrechen / Start bzw. Fertig, automatisches Scrollen) und Live-Vorschau (Capso `ScrollCaptureHUD`).
#[component]
pub fn ScrollHud() -> impl IntoView {
    let capturing = RwSignal::new(false);
    let status = RwSignal::new(String::new());
    let preview = RwSignal::new(None::<String>);
    let auto = RwSignal::new(false);

    on_data(move |v| {
        capturing.set(false);
        preview.set(None);
        auto.set(false);
        status.set(v["status"].as_str().unwrap_or("").to_owned());
    });
    api::on("scroll-progress", move |v| {
        if let Some(s) = v["status"].as_str() {
            status.set(s.to_owned());
        }
        if v["capturing"].as_bool() == Some(true) {
            capturing.set(true);
        }
        if let Some(p) = v["preview"].as_str() {
            preview.set(Some(api::asset_url(p)));
        }
    });
    let key = window_event_listener(leptos::ev::keydown, move |e| match e.key().as_str() {
        "Escape" => api::fire("scroll", "cancel", Value::Null),
        // Capso: Return startet nur (während der Aufnahme ohne Wirkung).
        "Enter" if !capturing.get_untracked() => api::fire("scroll", "start", Value::Null),
        _ => {}
    });
    on_cleanup(move || key.remove());

    let btn = "inline-flex items-center gap-1.5 rounded-lg px-4 py-2 text-[13px] font-medium text-white [&_svg]:size-3";
    view! {
        <div class="fixed inset-0 flex flex-col gap-2 p-1 text-white">
            <div class="flex items-center justify-center gap-3 rounded-xl bg-zinc-900/85 p-2 shadow-lg backdrop-blur-xl">
                <button class=format!("{btn} bg-white/15 hover:bg-white/25") on:click=move |_| api::fire("scroll", "cancel", Value::Null)>
                    <Icon d=ui::X />{move || t("Abbrechen", "Cancel")}
                </button>
                <Show
                    when=move || capturing.get()
                    fallback=move || view! {
                        <button class=format!("{btn} bg-green-600 hover:bg-green-500") on:click=move |_| {
                            capturing.set(true);
                            api::fire("scroll", "start", Value::Null)
                        }>
                            <Icon d=ui::PLAY />{move || t("Aufnahme starten", "Start Capture")}
                        </button>
                    }
                >
                    <button class=format!("{btn} bg-blue-500 hover:bg-blue-400") on:click=move |_| api::fire("scroll", "done", Value::Null)>
                        <Icon d=ui::CHECK />{move || t("Fertig", "Done")}
                    </button>
                </Show>
            </div>
            <label class="flex items-center justify-center gap-2 self-center rounded-full bg-zinc-900/85 px-3 py-1 text-[11px] backdrop-blur-xl">
                <input type="checkbox" prop:checked=move || auto.get() on:change=move |e| {
                    let on = event_target_checked(&e);
                    auto.set(on);
                    api::fire("scroll", "auto", json!({ "on": on }));
                } />
                {move || t("Automatisch scrollen", "Auto-scroll")}
            </label>
            <div class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[10px] bg-zinc-900/80 shadow-lg backdrop-blur-xl">
                <div class="relative min-h-0 flex-1 overflow-hidden">
                    {move || match preview.get() {
                        Some(src) => view! { <img src=src class="absolute left-0 top-0 w-full object-contain object-top" style="max-height:100%" /> }.into_any(),
                        None => view! {
                            <div class="flex size-full flex-col items-center justify-center gap-1.5 p-4 text-center text-[11px] text-white/45">
                                <span class="[&_svg]:size-5"><Icon d=ui::SCROLL /></span>
                                <span>{move || status.get()}</span>
                            </div>
                        }.into_any(),
                    }}
                </div>
                <div class="truncate bg-black/30 px-2 py-1 font-mono text-[10px] font-medium text-white/60">{move || status.get()}</div>
            </div>
        </div>
    }
}

// ───────────────────────────── Rahmen ─────────────────────────────

/// Rahmen um den Aufnahmebereich (klick-durchlässig). "scroll" = laufende Strichlinie, sonst durchgezogen.
#[component]
pub fn Border() -> impl IntoView {
    let style = RwSignal::new(String::from("timer"));
    on_data(move |v| style.set(v["style"].as_str().unwrap_or("timer").to_owned()));
    view! {
        <style>{"@keyframes nrz-ants{to{stroke-dashoffset:-16}}"}</style>
        <svg class="fixed inset-0 size-full" style="overflow:visible">
            <rect x="3" y="3" style="width:calc(100% - 6px);height:calc(100% - 6px)" fill="none" stroke="black" stroke-opacity="0.35" stroke-width="3" />
            <rect x="3" y="3" style=move || {
                let anim = if style.get() == "scroll" { "animation:nrz-ants .6s linear infinite;" } else { "" };
                format!("width:calc(100% - 6px);height:calc(100% - 6px);{anim}")
            }
                fill="none" stroke="white" stroke-width="1.5"
                stroke-dasharray=move || if style.get() == "scroll" { "8 8" } else { "none" } />
        </svg>
    }
}
