//! Wiedergabe-Steuerung und Zeitleiste (Port von Capso `EditorPlaybackControls`, `EditorTimelineView`, `EffectsTrackView`).
use super::{model::*, Ed, Sel};
use crate::{
    api::t,
    ui::{Icon, TextButton, COPY, DOWNLOAD, PAUSE, PLAY},
};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::PointerEvent;

const SCISSORS: &str = r#"<circle cx="6" cy="6" r="3"/><path d="M8.12 8.12 12 12"/><path d="M20 4 8.12 15.88"/><circle cx="6" cy="18" r="3"/><path d="M14.8 14.8 20 20"/>"#;
const ZOOM_IN: &str = r#"<circle cx="11" cy="11" r="8"/><line x1="21" x2="16.65" y1="21" y2="16.65"/><line x1="11" x2="11" y1="8" y2="14"/><line x1="8" x2="14" y1="11" y2="11"/>"#;
const DROPLET: &str = r#"<path d="M12 22a7 7 0 0 0 7-7c0-2-1-3.9-3-5.5s-3.5-4-4-6.5c-.5 2.5-2 4.9-4 6.5C6 11.1 5 13 5 15a7 7 0 0 0 7 7z"/>"#;
const PLUS: &str = r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#;

/// Menü „Effekt hinzufügen“ (Schnitt, Zoom, Weichzeichner) an der aktuellen Zeit.
#[component]
pub fn AddEffectMenu(ed: Ed) -> impl IntoView {
    let open = RwSignal::new(false);
    let item = move |icon: &'static str, label: &'static str, f: fn(&Ed, f64)| {
        view! {
            <button class="flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-blue-500 hover:text-white [&_svg]:size-3.5"
                on:click=move |_| { open.set(false); f(&ed, ed.time.get_untracked()) }>
                <Icon d=icon />{label}
            </button>
        }
    };
    view! {
        <div class="relative">
            <TextButton on_click=move || open.update(|o| *o = !*o)><Icon d=PLUS />{t("Effekt hinzufügen", "Add Effect")}</TextButton>
            <Show when=move || open.get()>
                <div class="fixed inset-0 z-10" on:click=move |_| open.set(false) />
                <div class="absolute bottom-8 left-0 z-20 w-40 rounded-lg border border-black/10 bg-white p-1 text-[13px] shadow-lg dark:border-white/10 dark:bg-zinc-800">
                    {item(SCISSORS, t("Schnitt", "Cut"), Ed::add_cut)}
                    {item(ZOOM_IN, t("Zoom", "Zoom"), Ed::add_zoom)}
                    {item(DROPLET, t("Weichzeichner", "Blur"), Ed::add_blur)}
                </div>
            </Show>
        </div>
    }
}

#[component]
pub fn PlaybackControls(ed: Ed) -> impl IntoView {
    let eff_time = move || {
        let t = ed.time.get();
        ed.project.with(|p| source_to_effective(&p.kept_ranges(), t))
    };
    view! {
        <div class="flex h-[30px] shrink-0 items-center gap-3">
            <button class="flex h-[26px] w-[30px] items-center justify-center rounded-md hover:bg-black/5 dark:hover:bg-white/10 [&_svg]:size-4 [&_svg]:fill-current"
                title=move || format!("{} (Space)", if ed.playing.get() { t("Pause", "Pause") } else { t("Abspielen", "Play") })
                on:click=move |_| ed.toggle_play()>
                <Icon d=Signal::derive(move || if ed.playing.get() { PAUSE } else { PLAY }) />
            </button>
            <span class="font-mono text-[12px] font-medium text-black/60 dark:text-white/60">{move || format_time(eff_time())}</span>
            <span class="text-[11px] text-black/30 dark:text-white/30">"/"</span>
            <span class="font-mono text-[12px] font-medium text-black/35 dark:text-white/35">{move || format_time(ed.project.with(|p| p.effective_duration()))}</span>
            <div class="h-[18px] w-px bg-black/10 dark:bg-white/15" />
            <AddEffectMenu ed=ed />
            <div class="flex-1" />
            {move || match ed.export.get() {
                Some((p, stage)) => view! {
                    <div class="flex flex-col items-end gap-1">
                        <span class="text-[11px] font-medium text-black/50 dark:text-white/50">{stage}</span>
                        <div class="flex items-center gap-2">
                            <div class="h-1 w-40 overflow-hidden rounded-full bg-black/10 dark:bg-white/15">
                                <div class="h-full bg-blue-500" style=format!("width:{:.1}%", p * 100.0) />
                            </div>
                            <span class="w-9 text-right font-mono text-[11px] text-black/50 dark:text-white/50">{format!("{:.0}%", p * 100.0)}</span>
                        </div>
                    </div>
                }.into_any(),
                None => view! {
                    <div class="flex items-center gap-2">
                        <TextButton variant="primary" disabled=Signal::derive(move || !ed.loaded.get()) on_click=move || ed.start_export(false)>
                            <Icon d=DOWNLOAD />{t("Sichern", "Save")}
                        </TextButton>
                        <TextButton disabled=Signal::derive(move || !ed.loaded.get()) on_click=move || ed.start_export(true)>
                            <Icon d=COPY />{t("Kopieren", "Copy")}
                        </TextButton>
                    </div>
                }.into_any(),
            }}
        </div>
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Move,
    Lead,
    Trail,
}

#[derive(Clone)]
enum TlDrag {
    Scrub { left: f64, width: f64 },
    Head { left: f64, width: f64 },
    Tail { left: f64, width: f64 },
    Seg { id: String, cut: bool, edge: Edge, start: f64, end: f64, x0: f64, width: f64 },
}

fn rect_of(e: &PointerEvent, closest: &str) -> Option<web_sys::DomRect> {
    let el = e.current_target()?.dyn_into::<web_sys::Element>().ok()?;
    let el = if closest.is_empty() { el } else { el.closest(closest).ok()?? };
    Some(el.get_bounding_client_rect())
}

fn capture(e: &PointerEvent) {
    e.stop_propagation();
    e.prevent_default();
    if let Some(t) = e.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
        let _ = t.set_pointer_capture(e.pointer_id());
    }
}

fn apply_drag(ed: Ed, d: &TlDrag, e: &PointerEvent) {
    let dur = ed.duration();
    let x = e.client_x() as f64;
    let at = |left: f64, width: f64| ((x - left) / width.max(1.0) * dur).clamp(0.0, dur);
    match d {
        TlDrag::Scrub { left, width } => ed.seek(at(*left, *width)),
        TlDrag::Head { left, width } => {
            let end = ed.project.with_untracked(|p| p.effective_end());
            let tm = at(*left, *width).min(end - 0.2).max(0.0);
            ed.project.update(|p| p.set_head_trim(tm));
            if ed.time.get_untracked() < tm {
                ed.seek(tm);
            }
        }
        TlDrag::Tail { left, width } => {
            let start = ed.project.with_untracked(|p| p.effective_start());
            let tm = at(*left, *width).max(start + 0.2).min(dur);
            ed.project.update(|p| p.set_tail_trim(tm));
            if ed.time.get_untracked() > tm {
                ed.seek(tm);
            }
        }
        TlDrag::Seg { id, cut, edge, start, end, x0, width } => {
            let dt = (x - x0) / width.max(1.0) * dur;
            let min_len = if *cut { 0.2 } else { 0.5 };
            let len = end - start;
            let (ns, ne) = match edge {
                Edge::Move => {
                    let s = (start + dt).clamp(0.0, (dur - len).max(0.0));
                    (s, s + len)
                }
                Edge::Lead => ((start + dt).clamp(0.0, end - min_len), *end),
                Edge::Trail => (*start, (end + dt).clamp(start + min_len, dur)),
            };
            if *cut {
                ed.project.update(|p| {
                    if let Some(r) = p.trim_regions.iter_mut().find(|r| &r.id == id) {
                        r.start_time = ns;
                        r.end_time = ne;
                    }
                });
            } else {
                ed.update_effect(id, |s| {
                    s.start_time = ns;
                    s.end_time = ne;
                });
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LaneKind {
    Cut,
    Zoom,
    Blur,
}

#[derive(Clone, PartialEq)]
struct Item {
    id: String,
    start: f64,
    end: f64,
    label: String,
    auto: bool,
}

#[component]
pub fn Timeline(ed: Ed) -> impl IntoView {
    let drag: StoredValue<Option<TlDrag>> = StoredValue::new(None);
    let pct = move |v: f64| {
        let d = ed.project.with(|p| p.video_duration).max(0.001);
        (v / d * 100.0).clamp(0.0, 100.0)
    };
    let mv = move |e: PointerEvent| {
        if let Some(d) = drag.get_value() {
            apply_drag(ed, &d, &e);
        }
    };
    let up = move |_e: PointerEvent| drag.set_value(None);

    let trim_down = move |e: PointerEvent, which: u8| {
        capture(&e);
        let Some(r) = rect_of(&e, ".receditor-track") else { return };
        let (left, width) = (r.left(), r.width());
        let d = match which {
            1 => TlDrag::Head { left, width },
            2 => TlDrag::Tail { left, width },
            _ => TlDrag::Scrub { left, width },
        };
        apply_drag(ed, &d, &e);
        drag.set_value(Some(d));
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col gap-2">
            // Trim-Spur
            <div class="receditor-track relative h-9 shrink-0 cursor-pointer rounded bg-black/[0.08] dark:bg-white/[0.08]"
                on:pointerdown=move |e| trim_down(e, 0) on:pointermove=mv on:pointerup=up>
                <div class="absolute inset-y-0 rounded bg-blue-500/15 dark:bg-blue-400/20"
                    style=move || { let (s, e) = ed.project.with(|p| (p.effective_start(), p.effective_end())); format!("left:{}%;width:{}%", pct(s), pct(e) - pct(s)) } />
                <For each=move || ed.project.with(|p| p.timeline_cuts()) key=|r| (r.id.clone(), (r.start_time * 1000.0) as i64, (r.end_time * 1000.0) as i64) let:r>
                    <div class="pointer-events-none absolute inset-y-0 bg-[repeating-linear-gradient(45deg,rgba(0,0,0,0.18)_0,rgba(0,0,0,0.18)_3px,transparent_3px,transparent_7px)]"
                        style=format!("left:{}%;width:{}%", pct(r.start_time), pct(r.end_time) - pct(r.start_time)) />
                </For>
                <div class="absolute inset-y-0 left-0 rounded-l bg-black/25 dark:bg-black/40" style=move || format!("width:{}%", pct(ed.project.with(|p| p.effective_start()))) />
                <div class="absolute inset-y-0 right-0 rounded-r bg-black/25 dark:bg-black/40" style=move || format!("width:{}%", 100.0 - pct(ed.project.with(|p| p.effective_end()))) />
                <div class="absolute inset-y-0 z-10 flex w-2.5 -translate-x-full cursor-ew-resize items-center justify-center rounded-l bg-amber-400 shadow"
                    title=t("Anfang trimmen", "Trim start")
                    style=move || format!("left:{}%", pct(ed.project.with(|p| p.effective_start())))
                    on:pointerdown=move |e| trim_down(e, 1) on:pointermove=mv on:pointerup=up>
                    <div class="h-3 w-0.5 rounded bg-black/40" />
                </div>
                <div class="absolute inset-y-0 z-10 flex w-2.5 cursor-ew-resize items-center justify-center rounded-r bg-amber-400 shadow"
                    title=t("Ende trimmen", "Trim end")
                    style=move || format!("left:{}%", pct(ed.project.with(|p| p.effective_end())))
                    on:pointerdown=move |e| trim_down(e, 2) on:pointermove=mv on:pointerup=up>
                    <div class="h-3 w-0.5 rounded bg-black/40" />
                </div>
                <div class="pointer-events-none absolute -top-1 bottom-0 z-20 flex -translate-x-1/2 flex-col items-center" style=move || format!("left:{}%", pct(ed.time.get()))>
                    <div class="size-2.5 rounded-full bg-blue-500 shadow" />
                    <div class="w-0.5 flex-1 bg-blue-500" />
                </div>
            </div>
            // Effekte-Spuren
            <div class="flex shrink-0 flex-col gap-1.5 rounded-md bg-black/[0.03] p-1.5 dark:bg-white/[0.04]"
                on:click=move |_| ed.sel.set(None)>
                <Lane ed=ed kind=LaneKind::Cut drag=drag />
                <Lane ed=ed kind=LaneKind::Zoom drag=drag />
                <Lane ed=ed kind=LaneKind::Blur drag=drag />
            </div>
        </div>
    }
}

#[component]
fn Lane(ed: Ed, kind: LaneKind, drag: StoredValue<Option<TlDrag>>) -> impl IntoView {
    let (title, color, sel_color) = match kind {
        LaneKind::Cut => (t("Schnitt", "Cut"), "bg-zinc-500/80", "ring-zinc-700 dark:ring-white"),
        LaneKind::Zoom => (t("Zoom", "Zoom"), "bg-blue-500/85", "ring-blue-800 dark:ring-white"),
        LaneKind::Blur => (t("Weichzeichner", "Blur"), "bg-red-500/80", "ring-red-800 dark:ring-white"),
    };
    let items = move || -> Vec<Item> {
        ed.project.with(|p| match kind {
            LaneKind::Cut => p.timeline_cuts().into_iter().map(|r| Item { id: r.id, start: r.start_time, end: r.end_time, label: t("Schnitt", "Cut").into(), auto: false }).collect(),
            _ => p
                .effect_segments
                .iter()
                .filter_map(|e| match (&e.payload, kind) {
                    (EffectPayload::Zoom(z), LaneKind::Zoom) => Some(Item {
                        id: e.id.clone(),
                        start: e.start_time,
                        end: e.end_time,
                        label: format!("{:.2}×", z.zoom_level).replace(".00×", "×").replace("0×", "×"),
                        auto: e.source == SegmentSource::Auto,
                    }),
                    (EffectPayload::Blur(_), LaneKind::Blur) => Some(Item { id: e.id.clone(), start: e.start_time, end: e.end_time, label: t("Weichzeichner", "Blur").into(), auto: false }),
                    _ => None,
                })
                .collect(),
        })
    };
    let pct = move |v: f64| {
        let d = ed.project.with(|p| p.video_duration).max(0.001);
        (v / d * 100.0).clamp(0.0, 100.0)
    };
    let is_sel = move |id: &str| match ed.sel.get() {
        Some(Sel::Effect(s)) | Some(Sel::Cut(s)) => s == id,
        None => false,
    };
    let mv = move |e: PointerEvent| {
        if let Some(d) = drag.get_value() {
            apply_drag(ed, &d, &e);
        }
    };
    let up = move |_e: PointerEvent| drag.set_value(None);
    // Kontextmenü (Capso EffectsTrackView.contextMenu): (x, y, id)
    let menu: RwSignal<Option<(f64, f64, String)>> = RwSignal::new(None);
    let delete = move |id: String| {
        match kind {
            LaneKind::Cut => ed.project.update(|p| p.trim_regions.retain(|r| r.id != id)),
            _ => ed.project.update(|p| p.effect_segments.retain(|e| e.id != id)),
        }
        if matches!(ed.sel.get_untracked(), Some(Sel::Effect(ref s)) | Some(Sel::Cut(ref s)) if *s == id) {
            ed.sel.set(None);
        }
    };
    let set_level = move |id: String, level: f64| {
        ed.update_effect(&id, |e| {
            if let EffectPayload::Zoom(z) = &mut e.payload {
                z.zoom_level = level;
            }
        });
    };
    let on_dbl = move |e: web_sys::MouseEvent| {
        let Some(el) = e.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
        let r = el.get_bounding_client_rect();
        let at = ((e.client_x() as f64 - r.left()) / r.width().max(1.0) * ed.duration()).max(0.0);
        match kind {
            LaneKind::Cut => ed.add_cut(at),
            LaneKind::Zoom => ed.add_zoom(at),
            LaneKind::Blur => ed.add_blur(at),
        }
    };

    view! {
        <div class="receditor-lane relative h-[30px] rounded bg-black/[0.08] dark:bg-white/[0.06]" on:dblclick=on_dbl>
            <span class="pointer-events-none absolute left-1.5 top-1/2 -translate-y-1/2 text-[9px] font-medium text-black/25 dark:text-white/25">{title}</span>
            <For each=items key=|i| i.id.clone() let:item>
                {
                    let id = item.id.clone();
                    let id_cls = id.clone();
                    let down = move |e: PointerEvent, edge: Edge| {
                        capture(&e);
                        let Some(r) = rect_of(&e, ".receditor-lane") else { return };
                        let (start, end) = ed.project.with_untracked(|p| match kind {
                            LaneKind::Cut => p.trim_regions.iter().find(|x| x.id == id).map(|x| (x.start_time, x.end_time)),
                            _ => p.effect_segments.iter().find(|x| x.id == id).map(|x| (x.start_time, x.end_time)),
                        }).unwrap_or((0.0, 0.0));
                        ed.sel.set(Some(if kind == LaneKind::Cut { Sel::Cut(id.clone()) } else { Sel::Effect(id.clone()) }));
                        drag.set_value(Some(TlDrag::Seg { id: id.clone(), cut: kind == LaneKind::Cut, edge, start, end, x0: e.client_x() as f64, width: r.width() }));
                    };
                    let down2 = down.clone();
                    let down3 = down.clone();
                    let style = {
                        let id = item.id.clone();
                        move || {
                            let (s, e) = items().into_iter().find(|i| i.id == id).map(|i| (i.start, i.end)).unwrap_or((item.start, item.end));
                            format!("left:{}%;width:{}%", pct(s), (pct(e) - pct(s)).max(0.3))
                        }
                    };
                    let label = {
                        let id = item.id.clone();
                        move || items().into_iter().find(|i| i.id == id).map(|i| i.label).unwrap_or_default()
                    };
                    view! {
                        <div
                            class=move || format!(
                                "group absolute inset-y-0.5 flex cursor-grab items-center overflow-hidden rounded-[5px] text-[10px] font-medium text-white shadow-sm active:cursor-grabbing {color} {}",
                                if is_sel(&id_cls) { format!("ring-2 {sel_color}") } else { String::new() }
                            )
                            style=style
                            on:pointerdown=move |e| down(e, Edge::Move)
                            on:pointermove=mv
                            on:pointerup=up
                            on:click=|e| e.stop_propagation()
                            on:dblclick=|e| e.stop_propagation()
                            on:contextmenu={
                                let id = item.id.clone();
                                move |e: web_sys::MouseEvent| {
                                    e.prevent_default();
                                    e.stop_propagation();
                                    ed.sel.set(Some(if kind == LaneKind::Cut { Sel::Cut(id.clone()) } else { Sel::Effect(id.clone()) }));
                                    menu.set(Some((e.client_x() as f64, e.client_y() as f64, id.clone())));
                                }
                            }
                        >
                            <div class="absolute inset-y-0 left-0 w-2.5 cursor-ew-resize bg-white/0 hover:bg-white/25"
                                on:pointerdown=move |e| down2(e, Edge::Lead) on:pointermove=mv on:pointerup=up />
                            <span class="pointer-events-none truncate px-3">
                                {label}
                                {item.auto.then(|| view! { <span class="ml-1 rounded bg-white/25 px-1 text-[9px]">"Auto"</span> })}
                            </span>
                            <div class="absolute inset-y-0 right-0 w-2.5 cursor-ew-resize bg-white/0 hover:bg-white/25"
                                on:pointerdown=move |e| down3(e, Edge::Trail) on:pointermove=mv on:pointerup=up />
                        </div>
                    }
                }
            </For>
            {move || menu.get().map(|(x, y, id)| {
                let (id1, id2) = (id.clone(), id.clone());
                let row = "flex w-full items-center rounded px-2 py-1 text-left hover:bg-blue-500 hover:text-white";
                // Menü möglichst über dem Klickpunkt (die Zeitleiste liegt unten im Fenster).
                let h = if kind == LaneKind::Zoom { 196.0 } else { 34.0 };
                let style = format!("left:{x}px;top:{}px", (y - h).max(4.0));
                view! {
                    <div class="fixed inset-0 z-30" on:click=move |e| { e.stop_propagation(); menu.set(None) }
                        on:contextmenu=move |e| { e.prevent_default(); menu.set(None) } />
                    <div class="fixed z-40 w-40 rounded-lg border border-black/10 bg-white p-1 text-[13px] shadow-lg dark:border-white/10 dark:bg-zinc-800"
                        style=style on:click=|e| e.stop_propagation()>
                        <button class=row on:click=move |_| { menu.set(None); delete(id1.clone()) }>{t("Löschen", "Delete")}</button>
                        {(kind == LaneKind::Zoom).then(|| view! {
                            <div class="my-1 h-px bg-black/10 dark:bg-white/10" />
                            <div class="px-2 py-0.5 text-[11px] text-black/45 dark:text-white/45">{t("Zoomstufe", "Zoom Level")}</div>
                            {[1.25, 1.5, 1.8, 2.0, 2.5, 3.0].into_iter().map(|level| {
                                let id = id2.clone();
                                view! {
                                    <button class=row on:click=move |_| { menu.set(None); set_level(id.clone(), level) }>
                                        {format!("{level:.1}×")}
                                    </button>
                                }
                            }).collect_view()}
                        })}
                    </div>
                }
            })}
        </div>
    }
}
