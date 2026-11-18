//! Klick-durchlässige Overlays: Countdown, Auswahl-/Aufnahmerahmen mit Abdunkeln, Klick-Hervorhebung, Tasten-Overlay.
use crate::api;
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};
use std::{cell::Cell, time::Duration};

const CSS: &str = "@keyframes nrz-pop{from{transform:scale(.5);opacity:0}to{transform:scale(1);opacity:1}}\
@keyframes nrz-click{from{transform:scale(1);opacity:1}to{transform:scale(2);opacity:0}}";

/// Countdown 3-2-1 (Capso `CountdownView`: 120 pt Kreis, 72 pt fette Ziffer).
#[component]
pub fn Countdown() -> impl IntoView {
    let n = RwSignal::new(3i64);
    api::on("rec-countdown", move |v| n.set(v["n"].as_i64().unwrap_or(0)));
    view! {
        <style>{CSS}</style>
        <div class="flex h-screen w-screen items-center justify-center">
            {move || {
                let v = n.get();
                view! {
                    <div class="flex size-[120px] items-center justify-center rounded-full bg-black/70 shadow-[0_6px_32px_rgba(0,0,0,0.4)]"
                        style="animation:nrz-pop .25s ease-out">
                        <span class="text-[72px] font-bold leading-none text-white" style="font-family:ui-rounded,-apple-system">{v}</span>
                    </div>
                }
            }}
        </div>
    }
}

/// Auswahlrahmen (vor der Aufnahme, weiß mit Eckgriffen) bzw. roter Aufnahmerahmen (3 pt, außerhalb des Bereichs)
/// mit optionalem Abdunkeln außerhalb (`dimScreenWhileRecording`).
#[component]
pub fn Frame() -> impl IntoView {
    let st = RwSignal::new(Value::Null);
    spawn_local(async move { st.set(api::window_data().await) });
    api::on("rec-frame", move |v| st.set(v));
    let r = move |i: usize| st.with(|s| s["rect"][i].as_f64().unwrap_or(0.0));
    let recording = move || st.with(|s| s["mode"] == "recording");
    let dim = move || st.with(|s| s["dim"].as_bool().unwrap_or(false));
    // Rahmen sitzt komplett außerhalb des Aufnahmebereichs (3 pt Linie + 2 pt Sicherheitsabstand), damit kein Rot ins Video gerät.
    let out = move || if recording() { 5.0 } else { 1.0 };
    let shade = move |style: String| view! { <div class="absolute bg-black/35" style=style></div> };
    view! {
        <div class="relative h-screen w-screen">
            <Show when=dim>
                {move || {
                    let (x, y, w, h, o) = (r(0), r(1), r(2), r(3), out());
                    let (x0, y0, x1, y1) = (x - o, y - o, x + w + o, y + h + o);
                    view! {
                        {shade(format!("left:0;top:0;right:0;height:{y0}px"))}
                        {shade(format!("left:0;top:{y1}px;right:0;bottom:0"))}
                        {shade(format!("left:0;top:{y0}px;width:{x0}px;height:{}px", y1 - y0))}
                        {shade(format!("left:{x1}px;top:{y0}px;right:0;height:{}px", y1 - y0))}
                    }
                }}
            </Show>
            <div class=move || if recording() { "absolute border-[3px] border-red-500" } else { "absolute border border-white shadow-[0_0_0_1px_rgba(0,0,0,0.35)]" }
                style=move || {
                    let o = out();
                    format!("left:{}px;top:{}px;width:{}px;height:{}px", r(0) - o, r(1) - o, r(2) + 2.0 * o, r(3) + 2.0 * o)
                }>
                <Show when=move || !recording()>
                    {["-left-1 -top-1", "-right-1 -top-1", "-left-1 -bottom-1", "-right-1 -bottom-1"].into_iter().map(|c| view! {
                        <span class=format!("absolute size-2 rounded-[2px] border border-black/30 bg-white {c}")></span>
                    }).collect_view()}
                </Show>
            </div>
        </div>
    }
}

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}
fn next_id() -> u64 {
    NEXT_ID.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

/// Klick-Hervorhebung (Capso `ClickHighlightWindow`: 40 pt Kreis, systemBlue 35 %, 0,3 s Skalierung 1→2 + Ausblenden).
#[component]
pub fn Clicks() -> impl IntoView {
    let list = RwSignal::new(Vec::<(u64, f64, f64)>::new());
    api::on("rec-click", move |v| {
        let id = next_id();
        let (x, y) = (v["x"].as_f64().unwrap_or(0.0), v["y"].as_f64().unwrap_or(0.0));
        list.update(|l| l.push((id, x, y)));
        set_timeout(move || list.update(|l| l.retain(|c| c.0 != id)), Duration::from_millis(350));
    });
    view! {
        <style>{CSS}</style>
        <div class="relative h-screen w-screen">
            <For each=move || list.get() key=|c| c.0 let:c>
                <div class="pointer-events-none absolute size-10 rounded-full bg-[rgba(0,122,255,0.35)]"
                    style=format!("left:{}px;top:{}px;animation:nrz-click .3s ease-out forwards", c.1 - 20.0, c.2 - 20.0)></div>
            </For>
        </div>
    }
}

#[derive(Clone)]
struct Bezel {
    id: u64,
    text: RwSignal<String>,
    fading: RwSignal<bool>,
    gen: RwSignal<u64>,
}

/// Neue Blase nach 0,5 s Pause (KeyCastr), Ausblenden nach 2 s (0,2 s), höchstens 6 Blasen.
const KEYSTROKE_DELAY: f64 = 500.0;
const FADE_DELAY: u64 = 2000;
const FADE_MS: u64 = 200;
const MAX_BEZELS: usize = 6;

/// Tasten-Overlay (Capso `KeyPressOverlayWindow`): Blasen stapeln sich von unten links nach oben.
#[component]
pub fn Keys() -> impl IntoView {
    let list = RwSignal::new(Vec::<Bezel>::new());
    let current = RwSignal::new(None::<u64>);
    let last = RwSignal::new(0.0f64);

    let schedule_fade = move |b: Bezel| {
        let g = b.gen.get_untracked() + 1;
        b.gen.set(g);
        b.fading.set(false);
        set_timeout(
            move || {
                if b.gen.get_untracked() != g {
                    return;
                }
                b.fading.set(true);
                set_timeout(
                    move || {
                        if b.gen.get_untracked() == g {
                            list.update(|l| l.retain(|x| x.id != b.id));
                            if current.get_untracked() == Some(b.id) {
                                current.set(None);
                            }
                        }
                    },
                    Duration::from_millis(FADE_MS),
                );
            },
            Duration::from_millis(FADE_DELAY),
        );
    };

    api::on("rec-key", move |v| {
        let text = v["text"].as_str().unwrap_or("").to_owned();
        if text.is_empty() {
            return;
        }
        let now = js_sys::Date::now();
        let command = v["command"].as_bool().unwrap_or(false);
        if command || now - last.get_untracked() > KEYSTROKE_DELAY {
            current.set(None);
        }
        last.set(now);
        let existing = current.get_untracked().and_then(|id| list.with_untracked(|l| l.iter().find(|b| b.id == id).cloned()));
        let b = match existing {
            Some(b) => {
                b.text.update(|t| t.push_str(&text));
                b
            }
            None => {
                let b = Bezel { id: next_id(), text: RwSignal::new(text), fading: RwSignal::new(false), gen: RwSignal::new(0) };
                list.update(|l| {
                    l.push(b.clone());
                    let n = l.len();
                    if n > MAX_BEZELS {
                        l.drain(..n - MAX_BEZELS);
                    }
                });
                current.set(Some(b.id));
                b
            }
        };
        schedule_fade(b);
    });

    // Trefferfläche der Blasen ans Backend (nur dort nimmt das Overlay Mausereignisse an → ziehbar).
    let stack = NodeRef::<leptos::html::Div>::new();
    Effect::new(move |_| {
        // Anzahl und Textlängen verfolgen (Blasen wachsen beim Tippen).
        let n = list.with(|l| l.iter().map(|b| 1 + b.text.with(|t| t.len())).sum::<usize>());
        set_timeout(
            move || {
                let rect = stack.get_untracked().filter(|_| n > 0).map(|el| {
                    let r = el.get_bounding_client_rect();
                    json!([r.left(), r.top(), r.width(), r.height()])
                });
                api::fire("record", "keysHit", json!({ "rect": rect }));
            },
            Duration::from_millis(30),
        );
    });

    view! {
        <div class="flex h-screen w-screen flex-col items-start justify-end overflow-hidden">
            <div node_ref=stack class="flex max-w-full cursor-grab flex-col items-start gap-[10px]"
                on:pointerdown=move |e| {
                    if e.button() == 0 {
                        e.prevent_default();
                        api::fire("record", "keysDrag", Value::Null);
                    }
                }>
            <For each=move || list.get() key=|b| b.id let:b>
                <div class=move || format!("max-w-full truncate rounded-xl bg-black/75 px-[14px] py-[10px] text-[28px] font-semibold leading-none text-white shadow-lg transition-opacity duration-200 {}",
                        if b.fading.get() { "opacity-0" } else { "opacity-100" })>
                    {move || b.text.get()}
                </div>
            </For>
            </div>
        </div>
    }
}
