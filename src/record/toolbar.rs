//! Aufnahme-Leiste (Capso `RecordingToolbarView`): Größe, Mikrofon/System-Audio/Kamera, Cursor/Klicks/Tasten,
//! „GIF aufnehmen“ (⌥↩), „Video aufnehmen“ (↩), „Bereich ändern“; Esc bricht ab.
use super::*;
use crate::{api, ui::*};
use leptos::{ev, prelude::*, task::spawn_local};
use serde_json::{json, Value};

fn rec(format: &'static str) {
    api::fire("record", "record", json!({ "format": format }));
}

/// Kleiner Umschalt-Knopf 36×30 (Capso `toolbarToggle`).
#[component]
fn Tog(#[prop(into)] title: String, #[prop(into)] on: Signal<bool>, #[prop(into)] icon: Signal<&'static str>, on_click: impl Fn() + 'static) -> impl IntoView {
    view! {
        <button title=title
            class=move || format!("flex h-[30px] w-9 items-center justify-center rounded-md transition-colors [&_svg]:size-4 {}",
                if on.get() { "bg-white/15 text-white" } else { "text-white/55 hover:bg-white/10" })
            on:click=move |e| { e.stop_propagation(); on_click() }>
            <Icon d=icon />
        </button>
    }
}

#[component]
fn MenuItem(#[prop(into)] label: String, #[prop(into)] checked: Signal<bool>, on_click: impl Fn() + 'static) -> impl IntoView {
    view! {
        <button class="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-[12px] text-white hover:bg-blue-500"
            on:click=move |e| { e.stop_propagation(); on_click() }>
            <span class="w-3 text-[11px]">{move || if checked.get() { "✓" } else { "" }}</span>
            <span class="truncate">{label}</span>
        </button>
    }
}

fn section(title: &'static str) -> impl IntoView {
    view! { <div class="px-2 pt-1.5 pb-0.5 text-[10px] font-semibold tracking-wide text-white/40">{title}</div> }
}

#[component]
pub fn Toolbar() -> impl IntoView {
    let info = RwSignal::new(Value::Null);
    let menu = RwSignal::new("");
    let camera_open = RwSignal::new(false);
    // Pro Sitzung (Capso RecordingToolbarState): System-Audio an, Kamera aus – nicht gemerkt.
    let sys = RwSignal::new(true);
    let cameras = RwSignal::new(Vec::<(String, String)>::new());
    spawn_local(async move {
        if let Ok(v) = api::rpc("record", "toolbarInfo", Value::Null).await {
            camera_open.set(v["cameraOpen"].as_bool().unwrap_or(false));
            sys.set(v["systemAudio"].as_bool().unwrap_or(true));
            info.set(v);
        }
        cameras.set(crate::camera::video_devices().await);
    });
    api::on("window-data", move |d| info.update(|i| i["data"] = d));

    let _k = window_event_listener(ev::keydown, move |e| match e.key().as_str() {
        "Enter" => {
            e.prevent_default();
            rec(if e.alt_key() { "gif" } else { "mp4" })
        }
        "Escape" => {
            if menu.get_untracked().is_empty() {
                // Backend: erst Kamera-Präsentationsmodus verlassen, sonst abbrechen.
                api::fire("record", "escape", Value::Null)
            } else {
                menu.set("")
            }
        }
        _ => {}
    });

    let dim = move |k: &str| info.with(|i| i["data"][k].as_f64().unwrap_or(0.0) as i64);
    let is_window = move || info.with(|i| i["data"]["window"].as_bool().unwrap_or(false));
    let b = |k: &'static str| Signal::derive(move || api::setting_bool(k));
    let flip = |k: &'static str| move || api::set_setting(k, json!(!api::setting_bool(k)));
    let mic = b("recordingMicEnabled");
    let toggle_sys = move || {
        let on = !sys.get_untracked();
        sys.set(on);
        api::fire("record", "setSystemAudio", json!({ "on": on }));
    };
    let toggle_menu = move |m: &'static str| menu.update(|v| *v = if *v == m { "" } else { m });

    let mics = move || info.with(|i| i["mics"].as_array().cloned().unwrap_or_default());
    let mic_id = move || api::setting_str("recordingMicDeviceId");
    let choose_mic = move |id: Option<String>| {
        api::set_setting("recordingMicEnabled", json!(true));
        api::set_setting("recordingMicDeviceId", json!(id));
        menu.set("");
    };

    let toggle_camera = move || {
        spawn_local(async move {
            if let Ok(v) = api::rpc("record", "toggleCamera", Value::Null).await {
                camera_open.set(v.as_bool().unwrap_or(false));
            }
        });
    };
    let camera_id = move || api::setting_str("cameraDeviceId");
    let choose_camera = move |id: String| {
        menu.set("");
        spawn_local(async move {
            if let Ok(v) = api::rpc("record", "selectCamera", json!({ "id": id })).await {
                camera_open.set(v.as_bool().unwrap_or(false));
            }
        });
    };
    let shape = move || api::setting_str("cameraShape");
    let size = move || api::setting_str("cameraSize");

    let kbd = |k: &'static str| view! { <span class="rounded bg-white/10 px-1 py-px font-mono text-[10px] text-white/60">{k}</span> };
    let row = "flex w-full items-center gap-2.5 px-3.5 py-2 text-[13px] font-medium text-white hover:bg-white/10";

    view! {
        <div class="flex h-screen w-screen flex-col items-center" on:click=move |_| menu.set("")>
            <div class="relative mt-1 w-[220px] overflow-visible rounded-[10px] border border-white/10 bg-black/80 shadow-[0_6px_24px_rgba(0,0,0,0.5)] backdrop-blur-xl">
                // Größe + Optionen
                <div class="flex flex-col items-center gap-2 px-4 py-3">
                    <div class="flex items-center gap-1.5 font-mono text-[13px] font-medium text-white/90">
                        <span class="min-w-12 rounded bg-white/10 px-2 py-1 text-center">{move || dim("width")}</span>
                        <span class="text-white/50">"×"</span>
                        <span class="min-w-12 rounded bg-white/10 px-2 py-1 text-center">{move || dim("height")}</span>
                        <span class="text-[11px] text-white/45">"px"</span>
                    </div>
                    <div class="flex items-center gap-1">
                        <Tog title=api::t("Mikrofon", "Microphone") on=mic icon=Signal::derive(move || if mic.get() { MIC } else { MIC_OFF }) on_click=move || toggle_menu("mic") />
                        <Tog title=api::t("System-Audio", "System Audio") on=sys icon=Signal::derive(move || if sys.get() { SPEAKER } else { SPEAKER_OFF }) on_click=toggle_sys />
                        <Tog title=api::t("Kamera", "Camera") on=camera_open icon=Signal::derive(move || if camera_open.get() { CAMERA } else { CAMERA_OFF }) on_click=move || toggle_menu("camera") />
                    </div>
                    <div class="flex items-center gap-1">
                        <Tog title=api::t("Mauszeiger anzeigen", "Show Cursor") on=b("showCursor") icon=POINTER on_click=flip("showCursor") />
                        <Tog title=api::t("Klicks hervorheben", "Highlight Clicks") on=Signal::derive(move || api::setting_bool("highlightClicks") && !is_window()) icon=CLICK on_click=flip("highlightClicks") />
                        <Tog title=api::t("Tastendrücke anzeigen", "Show Key Presses") on=Signal::derive(move || api::setting_bool("showKeyPresses") && !is_window()) icon=KEYBOARD on_click=flip("showKeyPresses") />
                    </div>
                </div>
                <div class="h-px bg-white/15"></div>
                // Aktionen
                <button class=row on:click=move |_| rec("gif")>
                    <span class="rounded bg-purple-500/60 px-1.5 py-0.5 font-mono text-[11px] font-bold">"GIF"</span>
                    <span class="flex-1 text-left">{api::t("GIF aufnehmen", "Record GIF")}</span>
                    {kbd("⌥")}{kbd("↩")}
                </button>
                <div class="h-px bg-white/10"></div>
                <button class=row on:click=move |_| rec("mp4")>
                    <span class="size-3.5 rounded-full bg-red-500 ring-2 ring-red-500/30"></span>
                    <span class="flex-1 text-left">{api::t("Video aufnehmen", "Record Video")}</span>
                    {kbd("↩")}
                </button>
                <div class="h-px bg-white/10"></div>
                <button class=format!("{row} rounded-b-[10px]") on:click=move |_| api::fire("record", "changeArea", Value::Null)>
                    <span class="text-white/80 [&_svg]:size-3.5"><Icon d=AREA /></span>
                    <span class="flex-1 text-left">{api::t("Bereich ändern", "Change Area")}</span>
                </button>

                // Mikrofon-Menü
                <Show when=move || menu.get() == "mic">
                    <div class="absolute left-3 top-[96px] z-10 max-h-[230px] w-[230px] overflow-y-auto rounded-lg border border-white/10 bg-zinc-800/95 p-1 shadow-xl" on:click=|e| e.stop_propagation()>
                        <MenuItem label=api::t("Mikrofon nicht aufnehmen", "Do Not Record Microphone") checked=Signal::derive(move || !mic.get())
                            on_click=move || { api::set_setting("recordingMicEnabled", json!(false)); menu.set("") } />
                        <div class="my-1 h-px bg-white/10"></div>
                        <MenuItem label=api::t("Systemstandard", "System Default") checked=Signal::derive(move || mic.get() && mic_id().is_empty())
                            on_click=move || choose_mic(None) />
                        {move || mics().into_iter().map(|m| {
                            let id = m["id"].as_str().unwrap_or("").to_owned();
                            let (id2, id3) = (id.clone(), id.clone());
                            view! { <MenuItem label=m["name"].as_str().unwrap_or("?").to_owned()
                                checked=Signal::derive(move || mic.get() && mic_id() == id2)
                                on_click=move || choose_mic(Some(id3.clone())) /> }
                        }).collect_view()}
                    </div>
                </Show>

                // Kamera-Menü
                <Show when=move || menu.get() == "camera">
                    <div class="absolute left-3 top-[96px] z-10 w-[200px] rounded-lg border border-white/10 bg-zinc-800/95 p-1 shadow-xl" on:click=|e| e.stop_propagation()>
                        <MenuItem label=api::t("Keine", "None") checked=Signal::derive(move || !camera_open.get())
                            on_click=move || if camera_open.get_untracked() { toggle_camera() } />
                        {move || {
                            let list = cameras.get();
                            if list.is_empty() {
                                return view! { <MenuItem label=api::t("Kamera", "Camera") checked=camera_open
                                    on_click=move || if !camera_open.get_untracked() { toggle_camera() } /> }.into_any();
                            }
                            let first = list[0].0.clone();
                            list.into_iter().map(|(id, name)| {
                                let (id2, first) = (id.clone(), first.clone());
                                let checked = Signal::derive(move || {
                                    let cur = camera_id();
                                    camera_open.get() && (cur == id2 || (cur.is_empty() && id2 == first))
                                });
                                view! { <MenuItem label=name checked=checked on_click=move || choose_camera(id.clone()) /> }
                            }).collect_view().into_any()
                        }}
                        {section(api::t("FORM", "SHAPE"))}
                        {[("circle", api::t("Kreis", "Circle")), ("square", api::t("Quadrat", "Square")), ("portrait", api::t("Hochformat", "Portrait")), ("landscape", api::t("Querformat", "Landscape"))]
                            .into_iter().map(|(k, l)| view! { <MenuItem label=l checked=Signal::derive(move || shape() == k) on_click=move || api::set_setting("cameraShape", json!(k)) /> }).collect_view()}
                        {section(api::t("GRÖSSE", "SIZE"))}
                        {[("small", api::t("Klein", "Small")), ("medium", api::t("Mittel", "Medium")), ("large", api::t("Groß", "Large"))]
                            .into_iter().map(|(k, l)| view! { <MenuItem label=l checked=Signal::derive(move || size() == k)
                                on_click=move || { api::set_setting("cameraCustomSizePt", json!(0.0)); api::set_setting("cameraSize", json!(k)) } /> }).collect_view()}
                        <div class="my-1 h-px bg-white/10"></div>
                        <MenuItem label=api::t("Spiegeln", "Mirror") checked=b("cameraMirror") on_click=flip("cameraMirror") />
                        <MenuItem label=api::t("Bei Hover ausblenden", "Fade on Hover") checked=b("cameraPipFadeOnHover") on_click=flip("cameraPipFadeOnHover") />
                        <MenuItem label=api::t("Klicks durchlassen", "Click Through PiP") checked=b("cameraPipClickThrough") on_click=flip("cameraPipClickThrough") />
                    </div>
                </Show>
            </div>
        </div>
    }
}
