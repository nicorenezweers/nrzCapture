//! Zuschneiden (Port von Capso `CropEditorView`, `CropToolbar`, `CropAreaNSView`, `CustomCropRatio`):
//! Seitenverhältnis-Vorgaben + eigene, Pixelmaße, Drehen/Spiegeln, Einrasten an Bildkanten (⌘ = aus), Ausgabegröße.
use super::canvas::{Ed, Map};
use super::model::*;
use super::render;
use super::toolbar::{sep, I_CHEVRON, I_CROP, I_FLIP, I_ROTATE, TB};
use crate::api::{self, t};
use crate::ui::{Icon, TextButton};
use leptos::{ev, html, prelude::*};
use serde_json::json;
use wasm_bindgen::{prelude::Closure, JsCast};

#[derive(Clone, Copy, PartialEq)]
enum Ratio {
    Preset(CropPreset),
    Custom(u32, u32),
}

#[derive(Clone)]
enum CropDrag {
    None,
    Resize { h: CropHandle, start: Pt, orig: Rect },
    Create { start: Pt },
}

fn custom_ratios() -> Vec<(u32, u32)> {
    api::setting("customCropRatios")
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()?.split_once(':').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).collect())
        .unwrap_or_default()
}

fn save_custom_ratios(l: &[(u32, u32)]) {
    api::set_setting("customCropRatios", json!(l.iter().map(|(w, h)| format!("{w}:{h}")).collect::<Vec<_>>()));
}

/// Maß-Etikett „B × H“ am Zuschnittrahmen während des Ziehens (Capso `CropAreaNSView.drawDimensionBadge`).
fn draw_size_badge(g: &web_sys::CanvasRenderingContext2d, m: &Map, c: Rect, vw: f64, vh: f64) {
    let text = format!("{} × {}", c.w as i64, c.h as i64);
    let _ = g.set_transform(m.dpr, 0.0, 0.0, m.dpr, 0.0, 0.0);
    g.set_font("600 11px ui-monospace, SFMono-Regular, Menlo, monospace");
    let tw = g.measure_text(&text).map(|t| t.width()).unwrap_or(60.0);
    let (hp, vp, th) = (7.0, 3.0, 13.0);
    let (bw, bh) = (tw + 2.0 * hp, th + 2.0 * vp);
    let tl = m.to_css(pt(c.x, c.y));
    let br = m.to_css(pt(c.max_x(), c.max_y()));
    let below = br.y + 6.0;
    let y = if below + bh <= vh { below } else { (tl.y - 6.0 - bh).max(2.0) };
    let x = ((tl.x + br.x) / 2.0 - bw / 2.0).min(vw - bw - 2.0).max(2.0);
    g.set_fill_style_str("rgba(0,0,0,0.78)");
    g.begin_path();
    let _ = g.round_rect_with_f64(x, y, bw, bh, 4.0);
    g.fill();
    g.set_fill_style_str("#fff");
    g.set_text_baseline("middle");
    g.set_text_align("left");
    let _ = g.fill_text(&text, x + hp, y + bh / 2.0);
}

#[component]
pub fn CropView(ed: Ed) -> impl IntoView {
    let (init_xf, init_crop, init_out, can_xform) = ed.doc.with_untracked(|d| (d.xform, d.crop.unwrap_or(d.image_rect()), d.output, d.objects.is_empty()));
    let (orig_w, orig_h) = ed.doc.with_untracked(|d| (d.orig_w, d.orig_h));
    let xf = RwSignal::new(init_xf);
    let crop = RwSignal::new(init_crop);
    // Zuletzt gewählte Vorgabe merken (Capso `@AppStorage("annotationLastCropPreset")`).
    let ratio = RwSignal::new(Ratio::Preset(CropPreset::from_id(&api::setting_str("annotationLastCropPreset"))));
    // Rückgängig/Wiederholen im Zuschneiden (Capso `undoStack`/`redoStack`: Rechteck + Vorgabe).
    let undo_stack = StoredValue::new(Vec::<(Rect, Ratio)>::new());
    let redo_stack = StoredValue::new(Vec::<(Rect, Ratio)>::new());
    let push_history = move || {
        undo_stack.update_value(|u| u.push((crop.get_untracked(), ratio.get_untracked())));
        redo_stack.update_value(|r| r.clear());
    };
    let drag_orig = StoredValue::new(None::<Rect>);
    let customs = RwSignal::new(custom_ratios());
    let snap = RwSignal::new(api::setting("cropSnapToEdges").as_bool().unwrap_or(true));
    let resize_out = RwSignal::new(init_out.is_some());
    let out = RwSignal::new(init_out.unwrap_or((init_crop.w.round() as u32, init_crop.h.round() as u32)));
    let out_lock = RwSignal::new(true);
    let menu = RwSignal::new(false);
    let adding = RwSignal::new(None::<(String, String)>);
    let drag = StoredValue::new(CropDrag::None);
    let src = StoredValue::new_local(ed.src.get_value());
    let map = RwSignal::new(Map::default());
    let vp = RwSignal::new((0.0f64, 0.0f64));
    let cursor = RwSignal::new("crosshair");
    let outer = NodeRef::<html::Div>::new();
    let canvas_ref = NodeRef::<html::Canvas>::new();

    let size = move || xf.with(|x| x.size(orig_w, orig_h));
    let cur_ratio = move || match ratio.get() {
        Ratio::Preset(p) => {
            let (w, h) = size();
            p.ratio(w, h)
        }
        Ratio::Custom(w, h) => Some(w as f64 / h.max(1) as f64),
    };
    let set_ratio = move |r: Ratio| {
        menu.set(false);
        if r == ratio.get_untracked() {
            return;
        }
        push_history();
        ratio.set(r);
        if let Some(k) = cur_ratio() {
            let (w, h) = size();
            crop.set(fit_ratio(rect(0.0, 0.0, w, h), k));
        }
    };
    Effect::new(move |prev: Option<Ratio>| {
        let r = ratio.get();
        if let (Some(_), Ratio::Preset(p)) = (prev, r) {
            if prev != Some(r) {
                api::set_setting("annotationLastCropPreset", json!(p.id()));
            }
        }
        r
    });
    let undo = move || {
        if let Some((c, r)) = undo_stack.try_update_value(|u| u.pop()).flatten() {
            redo_stack.update_value(|s| s.push((crop.get_untracked(), ratio.get_untracked())));
            crop.set(c);
            ratio.set(r);
        }
    };
    let redo = move || {
        if let Some((c, r)) = redo_stack.try_update_value(|s| s.pop()).flatten() {
            undo_stack.update_value(|u| u.push((crop.get_untracked(), ratio.get_untracked())));
            crop.set(c);
            ratio.set(r);
        }
    };
    // Ausgabegröße folgt dem Zuschnitt (bei gesperrtem Verhältnis nur die Höhe)
    Effect::new(move |_| {
        let c = crop.get();
        if !resize_out.get_untracked() {
            out.set((c.w.round() as u32, c.h.round() as u32));
        } else if out_lock.get() {
            let w = out.get_untracked().0.max(1);
            out.set((w, ((w as f64) * c.h / c.w.max(1.0)).round().max(1.0) as u32));
        }
    });

    // Größe beobachten
    Effect::new(move |_| {
        let Some(el) = outer.get() else { return };
        let el2 = el.clone();
        let cb = Closure::<dyn Fn()>::new(move || vp.set((el2.client_width() as f64, el2.client_height() as f64)));
        let ro = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()).unwrap();
        ro.observe(&el);
        cb.forget();
        vp.set((el.client_width() as f64, el.client_height() as f64));
    });
    Effect::new(move |_| {
        let (vw, vh) = vp.get();
        let (w, h) = size();
        let m = 36.0;
        let z = ((vw - 2.0 * m) / w).min((vh - 2.0 * m) / h).clamp(0.02, 2.0);
        let dpr = web_sys::window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0);
        map.set(Map { z, ox: (vw - w * z) / 2.0, oy: (vh - h * z) / 2.0, dpr });
    });
    // Quellbild für die vorläufige Transformation
    Effect::new(move |_| {
        let x = xf.get();
        if let Some(img) = ed.img.get_value() {
            src.set_value(Some(render::build_source(&img, x)));
        }
    });
    let dragging = RwSignal::new(false);
    Effect::new(move |_| {
        let m = map.get();
        let c = crop.get();
        let x = xf.get();
        let dr = dragging.get();
        let (vw, vh) = vp.get();
        let Some(cv) = canvas_ref.get() else { return };
        cv.set_width((vw * m.dpr).round() as u32);
        cv.set_height((vh * m.dpr).round() as u32);
        let g = render::ctx(&cv);
        let Some(s) = src.get_value() else { return };
        m.apply(&g);
        let mut d = ed.doc.get_untracked();
        d.crop = None;
        d.xform = x;
        d.selected = None;
        render::draw_doc(&g, &s, &d, None, None);
        let (w, h) = x.size(orig_w, orig_h);
        let px = 1.0 / m.z;
        // Außerhalb abdunkeln
        g.set_fill_style_str("rgba(0,0,0,0.5)");
        g.begin_path();
        g.rect(0.0, 0.0, w, h);
        g.rect(c.x, c.y, c.w, c.h);
        g.fill_with_canvas_winding_rule(web_sys::CanvasWindingRule::Evenodd);
        g.set_stroke_style_str("#fff");
        g.set_line_width(px);
        g.stroke_rect(c.x, c.y, c.w, c.h);
        if dr {
            g.set_stroke_style_str("rgba(255,255,255,0.45)");
            g.begin_path();
            for i in 1..3 {
                let f = i as f64 / 3.0;
                g.move_to(c.x + c.w * f, c.y);
                g.line_to(c.x + c.w * f, c.max_y());
                g.move_to(c.x, c.y + c.h * f);
                g.line_to(c.max_x(), c.y + c.h * f);
            }
            g.stroke();
        }
        g.set_fill_style_str("#fff");
        for hnd in CropHandle::EDGES {
            let p = hnd.point(&c);
            let r = 4.0 * px;
            g.fill_rect(p.x - r, p.y - r, 2.0 * r, 2.0 * r);
        }
        if dr {
            draw_size_badge(&g, &m, c, vw, vh);
        }
    });

    let local = move |e: &web_sys::MouseEvent| {
        let r = outer.get_untracked().map(|o| o.get_bounding_client_rect());
        let (x, y) = r.map(|r| (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top())).unwrap_or_default();
        map.get_untracked().to_img(x, y)
    };
    let on_down = move |e: ev::PointerEvent| {
        if e.button() != 0 {
            return;
        }
        if let Some(o) = outer.get_untracked() {
            let _ = o.set_pointer_capture(e.pointer_id());
        }
        let p = local(&e);
        let z = map.get_untracked().z;
        let c = crop.get_untracked();
        drag_orig.set_value(Some(c));
        dragging.set(true);
        match crop_handle_at(&c, p, 10.0 / z) {
            Some(h) => drag.set_value(CropDrag::Resize { h, start: p, orig: c }),
            None => {
                let (w, h) = size();
                drag.set_value(CropDrag::Create { start: pt(p.x.clamp(0.0, w), p.y.clamp(0.0, h)) });
            }
        }
    };
    let on_move = move |e: ev::PointerEvent| {
        let p = local(&e);
        let z = map.get_untracked().z;
        let (w, h) = size();
        let th = if snap.get_untracked() && !e.meta_key() { 6.0 / z } else { 0.0 };
        match drag.get_value() {
            CropDrag::None => cursor.set(crop_handle_at(&crop.get_untracked(), p, 10.0 / z).map(|h| h.cursor()).unwrap_or("crosshair")),
            CropDrag::Resize { h: hd, start, orig } => crop.set(crop_resize(orig, hd, pt(p.x - start.x, p.y - start.y), cur_ratio(), w, h, th)),
            CropDrag::Create { start } => {
                let r = crop_creation_rect(start, p, cur_ratio(), w, h);
                if r.w >= MIN_CROP && r.h >= MIN_CROP {
                    crop.set(if cur_ratio().is_some() { r } else { snap_rect(r, w, h, th) });
                }
            }
        }
    };
    let on_up = move |_e: ev::PointerEvent| {
        drag.set_value(CropDrag::None);
        dragging.set(false);
        // Zieh-Geste = ein Undo-Schritt (Capso `onDragEnded(oldRect)`).
        if let Some(o) = drag_orig.try_update_value(|d| d.take()).flatten() {
            if o != crop.get_untracked() {
                undo_stack.update_value(|u| u.push((o, ratio.get_untracked())));
                redo_stack.update_value(|r| r.clear());
            }
        }
    };

    let clear_history = move || {
        undo_stack.update_value(|u| u.clear());
        redo_stack.update_value(|r| r.clear());
    };
    let rotate = move || {
        clear_history();
        let (w, _) = size();
        crop.update(|c| *c = rect_rotate_ccw(*c, w));
        xf.update(|x| *x = x.rotate_ccw());
    };
    let flip = move || {
        clear_history();
        let (w, _) = size();
        crop.update(|c| *c = rect_flip_h(*c, w));
        xf.update(|x| *x = x.flip_h());
    };
    let revert = move || {
        clear_history();
        xf.set(Xform::default());
        ratio.set(Ratio::Preset(CropPreset::Freeform));
        resize_out.set(false);
        crop.set(rect(0.0, 0.0, orig_w, orig_h));
    };
    let apply = move || {
        let (w, h) = size();
        let r = crop.get_untracked().rounded().intersect(&rect(0.0, 0.0, w, h));
        let full = r == rect(0.0, 0.0, w, h);
        let output = resize_out.get_untracked().then(|| out.get_untracked()).filter(|&(ow, oh)| ow > 0 && oh > 0 && (ow as f64 != r.w || oh as f64 != r.h));
        let c = if full && output.is_none() { None } else { Some(r) };
        let x = xf.get_untracked();
        let changed = ed.doc.with_untracked(|d| d.crop != c || d.xform != x || d.output != output);
        if changed {
            ed.doc.update(|d| {
                d.set_crop(c, x, output);
                d.selected = None;
            });
            ed.after_xform_change();
        }
        ed.fit.update(|f| *f += 1);
        ed.crop_mode.set(false);
    };
    // Tasten (Capso `CropEditorView.onKeyPress` / `CropAreaNSView.keyDown`): ⏎ anwenden, R/⇧R Vorgabe wechseln,
    // 1–6 Vorgabe wählen, ⌘Z/⇧⌘Z Zuschnitt rückgängig/wiederholen. Esc behandelt `Ed::handle_key`.
    let on_key = window_event_listener(ev::keydown, move |e| {
        if adding.get_untracked().is_some() || e.alt_key() || e.ctrl_key() {
            return;
        }
        let in_field = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "SELECT" | "TEXTAREA"));
        let key = e.key();
        let cur = match ratio.get_untracked() {
            Ratio::Preset(p) => p,
            Ratio::Custom(..) => CropPreset::Freeform,
        };
        let handled = match (e.meta_key(), key.to_lowercase().as_str()) {
            (true, "z") if e.shift_key() => {
                redo();
                true
            }
            (true, "z") => {
                undo();
                true
            }
            (false, _) if in_field => false,
            (false, "enter") => {
                apply();
                true
            }
            (false, "r") => {
                set_ratio(Ratio::Preset(cur.cycle(!e.shift_key())));
                true
            }
            (false, d) => match CropPreset::from_digit(d) {
                Some(p) => {
                    set_ratio(Ratio::Preset(p));
                    true
                }
                None => false,
            },
            _ => false,
        };
        if handled {
            e.prevent_default();
            e.stop_propagation();
        }
    });
    on_cleanup(move || on_key.remove());
    let num_input = "h-6 w-16 rounded-md border border-black/10 dark:border-white/15 bg-white/80 dark:bg-white/10 px-1.5 text-[12px] tabular-nums outline-none";
    let set_crop_w = move |v: f64| {
        let (w, h) = size();
        crop.update(|c| {
            c.w = v.clamp(MIN_CROP, w - c.x);
            if let Some(k) = cur_ratio() {
                c.h = (c.w / k).min(h - c.y);
                c.w = c.h * k;
            }
        });
    };
    let set_crop_h = move |v: f64| {
        let (w, h) = size();
        crop.update(|c| {
            c.h = v.clamp(MIN_CROP, h - c.y);
            if let Some(k) = cur_ratio() {
                c.w = (c.h * k).min(w - c.x);
                c.h = c.w / k;
            }
        });
    };
    let ratio_label = move || match ratio.get() {
        Ratio::Preset(p) => {
            let (de, en) = p.label();
            t(de, en).to_string()
        }
        Ratio::Custom(w, h) => format!("{w} : {h}"),
    };
    let item = "flex w-full items-center gap-2 rounded px-2 py-1 text-left text-[12px] hover:bg-blue-500 hover:text-white";

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            // Werkzeugleiste
            <div class="flex h-11 shrink-0 items-center gap-2 border-b border-black/10 dark:border-white/10 bg-white/70 dark:bg-zinc-800/70 px-3">
                <span class="text-zinc-500 [&_svg]:size-4"><Icon d=I_CROP /></span>
                <input type="number" class=num_input title=move || t("Breite in Pixeln", "Width in pixels")
                    prop:value=move || (crop.get().w.round() as i64).to_string()
                    on:change=move |e| { if let Ok(v) = event_target_value(&e).parse::<f64>() { set_crop_w(v) } } />
                <span class="text-zinc-400">"×"</span>
                <input type="number" class=num_input title=move || t("Höhe in Pixeln", "Height in pixels")
                    prop:value=move || (crop.get().h.round() as i64).to_string()
                    on:change=move |e| { if let Ok(v) = event_target_value(&e).parse::<f64>() { set_crop_h(v) } } />
                <span class="text-[12px] text-zinc-400">"px"</span>
                {sep()}
                <TB title=if can_xform { t("90° nach links drehen", "Rotate 90° Left") } else { t("Drehen ist nur ohne Anmerkungen möglich", "Rotation is only available without annotations") }
                    icon=I_ROTATE disabled=!can_xform on_click=rotate />
                <TB title=if can_xform { t("Horizontal spiegeln", "Flip Horizontal") } else { t("Spiegeln ist nur ohne Anmerkungen möglich", "Flipping is only available without annotations") }
                    icon=I_FLIP disabled=!can_xform on_click=flip />
                {sep()}
                <div class="relative">
                    <button class="flex h-6 items-center gap-1 rounded-md border border-black/10 dark:border-white/15 bg-white/80 dark:bg-white/10 px-2 text-[12px] [&_svg]:size-3"
                        on:click=move |_| menu.update(|m| *m = !*m)>
                        {ratio_label}<Icon d=I_CHEVRON />
                    </button>
                    <Show when=move || menu.get()>
                        <div class="fixed inset-0 z-40" on:click=move |_| menu.set(false)></div>
                        <div class="absolute left-0 top-8 z-50 w-56 rounded-lg border border-black/10 dark:border-white/10 bg-white dark:bg-zinc-800 p-1 shadow-xl">
                            {CropPreset::ALL.iter().map(|&p| view! {
                                <button class=item on:click=move |_| set_ratio(Ratio::Preset(p))>
                                    <span class="w-3">{move || if ratio.get() == Ratio::Preset(p) { "✓" } else { "" }}</span>
                                    {move || { let (de, en) = p.label(); t(de, en) }}
                                </button>
                            }).collect_view()}
                            <For each=move || customs.get() key=|c| *c let:c>
                                <div class="group flex items-center">
                                    <button class=item on:click=move |_| set_ratio(Ratio::Custom(c.0, c.1))>
                                        <span class="w-3">{move || if ratio.get() == Ratio::Custom(c.0, c.1) { "✓" } else { "" }}</span>
                                        {format!("{} : {}", c.0, c.1)}
                                    </button>
                                    <button title=move || t("Entfernen", "Remove") class="px-1.5 text-zinc-400 opacity-0 hover:text-red-500 group-hover:opacity-100"
                                        on:click=move |_| { customs.update(|l| l.retain(|x| *x != c)); save_custom_ratios(&customs.get_untracked()); }>"×"</button>
                                </div>
                            </For>
                            <div class="my-1 h-px bg-black/10 dark:bg-white/10"></div>
                            <button class=item on:click=move |_| { menu.set(false); adding.set(Some((String::new(), String::new()))); }>
                                {move || t("Eigenes Verhältnis hinzufügen …", "Add Custom Ratio…")}
                            </button>
                        </div>
                    </Show>
                </div>
                <div class="flex-1"></div>
                <TextButton on_click=move || ed.crop_mode.set(false)>{move || t("Abbrechen", "Cancel")}</TextButton>
                <TextButton variant="primary" on_click=apply>{move || t("Anwenden", "Apply")}</TextButton>
            </div>
            // Fläche
            <div node_ref=outer class="relative min-h-0 flex-1 overflow-hidden bg-[#1f1f1f]"
                style:cursor=move || cursor.get()
                on:pointerdown=on_down on:pointermove=on_move on:pointerup=on_up on:pointercancel=on_up>
                <canvas node_ref=canvas_ref class="pointer-events-none absolute left-0 top-0"
                    style:width=move || format!("{}px", vp.get().0) style:height=move || format!("{}px", vp.get().1)></canvas>
                <Show when=move || adding.get().is_some()>
                    <div class="absolute inset-0 z-50 grid place-items-center bg-black/30" on:pointerdown=|e| e.stop_propagation()>
                        <div class="w-64 rounded-xl bg-white dark:bg-zinc-800 p-4 shadow-2xl">
                            <div class="mb-3 text-[14px] font-semibold">{move || t("Eigenes Verhältnis hinzufügen", "Add Custom Ratio")}</div>
                            <div class="mb-4 flex items-center gap-2">
                                <input type="number" min="1" class=num_input placeholder=move || t("Breite", "Width")
                                    on:input=move |e| { let v = event_target_value(&e); adding.update(|a| if let Some(a) = a { a.0 = v }); } />
                                <span>":"</span>
                                <input type="number" min="1" class=num_input placeholder=move || t("Höhe", "Height")
                                    on:input=move |e| { let v = event_target_value(&e); adding.update(|a| if let Some(a) = a { a.1 = v }); } />
                            </div>
                            <div class="flex justify-end gap-2">
                                <TextButton on_click=move || adding.set(None)>{move || t("Abbrechen", "Cancel")}</TextButton>
                                <TextButton variant="primary"
                                    disabled=Signal::derive(move || adding.with(|a| a.as_ref().is_none_or(|(w, h)| w.parse::<u32>().unwrap_or(0) == 0 || h.parse::<u32>().unwrap_or(0) == 0)))
                                    on_click=move || {
                                        if let Some((w, h)) = adding.get_untracked() {
                                            if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                                                if w > 0 && h > 0 {
                                                    customs.update(|l| if !l.contains(&(w, h)) { l.push((w, h)) });
                                                    save_custom_ratios(&customs.get_untracked());
                                                    set_ratio(Ratio::Custom(w, h));
                                                }
                                            }
                                        }
                                        adding.set(None);
                                    }>{move || t("Hinzufügen", "Add")}</TextButton>
                            </div>
                        </div>
                    </div>
                </Show>
            </div>
            // Fußleiste
            <div class="flex h-9 shrink-0 items-center gap-4 border-t border-black/10 dark:border-white/10 bg-white/70 dark:bg-zinc-800/70 px-3 text-[12px]">
                <span class="w-10 tabular-nums text-zinc-500">{move || format!("{}%", (map.get().z * 100.0).round() as i64)}</span>
                <label class="flex items-center gap-1.5" title=move || t("Beim Ziehen nahe am Bildrand genau an der Kante einrasten. ⌘ gedrückt halten, um es vorübergehend abzuschalten.", "When dragging a crop handle near the image boundary, snap it exactly to that edge. Hold ⌘ while dragging to temporarily disable.")>
                    <input type="checkbox" class="accent-blue-500" prop:checked=move || snap.get()
                        on:change=move |e| { let v = event_target_checked(&e); snap.set(v); api::set_setting("cropSnapToEdges", json!(v)); } />
                    {move || t("Griffe an Bildkanten einrasten", "Snap handles to image edges")}
                </label>
                <span class="text-zinc-400">{move || t("⌘ halten zum vorübergehenden Abschalten.", "Hold ⌘ while dragging to disable temporarily.")}</span>
                <div class="flex-1"></div>
                <TextButton variant="ghost" on_click=revert>{move || t("Original wiederherstellen", "Revert to Original")}</TextButton>
                <label class="flex items-center gap-1.5">
                    <input type="checkbox" class="accent-blue-500" prop:checked=move || resize_out.get()
                        on:change=move |e| { let v = event_target_checked(&e); resize_out.set(v); let c = crop.get_untracked(); out.set((c.w.round() as u32, c.h.round() as u32)); } />
                    {move || t("Ausgabegröße ändern", "Resize output")}
                </label>
                <Show when=move || resize_out.get()>
                    <input type="number" min="1" class=num_input title=move || t("Ausgabebreite in Pixeln", "Output width in pixels")
                        prop:value=move || out.get().0.to_string()
                        on:change=move |e| { if let Ok(v) = event_target_value(&e).parse::<u32>() {
                            let c = crop.get_untracked();
                            let h = if out_lock.get_untracked() { ((v as f64) * c.h / c.w.max(1.0)).round() as u32 } else { out.get_untracked().1 };
                            out.set((v.max(1), h.max(1)));
                        } } />
                    <span class="text-zinc-400">"×"</span>
                    <input type="number" min="1" class=num_input title=move || t("Ausgabehöhe in Pixeln", "Output height in pixels")
                        prop:value=move || out.get().1.to_string()
                        on:change=move |e| { if let Ok(v) = event_target_value(&e).parse::<u32>() {
                            let c = crop.get_untracked();
                            let w = if out_lock.get_untracked() { ((v as f64) * c.w / c.h.max(1.0)).round() as u32 } else { out.get_untracked().0 };
                            out.set((w.max(1), v.max(1)));
                        } } />
                    <span class="text-zinc-400">"px"</span>
                    <button class="text-[13px]" title=move || if out_lock.get() { t("Seitenverhältnis des Zuschnitts beibehalten", "Keep crop aspect ratio") } else { t("Eigenes Ausgabeverhältnis erlauben", "Allow custom output ratio") }
                        on:click=move |_| out_lock.update(|l| *l = !*l)>
                        {move || if out_lock.get() { "🔒" } else { "🔓" }}
                    </button>
                </Show>
            </div>
        </div>
    }
}
