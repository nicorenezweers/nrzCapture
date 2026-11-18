//! Quick Access (gestapelte Vorschau-Karten), Großansicht und Toast. Port von Capso `QuickAccessView` / `ToastWindow`.
use crate::{api, ui::*};
use leptos::{ev, html, prelude::*, task::spawn_local};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

pub fn route(r: &str) -> Option<AnyView> {
    match r {
        "quick" => Some(view! { <Quick /> }.into_any()),
        "quick-preview" => Some(view! { <Preview /> }.into_any()),
        "toast" => Some(view! { <Toast /> }.into_any()),
        "quick-fly" => Some(view! { <Fly /> }.into_any()),
        _ => None,
    }
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
struct Shot {
    id: usize,
    /// {path, kind, saved, thumb, width, height, duration, created}
    card: RwSignal<Value>,
    hover: RwSignal<bool>,
    dragging: RwSignal<bool>,
    /// Generation des Auto-Schließen-Timers (Erhöhen = abbrechen).
    timer: StoredValue<u32>,
    /// 0 = bereit, 1 = lädt hoch, 2 = Link kopiert
    upload: RwSignal<u8>,
    /// Fehlermeldung des letzten Uploads (Capso `FailureToast` in der Karte).
    upload_err: RwSignal<Option<String>>,
    /// Animationsphase, siehe `PH_*`.
    phase: RwSignal<u8>,
}

/// Unsichtbar, klein in der Mitte (Start der Einblende-Animation).
const PH_START: u8 = 0;
/// Unsichtbar, solange das Einflug-Abbild noch fliegt.
const PH_FLYING: u8 = 1;
/// Aufziehen + Einblenden (Capso `showPanel`, 0,42 s).
const PH_ENTER: u8 = 2;
/// Sofort sichtbar (Einflug gelandet).
const PH_LANDED: u8 = 3;
/// Normalzustand.
const PH_IDLE: u8 = 4;
/// Wird verdrängt: gleitet nach links raus (Capso `slideOffLeftAndClose`, 0,38 s).
const PH_LEAVING: u8 = 5;

type List = RwSignal<Vec<Shot>>;

fn find(list: List, id: usize) -> Option<Shot> {
    list.with_untracked(|l| l.iter().find(|s| s.id == id).copied())
}

/// Karte nach einer Aktion entfernen (kein „wieder öffnen“).
fn dismiss(list: List, id: usize) {
    list.update(|l| l.retain(|s| s.id != id));
}

/// Karte ungenutzt schließen (Schließen-Knopf, Esc, Auto-Schließen, Verdrängen) → kann wieder geöffnet werden.
fn close(list: List, id: usize) {
    if let Some(s) = find(list, id) {
        api::fire("quick", "closed", json!({"item": s.card.get_untracked()}));
    }
    dismiss(list, id);
}

fn schedule_auto_close(list: List, s: Shot) {
    let gen = s.timer.get_value().wrapping_add(1);
    s.timer.set_value(gen);
    if !api::setting_bool("quickAccessAutoClose") || s.hover.get_untracked() || s.dragging.get_untracked() {
        return;
    }
    let secs = api::setting_f64("quickAccessAutoCloseInterval").max(1.0);
    set_timeout(
        move || {
            if find(list, s.id).is_some() && s.timer.get_value() == gen && !s.hover.get_untracked() && !s.dragging.get_untracked() {
                close(list, s.id);
            }
        },
        Duration::from_secs_f64(secs),
    );
}

fn cancel_auto_close(s: Shot) {
    s.timer.update_value(|g| *g = g.wrapping_add(1));
}

/// Älteste Karte verdrängen: gleitet nach links aus dem Bild (Capso `slideOffLeftAndClose`, 0,38 s) und gilt als
/// ungenutzt geschlossen.
fn evict(list: List, s: Shot) {
    cancel_auto_close(s);
    api::fire("quick", "closed", json!({"item": s.card.get_untracked()}));
    s.phase.set(PH_LEAVING);
    set_timeout(move || dismiss(list, s.id), Duration::from_millis(380));
}

fn active(l: &[Shot]) -> impl Iterator<Item = &Shot> {
    l.iter().filter(|s| s.phase.get_untracked() != PH_LEAVING)
}

/// Karte wird sichtbar: Fokus bekommen (Capso `makeKey()`), Auto-Schließen starten.
fn reveal(list: List, focused: RwSignal<Option<usize>>, s: Shot) {
    let next = match s.phase.get_untracked() {
        PH_START => PH_ENTER,
        PH_FLYING => PH_LANDED,
        _ => return,
    };
    if find(list, s.id).is_none() {
        return;
    }
    s.phase.set(next);
    set_timeout(move || if s.phase.get_untracked() == next { s.phase.set(PH_IDLE) }, Duration::from_millis(if next == PH_ENTER { 420 } else { 30 }));
    focused.set(Some(s.id));
    api::fire("quick", "focus", Value::Null);
    schedule_auto_close(list, s);
}

fn add(list: List, focused: RwSignal<Option<usize>>, card: Value, max: usize) {
    while list.with_untracked(|l| active(l).count()) >= max {
        let Some(oldest) = list.with_untracked(|l| active(l).next().copied()) else { break };
        evict(list, oldest);
    }
    let fly = card["fly"].as_bool().unwrap_or(false);
    let auto_upload = card["autoUpload"].as_bool().unwrap_or(false);
    let s = Shot {
        id: NEXT.fetch_add(1, Ordering::Relaxed),
        card: RwSignal::new(card),
        hover: RwSignal::new(false),
        dragging: RwSignal::new(false),
        timer: StoredValue::new(0),
        upload: RwSignal::new(0),
        upload_err: RwSignal::new(None),
        phase: RwSignal::new(if fly { PH_FLYING } else { PH_START }),
    };
    list.update(|l| l.push(s));
    if fly {
        // Einflug-Abbild fliegt (Backend meldet `quick-landed`); Sicherheitsnetz, falls das Ereignis ausbleibt.
        set_timeout(move || reveal(list, focused, s), Duration::from_millis(1500));
    } else {
        // Capso `showPanel`: aus einem kleinen Rahmen in der Mitte aufziehen und einblenden (0,42 s).
        request_animation_frame(move || request_animation_frame(move || reveal(list, focused, s)));
    }
    if auto_upload {
        run(list, s, "upload");
    }
}

fn toast(msg: String) {
    api::fire("core", "toast", json!({ "message": msg }));
}

/// Aktion einer Karte ausführen (Capso: Kopieren/Sichern/Annotieren/Pin/OCR/Übersetzen schließen die Karte).
fn run(list: List, s: Shot, action: &'static str) {
    let item = s.card.get_untracked();
    match action {
        "close" => close(list, s.id),
        "reveal" => api::fire("quick", "action", json!({"action": "reveal", "item": item})),
        "open" => api::fire("quick", "action", json!({"action": "open", "item": item})),
        // Capso `performUpload`: Status in der Karte (lädt → Link kopiert, 3 s; Fehler als Hinweis mit „Erneut“).
        "upload" => {
            if s.upload.get_untracked() == 1 {
                return;
            }
            s.upload.set(1);
            s.upload_err.set(None);
            spawn_local(async move {
                match api::rpc("quick", "action", json!({"action": "upload", "item": item})).await {
                    Ok(_) => {
                        s.upload.set(2);
                        set_timeout(move || if s.upload.get_untracked() == 2 { s.upload.set(0) }, Duration::from_secs(3));
                    }
                    Err(e) => {
                        s.upload.set(0);
                        s.upload_err.set(Some(e));
                    }
                }
            });
        }
        "copy" | "save" | "saveAs" => spawn_local(async move {
            match api::rpc("quick", "action", json!({"action": action, "item": item})).await {
                Ok(v) if action == "copy" || !v.is_null() => dismiss(list, s.id),
                Ok(_) => {}
                Err(e) => toast(format!("{}: {e}", api::t("Speichern fehlgeschlagen", "Failed to save"))),
            }
        }),
        _ => {
            api::fire("quick", "action", json!({"action": action, "item": item}));
            dismiss(list, s.id);
        }
    }
}

fn label(a: &str, kind: &str) -> &'static str {
    match a {
        "drag" => api::t("Screenshot ziehen", "Drag Screenshot"),
        "copy" => api::t("Kopieren", "Copy"),
        "save" => api::t("Sichern", "Save"),
        "saveAs" => api::t("Sichern unter …", "Save As…"),
        "upload" => api::t("In die Cloud hochladen", "Upload to Cloud"),
        "annotate" if kind == "video" => api::t("Im Aufnahme-Editor öffnen", "Open in Recording Editor"),
        "annotate" => api::t("Annotieren", "Annotate"),
        "ocr" => api::t("Text erkennen", "Extract Text"),
        "translate" => api::t("Übersetzen", "Translate"),
        "pin" => api::t("Anheften", "Pin"),
        "reveal" => api::t("Im Finder zeigen", "Show in Finder"),
        "close" => api::t("Schließen", "Close"),
        _ => "",
    }
}

fn icon(a: &str) -> &'static str {
    match a {
        "drag" => GRIP,
        "copy" => COPY,
        "save" => DOWNLOAD,
        "saveAs" => SAVE,
        "upload" => CLOUD_UPLOAD,
        "annotate" => PENCIL,
        "ocr" => SCAN_TEXT,
        "translate" => LANGUAGES,
        "pin" => PIN,
        "reveal" => FOLDER,
        _ => MORE,
    }
}

/// Welche Aktionen gibt es für diese Art? (Videos: kein OCR/Übersetzen/Pin, GIFs: kein OCR/Übersetzen/Annotieren)
fn supported(a: &str, kind: &str) -> bool {
    match kind {
        "video" => !matches!(a, "ocr" | "translate" | "pin"),
        "gif" => !matches!(a, "ocr" | "translate" | "annotate"),
        _ => true,
    }
}

/// Tastenkürzel wie Capso.
fn shortcut(e: &web_sys::KeyboardEvent) -> Option<&'static str> {
    if e.key() == "Escape" {
        return Some("close");
    }
    if !e.meta_key() {
        return None;
    }
    match (e.key().to_lowercase().as_str(), e.shift_key()) {
        ("c", false) => Some("copy"),
        ("s", false) => Some("save"),
        ("s", true) => Some("saveAs"),
        ("e", false) => Some("annotate"),
        ("p", false) => Some("pin"),
        ("o", true) => Some("ocr"),
        ("t", true) => Some("translate"),
        ("w", false) => Some("close"),
        _ => None,
    }
}

fn relative(created_ms: f64, now_ms: f64) -> String {
    let s = ((now_ms - created_ms) / 1000.0).max(0.0);
    let de = api::lang() == "de";
    match s as u64 {
        0..=59 => if de { "jetzt".into() } else { "now".into() },
        60..=3599 => {
            let m = (s / 60.0) as u64;
            if de { format!("vor {m} Min.") } else { format!("{m} min. ago") }
        }
        _ => {
            let h = (s / 3600.0) as u64;
            if de { format!("vor {h} Std.") } else { format!("{h} hr. ago") }
        }
    }
}

fn duration_text(secs: f64) -> String {
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[component]
fn ToolButton(#[prop(into)] title: String, d: &'static str, #[prop(optional)] primary: bool, on_click: impl Fn(ev::MouseEvent) + 'static) -> impl IntoView {
    let class = if primary {
        "flex h-[26px] w-7 items-center justify-center rounded-md text-blue-500 hover:bg-blue-500/15 [&_svg]:size-[15px]"
    } else {
        "flex h-[26px] w-7 items-center justify-center rounded-md text-zinc-700 dark:text-zinc-200 hover:bg-black/10 dark:hover:bg-white/15 [&_svg]:size-[15px]"
    };
    view! {
        <button title=title class=class on:click=move |e| { e.stop_propagation(); on_click(e) } on:dblclick=|e| e.stop_propagation()>
            <Icon d=d />
        </button>
    }
}

fn divider() -> impl IntoView {
    view! { <div class="mx-0.5 h-4 w-px bg-black/15 dark:bg-white/15"></div> }
}

#[component]
fn Card(shot: Shot, list: List, focused: RwSignal<Option<usize>>, actions: RwSignal<(Vec<String>, Vec<String>)>, now: RwSignal<f64>) -> impl IntoView {
    let s = shot;
    let menu = RwSignal::new(false);
    let card = move || s.card.get();
    let kind = move || card()["kind"].as_str().unwrap_or("image").to_owned();
    let src = move || {
        let c = card();
        let thumb = c["thumb"].as_str().filter(|t| !t.is_empty()).or(c["path"].as_str()).unwrap_or("").to_owned();
        api::asset_url(&thumb)
    };
    let meta = move || {
        let c = card();
        let mut parts = vec![];
        if let (Some(w), Some(h)) = (c["width"].as_u64(), c["height"].as_u64()) {
            parts.push(format!("{w}×{h}"));
        }
        if let Some(d) = c["duration"].as_f64() {
            parts.push(duration_text(d));
        }
        parts.push(relative(c["created"].as_f64().unwrap_or(0.0), now.get()));
        parts.join(" · ")
    };
    let start_drag = move |e: ev::DragEvent| {
        e.prevent_default();
        cancel_auto_close(s);
        s.dragging.set(true);
        menu.set(false);
        api::fire("quick", "drag", json!({"item": s.card.get_untracked()}));
    };
    let revealed = move || s.hover.get() || menu.get();

    let tools = move || {
        let k = kind();
        let (visible, _) = actions.get();
        visible
            .into_iter()
            .filter(|a| supported(a, &k))
            .map(|a| {
                let a: &'static str = match a.as_str() {
                    "drag" => "drag",
                    "copy" => "copy",
                    "save" => "save",
                    "upload" => "upload",
                    "annotate" => "annotate",
                    "ocr" => "ocr",
                    "translate" => "translate",
                    _ => "pin",
                };
                let sep = matches!(a, "upload" | "annotate" | "pin").then(divider);
                let title = label(a, &k);
                let tool = match a {
                    "drag" => view! {
                        <div title=title draggable="true" on:dragstart=start_drag
                            class="flex h-[26px] w-7 cursor-grab items-center justify-center rounded-md text-zinc-700 dark:text-zinc-200 hover:bg-black/10 dark:hover:bg-white/15 [&_svg]:size-[15px]">
                            <Icon d=GRIP />
                        </div>
                    }
                    .into_any(),
                    "upload" => view! {
                        <button title=move || if s.upload.get() == 2 { api::t("Link kopiert!", "Link Copied!") } else { title }
                            class="flex h-[26px] w-7 items-center justify-center rounded-md text-zinc-700 dark:text-zinc-200 hover:bg-black/10 dark:hover:bg-white/15 [&_svg]:size-[15px]"
                            class:text-green-500=move || s.upload.get() == 2
                            on:click=move |e| { e.stop_propagation(); run(list, s, "upload") }>
                            <span class:animate-pulse=move || s.upload.get() == 1>
                                <Icon d=Signal::derive(move || if s.upload.get() == 2 { CHECK } else { CLOUD_UPLOAD }) />
                            </span>
                        </button>
                    }
                    .into_any(),
                    // Capso: Wahltaste + Klick auf Sichern = Sichern unter.
                    "save" => view! { <ToolButton title=title d=icon(a) primary=true on_click=move |e: ev::MouseEvent| run(list, s, if e.alt_key() { "saveAs" } else { "save" }) /> }.into_any(),
                    _ => view! { <ToolButton title=title d=icon(a) on_click=move |_| run(list, s, a) /> }.into_any(),
                };
                view! { {sep} {tool} }
            })
            .collect_view()
    };

    let overflow_items = move || {
        let k = kind();
        let (_, over) = actions.get();
        let mut items: Vec<&'static str> = vec!["saveAs"];
        for a in over.iter() {
            let a: &'static str = match a.as_str() {
                "drag" => continue,
                "copy" => "copy",
                "save" => "save",
                "upload" => "upload",
                "annotate" => "annotate",
                "ocr" => "ocr",
                "translate" => "translate",
                _ => "pin",
            };
            if supported(a, &k) {
                items.push(a);
            }
        }
        items.push("reveal");
        items
            .into_iter()
            .map(|a| {
                let title = label(a, &k);
                view! {
                    <button class="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-[12px] hover:bg-blue-500 hover:text-white [&_svg]:size-3.5"
                        on:click=move |e| { e.stop_propagation(); menu.set(false); run(list, s, a) }>
                        <Icon d=icon(a) /> {title}
                    </button>
                }
            })
            .collect_view()
    };

    view! {
        <div
            class="relative flex h-[200px] w-[288px] shrink-0 flex-col gap-2 rounded-[14px] border border-black/10 dark:border-white/10 bg-white/75 dark:bg-zinc-800/75 p-2 text-zinc-900 dark:text-zinc-100 shadow-[0_8px_18px_rgba(0,0,0,0.24),0_1px_3px_rgba(0,0,0,0.12)] backdrop-blur-2xl transition-[opacity,transform] duration-200"
            class:opacity-0=move || s.dragging.get() || matches!(s.phase.get(), PH_START | PH_FLYING | PH_LEAVING)
            class:pointer-events-none=move || matches!(s.phase.get(), PH_START | PH_FLYING | PH_LEAVING)
            // Verdrängte (unterste) Karte bleibt an ihrem Platz, die übrigen rücken nach.
            style:position=move || if s.phase.get() == PH_LEAVING { "absolute" } else { "relative" }
            style:left=move || if s.phase.get() == PH_LEAVING { "14px" } else { "auto" }
            style:bottom=move || if s.phase.get() == PH_LEAVING { "14px" } else { "auto" }
            style:transform=move || match s.phase.get() {
                PH_LEAVING => "translateX(-360px)",
                PH_START => "scale(0.18)",
                _ if revealed() => "translateY(-2px)",
                _ => "none",
            }
            style:transition-duration=move || match s.phase.get() {
                PH_LEAVING => "380ms",
                PH_ENTER => "420ms",
                PH_START | PH_FLYING | PH_LANDED => "0ms",
                _ => "260ms",
            }
            style:transition-timing-function=move || match s.phase.get() {
                PH_LEAVING => "ease-in",
                PH_ENTER => "cubic-bezier(0.22,0.8,0.2,1)",
                _ => "ease-out",
            }
            on:mousedown=move |_| focused.set(Some(s.id))
            on:mouseenter=move |_| { s.hover.set(true); cancel_auto_close(s) }
            on:mouseleave=move |_| { s.hover.set(false); menu.set(false); schedule_auto_close(list, s) }
        >
            // Vorschau
            <div class="relative h-[142px] w-[268px] overflow-hidden rounded-[7px] border border-black/10 dark:border-white/10 bg-black/10">
                <img src=src draggable="true" on:dragstart=start_drag
                    class="size-full object-cover"
                    title=move || if kind() == "video" { api::t("Klicken zum Bearbeiten", "Click to edit") } else { api::t("Doppelklick für Vorschau", "Double-click to preview") }
                    on:click=move |_| if kind() == "video" { run(list, s, "open") }
                    on:dblclick=move |_| if kind() != "video" { run(list, s, "open") } />
                <Show when=move || kind() == "video">
                    <div class="pointer-events-none absolute inset-0 flex items-center justify-center">
                        <div class="flex size-10 items-center justify-center rounded-full bg-black/45 text-white backdrop-blur [&_svg]:size-4 [&_svg]:fill-current"><Icon d=PLAY /></div>
                    </div>
                </Show>
                <Show when=move || kind() == "gif">
                    <div class="pointer-events-none absolute bottom-1.5 left-1.5 rounded bg-black/55 px-1.5 py-0.5 text-[10px] font-bold text-white">"GIF"</div>
                </Show>
                <button title=api::t("Schließen", "Close")
                    class="absolute right-[7px] top-[7px] flex size-6 items-center justify-center rounded-full border border-black/10 bg-white/80 dark:bg-zinc-700/80 text-zinc-700 dark:text-zinc-100 backdrop-blur transition-opacity [&_svg]:size-3"
                    class:opacity-0=move || !revealed()
                    class:pointer-events-none=move || !revealed()
                    on:click=move |e| { e.stop_propagation(); close(list, s.id) }>
                    <Icon d=X />
                </button>
            </div>
            // Untertitel (Ruhe) bzw. Werkzeugleiste (Hover) im selben Platz
            <div class="relative h-[34px]">
                <div class="absolute inset-0 flex items-center justify-between px-1.5 transition-opacity duration-200" class:opacity-0=revealed>
                    <span class="flex items-center gap-1.5 text-[13px] font-semibold">
                        <span class="text-green-500 [&_svg]:size-3.5"><Icon d=CHECK /></span>
                        {move || match kind().as_str() {
                            "video" | "gif" => api::t("Aufgenommen", "Recorded"),
                            _ => api::t("Aufgenommen", "Captured"),
                        }}
                    </span>
                    <span class="font-mono text-[10px] font-medium uppercase text-zinc-500 dark:text-zinc-400">{meta}</span>
                </div>
                <div class="absolute inset-0 flex items-center justify-center transition-opacity duration-200" class:opacity-0=move || !revealed() class:pointer-events-none=move || !revealed()>
                    <div class="flex items-center gap-0.5 rounded-lg bg-black/[0.045] dark:bg-white/[0.06] px-1 py-[3px]">
                        {tools}
                        {divider()}
                        <ToolButton title=api::t("Weitere Aktionen", "More actions") d=MORE on_click=move |_| menu.update(|m| *m = !*m) />
                    </div>
                </div>
            </div>
            <Show when=move || menu.get()>
                <div class="absolute bottom-[46px] right-2 z-10 w-48 rounded-lg border border-black/10 dark:border-white/10 bg-white/95 dark:bg-zinc-800/95 p-1 shadow-xl backdrop-blur-xl"
                    on:click=|e| e.stop_propagation()>
                    {overflow_items}
                </div>
            </Show>
            // Upload-Fehler (Capso `FailureToast`): Meldung mit „Erneut versuchen“ und Schließen.
            {move || s.upload_err.get().map(|err| view! {
                <div class="absolute inset-x-2 bottom-2 z-20 flex items-center gap-1.5 rounded-lg border border-black/10 dark:border-white/10 bg-white/95 dark:bg-zinc-800/95 px-2 py-1.5 text-[11px] shadow-lg backdrop-blur-xl"
                    on:click=|e| e.stop_propagation()>
                    <span class="text-orange-500 [&_svg]:size-3.5"><Icon d=INFO /></span>
                    <span class="min-w-0 flex-1 truncate" title=err.clone()>{api::t("Hochladen fehlgeschlagen", "Upload failed")}": "{err.clone()}</span>
                    <button class="shrink-0 rounded px-1.5 py-0.5 font-medium text-blue-500 hover:bg-blue-500/15"
                        on:click=move |e| { e.stop_propagation(); run(list, s, "upload") }>
                        {api::t("Erneut", "Retry")}
                    </button>
                    <button title=api::t("Schließen", "Dismiss") class="shrink-0 rounded p-0.5 hover:bg-black/10 dark:hover:bg-white/15 [&_svg]:size-3"
                        on:click=move |e| { e.stop_propagation(); s.upload_err.set(None) }>
                        <Icon d=X />
                    </button>
                </div>
            })}
        </div>
    }
}

#[component]
fn Quick() -> impl IntoView {
    let list: List = RwSignal::new(Vec::new());
    let focused: RwSignal<Option<usize>> = RwSignal::new(None);
    let actions = RwSignal::new((vec![], vec![]));
    let max = StoredValue::new(5usize);
    let now = RwSignal::new(js_sys::Date::now());
    let _ = set_interval_with_handle(move || now.set(js_sys::Date::now()), Duration::from_secs(20));

    let load_actions = move || {
        spawn_local(async move {
            if let Ok(v) = api::rpc("quick", "actions", Value::Null).await {
                let ids = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                actions.set((ids("visible"), ids("overflow")));
                max.set_value(v["maxStack"].as_u64().unwrap_or(5) as usize);
            }
        })
    };
    let drain = move || {
        spawn_local(async move {
            if let Ok(Value::Array(cards)) = api::rpc("quick", "pending", Value::Null).await {
                for c in cards {
                    add(list, focused, c, max.get_value());
                }
            }
        })
    };
    load_actions();
    drain();
    // Aktions-Layout folgt den Einstellungen (Cloud-Freigabe, quickAccessActions).
    Effect::new(move |_| {
        api::settings().track();
        load_actions();
    });
    // Verdrängte Karten zählen nicht mehr (sie gleiten nur noch heraus).
    Effect::new(move |_| {
        let n = list.with(|l| l.iter().filter(|s| s.phase.get() != PH_LEAVING).count());
        api::fire("quick", "layout", json!({"n": n}))
    });
    // Hover-Status fürs globale Übersetzen-Kürzel melden.
    Effect::new(move |prev: Option<bool>| {
        let hover = list.with(|l| l.iter().any(|s| s.hover.get()));
        if prev != Some(hover) {
            api::fire("quick", "hover", json!({"hover": hover}));
        }
        hover
    });
    // Kürzel/Übersetzen gelten für die Karte unter dem Mauszeiger, sonst die fokussierte, sonst die neueste.
    let target = move || {
        list.with_untracked(|l| {
            let live = || active(l);
            live().find(|s| s.hover.get_untracked()).or_else(|| live().find(|s| Some(s.id) == focused.get_untracked())).or_else(|| live().last()).copied()
        })
    };

    api::on("quick-add", move |_| drain());
    api::on("quick-landed", move |created: Value| {
        let hit = list.with_untracked(|l| l.iter().find(|s| s.card.with_untracked(|c| c["created"] == created)).copied());
        if let Some(s) = hit {
            reveal(list, focused, s);
        }
    });
    api::on("quick-translate", move |_| {
        if let Some(s) = target() {
            if s.card.with_untracked(|c| supported("translate", c["kind"].as_str().unwrap_or("image"))) {
                run(list, s, "translate");
            }
        }
    });
    api::on("dropped", move |v: Value| {
        let (path, dropped) = (v[0].as_str().unwrap_or_default().to_owned(), v[1].as_bool().unwrap_or(false));
        let Some(s) = list.with_untracked(|l| l.iter().find(|s| s.card.with_untracked(|c| c["path"] == path.as_str())).copied()) else { return };
        if dropped && api::setting_bool("quickAccessCloseAfterDrag") {
            close(list, s.id);
        } else {
            s.dragging.set(false);
            s.hover.set(false);
            schedule_auto_close(list, s);
        }
    });
    api::on("quick-close-all", move |_| list.set(vec![]));
    api::on("quick-save-all", move |_| {
        for s in list.get_untracked() {
            run(list, s, "save");
        }
    });

    let _ = window_event_listener(ev::keydown, move |e| {
        let Some(a) = shortcut(&e) else { return };
        if let Some(s) = target() {
            e.prevent_default();
            let k = s.card.with_untracked(|c| c["kind"].as_str().unwrap_or("image").to_owned());
            if supported(a, &k) {
                run(list, s, a);
            }
        }
    });

    view! {
        <div class="relative flex h-screen flex-col-reverse items-center gap-3 p-[14px]">
            <For each=move || list.get() key=|s| s.id let:shot>
                <Card shot=shot list=list focused=focused actions=actions now=now />
            </For>
        </div>
    }
}

// ───────────────────────────── Einflug ─────────────────────────────

/// Kartenabbild, das vom aufgenommenen Bereich an seinen Platz im Stapel fliegt (Capso `QuickAccessTransitionWindow`).
/// Das Backend bewegt und skaliert das Fenster; der Inhalt füllt es proportional.
#[component]
fn Fly() -> impl IntoView {
    let src = RwSignal::new(String::new());
    let set = move |d: Value| {
        let t = d["thumb"].as_str().filter(|t| !t.is_empty()).or(d["path"].as_str()).unwrap_or("").to_owned();
        src.set(api::asset_url(&t));
    };
    spawn_local(async move { set(api::window_data().await) });
    api::on("window-data", set);
    view! {
        <div class="flex h-screen w-screen flex-col gap-[4%] overflow-hidden rounded-[14px] border border-black/10 dark:border-white/10 bg-white/90 dark:bg-zinc-800/90 p-[2.8%]">
            <div class="min-h-0 flex-[71] overflow-hidden rounded-[7px] border border-black/10 dark:border-white/10 bg-black/10">
                <img src=move || src.get() class="size-full object-cover" draggable="false" />
            </div>
            <div class="flex-[17]"></div>
        </div>
    }
}

// ───────────────────────────── Großansicht ─────────────────────────────

#[component]
fn Preview() -> impl IntoView {
    let path = RwSignal::new(String::new());
    spawn_local(async move {
        let d = api::window_data().await;
        path.set(d["path"].as_str().unwrap_or("").to_owned());
    });
    api::on("window-data", move |d: Value| path.set(d["path"].as_str().unwrap_or("").to_owned()));
    let _ = window_event_listener(ev::keydown, move |e| {
        if e.key() == "Escape" || (e.meta_key() && e.key() == "w") {
            api::close_window();
        }
    });
    view! {
        <div class="app-window flex h-screen flex-col">
            <div class="h-7 shrink-0"></div>
            <div class="flex min-h-0 flex-1 items-center justify-center bg-black/5 dark:bg-black/30">
                <Show when=move || !path.get().is_empty()>
                    <img src=move || api::asset_url(&path.get()) class="max-h-full max-w-full object-contain" draggable="false" />
                </Show>
            </div>
        </div>
    }
}

// ───────────────────────────── Toast ─────────────────────────────

#[component]
fn Toast() -> impl IntoView {
    let msg = RwSignal::new(String::new());
    let error = RwSignal::new(false);
    let shown = RwSignal::new(false);
    let pill: NodeRef<html::Div> = NodeRef::new();

    let present = move |d: Value| {
        msg.set(d["message"].as_str().unwrap_or("").to_owned());
        error.set(d["error"].as_bool().unwrap_or(false));
        shown.set(false);
        // Nach dem Rendern messen, Fenster anpassen und einblenden.
        request_animation_frame(move || {
            if let Some(el) = pill.get_untracked() {
                let r = el.get_bounding_client_rect();
                api::fire("quick", "toastFit", json!({"w": r.width().ceil() + 24.0, "h": r.height().ceil() + 24.0}));
            }
            request_animation_frame(move || shown.set(true));
        });
    };
    spawn_local(async move {
        let d = api::window_data().await;
        if !d.is_null() {
            present(d);
        }
    });
    api::on("toast-show", present);
    api::on("toast-hide", move |_| shown.set(false));

    view! {
        <div class="flex h-screen items-center justify-center">
            <div node_ref=pill
                class="flex w-max items-center gap-2 whitespace-nowrap rounded-full border border-black/5 dark:border-white/10 bg-white/70 dark:bg-zinc-800/70 px-4 py-2.5 text-[13px] font-medium text-zinc-900 dark:text-zinc-100 shadow-[0_4px_8px_rgba(0,0,0,0.2)] backdrop-blur-2xl transition-all ease-out"
                style:transition-duration=move || if shown.get() { "350ms" } else { "250ms" }
                style:opacity=move || if shown.get() { "1" } else { "0" }
                style:transform=move || if shown.get() { "translateY(0)" } else { "translateY(12px)" }
            >
                <span class="[&_svg]:size-4" class:text-green-500=move || !error.get() class:text-orange-500=move || error.get()>
                    <Icon d=Signal::derive(move || if error.get() { INFO } else { CHECK }) />
                </span>
                {move || msg.get()}
            </div>
        </div>
    }
}
