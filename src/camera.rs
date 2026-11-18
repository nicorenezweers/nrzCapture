//! Kamera-Bild-im-Bild (Frontend, Route `camera`). Port von Capso `CameraPiPView`.
//! Webcam per `getUserMedia` (Gerät aus `cameraDeviceId` = WebView-`MediaDeviceInfo.deviceId`, sonst Standard),
//! Form (Kreis/Quadrat/Hoch/Quer) per CSS, Spiegeln, Fade-on-Hover (Deckkraft kommt vom Backend, Event `camera-hover`).
//! Bedienung wie Capso: Ziehen = Fenster verschieben (Einrasten an Ecken im Backend), Klick = Präsentationsmodus,
//! Griff unten rechts (20×20) = Größe ändern, Rechtsklick = Kontextmenü.
use crate::api;
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};
use std::cell::RefCell;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

pub fn route(r: &str) -> Option<AnyView> {
    (r == "camera").then(|| view! { <CameraPip /> }.into_any())
}

thread_local! {
    static STREAM: RefCell<Option<web_sys::MediaStream>> = const { RefCell::new(None) };
}

const CLICK_THRESHOLD: f64 = 5.0;
const HANDLE: f64 = 20.0;

fn stop_stream() {
    STREAM.with(|s| {
        if let Some(st) = s.borrow_mut().take() {
            for t in st.get_tracks().iter() {
                if let Ok(t) = t.dyn_into::<web_sys::MediaStreamTrack>() {
                    t.stop();
                }
            }
        }
    });
}

fn set(o: &js_sys::Object, k: &str, v: impl Into<JsValue>) {
    let _ = js_sys::Reflect::set(o, &k.into(), &v.into());
}

fn constraints(device: Option<&str>) -> JsValue {
    let ideal = |n: f64| {
        let o = js_sys::Object::new();
        set(&o, "ideal", n);
        o
    };
    let video = js_sys::Object::new();
    set(&video, "width", ideal(1280.0));
    set(&video, "height", ideal(720.0));
    if let Some(d) = device {
        let o = js_sys::Object::new();
        set(&o, "exact", d);
        set(&video, "deviceId", o);
    }
    let c = js_sys::Object::new();
    set(&c, "video", video);
    set(&c, "audio", false);
    c.into()
}

fn js_err(e: JsValue) -> String {
    js_sys::Reflect::get(&e, &"name".into()).ok().and_then(|v| v.as_string()).or_else(|| e.as_string()).unwrap_or_else(|| format!("{e:?}"))
}

async fn get_media(device: Option<&str>) -> Result<web_sys::MediaStream, String> {
    let md = web_sys::window().ok_or("window")?.navigator().media_devices().map_err(js_err)?;
    let p = md.get_user_media_with_constraints(constraints(device).unchecked_ref()).map_err(js_err)?;
    JsFuture::from(p).await.map(|v| v.unchecked_into()).map_err(js_err)
}

/// Stream holen; schlägt das gewählte Gerät fehl (abgesteckt), auf die Standardkamera zurückfallen.
async fn start_stream(device: Option<String>) -> Result<web_sys::MediaStream, String> {
    match get_media(device.as_deref()).await {
        Err(e) if device.is_some() && e != "NotAllowedError" => get_media(None).await,
        r => r,
    }
}

/// Kameras des WebViews: (`deviceId`, Name) – für Geräteauswahl in Aufnahme-Leiste und PiP-Kontextmenü.
pub async fn video_devices() -> Vec<(String, String)> {
    let Some(md) = web_sys::window().and_then(|w| w.navigator().media_devices().ok()) else { return vec![] };
    let Ok(p) = md.enumerate_devices() else { return vec![] };
    let Ok(list) = JsFuture::from(p).await else { return vec![] };
    let mut out = vec![];
    for (i, d) in js_sys::Array::from(&list).iter().enumerate() {
        let get = |k: &str| js_sys::Reflect::get(&d, &k.into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
        if get("kind") != "videoinput" || get("deviceId").is_empty() {
            continue;
        }
        let label = get("label");
        let name = if label.is_empty() { format!("{} {}", api::t("Kamera", "Camera"), i + 1) } else { label };
        out.push((get("deviceId"), name));
    }
    out
}

pub fn devices_json(list: &[(String, String)]) -> Value {
    json!(list.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>())
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Idle,
    Pending { x: f64, y: f64 },
    Drag,
    Resize { x: f64, y: f64, start: f64, last: f64 },
}

fn viewport() -> (f64, f64) {
    let w = web_sys::window().unwrap();
    (
        w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(0.0),
        w.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(0.0),
    )
}

#[component]
fn CameraPip() -> impl IntoView {
    let state = RwSignal::new(Value::Null);
    let alpha = RwSignal::new(1.0_f64);
    let error = RwSignal::new(None::<String>);
    let live = RwSignal::new(false);
    let hover = RwSignal::new(false);
    let video = NodeRef::<leptos::html::Video>::new();
    let mode = StoredValue::new(Mode::Idle);

    spawn_local(async move {
        let d = api::window_data().await;
        if !d.is_null() {
            state.set(d);
        }
    });
    api::on("window-data", move |v| state.set(v));
    api::on("camera-state", move |v| {
        if let Some(a) = v["alpha"].as_f64() {
            alpha.set(a);
        }
        state.set(v);
    });
    api::on("camera-hover", move |v| alpha.set(v["alpha"].as_f64().unwrap_or(1.0)));

    let presentation = Memo::new(move |_| state.with(|s| s["presentation"].as_bool().unwrap_or(false)));

    // Form/Größe geändert (Menü, Einstellungen, Menüleiste) → Backend passt die Fenstergröße an.
    let layout_key = Memo::new(move |_| (api::setting_str("cameraShape"), api::setting_str("cameraSize"), api::setting_f64("cameraCustomSizePt")));
    Effect::new(move |prev: Option<()>| {
        layout_key.track();
        if prev.is_some() {
            api::fire("camera", "relayout", Value::Null);
        }
    });

    // Kamera starten / bei Gerätewechsel neu starten.
    let device = Memo::new(move |_| api::setting("cameraDeviceId").as_str().filter(|s| !s.is_empty()).map(str::to_owned));
    Effect::new(move |_| {
        let d = device.get();
        spawn_local(async move {
            stop_stream();
            match start_stream(d).await {
                Ok(s) => {
                    if let Some(v) = video.get_untracked() {
                        v.set_muted(true);
                        v.set_src_object(Some(&s));
                        let _ = v.play();
                    }
                    STREAM.with(|x| *x.borrow_mut() = Some(s));
                    error.set(None);
                    live.set(true);
                }
                Err(e) => {
                    live.set(false);
                    error.set(Some(e));
                }
            }
        });
    });
    on_cleanup(stop_stream);

    let on_down = move |e: web_sys::PointerEvent| {
        if e.button() != 0 {
            return;
        }
        let (w, h) = viewport();
        let (cx, cy) = (e.client_x() as f64, e.client_y() as f64);
        let (sx, sy) = (e.screen_x() as f64, e.screen_y() as f64);
        if let Some(t) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
            let _ = t.set_pointer_capture(e.pointer_id());
        }
        if !presentation.get_untracked() && cx >= w - HANDLE && cy >= h - HANDLE {
            let start = w.min(h);
            mode.set_value(Mode::Resize { x: sx, y: sy, start, last: start });
        } else {
            mode.set_value(Mode::Pending { x: sx, y: sy });
        }
    };
    let on_move = move |e: web_sys::PointerEvent| {
        let (sx, sy) = (e.screen_x() as f64, e.screen_y() as f64);
        match mode.get_value() {
            Mode::Pending { x, y } if ((sx - x).powi(2) + (sy - y).powi(2)).sqrt() > CLICK_THRESHOLD => {
                if let Some(t) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                    let _ = t.release_pointer_capture(e.pointer_id());
                }
                mode.set_value(Mode::Drag);
                api::fire("camera", "startDrag", Value::Null);
            }
            Mode::Resize { x, y, start, last } => {
                let size = (start + (sx - x).max(sy - y)).clamp(80.0, 400.0).round();
                if size != last {
                    mode.set_value(Mode::Resize { x, y, start, last: size });
                    api::fire("camera", "resize", json!({ "size": size }));
                }
            }
            _ => {}
        }
    };
    let on_up = move |_e: web_sys::PointerEvent| {
        match mode.get_value() {
            Mode::Pending { .. } => api::fire("camera", "togglePresentation", Value::Null),
            Mode::Resize { last, start, .. } if last != start => api::fire("camera", "resize", json!({ "size": last, "done": true })),
            _ => {}
        }
        mode.set_value(Mode::Idle);
    };

    let shape = move || if presentation.get() { "square".to_owned() } else { api::setting_str("cameraShape") };
    let frame_class = move || {
        let clip = if shape() == "circle" { "rounded-full" } else { "rounded-xl" };
        let border = if presentation.get() { "border border-white/20" } else { "border-[2.5px] border-white/40" };
        format!("relative size-full overflow-hidden bg-black {clip} {border}")
    };
    let video_class = move || format!("size-full object-cover {}", if api::setting_bool("cameraMirror") { "-scale-x-100" } else { "" });

    view! {
        <div
            class="fixed inset-0 select-none"
            style=move || format!("opacity:{};transition:opacity 180ms ease-in-out", alpha.get())
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=move |_| mode.set_value(Mode::Idle)
            on:contextmenu=move |e| {
                e.prevent_default();
                spawn_local(async move {
                    let devices = video_devices().await;
                    api::fire("camera", "menu", json!({ "devices": devices_json(&devices) }));
                });
            }
            on:mouseenter=move |_| hover.set(true)
            on:mouseleave=move |_| hover.set(false)
        >
            <div class=frame_class>
                <video
                    node_ref=video
                    autoplay=true
                    muted=true
                    playsinline=true
                    class=video_class
                    style:display=move || if live.get() { "block" } else { "none" }
                />
                <Show when=move || !live.get()>
                    <div class="absolute inset-0 flex flex-col items-center justify-center gap-1 p-3 text-center text-white/60">
                        <svg viewBox="0 0 24 24" class="size-8" fill="currentColor">
                            <path d="M4 7a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v1.6l3.4-2.3A1 1 0 0 1 21 7.1v9.8a1 1 0 0 1-1.6.8L16 15.4V17a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z" />
                        </svg>
                        {move || error.get().map(|e| {
                            let msg = if e == "NotAllowedError" {
                                api::t("Kein Kamerazugriff – in den Systemeinstellungen erlauben", "No camera access – allow it in System Settings")
                            } else {
                                api::t("Kamera nicht verfügbar", "Camera unavailable")
                            };
                            view! { <span class="text-[10px] leading-tight">{msg}</span> }
                        })}
                    </div>
                </Show>
            </div>
            <Show when=move || hover.get() && !presentation.get()>
                <svg viewBox="0 0 10 10" class="pointer-events-none absolute bottom-1 right-1 size-3 text-white/80 drop-shadow" stroke="currentColor" stroke-width="1.2" stroke-linecap="round">
                    <path d="M9 3L3 9M9 6L6 9" />
                </svg>
            </Show>
        </div>
    }
}
