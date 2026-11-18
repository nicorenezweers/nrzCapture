//! Auswahl-Overlay je Bildschirm (Port von Capso `CaptureOverlayView` + `CaptureAllInOneToolbarWindow`).
//!
//! Bereichsmodus: Standbild, Abdunklung mit hellem Ausschnitt, Fadenkreuz + Koordinaten, Lupe, Größenanzeige,
//! Shift = 1:1, ⇧ + Square-Center-Lock-Taste (Standard C) = von der Mitte aus, R / ⇧R = Presets durchschalten,
//! Rechtsklick = Preset-Menü (Seitenverhältnisse / Feste Größen / Presets verwalten…), Fenster unter dem
//! Mauszeiger werden hervorgehoben (Klick = Fenster, ⇧-Klick = mehrere Fenster, ⇧ loslassen = aufnehmen),
//! Leertaste = Bereich ⇄ Fenster, Esc = Abbrechen.
//! Anpassen-Phase (All-in-One, Aufnahme): verschieben, 8 Griffe, Pfeiltasten, Enter/Doppelklick bestätigt.
//! All-in-One: Inline-Annotation (`editor::InlineAnnotator`) sofort nach dem Auswählen, seitliche Leiste mit
//! Modi + Aktionen (kompakt mit „Mehr“), Leertaste halten = Auswahl verschieben, Griffe bleiben aktiv.
//! Aufnahme-Auswahl: Modusleiste Bereich / Fenster / Vollbild / Letzter Bereich (A / W / F / R).
use super::geom::{self, Handle, Hit, Preset, Rect};
use crate::api::{self, t};
use crate::editor::InlineAnnotator;
use crate::ui::{self, Icon};
use leptos::{prelude::*, task::spawn_local};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct ScreenInfo {
    id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct WinInfo {
    id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    /// Sichtbarer Teil auf diesem Bildschirm (lokal).
    visible: [f64; 4],
    app: String,
    title: String,
}

impl WinInfo {
    fn visible_rect(&self) -> Rect {
        Rect::new(self.visible[0], self.visible[1], self.visible[2], self.visible[3])
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct PresetInfo {
    id: String,
    name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct Data {
    session: u64,
    mode: String,
    purpose: String,
    screen: ScreenInfo,
    frozen: Option<String>,
    windows: Vec<WinInfo>,
    presets_enabled: bool,
    presets: Vec<PresetInfo>,
    preset: String,
    square_key: String,
    magnifier: bool,
    adjust: bool,
    confirm_label: String,
    focused: bool,
    cursor: Option<(f64, f64)>,
    multi_window: bool,
    can_last_area: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Idle,
    Drag,
    Adjust,
    Annotate,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    None,
    Create((f64, f64)),
    Move(Rect, (f64, f64)),
    Resize(Handle, Rect),
}

const DIM: &str = "rgba(0,0,0,.38)";
const RAIL_W: f64 = 84.0;
const REC_PANEL: (f64, f64) = (520.0, 68.0);
const MAG: f64 = 128.0;
const ZOOM: f64 = 4.0;

#[component]
pub fn Select() -> impl IntoView {
    let data = RwSignal::new(Data::default());
    let window_mode = RwSignal::new(false);
    let phase = RwSignal::new(Phase::Idle);
    let sel = RwSignal::new(None::<Rect>);
    let mouse = RwSignal::new(None::<(f64, f64)>);
    let shift = RwSignal::new(false);
    let center = RwSignal::new(false);
    let preset = RwSignal::new(String::from("freeform"));
    let menu = RwSignal::new(None::<(f64, f64)>);
    let more = RwSignal::new(false);
    let annot = RwSignal::new(None::<(String, f64)>);
    let op = StoredValue::new(Op::None);
    let last_raw = StoredValue::new((0.0, 0.0));
    let natural_w = StoredValue::new(0.0f64);
    let sent = StoredValue::new(false);
    let focus_asked = StoredValue::new(0.0f64);
    let space = RwSignal::new(false);
    let multi = RwSignal::new(Vec::<u32>::new());
    // Aktion der Seitenleiste, die das gerenderte Annotationsergebnis braucht (über ⇧⌘C ausgelöst).
    let pending = StoredValue::new(None::<String>);

    let all_in_one = Memo::new(move |_| data.with(|d| d.mode == "allInOne"));
    let recording = Memo::new(move |_| data.with(|d| d.purpose == "recording"));
    let adjustable = Memo::new(move |_| data.with(|d| d.adjust || d.mode == "allInOne"));
    let bounds = Memo::new(move |_| data.with(|d| Rect::new(0.0, 0.0, d.screen.width.max(1.0), d.screen.height.max(1.0))));
    let presets_on = Memo::new(move |_| data.with(|d| d.presets_enabled));
    let active = Memo::new(move |_| if presets_on.get() { Preset::parse(&preset.get()).unwrap_or(Preset::Freeform) } else { Preset::Freeform });
    let preset_list = Memo::new(move |_| data.with(|d| d.presets.iter().filter_map(|p| Preset::parse(&p.id)).collect::<Vec<_>>()));
    let preset_name = move |id: &str| {
        let pid = Preset::parse(id).map(|p| p.id()).unwrap_or_default();
        data.with(|d| d.presets.iter().find(|p| Preset::parse(&p.id).map(|x| x.id()) == Some(pid.clone())).map(|p| p.name.clone()))
            .unwrap_or_else(|| Preset::parse(id).unwrap_or(Preset::Freeform).display_name(t("Frei", "Freeform")))
    };

    let ready = move || {
        let s = data.with_untracked(|d| d.session);
        api::fire("capture", "overlayReady", json!({ "session": s }));
    };

    let reset = move |d: Data| {
        window_mode.set(d.mode == "window");
        preset.set(if d.preset.is_empty() { "freeform".into() } else { d.preset.clone() });
        phase.set(Phase::Idle);
        sel.set(None);
        menu.set(None);
        more.set(false);
        annot.set(None);
        shift.set(false);
        center.set(false);
        space.set(false);
        multi.set(vec![]);
        pending.set_value(None);
        op.set_value(Op::None);
        sent.set_value(false);
        mouse.set(d.cursor);
        let no_image = d.frozen.is_none();
        data.set(d);
        if no_image {
            ready();
        }
    };
    spawn_local(async move {
        if let Ok(d) = serde_json::from_value::<Data>(api::window_data().await) {
            reset(d);
        }
    });
    api::on("window-data", move |v| {
        if let Ok(d) = serde_json::from_value::<Data>(v) {
            reset(d);
        }
    });

    // Größenvorgabe auf allen Bildschirmen synchron halten (Capso `capturePresetChanged`).
    Effect::new(move |_| {
        let v = api::settings().with(|s| s["capturePreset"].as_str().map(str::to_owned));
        if let Some(id) = v.filter(|id| !id.is_empty()) {
            if id != preset.get_untracked() {
                preset.set(id);
            }
        }
    });
    // Aufnahme-Auswahl: Bereich/Fenster auf allen Bildschirmen synchron (gespeichert in recordingSelectsWindow).
    Effect::new(move |_| {
        let w = api::settings().with(|s| s["recordingSelectsWindow"].as_bool());
        if let Some(w) = w {
            if recording.get_untracked() && phase.get_untracked() == Phase::Idle && window_mode.get_untracked() != w {
                window_mode.set(w);
            }
        }
    });

    // Ergebnis ans Backend; Zustand kurz danach zurücksetzen (Fenster ist dann schon versteckt).
    let finish_full = move |kind: &'static str, r: Option<Rect>, wid: Option<u32>, action: Option<String>, data_url: Option<String>, ids: Vec<u32>| {
        if sent.get_value() {
            return;
        }
        sent.set_value(true);
        let (session, screen) = data.with_untracked(|d| (d.session, d.screen.id));
        let r = r.unwrap_or_default();
        api::fire(
            "capture",
            "overlayResult",
            json!({ "session": session, "kind": kind, "screen": screen, "x": r.x, "y": r.y, "width": r.w, "height": r.h,
                    "windowId": wid, "action": action, "dataUrl": data_url, "windowIds": ids }),
        );
        set_timeout(
            move || {
                phase.set(Phase::Idle);
                sel.set(None);
                annot.set(None);
                menu.set(None);
                more.set(false);
                multi.set(vec![]);
            },
            Duration::from_millis(200),
        );
    };
    let finish = move |kind: &'static str, r: Option<Rect>, wid: Option<u32>, action: Option<String>, data_url: Option<String>| {
        finish_full(kind, r, wid, action, data_url, vec![])
    };
    // ⇧ losgelassen: gesammelte Fenster aufnehmen (von vorne nach hinten sortiert).
    let confirm_multi = move || {
        let ids = multi.get_untracked();
        if ids.is_empty() {
            return;
        }
        let wins: Vec<WinInfo> = data.with_untracked(|d| d.windows.iter().filter(|w| ids.contains(&w.id)).cloned().collect());
        let a = all_in_one.get_untracked().then(|| "default".to_string());
        match wins.as_slice() {
            [] => multi.set(vec![]),
            [w] => finish("window", Some(Rect::new(w.x, w.y, w.width, w.height)), Some(w.id), a, None),
            _ => {
                let r = wins.iter().map(|w| Rect::new(w.x, w.y, w.width, w.height)).reduce(|a, b| {
                    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
                    Rect::new(x, y, a.max_x().max(b.max_x()) - x, a.max_y().max(b.max_y()) - y)
                });
                finish_full("windows", r, None, a, None, wins.iter().map(|w| w.id).collect());
            }
        }
    };
    let cancel = move || finish("cancel", None, None, None, None);
    let act = move |a: &'static str| {
        if let Some(r) = sel.get_untracked() {
            finish("area", Some(r), None, Some(a.into()), None);
        }
    };
    let confirm = move || {
        if let Some(r) = sel.get_untracked() {
            let a = if all_in_one.get_untracked() { Some("copy".to_string()) } else { None };
            finish("area", Some(r), None, a, None);
        }
    };

    let ratio_now = move || geom::selection_ratio(active.get_untracked().drag_ratio(), shift.get_untracked());
    // Capso `isCenterLocked = squareLock && centerLockKeyHeld`: nur mit gehaltener Umschalttaste.
    let center_on = move || center.get_untracked() && shift.get_untracked();
    let recompute = move || {
        if let Op::Create(start) = op.get_value() {
            if sel.get_untracked().is_some() {
                let r = geom::drag_rect(start, last_raw.get_value(), ratio_now(), center_on(), &bounds.get_untracked());
                sel.set(Some(r));
            }
        }
    };

    let hover_win = Memo::new(move |_| {
        if phase.get() != Phase::Idle || menu.get().is_some() {
            return None;
        }
        if !window_mode.get() && active.get().is_fixed() {
            return None;
        }
        let p = mouse.get()?;
        data.with(|d| d.windows.iter().position(|w| w.visible_rect().contains(p)))
    });
    let win_at = move |p: (f64, f64)| data.with_untracked(|d| d.windows.iter().find(|w| w.visible_rect().contains(p)).cloned());

    let choose_preset = move |id: String| {
        preset.set(id.clone());
        api::set_setting("capturePreset", json!(id));
        menu.set(None);
        if phase.get_untracked() == Phase::Adjust {
            if let (Some(r), Some(p)) = (sel.get_untracked(), Preset::parse(&id)) {
                let b = bounds.get_untracked();
                match p {
                    Preset::Fixed(..) => {
                        let (w, h) = p.fixed().unwrap();
                        sel.set(Some(geom::fixed_rect(w, h, r.mid(), &b)));
                    }
                    Preset::Ratio(..) => sel.set(Some(geom::fit_ratio(&r, p.ratio().unwrap(), &b, 10.0))),
                    Preset::Freeform => {}
                }
            }
        }
    };

    let start_annotation = move || {
        let Some(r) = sel.get_untracked() else { return };
        let Some(frozen) = data.with_untracked(|d| d.frozen.clone()) else { return };
        let sw = data.with_untracked(|d| d.screen.width.max(1.0));
        let k = if natural_w.get_value() > 0.0 { natural_w.get_value() / sw } else { data.with_untracked(|d| d.screen.scale.max(1.0)) };
        spawn_local(async move {
            let a = json!({ "path": frozen, "x": r.x * k, "y": r.y * k, "width": r.w * k, "height": r.h * k });
            if let Ok(Value::String(p)) = api::rpc("capture", "cropFrozen", a).await {
                // Auswahl hat sich inzwischen geändert (Ziehen) → verwerfen.
                if sel.get_untracked() != Some(r) || matches!(op.get_value(), Op::Move(..) | Op::Resize(..) | Op::Create(_)) {
                    return;
                }
                annot.set(Some((api::asset_url(&p), k)));
                phase.set(Phase::Annotate);
            }
        });
    };
    // Auswahl fertig: Anpassen-Phase; im All-in-One gleich die Annotationsfläche (Capso `showAnnotationOverlayIfPossible`).
    let selected = move || {
        phase.set(Phase::Adjust);
        if all_in_one.get_untracked() {
            start_annotation();
        }
    };
    // Aktion der Seitenleiste: mit aktiver Annotation erst das gerenderte Ergebnis holen
    // (Text übernehmen per ⌘↩, dann ⇧⌘C = „Bild kopieren“ im Annotator; Aktion wird ersetzt).
    let rail_act = move |a: &'static str| {
        if phase.get_untracked() == Phase::Annotate && annot.get_untracked().is_some() {
            pending.set_value(Some(a.to_string()));
            let send = |key: &str, shift: bool| {
                use wasm_bindgen::JsCast;
                let init = js_sys::Object::new();
                let set = |k: &str, v: wasm_bindgen::JsValue| {
                    let _ = js_sys::Reflect::set(&init, &k.into(), &v);
                };
                set("key", key.into());
                set("metaKey", true.into());
                set("shiftKey", shift.into());
                set("bubbles", true.into());
                set("cancelable", true.into());
                let Ok(ctor) = js_sys::Reflect::get(&window(), &"KeyboardEvent".into()) else { return };
                let args = js_sys::Array::of2(&"keydown".into(), &init);
                if let Ok(ev) = js_sys::Reflect::construct(ctor.unchecked_ref::<js_sys::Function>(), &args) {
                    let _ = window().dispatch_event(ev.unchecked_ref::<web_sys::Event>());
                }
            };
            send("Enter", false);
            send("C", true);
            return;
        }
        if let Some(r) = sel.get_untracked() {
            finish("area", Some(r), None, Some(a.into()), None);
        }
    };

    // ───────────── Maus ─────────────

    let pos = |e: &leptos::ev::MouseEvent| (e.client_x() as f64, e.client_y() as f64);

    let on_down = move |e: leptos::ev::MouseEvent| {
        if e.button() != 0 {
            return;
        }
        menu.set(None);
        let p = pos(&e);
        let b = bounds.get_untracked();
        shift.set(e.shift_key());
        if window_mode.get_untracked() {
            if let Some(w) = win_at(p) {
                // Capso `handleWindowClick`: ⇧-Klick sammelt Fenster, ⇧ loslassen nimmt sie auf.
                if e.shift_key() && data.with_untracked(|d| d.multi_window) {
                    multi.update(|m| match m.iter().position(|x| *x == w.id) {
                        Some(i) => {
                            m.remove(i);
                        }
                        None => m.push(w.id),
                    });
                    return;
                }
                let a = all_in_one.get_untracked().then(|| "default".to_string());
                finish("window", Some(Rect::new(w.x, w.y, w.width, w.height)), Some(w.id), a, None);
            }
            return;
        }
        match phase.get_untracked() {
            Phase::Idle => {
                if let Some((w, h)) = active.get_untracked().fixed() {
                    let r = geom::fixed_rect(w, h, p, &b);
                    if adjustable.get_untracked() {
                        sel.set(Some(r));
                        selected();
                    } else {
                        finish("area", Some(r), None, None, None);
                    }
                    return;
                }
                op.set_value(Op::Create(p));
                last_raw.set_value(p);
                sel.set(None);
                phase.set(Phase::Drag);
            }
            Phase::Adjust | Phase::Annotate => {
                let annotating = phase.get_untracked() == Phase::Annotate;
                let r = sel.get_untracked().unwrap_or_default();
                match geom::hit_target(p, &r, 6.0, 10.0) {
                    Some(Hit::Resize(h)) if !active.get_untracked().is_fixed() && !space.get_untracked() => {
                        op.set_value(Op::Resize(h, r));
                        annot.set(None);
                    }
                    // Beim Annotieren verschiebt nur die gehaltene Leertaste (sonst zeichnet der Annotator).
                    Some(Hit::Resize(_)) | Some(Hit::Move) if !annotating || space.get_untracked() => {
                        op.set_value(Op::Move(r, p));
                        annot.set(None);
                    }
                    Some(_) => {}
                    None => {
                        annot.set(None);
                        op.set_value(Op::Create(p));
                        last_raw.set_value(p);
                        sel.set(None);
                        phase.set(Phase::Drag);
                    }
                }
            }
            _ => {}
        }
    };

    let on_move = move |e: leptos::ev::MouseEvent| {
        let p = pos(&e);
        mouse.set(Some(p));
        if shift.get_untracked() != e.shift_key() {
            shift.set(e.shift_key());
        }
        // Mauszeiger auf diesem Bildschirm → Tastatur hierher holen (gedrosselt).
        if !document().has_focus().unwrap_or(true) {
            let now = js_sys::Date::now();
            if now - focus_asked.get_value() > 500.0 {
                focus_asked.set_value(now);
                api::fire("capture", "overlayFocus", Value::Null);
            }
        }
        let b = bounds.get_untracked();
        match op.get_value() {
            Op::Create(start) => {
                last_raw.set_value(p);
                if sel.get_untracked().is_none() && (p.0 - start.0).abs() + (p.1 - start.1).abs() < 3.0 {
                    return;
                }
                sel.set(Some(geom::drag_rect(start, p, ratio_now(), center_on(), &b)));
            }
            Op::Move(r0, p0) => sel.set(Some(geom::move_rect(&r0, p.0 - p0.0, p.1 - p0.1, &b))),
            Op::Resize(h, r0) => sel.set(Some(geom::resize(&r0, h, p, &b, 4.0, ratio_now()))),
            Op::None => {}
        }
    };

    let on_up = move |_e: leptos::ev::MouseEvent| match op.get_value() {
        Op::Create(start) => {
            op.set_value(Op::None);
            match sel.get_untracked().filter(|r| r.w > 5.0 && r.h > 5.0) {
                Some(r) => {
                    if adjustable.get_untracked() {
                        selected();
                    } else {
                        finish("area", Some(r), None, None, None);
                    }
                }
                None => {
                    sel.set(None);
                    phase.set(Phase::Idle);
                    // Klick ohne Ziehen: Fenster unter dem Mauszeiger.
                    if let Some(w) = win_at(start) {
                        if all_in_one.get_untracked() {
                            let r = w.visible_rect();
                            sel.set(Some(r));
                            selected();
                        } else {
                            finish("window", Some(Rect::new(w.x, w.y, w.width, w.height)), Some(w.id), None, None);
                        }
                    }
                }
            }
        }
        Op::Move(..) | Op::Resize(..) => {
            op.set_value(Op::None);
            // Annotationsfläche an der neuen Auswahl neu zuschneiden.
            if all_in_one.get_untracked() && matches!(phase.get_untracked(), Phase::Adjust | Phase::Annotate) {
                selected();
            }
        }
        Op::None => {}
    };

    let on_dbl = move |e: leptos::ev::MouseEvent| {
        if phase.get_untracked() == Phase::Adjust && sel.get_untracked().is_some_and(|r| r.contains(pos(&e))) {
            confirm();
        }
    };

    let on_ctx = move |e: leptos::ev::MouseEvent| {
        e.prevent_default();
        match phase.get_untracked() {
            Phase::Drag => {
                op.set_value(Op::None);
                sel.set(None);
                phase.set(Phase::Idle);
            }
            Phase::Idle if presets_on.get_untracked() && !window_mode.get_untracked() => menu.set(Some(pos(&e))),
            Phase::Annotate => {}
            _ => cancel(),
        }
    };

    // ───────────── Tastatur ─────────────

    let is_center_key = move |e: &leptos::ev::KeyboardEvent| {
        let k = data.with_untracked(|d| if d.square_key.is_empty() { "C".to_string() } else { d.square_key.to_uppercase() });
        e.key().to_uppercase() == k && !e.meta_key() && !e.alt_key() && !e.ctrl_key()
    };

    let in_text_field = |e: &leptos::ev::KeyboardEvent| {
        e.target()
            .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
            .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA") || el.closest("[contenteditable]").ok().flatten().is_some())
    };
    let kd = window_event_listener(leptos::ev::keydown, move |e| {
        // Leertaste halten = Auswahl verschieben (All-in-One, auch während der Annotation).
        if e.key() == " " && all_in_one.get_untracked() && matches!(phase.get_untracked(), Phase::Adjust | Phase::Annotate) && !in_text_field(&e) {
            e.prevent_default();
            if !space.get_untracked() {
                space.set(true);
            }
            return;
        }
        if phase.get_untracked() == Phase::Annotate {
            return;
        }
        let key = e.key();
        // Aufnahme-Modusleiste (Capso `RecordingSelectionModeWindow`): A / W / F / R, Leertaste = Bereich ⇄ Fenster.
        if recording.get_untracked() && phase.get_untracked() == Phase::Idle && !e.meta_key() && !e.ctrl_key() && !e.alt_key() {
            let set_window = move |w: bool| {
                window_mode.set(w);
                api::set_setting("recordingSelectsWindow", json!(w));
            };
            match (key.as_str(), e.code().as_str()) {
                (" ", _) => {
                    e.prevent_default();
                    set_window(!window_mode.get_untracked());
                    return;
                }
                (_, "KeyA") => return set_window(false),
                (_, "KeyW") => return set_window(true),
                (_, "KeyF") => return finish("fullscreen", None, None, None, None),
                (_, "KeyR") => {
                    if data.with_untracked(|d| d.can_last_area) {
                        finish("lastArea", None, None, None, None);
                    }
                    return;
                }
                _ => {}
            }
        }
        if key == "Shift" {
            shift.set(true);
            recompute();
            return;
        }
        if is_center_key(&e) && phase.get_untracked() == Phase::Drag && !e.alt_key() {
            if !e.repeat() {
                center.set(true);
                recompute();
            }
            e.prevent_default();
            return;
        }
        let aio_sel = all_in_one.get_untracked() && phase.get_untracked() == Phase::Adjust;
        if e.meta_key() && aio_sel {
            let a = match key.to_lowercase().as_str() {
                "c" => Some("copy"),
                "s" => Some("save"),
                "p" => Some("pin"),
                _ => None,
            };
            if let Some(a) = a {
                e.prevent_default();
                act(a);
            }
            return;
        }
        match key.as_str() {
            "Escape" => {
                if menu.get_untracked().is_some() || more.get_untracked() {
                    menu.set(None);
                    more.set(false);
                } else {
                    cancel();
                }
            }
            " " => {
                e.prevent_default();
                if phase.get_untracked() == Phase::Idle {
                    window_mode.update(|w| *w = !*w);
                }
            }
            "Enter" => {
                if phase.get_untracked() == Phase::Adjust {
                    confirm();
                }
            }
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" if phase.get_untracked() == Phase::Adjust => {
                e.prevent_default();
                let d = if e.shift_key() { 10.0 } else { 1.0 };
                let (dx, dy) = match key.as_str() {
                    "ArrowLeft" => (-d, 0.0),
                    "ArrowRight" => (d, 0.0),
                    "ArrowUp" => (0.0, -d),
                    _ => (0.0, d),
                };
                if let Some(r) = sel.get_untracked() {
                    sel.set(Some(geom::move_rect(&r, dx, dy, &bounds.get_untracked())));
                }
            }
            _ if e.code() == "KeyR" && !e.meta_key() && !e.ctrl_key() => {
                if presets_on.get_untracked() && !window_mode.get_untracked() && phase.get_untracked() != Phase::Drag {
                    let next = geom::cycle_preset(&preset_list.get_untracked(), &preset.get_untracked(), !e.shift_key());
                    choose_preset(next.id());
                }
            }
            _ => {}
        }
    });
    let ku = window_event_listener(leptos::ev::keyup, move |e| {
        if e.key() == " " {
            space.set(false);
        } else if e.key() == "Shift" {
            shift.set(false);
            recompute();
            if window_mode.get_untracked() && !multi.get_untracked().is_empty() {
                confirm_multi();
            }
        } else if is_center_key(&e) {
            center.set(false);
            recompute();
        }
    });
    on_cleanup(move || {
        kd.remove();
        ku.remove();
    });

    // ───────────── Darstellung ─────────────

    let cursor = move || match phase.get() {
        _ if space.get() && matches!(phase.get(), Phase::Adjust | Phase::Annotate) => "move".to_string(),
        Phase::Annotate => match (mouse.get(), sel.get()) {
            (Some(p), Some(r)) => match geom::hit_target(p, &r, 6.0, 10.0) {
                Some(Hit::Resize(h)) if !active.get().is_fixed() => h.cursor().into(),
                _ => "default".into(),
            },
            _ => "default".into(),
        },
        Phase::Drag => "none".into(),
        Phase::Adjust => match (mouse.get(), sel.get()) {
            (Some(p), Some(r)) => match geom::hit_target(p, &r, 6.0, 10.0) {
                Some(Hit::Resize(h)) if !active.get().is_fixed() => h.cursor().into(),
                Some(_) => "move".into(),
                None => "crosshair".into(),
            },
            _ => "crosshair".into(),
        },
        Phase::Idle if window_mode.get() => "default".into(),
        Phase::Idle => "none".into(),
    };

    // Heller Ausschnitt: Auswahl, Vorschau der festen Größe oder (Fenstermodus) Fenster unter dem Mauszeiger.
    let cut = Memo::new(move |_| match (phase.get(), sel.get()) {
        (Phase::Drag | Phase::Adjust | Phase::Annotate, Some(r)) => Some((r, true)),
        (Phase::Idle, _) if window_mode.get() => hover_win.get().and_then(|i| data.with(|d| d.windows.get(i).map(|w| (w.visible_rect(), false)))),
        (Phase::Idle, _) => {
            let (w, h) = active.get().fixed()?;
            Some((geom::fixed_rect(w, h, mouse.get()?, &bounds.get()), true))
        }
        _ => None,
    });

    let frozen_url = Memo::new(move |_| data.with(|d| d.frozen.as_deref().map(api::asset_url)));

    let reticle_at = move || -> Option<(f64, f64)> {
        if menu.get().is_some() {
            return None;
        }
        match phase.get() {
            Phase::Idle if !window_mode.get() && !active.get().is_fixed() => mouse.get(),
            Phase::Drag => {
                let p = mouse.get()?;
                let (Op::Create(start), Some(r)) = (op.get_value(), sel.get()) else { return Some(p) };
                Some((if p.0 >= start.0 { r.max_x() } else { r.x }, if p.1 >= start.1 { r.max_y() } else { r.y }))
            }
            _ => None,
        }
    };

    let dim_view = move || match cut.get() {
        Some((r, is_sel)) => {
            let border = if is_sel { "border:1.5px solid rgba(255,255,255,.9);" } else { "border:2px solid rgba(64,156,255,.95);background:rgba(64,156,255,.18);" };
            view! {
                <div class="pointer-events-none absolute"
                    style=format!("left:{}px;top:{}px;width:{}px;height:{}px;{border}box-shadow:0 0 10px rgba(0,0,0,.5),0 0 0 100000px {DIM};", r.x, r.y, r.w, r.h)>
                    <div class="absolute inset-0" style="box-shadow:inset 0 0 0 1px rgba(0,0,0,.18)"></div>
                </div>
            }
            .into_any()
        }
        None => view! { <div class="pointer-events-none absolute inset-0" style=format!("background:{DIM}")></div> }.into_any(),
    };

    // Fenster im Bereichsmodus dezent markieren (Klick = Fenster).
    let hover_outline = move || {
        if window_mode.get() {
            return None;
        }
        let i = hover_win.get()?;
        let r = data.with(|d| d.windows.get(i).map(|w| w.visible_rect()))?;
        Some(view! {
            <div class="pointer-events-none absolute rounded-[3px]"
                style=format!("left:{}px;top:{}px;width:{}px;height:{}px;border:1px dashed rgba(255,255,255,.55);background:rgba(255,255,255,.04)", r.x, r.y, r.w, r.h)></div>
        })
    };

    let window_label = move || {
        if !window_mode.get() {
            return None;
        }
        let i = hover_win.get()?;
        let (r, name) = data.with(|d| {
            let w = d.windows.get(i)?;
            Some((w.visible_rect(), if w.title.is_empty() { w.app.clone() } else { format!("{} – {}", w.app, w.title) }))
        })?;
        Some(view! {
            <div class="pointer-events-none absolute flex -translate-x-1/2 -translate-y-1/2 items-center gap-1.5 rounded-lg bg-zinc-900/85 px-3 py-1.5 text-[12px] font-medium text-white shadow-lg max-w-[60%]"
                style=format!("left:{}px;top:{}px", r.x + r.w / 2.0, r.y + r.h / 2.0)>
                <span class="truncate">{name}</span>
                <span class="text-white/50 tabular-nums">{geom::dimension_text(r.w, r.h)}</span>
            </div>
        })
    };

    let dimension_label = move || {
        let (r, _) = cut.get().filter(|(_, s)| *s)?;
        if phase.get() == Phase::Annotate {
            return None;
        }
        let b = bounds.get();
        let badge = active.get().badge();
        let fixed = active.get().is_fixed();
        let text = geom::dimension_text(r.w, r.h);
        let aio = all_in_one.get();
        // Capso: unter der Auswahl mittig; All-in-One: oben links (Leiste sitzt unten).
        let (x, y, tx) = if aio {
            let (x, y) = geom::dimension_label_origin(&r, (130.0, 22.0), &b);
            (x, y, "0")
        } else if r.max_y() + 34.0 <= b.max_y() {
            (r.x + r.w / 2.0, r.max_y() + 8.0, "-50%")
        } else {
            (r.x + r.w / 2.0, (r.y - 30.0).max(b.y + 4.0), "-50%")
        };
        Some(view! {
            <div class="pointer-events-none absolute flex items-center gap-1.5" style=format!("left:{x}px;top:{y}px;transform:translateX({tx})")>
                <span class="rounded bg-zinc-900/80 px-1.5 py-0.5 font-mono text-[12px] text-white tabular-nums">{text}</span>
                {badge.map(|b| view! {
                    <span class="rounded px-1 py-0.5 font-mono text-[10px] font-semibold text-white"
                        class=("bg-green-600", fixed) class=("bg-blue-500", !fixed)>{b}</span>
                })}
            </div>
        })
    };

    let reticle = move || {
        let (x, y) = reticle_at()?;
        let arm = |x1: f64, y1: f64, x2: f64, y2: f64| {
            view! {
                <line x1=x1 y1=y1 x2=x2 y2=y2 stroke="rgba(0,0,0,.5)" stroke-width="2.5" />
                <line x1=x1 y1=y1 x2=x2 y2=y2 stroke="white" stroke-width="1.25" />
            }
        };
        let (g, l) = (2.5, 12.0);
        Some(view! {
            <svg class="pointer-events-none absolute inset-0 size-full">
                {arm(x, y - g, x, y - g - l)}
                {arm(x, y + g, x, y + g + l)}
                {arm(x - g, y, x - g - l, y)}
                {arm(x + g, y, x + g + l, y)}
            </svg>
        })
    };

    let coord_label = move || {
        if phase.get() != Phase::Idle || window_mode.get() || active.get().is_fixed() || data.with(|d| d.magnifier && d.frozen.is_some()) {
            return None;
        }
        let (x, y) = mouse.get()?;
        let b = bounds.get();
        let (lx, ly) = (if x + 110.0 > b.max_x() { x - 100.0 } else { x + 14.0 }, if y + 40.0 > b.max_y() { y - 30.0 } else { y + 14.0 });
        Some(view! {
            <div class="pointer-events-none absolute rounded bg-zinc-900/75 px-1.5 py-0.5 font-mono text-[11px] text-white tabular-nums"
                style=format!("left:{lx}px;top:{ly}px")>{format!("{:.0}, {:.0}", x, y)}</div>
        })
    };

    // Lupe: vergrößerter Ausschnitt aus dem Standbild.
    let magnifier = move || {
        let url = frozen_url.get()?;
        if !data.with(|d| d.magnifier) || !matches!(phase.get(), Phase::Idle | Phase::Drag) || window_mode.get() || menu.get().is_some() {
            return None;
        }
        let (x, y) = reticle_at().or_else(|| mouse.get())?;
        let b = bounds.get();
        let lx = if x + 24.0 + MAG > b.max_x() { x - 24.0 - MAG } else { x + 24.0 };
        let ly = if y + 24.0 + MAG + 22.0 > b.max_y() { y - 24.0 - MAG - 22.0 } else { y + 24.0 };
        let style = format!(
            "left:{lx}px;top:{ly}px;width:{MAG}px;height:{MAG}px;background-image:url('{url}');background-repeat:no-repeat;\
             background-size:{}px {}px;background-position:{}px {}px;image-rendering:pixelated",
            b.w * ZOOM,
            b.h * ZOOM,
            -(x * ZOOM - MAG / 2.0),
            -(y * ZOOM - MAG / 2.0)
        );
        Some(view! {
            <div class="pointer-events-none absolute overflow-hidden rounded-full border-2 border-white/90 bg-black shadow-xl" style=style>
                <div class="absolute left-1/2 top-0 h-full w-px -translate-x-1/2 bg-blue-400/50"></div>
                <div class="absolute left-0 top-1/2 h-px w-full -translate-y-1/2 bg-blue-400/50"></div>
                <div class="absolute left-1/2 top-1/2 size-1 -translate-x-1/2 -translate-y-1/2 outline outline-1 outline-white"></div>
            </div>
            <div class="pointer-events-none absolute rounded bg-zinc-900/80 px-1.5 py-0.5 text-center font-mono text-[11px] text-white tabular-nums"
                style=format!("left:{lx}px;top:{}px;width:{MAG}px", ly + MAG + 4.0)>{format!("{:.0}, {:.0}", x, y)}</div>
        })
    };

    let top_inset = move || if data.with(|d| d.screen.y == 0.0 && d.screen.x == 0.0) { 40.0 } else { 16.0 };

    let preset_badge = move || {
        if !presets_on.get() || window_mode.get() || phase.get() != Phase::Idle || all_in_one.get() {
            return None;
        }
        let name = preset_name(&preset.get());
        Some(view! {
            <div class="pointer-events-none absolute left-1/2 -translate-x-1/2 rounded-full bg-black/75 px-3.5 py-1.5 text-[12px] text-white"
                style=format!("top:{}px", top_inset())>
                <span class="font-semibold">{name}</span>
                <span class="text-white/60">{move || format!("   {}", t("R zum Wechseln", "R to change"))}</span>
            </div>
        })
    };

    let mode_hint = move || {
        if phase.get() != Phase::Idle || all_in_one.get() {
            return None;
        }
        let text = if window_mode.get() {
            t("Klicke auf ein Fenster · Leertaste: Bereich · Esc: Abbrechen", "Click a window · Space: area · Esc: cancel")
        } else {
            t("Ziehen: Bereich · Klick: Fenster · Leertaste: Fenster · Esc: Abbrechen", "Drag: area · Click: window · Space: window · Esc: cancel")
        };
        Some(view! {
            <div class="pointer-events-none absolute bottom-6 left-1/2 -translate-x-1/2 rounded-full bg-black/70 px-3.5 py-1.5 text-[12px] text-white/85">{text}</div>
        })
    };

    let handles = move || {
        let ph = phase.get();
        if !matches!(ph, Phase::Adjust | Phase::Annotate) {
            return None;
        }
        let r = sel.get()?;
        // Beim Annotieren liegen die Griffe über der Zeichenfläche und fangen die Maus selbst (Größe bleibt änderbar).
        let live = ph == Phase::Annotate;
        Some(view! {
            {geom::visible_handles(&r, active.get().is_fixed())
                .into_iter()
                .map(|h| {
                    let (x, y) = h.point(&r);
                    view! {
                        <div class="absolute z-20 flex size-[16px] -translate-x-1/2 -translate-y-1/2 items-center justify-center"
                            class=("pointer-events-none", !live)
                            style=format!("left:{x}px;top:{y}px;cursor:{}", h.cursor())>
                            <div class="size-[9px] rounded-full border border-black/30 bg-white shadow"></div>
                        </div>
                    }
                })
                .collect_view()}
            {(live && space.get()).then(|| view! {
                <div class="absolute z-20" style=format!("left:{}px;top:{}px;width:{}px;height:{}px;cursor:move", r.x, r.y, r.w, r.h)></div>
            })}
        })
    };

    // ⇧-Klick-Fenster markieren (+ Anzahl).
    let multi_view = move || {
        let ids = multi.get();
        if ids.is_empty() || !window_mode.get() {
            return None;
        }
        let rects: Vec<Rect> = data.with(|d| d.windows.iter().filter(|w| ids.contains(&w.id)).map(|w| w.visible_rect()).collect());
        let (mx, my) = mouse.get().unwrap_or((40.0, 40.0));
        Some(view! {
            {rects.into_iter().map(|r| view! {
                <div class="pointer-events-none absolute" style=format!("left:{}px;top:{}px;width:{}px;height:{}px;border:2px solid rgba(64,156,255,.95);background:rgba(64,156,255,.28)", r.x, r.y, r.w, r.h)></div>
            }).collect_view()}
            <div class="pointer-events-none absolute flex size-6 items-center justify-center rounded-full bg-blue-500 text-[12px] font-semibold text-white shadow-lg"
                style=format!("left:{}px;top:{}px", mx + 14.0, my + 14.0)>{ids.len()}</div>
        })
    };

    let stop = |e: leptos::ev::MouseEvent| e.stop_propagation();

    // Bestätigen-Leiste (Aufnahme / Scroll-Aufnahme).
    let confirm_bar = move || {
        if phase.get() != Phase::Adjust || all_in_one.get() {
            return None;
        }
        let r = sel.get()?;
        let (x, y) = geom::place_near(&r, (300.0, 40.0), &bounds.get(), 12.0);
        let label = data.with(|d| d.confirm_label.clone());
        Some(view! {
            <div class="absolute flex w-[300px] items-center justify-center gap-2" style=format!("left:{x}px;top:{y}px;cursor:default")
                on:mousedown=stop on:mouseup=stop on:dblclick=stop>
                <button class="rounded-lg bg-zinc-900/85 px-3 py-1.5 text-[13px] text-white shadow-lg hover:bg-zinc-800" on:click=move |_| cancel()>
                    {move || t("Abbrechen", "Cancel")}
                </button>
                <button class="flex items-center gap-1.5 rounded-lg bg-blue-500 px-3 py-1.5 text-[13px] font-medium text-white shadow-lg hover:bg-blue-400" on:click=move |_| confirm()>
                    {label}<span class="text-white/70">"↵"</span>
                </button>
            </div>
        })
    };

    // Preset-Menü (Rechtsklick bzw. Knopf in der Leiste).
    // Capso `showPresetMenu`: Abschnitte „Seitenverhältnisse“ und „Feste Größen“, dann „Presets verwalten…“.
    let preset_menu = move || {
        let (x, y) = menu.get()?;
        let b = bounds.get();
        let items = data.with(|d| d.presets.clone());
        let is_fixed = |p: &PresetInfo| Preset::parse(&p.id).is_some_and(|x| x.is_fixed());
        let (fixed, ratios): (Vec<PresetInfo>, Vec<PresetInfo>) = items.into_iter().partition(is_fixed);
        let h = (fixed.len() + ratios.len()) as f64 * 26.0 + 2.0 * 26.0 + 26.0 + 24.0;
        let (x, y) = (x.min(b.max_x() - 230.0), if y + h > b.max_y() { (y - h).max(4.0) } else { y });
        let cur = Preset::parse(&preset.get()).map(|p| p.id()).unwrap_or_default();
        let entry = move |p: PresetInfo| {
            let on = Preset::parse(&p.id).map(|x| x.id()) == Some(cur.clone());
            let id = p.id.clone();
            view! {
                <button class="flex w-full items-center gap-2 px-3 py-1 text-left hover:bg-blue-500" on:click=move |_| choose_preset(id.clone())>
                    <span class="w-3 [&_svg]:size-3">{on.then(|| view! { <Icon d=ui::CHECK /> })}</span>
                    <span>{p.name.clone()}</span>
                </button>
            }
        };
        let header = |text: String| view! { <div class="px-3 pb-0.5 pt-1 text-[10px] font-semibold uppercase tracking-wide text-white/40">{text}</div> };
        let has_ratios = !ratios.is_empty();
        let has_fixed = !fixed.is_empty();
        Some(view! {
            <div class="absolute z-30 w-[220px] rounded-lg border border-white/10 bg-zinc-900/95 py-1 text-[13px] text-white shadow-2xl backdrop-blur"
                style=format!("left:{x}px;top:{y}px;cursor:default") on:mousedown=stop on:mouseup=stop on:contextmenu=|e| e.prevent_default()>
                {has_ratios.then(|| header(t("Seitenverhältnisse", "Aspect Ratios").to_string()))}
                {ratios.into_iter().map(entry.clone()).collect_view()}
                {has_fixed.then(|| view! { <div class="my-1 h-px bg-white/10"></div> })}
                {has_fixed.then(|| header(t("Feste Größen", "Fixed Sizes").to_string()))}
                {fixed.into_iter().map(entry).collect_view()}
                <div class="my-1 h-px bg-white/10"></div>
                <button class="flex w-full items-center gap-2 px-3 py-1 text-left hover:bg-blue-500"
                    on:click=move |_| {
                        cancel();
                        api::fire("capture", "managePresets", Value::Null);
                    }>
                    <span class="w-3"></span><span>{t("Presets verwalten …", "Manage Presets…")}</span>
                </button>
            </div>
        })
    };

    // ───────────── All-in-One-Seitenleiste (Capso `sideRail`) ─────────────

    let has_sel = move || matches!(phase.get(), Phase::Adjust | Phase::Annotate) && sel.get().is_some();
    // Capso: mit Standbild startet die Leiste kompakt (Kopieren/Sichern/Anheften/Schließen + „Mehr“).
    let compact = Memo::new(move |_| data.with(|d| d.frozen.is_some()));
    let rail_h = move || {
        let expanded = !compact.get() || more.get();
        let mut items: Vec<f64> = vec![];
        if expanded {
            items.extend([38.0; 7]);
            items.push(5.0);
            if presets_on.get() {
                items.push(42.0);
            }
        }
        items.extend([38.0; 4]);
        if compact.get() {
            items.push(5.0);
            items.push(38.0);
        }
        20.0 + items.iter().sum::<f64>() + 7.0 * (items.len().saturating_sub(1)) as f64
    };
    let rail_pos = move || {
        let b = bounds.get();
        let r = sel.get().unwrap_or_default();
        let (margin, gap) = (12.0, 10.0);
        let top = top_inset();
        let h = rail_h().min(b.h - top - margin);
        let x = if r.max_x() + gap + RAIL_W <= b.max_x() - margin {
            r.max_x() + gap
        } else if r.x - RAIL_W - gap >= margin {
            r.x - RAIL_W - gap
        } else {
            b.max_x() - RAIL_W - margin
        };
        let y = (r.y + r.h / 2.0 - h / 2.0).max(top).min(b.max_y() - h - margin);
        (x, y, h)
    };

    let rail_btn = move |icon: Option<&'static str>, text_icon: Option<&'static str>, label: &'static str, title: String, primary: bool, on: Box<dyn Fn() + Send + Sync>| {
        view! {
            <button title=title
                class="flex h-[38px] w-full flex-col items-center justify-center gap-0.5 rounded-lg text-white/90 transition-colors [&_svg]:size-[16px]"
                class=("bg-blue-500", primary) class=("hover:bg-blue-400", primary) class=("hover:bg-white/10", !primary)
                on:click=move |_| on()>
                {icon.map(|d| view! { <Icon d=d /> })}
                {text_icon.map(|s| view! { <span class="text-[14px] font-semibold leading-[16px]">{s}</span> })}
                <span class="text-[10px] leading-none">{label}</span>
            </button>
        }
    };
    let divider = || view! { <div class="mx-2 h-px shrink-0 bg-white/15"></div> };

    let toolbar = move || {
        if !all_in_one.get() || !has_sel() || matches!(op.get_value(), Op::Move(..) | Op::Resize(..)) {
            return None;
        }
        let (x, y, h) = rail_pos();
        let lang_de = api::lang() == "de";
        let l = move |de: &'static str, en: &'static str| if lang_de { de } else { en };
        let expanded = !compact.get() || more.get();
        let reset_area = move || {
            annot.set(None);
            window_mode.set(false);
            sel.set(None);
            phase.set(Phase::Idle);
        };
        Some(view! {
            <div class="absolute z-30 flex flex-col gap-[7px] overflow-y-auto rounded-2xl border border-white/15 bg-zinc-900/90 px-2 py-[10px] text-white shadow-2xl backdrop-blur-xl"
                style=format!("left:{x}px;top:{y}px;width:{RAIL_W}px;max-height:{h}px;cursor:default")
                on:mousedown=stop on:mouseup=stop on:dblclick=stop on:contextmenu=|e| e.prevent_default()>
                {expanded.then(|| view! {
                    {rail_btn(Some(ui::SQUARE_DASHED), None, l("Bereich", "Area"), l("Bereich", "Area").into(), false, Box::new(reset_area))}
                    {rail_btn(Some(ui::MONITOR), None, l("Vollbild", "Full"), l("Vollbild", "Fullscreen").into(), false, Box::new(move || finish("fullscreen", None, None, Some("default".into()), None)))}
                    {rail_btn(Some(ui::APP_WINDOW), None, l("Fenster", "Window"), l("Fenster", "Window").into(), false, Box::new(move || { annot.set(None); sel.set(None); phase.set(Phase::Idle); window_mode.set(true); }))}
                    {rail_btn(Some(ui::SCROLL), None, l("Scrollen", "Scroll"), l("Scroll-Aufnahme", "Scrolling").into(), false, Box::new(move || act("scroll")))}
                    {rail_btn(Some(ui::TIMER), None, "Timer", l("Selbstauslöser", "Timer").into(), false, Box::new(move || act("timer")))}
                    {rail_btn(None, Some("Aa"), "OCR", l("Text erkennen", "OCR").into(), false, Box::new(move || rail_act("ocr")))}
                    {rail_btn(Some(ui::VIDEO), None, l("Aufnahme", "Record"), l("Bildschirmaufnahme", "Recording").into(), false, Box::new(move || act("record")))}
                    {divider()}
                    <Show when=move || presets_on.get()>
                        <button class="flex h-[42px] w-full flex-col items-center justify-center rounded-lg bg-white/10 px-1 text-[11px] leading-tight hover:bg-white/15"
                            title=l("Größenvorgabe", "Capture Preset")
                            on:click=move |e| {
                                e.stop_propagation();
                                let (x, y, _) = rail_pos();
                                let b = bounds.get_untracked();
                                let mx = if x + RAIL_W + 230.0 <= b.max_x() { x + RAIL_W + 8.0 } else { x - 228.0 };
                                menu.set(if menu.get_untracked().is_some() { None } else { Some((mx, y)) });
                            }>
                            <span class="w-full truncate text-center">{move || preset_name(&preset.get())}</span>
                            <span class="text-white/50">"▾"</span>
                        </button>
                    </Show>
                })}
                {rail_btn(Some(ui::COPY), None, l("Kopieren", "Copy"), format!("{} (⌘C / ↵)", l("Bild kopieren und schließen", "Copy Image and Close")), true, Box::new(move || rail_act("copy")))}
                {rail_btn(Some(ui::SAVE), None, l("Sichern", "Save"), format!("{} (⌘S)", l("Auswahl sichern", "Save selected area")), false, Box::new(move || rail_act("save")))}
                {rail_btn(Some(ui::PIN), None, l("Anheften", "Pin"), format!("{} (⌘P)", l("Auswahl anheften", "Pin selected area")), false, Box::new(move || rail_act("pin")))}
                {rail_btn(Some(ui::X), None, l("Schließen", "Close"), format!("{} (Esc)", l("Abbrechen", "Cancel")), false, Box::new(move || cancel()))}
                {compact.get().then(|| view! {
                    {divider()}
                    {rail_btn(Some(ui::MORE), None, if more.get() { l("Weniger", "Less") } else { l("Mehr", "More") },
                        if more.get() { l("Aktionen ausblenden", "Hide actions").to_string() } else { l("Weitere Aktionen", "More actions").to_string() },
                        false, Box::new(move || more.update(|m| *m = !*m)))}
                })}
            </div>
        })
    };

    // Aufnahme-Modusleiste (Capso `RecordingSelectionModeWindow`, 520×68 oben mittig, nur auf dem Bildschirm mit Mauszeiger).
    let rec_panel = move || {
        if !recording.get() || !data.with(|d| d.focused) || phase.get() != Phase::Idle {
            return None;
        }
        let b = bounds.get();
        let (x, y) = ((b.w - REC_PANEL.0) / 2.0, top_inset() + 24.0);
        let can_last = data.with(|d| d.can_last_area);
        let set_window = move |w: bool| {
            window_mode.set(w);
            api::set_setting("recordingSelectsWindow", json!(w));
        };
        let item = move |icon: &'static str, label: String, key: &'static str, on_now: bool, enabled: bool, on: Box<dyn Fn() + Send + Sync>| {
            view! {
                <button class="flex min-h-[44px] flex-1 items-center justify-center gap-2 rounded-xl text-[13px] transition-colors disabled:opacity-35 [&_svg]:size-4"
                    class=("bg-white/20", on_now) class=("hover:bg-white/10", !on_now)
                    disabled=!enabled
                    title=if enabled { String::new() } else { t("Einmal einen Bereich aufnehmen, um „Letzter Bereich“ zu nutzen", "Record an area once to enable Last Area").to_string() }
                    on:click=move |_| on()>
                    <Icon d=icon />
                    <span>{label}</span>
                    <span class="rounded bg-white/15 px-1.5 font-mono text-[11px] text-white/70">{key}</span>
                </button>
            }
        };
        let wm = window_mode.get();
        Some(view! {
            <div class="absolute z-30 flex items-center gap-1.5 rounded-2xl border border-white/15 bg-zinc-900/90 p-3 text-white shadow-2xl backdrop-blur-xl"
                style=format!("left:{x}px;top:{y}px;width:{}px;height:{}px;cursor:default", REC_PANEL.0, REC_PANEL.1)
                on:mousedown=stop on:mouseup=stop on:dblclick=stop on:contextmenu=|e| e.prevent_default()>
                {item(ui::SQUARE_DASHED, t("Bereich", "Area").into(), "A", !wm, true, Box::new(move || set_window(false)))}
                {item(ui::APP_WINDOW, t("Fenster", "Window").into(), "W", wm, true, Box::new(move || set_window(true)))}
                {item(ui::MONITOR, t("Vollbild", "Full Screen").into(), "F", false, true, Box::new(move || finish("fullscreen", None, None, None, None)))}
                {item(ui::RESTART, t("Letzter Bereich", "Last Area").into(), "R", false, can_last, Box::new(move || finish("lastArea", None, None, None, None)))}
            </div>
        })
    };

    // Taste A im All-in-One: Inline-Annotation starten.
    let ka = window_event_listener(leptos::ev::keydown, move |e| {
        if all_in_one.get_untracked() && phase.get_untracked() == Phase::Adjust && e.key().eq_ignore_ascii_case("a") && !e.meta_key() && !e.ctrl_key() && !e.alt_key() {
            start_annotation();
        }
    });
    on_cleanup(move || ka.remove());

    let annotator = move || {
        if phase.get() != Phase::Annotate {
            return None;
        }
        let (url, k) = annot.get()?;
        let r = sel.get_untracked()?;
        let on_finish = Callback::new(move |(action, data_url): (String, Option<String>)| {
            if action == "cancel" {
                // Capso: Esc im All-in-One bricht die Aufnahme ab (Annotation ist Teil der Auswahl).
                pending.set_value(None);
                annot.set(None);
                cancel();
                return;
            }
            let a = pending.get_value().unwrap_or(if action == "editor" { "annotate".to_string() } else { action });
            pending.set_value(None);
            finish("area", Some(r), None, Some(a), data_url);
        });
        Some(view! {
            <div class="absolute z-10" style=format!("left:{}px;top:{}px;width:{}px;height:{}px;cursor:default", r.x, r.y, r.w, r.h)
                on:mousedown=stop on:mouseup=stop on:dblclick=stop on:contextmenu=|e| e.stop_propagation()>
                <InlineAnnotator image_url=url width=r.w height=r.h scale=k on_finish=on_finish />
            </div>
        })
    };

    view! {
        <div class="fixed inset-0 select-none overflow-hidden"
            style:cursor=cursor
            on:mousedown=on_down
            on:mousemove=on_move
            on:mouseup=on_up
            on:dblclick=on_dbl
            on:contextmenu=on_ctx
            on:mouseleave=move |_| if phase.get_untracked() == Phase::Idle { mouse.set(None) }
            on:mouseenter=move |_| if !document().has_focus().unwrap_or(true) { api::fire("capture", "overlayFocus", Value::Null) }
        >
            {move || frozen_url.get().map(|src| view! {
                <img src=src draggable="false" class="pointer-events-none absolute left-0 top-0 size-full"
                    on:load=move |e| {
                        let img = event_target::<web_sys::HtmlImageElement>(&e);
                        natural_w.set_value(img.natural_width() as f64);
                        ready();
                    }
                    on:error=move |_| ready() />
            })}
            {dim_view}
            {hover_outline}
            {window_label}
            {multi_view}
            {dimension_label}
            {preset_badge}
            {mode_hint}
            {coord_label}
            {reticle}
            {magnifier}
            {annotator}
            {handles}
            {confirm_bar}
            {toolbar}
            {rec_panel}
            {preset_menu}
        </div>
    }
}
