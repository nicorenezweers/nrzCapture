//! Bild-Komposition für den Export mit tiny-skia – Port von Capso `FrameCompositor`.
//! Reihenfolge wie dort: Zoom → Weichzeichner → Klick-Welle → Cursor (im Videorahmen) → Hintergrund, Schatten, Ecken.
//! Geometrie kommt aus `model::Layout`/`model::Engine`, die auch die Canvas-Vorschau nutzt.
use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LineJoin, LinearGradient, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, PixmapRef, Point, RadialGradient, Rect,
    SpreadMode, Stroke, Transform,
};

use super::model::{continuous_rounded_rect, BackgroundColorType, Engine, Layout, PathCmd, CURSOR_ARROW};

const GLASS_DOWNSCALE: f64 = 8.0;
const SHADOW_DOWNSCALE: f64 = 2.0;

pub struct Renderer {
    pub layout: Layout,
    pub engine: Engine,
    src_w: f64,
    src_h: f64,
    /// Hintergrund + Schatten (statisch, außer bei „Liquid Glass“).
    base: Pixmap,
    shadow: Option<Pixmap>,
    mask: Mask,
    canvas: Pixmap,
    viewport: Pixmap,
    vp_x: i32,
    vp_y: i32,
    glass: Option<Pixmap>,
    cursor_h: f64,
}

fn path(cmds: &[PathCmd]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for c in cmds {
        match *c {
            PathCmd::Move(x, y) => pb.move_to(x as f32, y as f32),
            PathCmd::Line(x, y) => pb.line_to(x as f32, y as f32),
            PathCmd::Cubic(a, b, c, d, e, f) => pb.cubic_to(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32),
            PathCmd::Close => pb.close(),
        }
    }
    pb.finish()
}

fn color(c: &super::model::CodableColor) -> Color {
    let [r, g, b, a] = c.rgba8();
    Color::from_rgba8(r, g, b, a)
}

impl Renderer {
    pub fn new(engine: Engine, layout: Layout) -> Result<Self, String> {
        let p = &engine.project;
        let (src_w, src_h) = (p.video_width, p.video_height);
        let (ow, oh) = (layout.out_w.round().max(2.0) as u32, layout.out_h.round().max(2.0) as u32);
        let vp_x = layout.frame_x.round() as i32;
        let vp_y = layout.frame_y.round() as i32;
        let vw = (layout.frame_w.round().max(1.0) as u32).min(ow);
        let vh = (layout.frame_h.round().max(1.0) as u32).min(oh);
        let err = || "Zeichenfläche konnte nicht angelegt werden".to_string();
        let mut base = Pixmap::new(ow, oh).ok_or_else(err)?;
        let canvas = Pixmap::new(ow, oh).ok_or_else(err)?;
        let viewport = Pixmap::new(vw, vh).ok_or_else(err)?;
        let bg = p.background_style.clone();
        let corner = layout.corner_radius;

        // Maske für den Videorahmen (weiche Ecken).
        let mut mask = Mask::new(ow, oh).ok_or_else(err)?;
        if let Some(rr) = path(&continuous_rounded_rect(vp_x as f64, vp_y as f64, vw as f64, vh as f64, corner)) {
            mask.fill_path(&rr, FillRule::Winding, true, Transform::identity());
        }

        // Statischer Hintergrund
        base.fill(Color::BLACK);
        if bg.enabled {
            match bg.color_type {
                BackgroundColorType::Solid | BackgroundColorType::LiquidGlass => base.fill(color(&bg.solid_color)),
                BackgroundColorType::Gradient => {
                    let ((sx, sy), (ex, ey)) = bg.gradient_unit_points();
                    let shader = LinearGradient::new(
                        Point::from_xy((sx * ow as f64) as f32, (sy * oh as f64) as f32),
                        Point::from_xy((ex * ow as f64) as f32, (ey * oh as f64) as f32),
                        vec![GradientStop::new(0.0, color(&bg.gradient_from)), GradientStop::new(1.0, color(&bg.gradient_to))],
                        SpreadMode::Pad,
                        Transform::identity(),
                    );
                    if let Some(shader) = shader {
                        let paint = Paint { shader, ..Default::default() };
                        base.fill_rect(Rect::from_xywh(0.0, 0.0, ow as f32, oh as f32).unwrap(), &paint, Transform::identity(), None);
                    }
                }
                BackgroundColorType::Image => {
                    base.fill(color(&bg.solid_color));
                    if let Some(img) = bg.image_path.as_deref().and_then(load_image) {
                        let (iw, ih) = (img.width() as f64, img.height() as f64);
                        let s = (ow as f64 / iw).max(oh as f64 / ih);
                        let t = Transform::from_translate(((ow as f64 - iw * s) / 2.0) as f32, ((oh as f64 - ih * s) / 2.0) as f32).pre_scale(s as f32, s as f32);
                        base.draw_pixmap(0, 0, img.as_ref(), &PixmapPaint { quality: FilterQuality::Bicubic, ..Default::default() }, t, None);
                    }
                }
            }
        }

        // Schlagschatten: schwarzes Rechteck mit Deckkraft, gaußartig weichgezeichnet.
        let shadow = if bg.enabled && bg.shadow_enabled && bg.shadow_opacity > 0.0 {
            let d = SHADOW_DOWNSCALE;
            let (sw, sh) = (((ow as f64) / d).ceil() as u32, ((oh as f64) / d).ceil() as u32);
            let mut s = Pixmap::new(sw, sh).ok_or_else(err)?;
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba(0.0, 0.0, 0.0, bg.shadow_opacity.clamp(0.0, 1.0) as f32).unwrap_or(Color::BLACK));
            if let Some(r) = Rect::from_xywh((vp_x as f64 / d) as f32, (vp_y as f64 / d) as f32, (vw as f64 / d) as f32, (vh as f64 / d) as f32) {
                s.fill_rect(r, &paint, Transform::identity(), None);
            }
            let sigma = bg.shadow_radius * layout.k / d;
            gaussian_blur(&mut s, 0, 0, sw, sh, sigma);
            Some(s)
        } else {
            None
        };
        let glass = (bg.enabled && bg.color_type == BackgroundColorType::LiquidGlass)
            .then(|| Pixmap::new((ow as f64 / GLASS_DOWNSCALE).ceil() as u32, (oh as f64 / GLASS_DOWNSCALE).ceil() as u32))
            .flatten();
        if glass.is_none() {
            if let Some(s) = &shadow {
                draw_scaled(&mut base, s.as_ref(), SHADOW_DOWNSCALE);
            }
        }
        let cursor_h = p.cursor_height();
        Ok(Self { layout, engine, src_w, src_h, base, shadow, mask, canvas, viewport, vp_x, vp_y, glass, cursor_h })
    }

    pub fn width(&self) -> u32 {
        self.canvas.width()
    }
    pub fn height(&self) -> u32 {
        self.canvas.height()
    }

    /// Bild zur Quellzeit `t` aus dem Quellbild `src` (Quellauflösung, RGBA vormultipliziert) rendern.
    pub fn render(&mut self, src: PixmapRef, t: f64) -> &Pixmap {
        let state = self.engine.frame(t);
        let k = self.layout.k;
        let (vw, vh) = (self.viewport.width() as f64, self.viewport.height() as f64);
        let (sw, sh) = (src.width() as f64, src.height() as f64);
        let (scale_x, scale_y) = (self.src_w / sw.max(1.0), self.src_h / sh.max(1.0));

        // 1. Hintergrund
        self.canvas.data_mut().copy_from_slice(self.base.data());
        if let Some(glass) = &mut self.glass {
            let d = GLASS_DOWNSCALE;
            let (ow, oh) = (self.canvas.width() as f64, self.canvas.height() as f64);
            let cover = (ow / sw).max(oh / sh) * 1.15 / d;
            let t = Transform::from_translate(((ow / d - sw * cover) / 2.0) as f32, ((oh / d - sh * cover) / 2.0) as f32).pre_scale(cover as f32, cover as f32);
            glass.fill(Color::BLACK);
            glass.draw_pixmap(0, 0, src, &PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() }, t, None);
            saturate(glass, 1.9, 0.95);
            let (gw, gh) = (glass.width(), glass.height());
            gaussian_blur(glass, 0, 0, gw, gh, 120.0 * k / d);
            draw_scaled(&mut self.canvas, glass.as_ref(), d);
            if let Some(s) = &self.shadow {
                draw_scaled(&mut self.canvas, s.as_ref(), SHADOW_DOWNSCALE);
            }
        }

        // 2. Videorahmen mit Zoom
        self.viewport.fill(Color::BLACK);
        let (fx, fy) = state.zoom.focus();
        let s = state.zoom.scale;
        let kx = vw / self.src_w;
        let ky = vh / self.src_h;
        let tr = Transform::from_translate((vw / 2.0) as f32, (vh / 2.0) as f32)
            .pre_scale((s * kx) as f32, (s * ky) as f32)
            .pre_translate((-fx * self.src_w) as f32, (-fy * self.src_h) as f32)
            .pre_scale(scale_x as f32, scale_y as f32);
        let q = if (s * kx - 1.0).abs() < 1e-6 && (scale_x - 1.0).abs() < 1e-6 { FilterQuality::Nearest } else { FilterQuality::Bilinear };
        self.viewport.draw_pixmap(0, 0, src, &PixmapPaint { quality: q, ..Default::default() }, tr, None);

        // 3. Weichzeichner-Regionen (Bildschirm-feste Rechtecke im Videorahmen)
        for b in &state.blurs {
            let r = b.rect.clamped(0.0);
            let (x0, y0) = ((r.x * vw).floor().max(0.0) as u32, (r.y * vh).floor().max(0.0) as u32);
            let (x1, y1) = (((r.x + r.width) * vw).ceil().min(vw) as u32, ((r.y + r.height) * vh).ceil().min(vh) as u32);
            if x1 > x0 && y1 > y0 {
                gaussian_blur(&mut self.viewport, x0, y0, x1 - x0, y1 - y0, b.radius * k);
            }
        }

        // 4. Klick-Welle + Cursor
        if let Some(c) = state.cursor {
            let (px, py) = self.layout.place(&state.zoom, c.x, c.y);
            let (px, py) = (px * vw / self.layout.frame_w.max(1.0), py * vh / self.layout.frame_h.max(1.0));
            let h = self.cursor_h * s * k;
            if let Some((rr, op)) = c.ripple {
                let radius = (self.cursor_h * 1.3 * rr * s * k) as f32;
                let a = (0.35 * op * c.alpha) as f32;
                let col = |alpha: f32| Color::from_rgba(0.25, 0.55, 1.0, alpha).unwrap();
                let center = Point::from_xy(px as f32, py as f32);
                if radius > 0.5 {
                    if let Some(shader) = RadialGradient::new(
                        center,
                        center,
                        radius,
                        vec![GradientStop::new(0.0, col(a)), GradientStop::new(0.82, col(a)), GradientStop::new(1.0, col(0.0))],
                        SpreadMode::Pad,
                        Transform::identity(),
                    ) {
                        if let Some(circle) = PathBuilder::from_circle(px as f32, py as f32, radius) {
                            let paint = Paint { shader, anti_alias: true, ..Default::default() };
                            self.viewport.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
                        }
                    }
                }
            }
            draw_cursor(&mut self.viewport, px, py, h * c.click_scale, c.alpha);
        }

        // 5. Rahmen mit weichen Ecken auf den Hintergrund
        let vp = std::mem::replace(&mut self.viewport, Pixmap::new(1, 1).unwrap());
        self.canvas.draw_pixmap(self.vp_x, self.vp_y, vp.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&self.mask));
        self.viewport = vp;
        &self.canvas
    }
}

/// macOS-ähnlicher Pfeil: schwarz mit weißem Rand, Spitze bei (x, y).
pub fn draw_cursor(pm: &mut Pixmap, x: f64, y: f64, h: f64, alpha: f64) {
    if h < 1.0 || alpha <= 0.0 {
        return;
    }
    let mut pb = PathBuilder::new();
    for (i, (px, py)) in CURSOR_ARROW.iter().enumerate() {
        let (qx, qy) = ((x + px * h) as f32, (y + py * h) as f32);
        if i == 0 {
            pb.move_to(qx, qy);
        } else {
            pb.line_to(qx, qy);
        }
    }
    pb.close();
    let Some(arrow) = pb.finish() else { return };
    let a = alpha.clamp(0.0, 1.0) as f32;
    let mut shadow = Paint { anti_alias: true, ..Default::default() };
    shadow.set_color(Color::from_rgba(0.0, 0.0, 0.0, 0.25 * a).unwrap());
    let off = Transform::from_translate(0.0, (h * 0.04) as f32);
    let stroke = Stroke { width: (h * 0.11) as f32, line_join: LineJoin::Round, ..Default::default() };
    pm.stroke_path(&arrow, &shadow, &stroke, off, None);
    let mut white = Paint { anti_alias: true, ..Default::default() };
    white.set_color(Color::from_rgba(1.0, 1.0, 1.0, a).unwrap());
    pm.stroke_path(&arrow, &white, &stroke, Transform::identity(), None);
    let mut black = Paint { anti_alias: true, ..Default::default() };
    black.set_color(Color::from_rgba(0.0, 0.0, 0.0, a).unwrap());
    pm.fill_path(&arrow, &black, FillRule::Winding, Transform::identity(), None);
}

fn draw_scaled(dst: &mut Pixmap, src: PixmapRef, factor: f64) {
    let f = factor as f32;
    dst.draw_pixmap(0, 0, src, &PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() }, Transform::from_scale(f, f), None);
}

fn load_image(path: &str) -> Option<Pixmap> {
    let p = std::path::Path::new(path);
    if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("png")).unwrap_or(false) {
        if let Ok(pm) = Pixmap::load_png(p) {
            return Some(pm);
        }
    }
    let tmp = crate::core::temp_file("png");
    crate::core::sh("sips", &["-s", "format", "png", path, "--out", &tmp.to_string_lossy()])?;
    let pm = Pixmap::load_png(&tmp).ok();
    let _ = std::fs::remove_file(tmp);
    pm
}

/// Sättigung und Kontrast (CIColorControls) auf deckende Pixel.
fn saturate(pm: &mut Pixmap, saturation: f64, contrast: f64) {
    for px in pm.data_mut().chunks_exact_mut(4) {
        let (r, g, b) = (px[0] as f64 / 255.0, px[1] as f64 / 255.0, px[2] as f64 / 255.0);
        let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let f = |c: f64| ((((l + (c - l) * saturation) - 0.5) * contrast + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
        let a = px[3];
        px[0] = f(r).min(a);
        px[1] = f(g).min(a);
        px[2] = f(b).min(a);
    }
}

/// Gauß-Näherung durch drei Kastenfilter je Richtung auf einem Teilbereich (Ränder geklemmt).
pub fn gaussian_blur(pm: &mut Pixmap, x: u32, y: u32, w: u32, h: u32, sigma: f64) {
    if sigma < 0.5 || w == 0 || h == 0 {
        return;
    }
    let stride = pm.width() as usize;
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let r = ((ideal - 1.0) / 2.0).round().max(1.0) as usize;
    let data = pm.data_mut();
    let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
    let mut line = vec![0u8; w.max(h) * 4];
    let mut out = vec![0u8; w.max(h) * 4];
    for _ in 0..3 {
        for row in 0..h {
            let start = ((y + row) * stride + x) * 4;
            line[..w * 4].copy_from_slice(&data[start..start + w * 4]);
            box_line(&line[..w * 4], &mut out[..w * 4], w, r);
            data[start..start + w * 4].copy_from_slice(&out[..w * 4]);
        }
        for col in 0..w {
            for row in 0..h {
                let i = ((y + row) * stride + x + col) * 4;
                line[row * 4..row * 4 + 4].copy_from_slice(&data[i..i + 4]);
            }
            box_line(&line[..h * 4], &mut out[..h * 4], h, r);
            for row in 0..h {
                let i = ((y + row) * stride + x + col) * 4;
                data[i..i + 4].copy_from_slice(&out[row * 4..row * 4 + 4]);
            }
        }
    }
}

fn box_line(src: &[u8], dst: &mut [u8], n: usize, r: usize) {
    let win = (2 * r + 1) as u32;
    let at = |i: isize, c: usize| src[(i.clamp(0, n as isize - 1) as usize) * 4 + c] as u32;
    for c in 0..4 {
        let mut sum: u32 = (-(r as isize)..=r as isize).map(|i| at(i, c)).sum();
        for i in 0..n {
            dst[i * 4 + c] = ((sum + win / 2) / win) as u8;
            sum += at(i as isize + r as isize + 1, c);
            sum -= at(i as isize - r as isize, c);
        }
    }
}
