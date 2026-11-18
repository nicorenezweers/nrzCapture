//! Vorschau: Live-Komposition auf `<canvas>` (Port von Capso `EditorPreviewView`/`MetalPreviewRenderer`/`BlurRegionOverlay`).
//! Gleiche Geometrie wie der Export (`model::Layout`, `model::Engine`); Weichzeichnen per Verkleinern/Vergrößern
//! (+ `ctx.filter`, wo der WebView es kann).
use super::{model::*, Ed};
use crate::api::{self, t};
use leptos::prelude::*;
use std::cell::RefCell;
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{CanvasRenderingContext2d as Ctx, HtmlCanvasElement, HtmlImageElement, HtmlVideoElement};

thread_local! {
    static SCRATCH: RefCell<Option<HtmlCanvasElement>> = const { RefCell::new(None) };
    static BG_IMAGE: RefCell<Option<(String, HtmlImageElement)>> = const { RefCell::new(None) };
}

fn scratch(w: u32, h: u32) -> Option<(HtmlCanvasElement, Ctx)> {
    let c = SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_none() {
            *s = document().create_element("canvas").ok().and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok());
        }
        s.clone()
    })?;
    c.set_width(w.max(1));
    c.set_height(h.max(1));
    let ctx = c.get_context("2d").ok()??.dyn_into::<Ctx>().ok()?;
    Some((c, ctx))
}

fn bg_image(path: &str, ed: Ed) -> Option<HtmlImageElement> {
    BG_IMAGE.with(|b| {
        let mut b = b.borrow_mut();
        if b.as_ref().map(|(p, _)| p != path).unwrap_or(true) {
            let img = HtmlImageElement::new().ok()?;
            let onload = Closure::<dyn Fn()>::new(move || ed.mark_dirty());
            img.set_onload(Some(onload.as_ref().unchecked_ref()));
            onload.forget();
            img.set_src(&api::asset_url(path));
            *b = Some((path.to_owned(), img));
        }
        b.as_ref().map(|(_, i)| i.clone()).filter(|i| i.complete() && i.natural_width() > 0)
    })
}

fn rounded_path(ctx: &Ctx, cmds: &[PathCmd]) {
    ctx.begin_path();
    for c in cmds {
        match *c {
            PathCmd::Move(x, y) => ctx.move_to(x, y),
            PathCmd::Line(x, y) => ctx.line_to(x, y),
            PathCmd::Cubic(a, b, c, d, e, f) => ctx.bezier_curve_to(a, b, c, d, e, f),
            PathCmd::Close => ctx.close_path(),
        }
    }
}

fn set_filter(ctx: &Ctx, f: &str) {
    let _ = js_sys::Reflect::set(ctx, &"filter".into(), &f.into());
}

/// Zeichnet das aktuelle Bild in die Vorschau.
pub fn draw(ed: Ed) {
    if !ed.loaded.get_untracked() {
        return;
    }
    let (Some(canvas), Some(video)) = (ed.canvas_ref.get_untracked(), ed.video_ref.get_untracked()) else { return };
    let Some(stage) = canvas.closest(".receditor-stage").ok().flatten() else { return };
    let (cw, ch) = (stage.client_width() as f64, stage.client_height() as f64);
    if cw < 4.0 || ch < 4.0 {
        return;
    }
    let t = ed.time.get_untracked();
    ed.engine.with_value(|engine| {
        let Some(engine) = engine else { return };
        let p = &engine.project;
        let base = Layout::base(p);
        let f = (cw / base.out_w).min(ch / base.out_h).max(0.01);
        let css = base.scaled(f);
        if ed.view_layout.get_untracked() != Some(css) {
            ed.view_layout.set(Some(css));
        }
        let dpr = window().device_pixel_ratio().max(1.0);
        let l = css.scaled(dpr);
        let (pw, ph) = (l.out_w.round().max(1.0) as u32, l.out_h.round().max(1.0) as u32);
        if canvas.width() != pw {
            canvas.set_width(pw);
        }
        if canvas.height() != ph {
            canvas.set_height(ph);
        }
        let Some(ctx) = canvas.get_context("2d").ok().flatten().and_then(|c| c.dyn_into::<Ctx>().ok()) else { return };
        draw_frame(&ctx, &canvas, &video, engine, &l, t, ed);
    });
}

fn draw_frame(ctx: &Ctx, canvas: &HtmlCanvasElement, video: &HtmlVideoElement, engine: &Engine, l: &Layout, t: f64, ed: Ed) {
    let p = &engine.project;
    let bg = &p.background_style;
    let (ow, oh) = (l.out_w, l.out_h);
    let (sw, sh) = (p.video_width.max(1.0), p.video_height.max(1.0));
    let ready = video.ready_state() >= 2;
    ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0).ok();
    ctx.set_global_alpha(1.0);
    ctx.set_image_smoothing_enabled(true);
    ctx.clear_rect(0.0, 0.0, ow, oh);

    // Hintergrund
    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, ow, oh);
    if bg.enabled {
        match bg.color_type {
            BackgroundColorType::Solid => {
                ctx.set_fill_style_str(&bg.solid_color.css());
                ctx.fill_rect(0.0, 0.0, ow, oh);
            }
            BackgroundColorType::Gradient => {
                let ((sx, sy), (ex, ey)) = bg.gradient_unit_points();
                let g = ctx.create_linear_gradient(sx * ow, sy * oh, ex * ow, ey * oh);
                let _ = g.add_color_stop(0.0, &bg.gradient_from.css());
                let _ = g.add_color_stop(1.0, &bg.gradient_to.css());
                ctx.set_fill_style_canvas_gradient(&g);
                ctx.fill_rect(0.0, 0.0, ow, oh);
            }
            BackgroundColorType::LiquidGlass => {
                ctx.set_fill_style_str(&bg.solid_color.css());
                ctx.fill_rect(0.0, 0.0, ow, oh);
                if ready {
                    let d = 16.0;
                    if let Some((sc, sctx)) = scratch((ow / d).ceil() as u32, (oh / d).ceil() as u32) {
                        let cover = (ow / sw).max(oh / sh) * 1.15 / d;
                        let (dw, dh) = (sw * cover, sh * cover);
                        let _ = sctx.draw_image_with_html_video_element_and_dw_and_dh(video, (ow / d - dw) / 2.0, (oh / d - dh) / 2.0, dw, dh);
                        set_filter(ctx, &format!("blur({:.0}px) saturate(1.9) contrast(0.95)", (120.0 * l.k / 4.0).max(4.0)));
                        let _ = ctx.draw_image_with_html_canvas_element_and_dw_and_dh(&sc, -ow * 0.05, -oh * 0.05, ow * 1.1, oh * 1.1);
                        set_filter(ctx, "none");
                    }
                }
            }
            BackgroundColorType::Image => {
                ctx.set_fill_style_str(&bg.solid_color.css());
                ctx.fill_rect(0.0, 0.0, ow, oh);
                if let Some(img) = bg.image_path.as_deref().and_then(|path| bg_image(path, ed)) {
                    let (iw, ih) = (img.natural_width() as f64, img.natural_height() as f64);
                    let s = (ow / iw).max(oh / ih);
                    let _ = ctx.draw_image_with_html_image_element_and_dw_and_dh(&img, (ow - iw * s) / 2.0, (oh - ih * s) / 2.0, iw * s, ih * s);
                }
            }
        }
        // Schatten (nur der Schatten wird gezeichnet: Form außerhalb, Versatz zurück)
        if bg.shadow_enabled && bg.shadow_opacity > 0.0 {
            const OFF: f64 = 100_000.0;
            ctx.save();
            ctx.set_shadow_color(&format!("rgba(0,0,0,{})", bg.shadow_opacity.clamp(0.0, 1.0)));
            ctx.set_shadow_blur(bg.shadow_radius * l.k * 2.0);
            ctx.set_shadow_offset_x(OFF);
            ctx.set_fill_style_str("#000");
            ctx.fill_rect(l.frame_x - OFF, l.frame_y, l.frame_w, l.frame_h);
            ctx.restore();
        }
    }

    // Videorahmen (weiche Ecken) mit Zoom
    let state = engine.frame(t);
    ctx.save();
    rounded_path(ctx, &continuous_rounded_rect(l.frame_x, l.frame_y, l.frame_w, l.frame_h, l.corner_radius));
    ctx.clip();
    ctx.set_fill_style_str("#000");
    ctx.fill_rect(l.frame_x, l.frame_y, l.frame_w, l.frame_h);
    let (fx, fy) = state.zoom.focus();
    let s = state.zoom.scale;
    if ready {
        ctx.translate(l.frame_x + l.frame_w / 2.0, l.frame_y + l.frame_h / 2.0).ok();
        ctx.scale(s * l.frame_w / sw, s * l.frame_h / sh).ok();
        ctx.translate(-fx * sw, -fy * sh).ok();
        let _ = ctx.draw_image_with_html_video_element_and_dw_and_dh(video, 0.0, 0.0, sw, sh);
        ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0).ok();
    }

    // Weichzeichner-Regionen (bildschirmfest im Rahmen)
    for b in &state.blurs {
        let r = b.rect.clamped(0.0);
        let (rx, ry, rw, rh) = (l.frame_x + r.x * l.frame_w, l.frame_y + r.y * l.frame_h, r.width * l.frame_w, r.height * l.frame_h);
        if rw < 2.0 || rh < 2.0 {
            continue;
        }
        let q = (b.radius * l.k / 2.5).clamp(2.0, 48.0);
        if let Some((sc, sctx)) = scratch((rw / q).ceil() as u32, (rh / q).ceil() as u32) {
            sctx.set_image_smoothing_enabled(true);
            let (scw, sch) = (sc.width() as f64, sc.height() as f64);
            let _ = sctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(canvas, rx, ry, rw, rh, 0.0, 0.0, scw, sch);
            set_filter(ctx, &format!("blur({:.1}px)", (b.radius * l.k * 0.5).max(1.0)));
            let _ = ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(&sc, 0.0, 0.0, scw, sch, rx, ry, rw, rh);
            set_filter(ctx, "none");
        }
    }

    // Klick-Welle + Cursor
    if let Some(c) = state.cursor {
        let (px, py) = l.place(&state.zoom, c.x, c.y);
        let (px, py) = (px + l.frame_x, py + l.frame_y);
        let ch = p.cursor_height();
        let h = ch * s * l.k;
        if let Some((rr, op)) = c.ripple {
            let radius = ch * 1.3 * rr * s * l.k;
            let a = 0.35 * op * c.alpha;
            if radius > 0.5 {
                if let Ok(g) = ctx.create_radial_gradient(px, py, 0.0, px, py, radius) {
                    let col = |a: f64| format!("rgba(64,140,255,{a})");
                    let _ = g.add_color_stop(0.0, &col(a));
                    let _ = g.add_color_stop(0.82, &col(a));
                    let _ = g.add_color_stop(1.0, &col(0.0));
                    ctx.set_fill_style_canvas_gradient(&g);
                    ctx.begin_path();
                    let _ = ctx.arc(px, py, radius, 0.0, std::f64::consts::TAU);
                    ctx.fill();
                }
            }
        }
        draw_cursor(ctx, px, py, h * c.click_scale, c.alpha);
    }
    ctx.restore();
}

/// macOS-ähnlicher Pfeil (wie `render::draw_cursor` im Export).
pub fn draw_cursor(ctx: &Ctx, x: f64, y: f64, h: f64, alpha: f64) {
    if h < 1.0 || alpha <= 0.0 {
        return;
    }
    ctx.save();
    ctx.begin_path();
    for (i, (px, py)) in CURSOR_ARROW.iter().enumerate() {
        if i == 0 {
            ctx.move_to(x + px * h, y + py * h);
        } else {
            ctx.line_to(x + px * h, y + py * h);
        }
    }
    ctx.close_path();
    ctx.set_line_join("round");
    ctx.set_line_width(h * 0.11);
    ctx.set_global_alpha(alpha.clamp(0.0, 1.0));
    ctx.translate(0.0, h * 0.04).ok();
    ctx.set_stroke_style_str("rgba(0,0,0,0.25)");
    ctx.stroke();
    ctx.translate(0.0, -h * 0.04).ok();
    ctx.set_stroke_style_str("#fff");
    ctx.stroke();
    ctx.set_fill_style_str("#000");
    ctx.fill();
    ctx.restore();
}

#[derive(Clone, Copy)]
enum Drag {
    Move { x0: f64, y0: f64, rect: NormalizedRect },
    Resize { x0: f64, y0: f64, rect: NormalizedRect },
}

#[component]
pub fn Preview(ed: Ed) -> impl IntoView {
    let src = move || {
        let v = ed.video.get();
        (!v.is_empty()).then(|| api::asset_url(&v))
    };
    let drag: StoredValue<Option<Drag>> = StoredValue::new(None);
    let frame_style = move || {
        ed.view_layout.get().map(|l| format!("left:{}px;top:{}px;width:{}px;height:{}px", l.frame_x, l.frame_y, l.frame_w, l.frame_h)).unwrap_or_default()
    };
    let size_style = move || ed.view_layout.get().map(|l| format!("width:{}px;height:{}px", l.out_w, l.out_h)).unwrap_or_else(|| "width:0;height:0".into());

    // Aktuelle Zoom-Transformation (für Fokus-Klicks)
    let zoom_now = move || {
        ed.project.track();
        let t = ed.time.get();
        ed.engine.with_value(|e| e.as_ref().map(|e| e.frame(t).zoom)).unwrap_or_default()
    };
    let sel_blur = move || ed.selected_effect().and_then(|e| match e.payload {
        EffectPayload::Blur(b) => Some((e.id.clone(), b)),
        _ => None,
    });
    let sel_manual_zoom = move || ed.selected_effect().and_then(|e| match e.payload {
        EffectPayload::Zoom(ZoomEffectPayload { focus_mode: ZoomFocusMode::Manual { x, y }, .. }) => Some((e.id.clone(), x, y)),
        _ => None,
    });

    let set_blur_rect = move |id: &str, rect: NormalizedRect| {
        ed.update_effect(id, |e| {
            if let EffectPayload::Blur(b) = &mut e.payload {
                b.rect = rect.clamped(0.04);
            }
        })
    };

    let on_frame_click = move |e: web_sys::MouseEvent| {
        let Some((id, _, _)) = sel_manual_zoom() else {
            if sel_blur().is_none() {
                ed.sel.set(None);
            }
            return;
        };
        let Some(l) = ed.view_layout.get_untracked() else { return };
        let Some(target) = e.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
        let r = target.get_bounding_client_rect();
        let (vx, vy) = (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top());
        let z = zoom_now();
        let (fx, fy) = z.focus();
        let x = ((vx - l.frame_w / 2.0) / (l.frame_w * z.scale) + fx).clamp(0.0, 1.0);
        let y = ((vy - l.frame_h / 2.0) / (l.frame_h * z.scale) + fy).clamp(0.0, 1.0);
        ed.update_effect(&id, |e| {
            if let EffectPayload::Zoom(zp) = &mut e.payload {
                zp.focus_mode = ZoomFocusMode::Manual { x, y };
            }
        });
    };

    view! {
        <div class="receditor-stage flex size-full items-center justify-center">
            <video
                node_ref=ed.video_ref
                src=src
                preload="auto"
                playsinline=true
                class="pointer-events-none fixed left-0 top-0 size-px opacity-0"
                on:loadeddata=move |_| { ed.seek(ed.time.get_untracked()); ed.mark_dirty() }
                on:seeked=move |_| ed.mark_dirty()
                on:pause=move |_| { ed.playing.set(false); ed.mark_dirty() }
            />
            <div class="relative" style=size_style>
                <canvas node_ref=ed.canvas_ref class="block size-full rounded-[3px] shadow-md" />
                <div class="absolute" style=frame_style on:click=on_frame_click>
                    {move || sel_blur().map(|(id, b)| {
                        let id2 = id.clone();
                        let r = b.rect;
                        let down = move |e: web_sys::PointerEvent, resize: bool| {
                            e.stop_propagation();
                            e.prevent_default();
                            if let Some(t) = e.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                                let _ = t.set_pointer_capture(e.pointer_id());
                            }
                            let (x0, y0) = (e.client_x() as f64, e.client_y() as f64);
                            drag.set_value(Some(if resize { Drag::Resize { x0, y0, rect: r } } else { Drag::Move { x0, y0, rect: r } }));
                        };
                        let mv = move |e: web_sys::PointerEvent| {
                            let (Some(d), Some(l)) = (drag.get_value(), ed.view_layout.get_untracked()) else { return };
                            e.stop_propagation();
                            let dx = |x0: f64| (e.client_x() as f64 - x0) / l.frame_w;
                            let dy = |y0: f64| (e.client_y() as f64 - y0) / l.frame_h;
                            let rect = match d {
                                Drag::Move { x0, y0, rect } => NormalizedRect { x: rect.x + dx(x0), y: rect.y + dy(y0), ..rect },
                                Drag::Resize { x0, y0, rect } => NormalizedRect { width: rect.width + dx(x0), height: rect.height + dy(y0), ..rect },
                            };
                            set_blur_rect(&id2, rect);
                        };
                        let up = move |_e: web_sys::PointerEvent| drag.set_value(None);
                        let mv2 = mv.clone();
                        view! {
                            <div
                                class="absolute cursor-move rounded border-2 border-dashed border-red-400 bg-red-400/10"
                                style=format!("left:{}%;top:{}%;width:{}%;height:{}%", r.x * 100.0, r.y * 100.0, r.width * 100.0, r.height * 100.0)
                                on:pointerdown=move |e| down(e, false)
                                on:pointermove=mv
                                on:pointerup=up
                                on:click=|e| e.stop_propagation()
                            >
                                <span class="absolute left-1 top-0.5 rounded bg-red-500 px-1 text-[10px] font-medium text-white">{t("Weichzeichner", "Blur")}</span>
                                <div
                                    class="absolute -bottom-1.5 -right-1.5 size-3 cursor-nwse-resize rounded-full border-2 border-white bg-red-500 shadow"
                                    on:pointerdown=move |e| down(e, true)
                                    on:pointermove=mv2
                                    on:pointerup=up
                                />
                            </div>
                        }
                    })}
                    {move || sel_manual_zoom().map(|(_, x, y)| {
                        let l = ed.view_layout.get().unwrap_or(Layout { out_w: 0.0, out_h: 0.0, frame_x: 0.0, frame_y: 0.0, frame_w: 0.0, frame_h: 0.0, k: 1.0, corner_radius: 0.0 });
                        let (px, py) = l.place(&zoom_now(), x, y);
                        view! {
                            <div class="pointer-events-none absolute size-6 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white bg-blue-500/40 shadow-[0_0_0_1px_rgba(0,0,0,0.3)]"
                                style=format!("left:{px}px;top:{py}px") />
                            <div class="pointer-events-none absolute bottom-2 left-1/2 -translate-x-1/2 rounded-full bg-black/60 px-2 py-0.5 text-[11px] text-white">
                                {t("Klicken, um den Zoom-Fokus zu setzen", "Click to set the zoom focus")}
                            </div>
                        }
                    })}
                </div>
            </div>
        </div>
    }
}
