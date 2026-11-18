//! Pin-Fenster: schwebendes Bild / Text / Farbfeld / Dateiliste. Port von Capso `PinnedScreenshot*`.
use crate::{api, ui::*};
use leptos::{ev, html, prelude::*, task::spawn_local};
use serde_json::{json, Value};
use std::time::Duration;
use wasm_bindgen::JsCast;

pub fn route(r: &str) -> Option<AnyView> {
    match r {
        "pin" => Some(view! { <Pin /> }.into_any()),
        _ => None,
    }
}

/// Ein RPC gleichzeitig; neuere Werte überschreiben wartende (flüssiges Zoomen/Ziehen ohne Rückstau).
#[derive(Clone, Copy)]
struct Throttle {
    busy: StoredValue<bool>,
    next: StoredValue<Option<(&'static str, Value)>>,
}

impl Throttle {
    fn new() -> Self {
        Self { busy: StoredValue::new(false), next: StoredValue::new(None) }
    }
    fn send(self, cmd: &'static str, a: Value, done: impl Fn(Value) + Copy + 'static) {
        self.next.set_value(Some((cmd, a)));
        self.flush(done);
    }
    fn flush(self, done: impl Fn(Value) + Copy + 'static) {
        if self.busy.get_value() {
            return;
        }
        let Some((cmd, a)) = self.next.try_update_value(|n| n.take()).flatten() else { return };
        self.busy.set_value(true);
        spawn_local(async move {
            if let Ok(v) = api::rpc("pin", cmd, a).await {
                done(v);
            }
            self.busy.set_value(false);
            self.flush(done);
        });
    }
}

fn chrome_btn() -> &'static str {
    "flex size-[26px] items-center justify-center rounded-full bg-white/90 text-black shadow-md hover:bg-white [&_svg]:size-3.5"
}

#[component]
fn Pin() -> impl IntoView {
    let data = RwSignal::new(Value::Null);
    let base_w = RwSignal::new(0.0f64);
    let aspect = RwSignal::new(1.0f64);
    let win_w = RwSignal::new(window().inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(0.0));
    let hover = RwSignal::new(false);
    let locked = RwSignal::new(false);
    let lock_hover = RwSignal::new((false, false));
    let opacity = RwSignal::new(1.0f64);
    let percent = RwSignal::new(100.0f64);
    let badge = RwSignal::new(false);
    let badge_gen = StoredValue::new(0u32);
    let measure: NodeRef<html::Div> = NodeRef::new();
    let zoom_acc = StoredValue::new(1.0f64);
    let throttle = Throttle::new();

    let kind = move || data.with(|d| d["kind"].as_str().unwrap_or("").to_owned());
    let is_image = move || matches!(kind().as_str(), "image" | "gif");

    let show_percent = move |v: Value| {
        if let Some(p) = v["percent"].as_f64() {
            percent.set(p);
            badge.set(true);
            let gen = badge_gen.get_value().wrapping_add(1);
            badge_gen.set_value(gen);
            set_timeout(move || if badge_gen.get_value() == gen { badge.set(false) }, Duration::from_millis(900));
        }
    };

    // Text/Dateien: nach dem Rendern Inhaltsgröße messen und das Fenster anpassen.
    let fit = move || {
        request_animation_frame(move || {
            let Some(el) = measure.get_untracked() else { return };
            let (ow, oh) = (el.offset_width() as f64, el.offset_height() as f64);
            let max_h = screen_avail("availHeight", 900.0) * 0.85;
            let (w, h) = (ow.max(100.0), oh.clamp(60.0, max_h));
            base_w.set(w);
            aspect.set(w / h);
            api::fire("pin", "fit", json!({"w": w, "h": h}));
        });
    };

    spawn_local(async move {
        if let Ok(v) = api::rpc("pin", "info", Value::Null).await {
            base_w.set(v["baseW"].as_f64().unwrap_or(320.0));
            aspect.set(v["aspect"].as_f64().unwrap_or(1.0));
            locked.set(v["locked"].as_bool().unwrap_or(false));
            data.set(v["data"].clone());
            if matches!(v["data"]["kind"].as_str(), Some("text")) {
                fit();
            }
        }
    });
    api::on("pin-locked", move |v: Value| {
        locked.set(v.as_bool().unwrap_or(false));
        lock_hover.set((false, false));
    });
    api::on("pin-hover", move |v: Value| lock_hover.set((v["inside"].as_bool().unwrap_or(false), v["button"].as_bool().unwrap_or(false))));
    api::on("pin-opacity", move |v: Value| opacity.set(v.as_f64().unwrap_or(1.0)));

    let _ = window_event_listener(ev::resize, move |_| win_w.set(window().inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(0.0)));

    let zoom = move |factor: f64| {
        zoom_acc.update_value(|z| *z *= factor);
        let f = zoom_acc.get_value();
        zoom_acc.set_value(1.0);
        // Zusammengefasst senden: Faktoren multiplizieren sich, solange ein Aufruf läuft.
        let pending = throttle.next.with_value(|n| n.as_ref().and_then(|(_, a)| a["factor"].as_f64())).unwrap_or(1.0);
        throttle.send("zoom", json!({"factor": f * pending}), show_percent);
    };

    let act = move |cmd: &'static str| api::fire("pin", cmd, Value::Null);

    let _ = window_event_listener(ev::keydown, move |e| {
        let k = e.key().to_lowercase();
        let cmd = e.meta_key();
        let handled = match k.as_str() {
            "escape" => {
                act("close");
                true
            }
            "w" if cmd => {
                act("close");
                true
            }
            "c" if cmd => {
                act("copy");
                true
            }
            "s" if cmd => {
                act("save");
                true
            }
            "e" if cmd => {
                act("annotate");
                true
            }
            "l" if cmd => {
                api::fire("pin", "lock", json!({"locked": true}));
                true
            }
            "=" | "+" if cmd => {
                zoom(1.1);
                true
            }
            "-" if cmd => {
                zoom(1.0 / 1.1);
                true
            }
            "0" if cmd => {
                throttle.send("reset", Value::Null, show_percent);
                true
            }
            _ => false,
        };
        if handled {
            e.prevent_default();
        }
    });

    // Pinch (WebKit-Gesten) zoomt ebenfalls.
    let last_scale = StoredValue::new(1.0f64);
    let _ = window_event_listener_untyped("gesturestart", move |e| {
        e.prevent_default();
        last_scale.set_value(1.0);
    });
    let _ = window_event_listener_untyped("gesturechange", move |e| {
        e.prevent_default();
        let s = js_sys::Reflect::get(&e, &"scale".into()).ok().and_then(|v| v.as_f64()).unwrap_or(1.0);
        let f = s / last_scale.get_value();
        last_scale.set_value(s);
        if (f - 1.0).abs() > 0.002 {
            zoom(f);
        }
    });

    let on_wheel = move |e: ev::WheelEvent| {
        e.prevent_default();
        if locked.get_untracked() {
            return;
        }
        let dy = if e.delta_y().abs() >= e.delta_x().abs() { e.delta_y() } else { e.delta_x() };
        let k = if e.ctrl_key() { 0.01 } else { 0.003 };
        let f = (-dy * k).exp().clamp(0.5, 2.0);
        if (f - 1.0).abs() > 0.001 {
            zoom(f);
        }
    };

    let on_down = move |e: ev::MouseEvent| {
        if e.button() != 0 || locked.get_untracked() {
            return;
        }
        // Knöpfe/Griff haben eigene Handler.
        let on_control = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).and_then(|el| el.closest("button,[data-grip]").ok().flatten()).is_some();
        if !on_control {
            api::fire("pin", "startDrag", Value::Null);
        }
    };

    // Größe per Griff (unten rechts), Seitenverhältnis bleibt.
    let grip_start = StoredValue::new(None::<(f64, f64, f64)>);
    let grip_down = move |e: ev::PointerEvent| {
        e.stop_propagation();
        e.prevent_default();
        if let Some(t) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
            let _ = t.set_pointer_capture(e.pointer_id());
        }
        grip_start.set_value(Some((e.screen_x() as f64, e.screen_y() as f64, win_w.get_untracked())));
    };
    let grip_move = move |e: ev::PointerEvent| {
        let Some((sx, sy, w0)) = grip_start.get_value() else { return };
        let (dx, dy) = (e.screen_x() as f64 - sx, e.screen_y() as f64 - sy);
        throttle.send("resize", json!({"w": w0 + dx.max(dy)}), show_percent);
    };
    let grip_up = move |_: ev::PointerEvent| grip_start.set_value(None);

    let content_opacity = move || {
        let base = opacity.get();
        if locked.get() {
            base * if lock_hover.get().0 { 0.08 } else { 0.72 }
        } else {
            base
        }
    };
    let scale = move || {
        let b = base_w.get();
        if b > 0.0 && win_w.get() > 0.0 { win_w.get() / b } else { 1.0 }
    };
    let chrome = move || hover.get() && !locked.get();

    let body = move || {
        let d = data.get();
        match d["kind"].as_str().unwrap_or("") {
            "image" | "gif" => {
                let src = api::asset_url(d["path"].as_str().unwrap_or(""));
                view! { <img src=src draggable="false" class="pointer-events-none size-full object-fill" /> }.into_any()
            }
            "color" => {
                let hex = d["hex"].as_str().unwrap_or("#000000").to_owned();
                let label = d["text"].as_str().unwrap_or("").to_uppercase();
                let fg = if d["light"].as_bool().unwrap_or(false) { "#000" } else { "#fff" };
                view! {
                    <div class="relative h-[180px] w-[360px]" style:background=hex>
                        <span class="absolute bottom-[26px] left-6 font-mono text-[22px] font-semibold" style:color=fg>{label}</span>
                    </div>
                }
                .into_any()
            }
            "files" => {
                let more = d["more"].as_u64().unwrap_or(0);
                let rows = d["files"].as_array().cloned().unwrap_or_default().into_iter().map(|f| {
                    let icon = f["icon"].as_str().map(api::asset_url);
                    view! {
                        <div class="flex h-[52px] items-center gap-3">
                            {match icon {
                                Some(src) => view! { <img src=src class="size-8 shrink-0" draggable="false" /> }.into_any(),
                                None => view! { <span class="size-8 shrink-0 text-zinc-400 [&_svg]:size-8"><Icon d=FOLDER /></span> }.into_any(),
                            }}
                            <div class="min-w-0">
                                <div class="truncate text-[13px] font-semibold">{f["name"].as_str().unwrap_or("").to_owned()}</div>
                                <div class="truncate text-[11px] text-zinc-500">{f["path"].as_str().unwrap_or("").to_owned()}</div>
                            </div>
                        </div>
                    }
                }).collect_view();
                view! {
                    <div class="w-[420px] bg-white px-4 py-4 text-zinc-900 dark:bg-zinc-900 dark:text-zinc-100">
                        {rows}
                        {(more > 0).then(|| view! { <div class="pt-1 text-[12px] text-zinc-500">{format!("+{more} {}", api::t("weitere", "more"))}</div> })}
                    </div>
                }
                .into_any()
            }
            "text" => {
                let text = d["text"].as_str().unwrap_or("").to_owned();
                let max_w = (screen_avail("availWidth", 1440.0) * 0.72).clamp(320.0, 760.0) + 52.0;
                view! {
                    <div node_ref=measure
                        class="min-w-[320px] min-h-[120px] w-max whitespace-pre-wrap break-words bg-white px-[26px] py-6 font-mono text-[18px] leading-[23px] text-zinc-900 dark:bg-zinc-900 dark:text-zinc-100"
                        style:max-width=format!("{max_w}px")>
                        {text}
                    </div>
                }
                .into_any()
            }
            _ => ().into_any(),
        }
    };

    view! {
        <div class="relative h-screen w-screen overflow-hidden"
            on:mouseenter=move |_| hover.set(true)
            on:mouseleave=move |_| hover.set(false)
            on:mousedown=on_down
            on:wheel=on_wheel
            on:contextmenu=move |e| { e.prevent_default(); if !locked.get_untracked() { api::fire("pin", "contextMenu", Value::Null) } }
        >
            // Inhalt (Bild füllt das Fenster; Text/Farbe/Dateien werden von der Ausgangsgröße skaliert)
            <div class="absolute inset-0 overflow-hidden rounded-lg border border-white/15 transition-opacity duration-150"
                style:opacity=move || content_opacity().to_string()>
                <Show when=is_image fallback=move || view! {
                    <div class="origin-top-left" style:transform=move || format!("scale({})", scale())>{body}</div>
                }>
                    {body}
                </Show>
            </div>

            // Chrome beim Hover
            <div class="pointer-events-none absolute inset-0 transition-opacity duration-150" class:opacity-0=move || !chrome()>
                <button title=api::t("Schließen (Esc)", "Close (Esc)") class=format!("pointer-events-auto absolute left-2.5 top-2.5 {}", chrome_btn())
                    class:hidden=move || !chrome()
                    on:click=move |_| act("close")><Icon d=X /></button>
                <div class="pointer-events-auto absolute bottom-2.5 left-1/2 flex -translate-x-1/2 gap-1.5" class:hidden=move || !chrome()>
                    <button title=api::t("Kopieren (⌘C)", "Copy (⌘C)") class=chrome_btn() on:click=move |_| act("copy")><Icon d=COPY /></button>
                    <Show when=move || kind() != "files">
                        <button title=api::t("Sichern (⌘S)", "Save (⌘S)") class=chrome_btn() on:click=move |_| act("save")><Icon d=DOWNLOAD /></button>
                    </Show>
                    <Show when=move || kind() == "image">
                        <button title=api::t("Annotieren (⌘E)", "Annotate (⌘E)") class=chrome_btn() on:click=move |_| act("annotate")><Icon d=PENCIL /></button>
                    </Show>
                </div>
                <div data-grip="" title=api::t("Größe ändern", "Resize")
                    class="pointer-events-auto absolute bottom-0 right-0 flex size-6 cursor-nwse-resize items-end justify-end p-[6px]"
                    class:hidden=move || !chrome()
                    on:pointerdown=grip_down on:pointermove=grip_move on:pointerup=grip_up>
                    <div class="size-2 rounded-[2px] bg-white/65 shadow"></div>
                </div>
            </div>

            // Schloss oben rechts: beim Hover sichtbar; gesperrt immer sichtbar und als einziges klickbar.
            <button
                title=move || if locked.get() { api::t("Entsperren", "Unlock") } else { api::t("Sperren (Klicks durchlassen)", "Lock (Click Through)") }
                class=format!("absolute right-2.5 top-2.5 transition-opacity {}", chrome_btn())
                style:opacity=move || if locked.get() { if lock_hover.get().1 { "1" } else { "0.75" } } else if hover.get() { "1" } else { "0" }
                on:click=move |_| api::fire("pin", "lock", json!({"locked": !locked.get_untracked()}))>
                <Icon d=Signal::derive(move || if locked.get() { LOCK } else { UNLOCK }) />
            </button>

            // Zoom-Anzeige
            <div class="pointer-events-none absolute left-1/2 top-3.5 -translate-x-1/2 rounded-[14px] bg-black/55 px-3 py-1 text-[14px] font-bold text-white transition-opacity duration-200"
                class:opacity-0=move || !badge.get()>
                {move || format!("{}%", percent.get())}
            </div>
        </div>
    }
}

/// `window.screen.availWidth/availHeight` (ohne zusätzliches web-sys-Feature).
fn screen_avail(key: &str, default: f64) -> f64 {
    js_sys::Reflect::get(&window(), &"screen".into())
        .and_then(|s| js_sys::Reflect::get(&s, &key.into()))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(default)
}
