//! Canvas-2D-Renderer (Port von Capso `AnnotationRenderer`, Objekt-`render`, `BeautifyRenderer`).
//! Zeichnet in Bildpixel-Koordinaten; der Aufrufer setzt vorher die Transformation (Zoom, Scroll).
use super::model::*;
use std::f64::consts::PI;
use wasm_bindgen::{Clamped, JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d as Ctx, HtmlCanvasElement, HtmlImageElement, ImageData};

pub const ACCENT: &str = "#0a84ff";
pub const FONT_STACK: &str = "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Helvetica Neue\", sans-serif";

pub fn canvas(w: f64, h: f64) -> HtmlCanvasElement {
    let c: HtmlCanvasElement = web_sys::window().unwrap().document().unwrap().create_element("canvas").unwrap().unchecked_into();
    c.set_width(w.max(1.0).round() as u32);
    c.set_height(h.max(1.0).round() as u32);
    c
}

pub fn ctx(c: &HtmlCanvasElement) -> Ctx {
    c.get_context("2d").ok().flatten().unwrap().unchecked_into()
}

/// Arbeitsbild = Original mit Drehen/Spiegeln als eigenes Canvas.
pub fn build_source(img: &HtmlImageElement, xf: Xform) -> HtmlCanvasElement {
    let (w, h) = (img.natural_width() as f64, img.natural_height() as f64);
    let (ww, wh) = xf.size(w, h);
    let c = canvas(ww, wh);
    let g = ctx(&c);
    let [a, b, cc, d, e, f] = xf.matrix(w, h);
    let _ = g.set_transform(a, b, cc, d, e, f);
    let _ = g.draw_image_with_html_image_element(img, 0.0, 0.0);
    c
}

fn round_rect(g: &Ctx, r: &Rect, rad: f64) {
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    g.move_to(r.x + rad, r.y);
    let _ = g.arc_to(r.max_x(), r.y, r.max_x(), r.max_y(), rad);
    let _ = g.arc_to(r.max_x(), r.max_y(), r.x, r.max_y(), rad);
    let _ = g.arc_to(r.x, r.max_y(), r.x, r.y, rad);
    let _ = g.arc_to(r.x, r.y, r.max_x(), r.y, rad);
    g.close_path();
}

/// `imageSmoothingQuality = "high"` (nicht in web-sys).
fn smooth_high(g: &Ctx) {
    let _ = js_sys::Reflect::set(g, &"imageSmoothingQuality".into(), &"high".into());
}

fn set_dash(g: &Ctx, d: &[f64]) {
    let arr = js_sys::Array::new();
    for v in d {
        arr.push(&JsValue::from_f64(*v));
    }
    let _ = g.set_line_dash(&arr);
}

pub fn text_font(fs: f64) -> String {
    format!("500 {fs}px {FONT_STACK}")
}

fn scratch_ctx() -> Ctx {
    thread_local!(static C: HtmlCanvasElement = canvas(1.0, 1.0));
    C.with(|c| ctx(c))
}

/// Textgröße (Breite der längsten Zeile, Zeilen × Zeilenhöhe).
pub fn measure_text(text: &str, fs: f64) -> (f64, f64) {
    let g = scratch_ctx();
    g.set_font(&text_font(fs));
    let lines: Vec<&str> = text.split('\n').collect();
    let w = lines.iter().map(|l| g.measure_text(l).map(|m| m.width()).unwrap_or(0.0)).fold(0.0, f64::max);
    (w.ceil().max(fs * 0.3), (lines.len() as f64 * fs * TEXT_LINE_HEIGHT).ceil())
}

/// Wörter mit nachfolgenden Leerzeichen (Umbruchstellen wie CSS `pre-wrap`).
fn wrap_tokens(para: &str) -> Vec<String> {
    let mut toks = vec![];
    let mut cur = String::new();
    let mut in_space = false;
    for c in para.chars() {
        if c != ' ' && in_space && !cur.is_empty() {
            toks.push(std::mem::take(&mut cur));
            in_space = false;
        }
        if c == ' ' {
            in_space = true;
        }
        cur.push(c);
    }
    if !cur.is_empty() || toks.is_empty() {
        toks.push(cur);
    }
    toks
}

/// Text in Zeilen der Breite `max_w` umbrechen (an Leerzeichen, überlange Wörter zeichenweise – wie das Textfeld).
pub fn wrap_lines(text: &str, fs: f64, max_w: f64) -> Vec<String> {
    let g = scratch_ctx();
    g.set_font(&text_font(fs));
    let mw = |s: &str| g.measure_text(s.trim_end()).map(|m| m.width()).unwrap_or(0.0);
    let mut out = vec![];
    for para in text.split('\n') {
        let mut line = String::new();
        for tok in wrap_tokens(para) {
            let cand = format!("{line}{tok}");
            if mw(&cand) <= max_w {
                line = cand;
                continue;
            }
            if !line.trim().is_empty() {
                out.push(line.trim_end().to_string());
                line = String::new();
            }
            if mw(&tok) <= max_w {
                line = tok;
                continue;
            }
            for ch in tok.chars() {
                let cand = format!("{line}{ch}");
                if !line.is_empty() && mw(&cand) > max_w {
                    out.push(line.trim_end().to_string());
                    line = ch.to_string();
                } else {
                    line = cand;
                }
            }
        }
        out.push(line.trim_end().to_string());
    }
    out
}

/// Größe des umbrochenen Textes (Breite der längsten Zeile, Zeilen × Zeilenhöhe).
pub fn measure_wrapped(text: &str, fs: f64, max_w: f64) -> (f64, f64) {
    let lines = wrap_lines(text, fs, max_w);
    let g = scratch_ctx();
    g.set_font(&text_font(fs));
    let w = lines.iter().map(|l| g.measure_text(l).map(|m| m.width()).unwrap_or(0.0)).fold(0.0, f64::max);
    (w.ceil(), (lines.len() as f64 * fs * TEXT_LINE_HEIGHT).ceil())
}

/// Schachbrett (weiß / hellgrau, 8 pt) für den Bereich, um den die Zeichenfläche über das Bild hinausgeht
/// (Capso `drawTransparencyChecker`). Zeichnet in CSS-Pixeln: `area` = Zeichenfläche, `img` = Bild.
pub fn draw_checker(g: &Ctx, dpr: f64, area: Rect, img: Rect) {
    if img.contains_rect(&area) || area.is_empty() {
        return;
    }
    let tile = 8.0;
    g.save();
    let _ = g.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    g.begin_path();
    g.rect(area.x, area.y, area.w, area.h);
    g.clip();
    g.set_fill_style_str("#fff");
    g.fill_rect(area.x, area.y, area.w, area.h);
    g.set_fill_style_str("rgb(217,217,217)");
    g.begin_path();
    let (cols, rows) = ((area.w / tile).ceil() as i64, (area.h / tile).ceil() as i64);
    for r in 0..rows {
        for c in (r % 2..cols).step_by(2) {
            g.rect(area.x + c as f64 * tile, area.y + r as f64 * tile, tile, tile);
        }
    }
    g.fill();
    g.restore();
}

// ───────────────────────────── Objekte ─────────────────────────────

pub fn draw_object(g: &Ctx, o: &Object, src: Option<&HtmlCanvasElement>) {
    let s = &o.style;
    let color = css_rgba(&s.color, 1.0);
    g.save();
    g.set_global_alpha(s.opacity);
    g.set_stroke_style_str(&color);
    g.set_fill_style_str(&color);
    g.set_line_width(s.line_width);
    match &o.shape {
        Shape::Arrow { start, end, ctrl } => draw_arrow(g, s, *start, *end, *ctrl),
        Shape::Line { start, end, ctrl } => {
            g.set_line_cap("round");
            set_dash(g, &s.pattern.dash(s.line_width));
            g.begin_path();
            g.move_to(start.x, start.y);
            match ctrl {
                Some(c) => g.quadratic_curve_to(c.x, c.y, end.x, end.y),
                None => g.line_to(end.x, end.y),
            }
            g.stroke();
        }
        Shape::Rect { rect, radius } => {
            g.begin_path();
            if *radius > 0.0 {
                round_rect(g, rect, *radius);
            } else {
                g.rect(rect.x, rect.y, rect.w, rect.h);
            }
            if s.filled {
                g.fill();
            } else {
                set_dash(g, &s.pattern.dash(s.line_width));
                g.stroke();
            }
        }
        Shape::Ellipse { rect } => {
            g.begin_path();
            let c = rect.center();
            let _ = g.ellipse(c.x, c.y, rect.w / 2.0, rect.h / 2.0, 0.0, 0.0, 2.0 * PI);
            if s.filled {
                g.fill();
            } else {
                set_dash(g, &s.pattern.dash(s.line_width));
                g.stroke();
            }
        }
        Shape::Freehand { points } => {
            g.set_line_cap("round");
            g.set_line_join("round");
            g.begin_path();
            if let Some(p0) = points.first() {
                g.move_to(p0.x, p0.y);
                if points.len() == 1 {
                    g.line_to(p0.x + 0.01, p0.y);
                }
                for (c1, c2, p) in smooth_segments(points) {
                    g.bezier_curve_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y);
                }
            }
            g.stroke();
        }
        Shape::Pixelate { rect, block, mode } => draw_redaction(g, s, *rect, *block, *mode, src),
        Shape::Counter { center, tip, radius, number } => {
            g.set_global_alpha(1.0);
            if counter_has_arrow(*center, *tip, *radius) {
                let t = tip.unwrap();
                let lw = counter_arrow_width(*radius, s.line_width);
                g.set_line_width(lw);
                g.set_line_cap("round");
                g.begin_path();
                g.move_to(center.x, center.y);
                g.line_to(t.x, t.y);
                let a = (t.y - center.y).atan2(t.x - center.x);
                let hl = 10f64.max(lw * 3.0);
                for sp in [-PI / 6.0, PI / 6.0] {
                    g.move_to(t.x, t.y);
                    g.line_to(t.x - hl * (a + sp).cos(), t.y - hl * (a + sp).sin());
                }
                g.stroke();
            }
            g.begin_path();
            let _ = g.arc(center.x, center.y, *radius, 0.0, 2.0 * PI);
            g.fill();
            g.set_stroke_style_str("rgba(0,0,0,0.25)");
            g.set_line_width(1.5);
            g.stroke();
            let fs = if *number < 10 { radius * 1.1 } else { radius * 0.85 };
            g.set_fill_style_str("#fff");
            g.set_font(&format!("700 {fs}px {FONT_STACK}"));
            g.set_text_align("center");
            g.set_text_baseline("middle");
            let _ = g.fill_text(&number.to_string(), center.x, center.y + fs * 0.04);
        }
        Shape::Text { text, origin, font_size, w, h, boxed, fill, outline, trace } => {
            g.set_global_alpha(1.0);
            if fill.is_some() || outline.is_some() {
                let r = rect(origin.x, origin.y, *w, *h).inset(-TEXT_BOX_PADDING, -TEXT_BOX_PADDING);
                g.begin_path();
                round_rect(g, &r, 4.0);
                if let Some(f) = fill {
                    g.set_fill_style_str(&css_rgba(f, 0.5));
                    g.fill();
                }
                if let Some(oc) = outline {
                    g.set_stroke_style_str(&css_rgba(oc, 1.0));
                    g.set_line_width(2.0);
                    g.stroke();
                }
            }
            g.set_font(&text_font(*font_size));
            g.set_text_baseline("top");
            g.set_text_align("left");
            let lh = font_size * TEXT_LINE_HEIGHT;
            let dy = (lh - font_size) / 2.0;
            // Textbox: innerhalb der Box (Innenabstand) umbrechen (Capso `draw(with: boxedTextRect)`).
            let (x0, y0, lines) = if *boxed {
                (origin.x + TEXT_BOX_PADDING, origin.y + TEXT_BOX_PADDING, wrap_lines(text, *font_size, (w - 2.0 * TEXT_BOX_PADDING).max(1.0)))
            } else {
                (origin.x, origin.y, text.split('\n').map(str::to_string).collect())
            };
            for (i, line) in lines.iter().enumerate() {
                let y = y0 + i as f64 * lh + dy;
                if let Some(tc) = trace {
                    g.set_stroke_style_str(&css_rgba(tc, 1.0));
                    g.set_line_width(font_size * 0.06);
                    g.set_line_join("round");
                    let _ = g.stroke_text(line, x0, y);
                }
                g.set_fill_style_str(&css_rgba(&s.color, s.opacity));
                let _ = g.fill_text(line, x0, y);
            }
        }
        Shape::Spotlight { .. } => {}
    }
    g.restore();
}

fn head_corners(tip: Pt, a: f64, len: f64, spread: f64) -> (Pt, Pt) {
    (pt(tip.x - len * (a - spread).cos(), tip.y - len * (a - spread).sin()), pt(tip.x - len * (a + spread).cos(), tip.y - len * (a + spread).sin()))
}

fn fill_head(g: &Ctx, tip: Pt, a: f64, len: f64) {
    let (p1, p2) = head_corners(tip, a, len, PI / 7.0);
    g.begin_path();
    g.move_to(tip.x, tip.y);
    g.line_to(p1.x, p1.y);
    g.line_to(p2.x, p2.y);
    g.close_path();
    g.fill();
}

/// Punkte der Schaftlinie, vorne/hinten um `ts`/`te` gekürzt.
fn shaft_points(s: Pt, e: Pt, c: Option<Pt>, ts: f64, te: f64) -> Vec<Pt> {
    let pts = curve_points(s, c, e, if c.is_none() { 1 } else { 32 });
    let mut lens = vec![0.0];
    for w in pts.windows(2) {
        lens.push(lens.last().unwrap() + w[0].dist(w[1]));
    }
    let total = *lens.last().unwrap();
    let from = ts.min(total / 2.0);
    let to = (total - te).max(from);
    let at = |d: f64| -> Pt {
        match lens.iter().position(|&l| l >= d) {
            Some(i) if i > 0 => {
                let t = (d - lens[i - 1]) / (lens[i] - lens[i - 1]).max(f64::EPSILON);
                pt(pts[i - 1].x + (pts[i].x - pts[i - 1].x) * t, pts[i - 1].y + (pts[i].y - pts[i - 1].y) * t)
            }
            _ => pts[0],
        }
    };
    let mut out = vec![at(from)];
    out.extend(pts.iter().zip(&lens).filter(|(_, &l)| l > from && l < to).map(|(p, _)| *p));
    out.push(at(to));
    out
}

fn draw_arrow(g: &Ctx, s: &Style, start: Pt, end: Pt, ctrl: Option<Pt>) {
    let hl = 15f64.max(s.line_width * 3.0);
    let ea = curve_end_angle(start, ctrl, end);
    let stroke_shaft = |cap: &str, ts: f64, te: f64| {
        g.set_line_cap(cap);
        set_dash(g, &s.pattern.dash(s.line_width));
        g.begin_path();
        if ts == 0.0 && te == 0.0 {
            g.move_to(start.x, start.y);
            match ctrl {
                Some(c) => g.quadratic_curve_to(c.x, c.y, end.x, end.y),
                None => g.line_to(end.x, end.y),
            }
        } else {
            for (i, p) in shaft_points(start, end, ctrl, ts, te).iter().enumerate() {
                if i == 0 {
                    g.move_to(p.x, p.y)
                } else {
                    g.line_to(p.x, p.y)
                }
            }
        }
        g.stroke();
        set_dash(g, &[]);
    };
    match s.arrow_head {
        ArrowHead::Standard => {
            stroke_shaft("round", 0.0, 0.0);
            let (p1, p2) = head_corners(end, ea, hl, PI / 6.0);
            g.set_line_join("round");
            g.begin_path();
            g.move_to(p1.x, p1.y);
            g.line_to(end.x, end.y);
            g.line_to(p2.x, p2.y);
            g.stroke();
        }
        ArrowHead::Filled | ArrowHead::Double => {
            let len = hl * 1.2;
            let trim = len * 0.8;
            let double = s.arrow_head == ArrowHead::Double;
            stroke_shaft("butt", if double { trim } else { 0.0 }, trim);
            fill_head(g, end, ea, len);
            if double {
                fill_head(g, start, curve_end_angle(end, ctrl, start), len);
            }
        }
        ArrowHead::Tapered => {
            let len = hl * 1.3;
            let pts = shaft_points(start, end, ctrl, 0.0, len * 0.8);
            let total: f64 = pts.windows(2).map(|w| w[0].dist(w[1])).sum();
            if total > 0.0 {
                let (mut left, mut right) = (vec![], vec![]);
                let mut tr = 0.0;
                for i in 0..pts.len() {
                    if i > 0 {
                        tr += pts[i].dist(pts[i - 1]);
                    }
                    let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(pts.len() - 1)]);
                    let ang = (b.y - a.y).atan2(b.x - a.x);
                    let half = s.line_width * (0.15 + 0.85 * (tr / total.max(1.0)).min(1.0));
                    left.push(pt(pts[i].x - half * ang.sin(), pts[i].y + half * ang.cos()));
                    right.push(pt(pts[i].x + half * ang.sin(), pts[i].y - half * ang.cos()));
                }
                g.begin_path();
                for (i, p) in left.iter().chain(right.iter().rev()).enumerate() {
                    if i == 0 {
                        g.move_to(p.x, p.y)
                    } else {
                        g.line_to(p.x, p.y)
                    }
                }
                g.close_path();
                g.fill();
            }
            fill_head(g, end, ea, len);
        }
    }
}

/// Verpixeln/Weichzeichnen aus dem Quellbild (ohne andere Anmerkungen), Capso `PixelateObject.renderWithSource`.
fn draw_redaction(g: &Ctx, s: &Style, r: Rect, block: f64, mode: RedactionMode, src: Option<&HtmlCanvasElement>) {
    if r.is_empty() {
        return;
    }
    if mode == RedactionMode::Solid {
        g.set_fill_style_str(&css_rgba(&s.color, 1.0));
        g.fill_rect(r.x, r.y, r.w, r.h);
        return;
    }
    g.set_global_alpha(1.0);
    let Some(src) = src else {
        // Vorschau ohne Quelle: graues Raster
        g.set_fill_style_str("rgba(128,128,128,0.3)");
        g.fill_rect(r.x, r.y, r.w, r.h);
        return;
    };
    let cr = r.intersect(&rect(0.0, 0.0, src.width() as f64, src.height() as f64)).integral();
    if cr.is_empty() {
        return;
    }
    let b = block.round().max(4.0);
    let (tw, th) = ((cr.w / b).floor().max(1.0), (cr.h / b).floor().max(1.0));
    let tiny = canvas(tw, th);
    let tg = ctx(&tiny);
    tg.set_image_smoothing_enabled(true);
    let _ = tg.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(src, cr.x, cr.y, cr.w, cr.h, 0.0, 0.0, tw, th);
    match mode {
        RedactionMode::Pixelate => {
            let (cw, ch) = ((tw / 2.0).floor().max(1.0), (th / 2.0).floor().max(1.0));
            let chunky = canvas(cw, ch);
            let cg = ctx(&chunky);
            let _ = cg.draw_image_with_html_canvas_element_and_dw_and_dh(&tiny, 0.0, 0.0, cw, ch);
            g.set_image_smoothing_enabled(false);
            let _ = g.draw_image_with_html_canvas_element_and_dw_and_dh(&chunky, cr.x, cr.y, cr.w, cr.h);
        }
        _ => {
            // Weichzeichnen: verkleinern + weich vergrößern (zweistufig gegen Blockkanten).
            let (cw, ch) = ((tw / 2.0).floor().max(1.0), (th / 2.0).floor().max(1.0));
            let small = canvas(cw, ch);
            let _ = ctx(&small).draw_image_with_html_canvas_element_and_dw_and_dh(&tiny, 0.0, 0.0, cw, ch);
            let mid = canvas((cw * 4.0).min(cr.w), (ch * 4.0).min(cr.h));
            let mg = ctx(&mid);
            mg.set_image_smoothing_enabled(true);
            let _ = mg.draw_image_with_html_canvas_element_and_dw_and_dh(&small, 0.0, 0.0, mid.width() as f64, mid.height() as f64);
            g.save();
            g.begin_path();
            g.rect(cr.x, cr.y, cr.w, cr.h);
            g.clip();
            g.set_image_smoothing_enabled(true);
            smooth_high(&g);
            let _ = g.draw_image_with_html_canvas_element_and_dw_and_dh(&mid, cr.x, cr.y, cr.w, cr.h);
            g.restore();
        }
    }
    g.set_image_smoothing_enabled(true);
}

/// Abdunkeln außerhalb aller Spotlight-Bereiche.
pub fn draw_spotlight_dim(g: &Ctx, objects: &[Object], extra: Option<Rect>, canvas_r: Rect) {
    let holes: Vec<Rect> = objects.iter().filter_map(|o| if let Shape::Spotlight { rect } = o.shape { Some(rect) } else { None }).chain(extra).collect();
    if holes.is_empty() {
        return;
    }
    g.save();
    g.set_fill_style_str(&format!("rgba(0,0,0,{SPOTLIGHT_DIM})"));
    g.begin_path();
    g.rect(canvas_r.x, canvas_r.y, canvas_r.w, canvas_r.h);
    for h in &holes {
        round_rect(g, h, SPOTLIGHT_RADIUS);
    }
    g.fill_with_canvas_winding_rule(web_sys::CanvasWindingRule::Evenodd);
    g.restore();
}

/// Bild + alle Anmerkungen (ohne `hide`, plus `extra` als Live-Vorschau).
pub fn draw_doc(g: &Ctx, src: &HtmlCanvasElement, doc: &Doc, hide: Option<u64>, extra: Option<&Object>) {
    let _ = g.draw_image_with_html_canvas_element(src, 0.0, 0.0);
    let canvas_r = canvas_rect(doc.image_rect(), &doc.objects).union(&doc.image_rect());
    let spot = extra.and_then(|o| if let Shape::Spotlight { rect } = o.shape { Some(rect) } else { None });
    draw_spotlight_dim(g, &doc.objects, spot, canvas_r);
    for o in doc.objects.iter().filter(|o| Some(o.id) != hide) {
        draw_object(g, o, Some(src));
    }
    if let Some(o) = extra {
        draw_object(g, o, Some(src));
    }
}

/// Auswahlrahmen und Griffe (Größen in Bildschirmpunkten, `zoom` = Bildschirm-px pro Bild-px).
pub fn draw_selection(g: &Ctx, o: &Object, zoom: f64) {
    let px = 1.0 / zoom;
    g.save();
    g.set_line_width(px);
    match &o.shape {
        Shape::Arrow { start, end, .. } | Shape::Line { start, end, .. } => {
            set_dash(g, &[3.0 * px, 3.0 * px]);
            g.set_stroke_style_str("rgba(10,132,255,0.6)");
            g.begin_path();
            g.move_to(start.x, start.y);
            g.line_to(end.x, end.y);
            g.stroke();
            set_dash(g, &[]);
        }
        Shape::Counter { .. } => {}
        _ => {
            let b = o.bounds();
            set_dash(g, &[4.0 * px, 3.0 * px]);
            g.set_stroke_style_str(ACCENT);
            g.stroke_rect(b.x, b.y, b.w, b.h);
            set_dash(g, &[]);
        }
    }
    for (h, c) in o.handles(zoom) {
        let r = 4.5 * px;
        g.begin_path();
        if matches!(h, Handle::Control | Handle::CounterTip | Handle::Start | Handle::End) {
            let _ = g.arc(c.x, c.y, r, 0.0, 2.0 * PI);
        } else {
            g.rect(c.x - r, c.y - r, 2.0 * r, 2.0 * r);
        }
        g.set_fill_style_str(if h == Handle::Control { ACCENT } else { "#fff" });
        g.fill();
        g.set_stroke_style_str(if h == Handle::Control { "#fff" } else { ACCENT });
        g.set_line_width(1.5 * px);
        g.stroke();
    }
    if let Shape::Counter { number, .. } = o.shape {
        for (a, r) in counter_buttons(o, zoom) {
            draw_action_button(g, r, a, a != CounterAction::Decrement || number > 1, px);
        }
    }
    g.restore();
}

/// Runder Aktionsknopf (Capso `drawActionButton`): dunkel, Akzentrand, weißes Symbol.
fn draw_action_button(g: &Ctx, r: Rect, kind: CounterAction, enabled: bool, px: f64) {
    let a = if enabled { 1.0 } else { 0.4 };
    let c = r.center();
    let rad = r.w / 2.0;
    g.save();
    g.begin_path();
    let _ = g.arc(c.x, c.y, rad, 0.0, 2.0 * PI);
    g.set_fill_style_str(&format!("rgba(31,31,31,{})", 0.94 * a));
    g.fill();
    g.begin_path();
    let _ = g.arc(c.x, c.y, rad - 0.7 * px, 0.0, 2.0 * PI);
    g.set_stroke_style_str(&format!("rgba(10,132,255,{})", 0.9 * a));
    g.set_line_width(1.4 * px);
    g.stroke();
    let gl = rad * 0.42;
    g.begin_path();
    match kind {
        CounterAction::Delete => {
            g.move_to(c.x - gl, c.y - gl);
            g.line_to(c.x + gl, c.y + gl);
            g.move_to(c.x + gl, c.y - gl);
            g.line_to(c.x - gl, c.y + gl);
        }
        CounterAction::Increment => {
            g.move_to(c.x - gl, c.y);
            g.line_to(c.x + gl, c.y);
            g.move_to(c.x, c.y - gl);
            g.line_to(c.x, c.y + gl);
        }
        CounterAction::Decrement => {
            g.move_to(c.x - gl, c.y);
            g.line_to(c.x + gl, c.y);
        }
    }
    g.set_stroke_style_str(&format!("rgba(255,255,255,{})", 0.96 * a));
    g.set_line_width(1.7 * px);
    g.set_line_cap("round");
    g.stroke();
    g.restore();
}

// ───────────────────────────── Beautify ─────────────────────────────

fn liquid_glass(src: &HtmlCanvasElement, w: f64, h: f64) -> HtmlCanvasElement {
    // Abdeckend skaliert (×1.15), stark verkleinert, Sättigung ×1.9, weich vergrößert ≈ Capso (Blur 120 px).
    let n = 24.0;
    let (sw, sh) = (src.width() as f64, src.height() as f64);
    let (tw, th) = if w >= h { (n, (n * h / w).max(1.0).round()) } else { ((n * w / h).max(1.0).round(), n) };
    let tiny = canvas(tw, th);
    let tg = ctx(&tiny);
    let k = (tw / sw).max(th / sh) * 1.15;
    let (dw, dh) = (sw * k, sh * k);
    let _ = tg.draw_image_with_html_canvas_element_and_dw_and_dh(src, (tw - dw) / 2.0, (th - dh) / 2.0, dw, dh);
    if let Ok(id) = tg.get_image_data(0.0, 0.0, tw, th) {
        let mut d = id.data().0;
        for px in d.chunks_mut(4) {
            let (r, g, b) = (px[0] as f64, px[1] as f64, px[2] as f64);
            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            for (i, v) in [r, g, b].into_iter().enumerate() {
                px[i] = (l + (v - l) * 1.9).clamp(0.0, 255.0) as u8;
            }
        }
        if let Ok(nd) = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&d), tw as u32, th as u32) {
            let _ = tg.put_image_data(&nd, 0.0, 0.0);
        }
    }
    let mid = canvas(tw * 8.0, th * 8.0);
    let mg = ctx(&mid);
    smooth_high(&mg);
    let _ = mg.draw_image_with_html_canvas_element_and_dw_and_dh(&tiny, 0.0, 0.0, tw * 8.0, th * 8.0);
    mid
}

/// Hintergrund in `outer` füllen.
pub fn draw_beautify_bg(g: &Ctx, b: &Beautify, outer: Rect, src: &HtmlCanvasElement) {
    match b.background_style.as_str() {
        "none" => {}
        "gradient" => {
            let a = b.gradient_angle.to_radians();
            let c = outer.center();
            let len = (outer.w * a.cos()).abs() / 2.0 + (outer.h * a.sin()).abs() / 2.0;
            let gr = g.create_linear_gradient(c.x - a.cos() * len, c.y - a.sin() * len, c.x + a.cos() * len, c.y + a.sin() * len);
            let _ = gr.add_color_stop(0.0, &css_rgba4(b.gradient[0]));
            let _ = gr.add_color_stop(1.0, &css_rgba4(b.gradient[1]));
            g.set_fill_style_canvas_gradient(&gr);
            g.fill_rect(outer.x, outer.y, outer.w, outer.h);
        }
        "liquidGlass" => {
            g.set_fill_style_str("rgb(26,26,26)");
            g.fill_rect(outer.x, outer.y, outer.w, outer.h);
            let lg = liquid_glass(src, outer.w, outer.h);
            g.save();
            g.set_image_smoothing_enabled(true);
            smooth_high(&g);
            let _ = g.draw_image_with_html_canvas_element_and_dw_and_dh(&lg, outer.x, outer.y, outer.w, outer.h);
            g.restore();
            g.set_fill_style_str("rgba(255,255,255,0.03)");
            g.fill_rect(outer.x, outer.y, outer.w, outer.h);
        }
        _ => {
            g.set_fill_style_str(&css_rgba4(b.background_color));
            g.fill_rect(outer.x, outer.y, outer.w, outer.h);
        }
    }
}

/// Schatten unter dem abgerundeten Bild und Clip setzen. `dev` = Gerätepixel pro Bildpixel (Schatten ignoriert die Transformation).
/// Danach zeichnet der Aufrufer das Bild; abschließend `g.restore()`.
pub fn begin_beautify_image(g: &Ctx, b: &Beautify, img: Rect, dev: f64) {
    g.save();
    if b.shadow_enabled && b.shadow_radius > 0.0 {
        g.save();
        g.set_shadow_color("rgba(0,0,0,0.25)");
        g.set_shadow_blur(b.shadow_radius * dev);
        // Form weit außerhalb zeichnen und nur den Schatten zurückversetzen → kein Schwarz unter transparenten Pixeln.
        const K: f64 = 100_000.0;
        g.set_shadow_offset_x(K * dev);
        g.set_shadow_offset_y(6.0 * dev);
        g.set_fill_style_str("#000");
        g.begin_path();
        round_rect(g, &rect(img.x - K, img.y, img.w, img.h), b.corner_radius);
        g.fill();
        g.restore();
    }
    g.begin_path();
    round_rect(g, &img, b.corner_radius.max(0.0));
    g.clip();
}

// ───────────────────────────── Export ─────────────────────────────

/// Fertiges Bild in voller Auflösung (Capso `renderedOutputImage`): Anmerkungen + Zuschnitt, dann Beautify,
/// zuletzt auf die Ausgabegröße skaliert.
pub fn render_export(doc: &Doc, src: &HtmlCanvasElement, beautify: Option<&Beautify>) -> HtmlCanvasElement {
    let cr = content_rect(doc);
    let c = canvas(cr.w, cr.h);
    let g = ctx(&c);
    let _ = g.translate(-cr.x, -cr.y);
    draw_doc(&g, src, doc, None, None);
    let out = match beautify {
        Some(b) if b.enabled => beautify_canvas(&c, b),
        _ => c,
    };
    match (doc.output, doc.crop) {
        (Some((ow, oh)), Some(_)) if ow > 0 && oh > 0 && (ow != out.width() || oh != out.height()) => {
            let s = canvas(ow as f64, oh as f64);
            let sg = ctx(&s);
            smooth_high(&sg);
            let _ = sg.draw_image_with_html_canvas_element_and_dw_and_dh(&out, 0.0, 0.0, ow as f64, oh as f64);
            s
        }
        _ => out,
    }
}

/// Capso `BeautifyRenderer.render`.
pub fn beautify_canvas(img: &HtmlCanvasElement, b: &Beautify) -> HtmlCanvasElement {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let (ow, oh, ix, iy) = b.layout(w, h);
    let c = canvas(ow, oh);
    let g = ctx(&c);
    draw_beautify_bg(&g, b, rect(0.0, 0.0, ow, oh), img);
    let r = rect(ix, iy, w, h);
    begin_beautify_image(&g, b, r, 1.0);
    let _ = g.draw_image_with_html_canvas_element(img, ix, iy);
    g.restore();
    c
}

pub fn png_data_url(c: &HtmlCanvasElement) -> String {
    c.to_data_url().unwrap_or_default()
}
