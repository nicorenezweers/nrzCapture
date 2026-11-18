//! Bildnachbearbeitung für `deliver`: Zuschneiden, Zeitstempel-Wasserzeichen (Port von
//! `ScreenshotTimestampRenderer`), Auto-Beautify (Port von `BeautifySettings`), Formatumwandlung, Zwischenablage.
use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::core;

pub fn load(path: &Path) -> Option<RgbaImage> {
    image::open(path).ok().map(|i| i.to_rgba8())
}

pub fn save_png(img: &RgbaImage, path: &Path) -> bool {
    img.save_with_format(path, image::ImageFormat::Png).is_ok()
}

/// Ausschnitt `[x, y, w, h]` in Pixeln als neue PNG im Temp-Ordner.
pub fn crop_file(path: &Path, x: u32, y: u32, w: u32, h: u32) -> Option<PathBuf> {
    let img = load(path)?;
    let x = x.min(img.width().saturating_sub(1));
    let y = y.min(img.height().saturating_sub(1));
    let w = w.min(img.width() - x).max(1);
    let h = h.min(img.height() - y).max(1);
    let out = image::imageops::crop_imm(&img, x, y, w, h).to_image();
    let p = core::temp_file("png");
    save_png(&out, &p).then_some(p)
}

// ───────────────────────────── Farben ─────────────────────────────

/// "#RRGGBB" / "RRGGBB" / "#RRGGBBAA" → RGBA.
pub fn parse_hex(s: &str) -> Option<[u8; 4]> {
    let h = s.trim().trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([b(0)?, b(2)?, b(4)?, 255]),
        8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
        _ => None,
    }
}

fn blend(dst: &mut Rgba<u8>, c: [u8; 4], cov: f32) {
    let a = (c[3] as f32 / 255.0) * cov.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let da = dst.0[3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    for i in 0..3 {
        let v = (c[i] as f32 * a + dst.0[i] as f32 * da * (1.0 - a)) / oa.max(1e-6);
        dst.0[i] = v.round().clamp(0.0, 255.0) as u8;
    }
    dst.0[3] = (oa * 255.0).round() as u8;
}

// ───────────────────────────── Zeitstempel ─────────────────────────────

fn font() -> Option<FontVec> {
    for p in [
        "/System/Library/Fonts/SFNSMono.ttf",
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ] {
        if let Ok(d) = std::fs::read(p) {
            if let Ok(f) = FontVec::try_from_vec_and_index(d, 0) {
                return Some(f);
            }
        }
    }
    None
}

/// Text des Zeitstempels gemäß Format ("dateTime" | "date" | "time" | "iso"/"iso8601").
pub fn timestamp_text(format: &str, now: chrono::DateTime<chrono::Local>) -> String {
    match format {
        "date" => now.format("%Y-%m-%d").to_string(),
        "time" => now.format("%H:%M:%S").to_string(),
        "iso" | "iso8601" => now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        _ => now.format("%Y-%m-%d %H:%M").to_string(),
    }
}

/// Ursprung (links oben) eines Textblocks `tw`×`th` an `position` mit Rand `m` im Bild `w`×`h`.
pub fn anchor(position: &str, w: f64, h: f64, tw: f64, th: f64, m: f64) -> (f64, f64) {
    let cx = (w - tw) / 2.0;
    let cy = (h - th) / 2.0;
    let (l, r, t, b) = (m, w - tw - m, m, h - th - m);
    match position {
        "topLeft" => (l, t),
        "top" => (cx, t),
        "topRight" => (r, t),
        "right" => (r, cy),
        "bottom" => (cx, b),
        "bottomLeft" => (l, b),
        "left" => (l, cy),
        _ => (r, b),
    }
}

fn draw_text(img: &mut RgbaImage, font: &FontVec, text: &str, px: f32, x: f32, y: f32, color: [u8; 4]) {
    let sf = font.as_scaled(PxScale::from(px));
    let mut pen = x;
    let base = y + sf.ascent();
    let (w, h) = (img.width() as i32, img.height() as i32);
    for ch in text.chars() {
        let id = sf.glyph_id(ch);
        let g = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(pen, base));
        pen += sf.h_advance(id);
        if let Some(o) = font.outline_glyph(g) {
            let bb = o.px_bounds();
            o.draw(|gx, gy, cov| {
                let (px_, py_) = (bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32);
                if px_ >= 0 && py_ >= 0 && px_ < w && py_ < h {
                    blend(img.get_pixel_mut(px_ as u32, py_ as u32), color, cov);
                }
            });
        }
    }
}

fn text_width(font: &FontVec, text: &str, px: f32) -> f32 {
    let sf = font.as_scaled(PxScale::from(px));
    text.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum()
}

/// Zeitstempel einzeichnen. `scale` = Pixel pro Punkt.
pub fn watermark(img: &mut RgbaImage, s: &crate::settings::Settings, scale: f64) {
    let Some(font) = font() else { return };
    let text = timestamp_text(&s.screenshot_timestamp_format, chrono::Local::now());
    let size = (s.screenshot_timestamp_font_size.clamp(8, 72) as f64 * scale) as f32;
    let color = parse_hex(&s.screenshot_timestamp_color).unwrap_or([255, 255, 255, 255]);
    let tw = text_width(&font, &text, size) as f64;
    let th = font.as_scaled(PxScale::from(size)).height() as f64;
    let m = (s.screenshot_timestamp_font_size as f64).max(8.0) * scale;
    let (x, y) = anchor(&s.screenshot_timestamp_position, img.width() as f64, img.height() as f64, tw, th, m);
    // Weicher Schatten (mehrere versetzte Durchgänge), dann der Text.
    let blur = (size * 0.16).max(2.0);
    for (dx, dy) in [(0.0, 1.0), (-blur / 2.0, 1.0), (blur / 2.0, 1.0), (0.0, 1.0 + blur / 2.0), (0.0, 1.0 - blur / 2.0)] {
        draw_text(img, &font, &text, size, x as f32 + dx, y as f32 + dy, [0, 0, 0, 70]);
    }
    draw_text(img, &font, &text, size, x as f32, y as f32, color);
}

// ───────────────────────────── Beautify ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Beautify {
    pub background: Option<[u8; 4]>, // None = transparent ("none")
    pub glass: bool,
    pub padding: f64,
    pub corner_radius: f64,
    pub shadow_radius: f64, // 0 = kein Schatten
}

impl Beautify {
    /// Aus `beautifyStyle` (Capso-JSON: backgroundStyle, backgroundColor [r,g,b,a 0…1], padding, cornerRadius,
    /// shadowEnabled, shadowRadius). Fehlende Werte = Capso-Standard.
    pub fn from_value(v: &Value) -> Self {
        let f = |k: &str, d: f64| v.get(k).and_then(|x| x.as_f64()).unwrap_or(d).max(0.0);
        let style = v.get("backgroundStyle").and_then(|x| x.as_str()).unwrap_or("solid");
        let color = match v.get("backgroundColor") {
            Some(Value::Array(a)) if a.len() >= 3 => {
                let c = |i: usize, d: f64| (a.get(i).and_then(|x| x.as_f64()).unwrap_or(d).clamp(0.0, 1.0) * 255.0).round() as u8;
                [c(0, 1.0), c(1, 1.0), c(2, 1.0), c(3, 1.0)]
            }
            Some(Value::String(s)) => parse_hex(s).unwrap_or([255; 4]),
            _ => [255; 4],
        };
        let shadow = v.get("shadowEnabled").and_then(|x| x.as_bool()).unwrap_or(true);
        Self {
            background: (style != "none").then_some(color),
            glass: style == "liquidGlass",
            padding: f("padding", 40.0),
            corner_radius: f("cornerRadius", 12.0),
            shadow_radius: if shadow { f("shadowRadius", 20.0) } else { 0.0 },
        }
    }
}

/// Abdeckung eines Pixels (Mitte) in einem abgerundeten Rechteck (Kantenglättung über Abstand).
fn rounded_cov(x: f64, y: f64, w: f64, h: f64, r: f64) -> f64 {
    let r = r.min(w / 2.0).min(h / 2.0);
    let qx = (x - w / 2.0).abs() - (w / 2.0 - r);
    let qy = (y - h / 2.0).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
    (0.5 - outside).clamp(0.0, 1.0)
}

/// Hintergrund, Rand, abgerundete Ecken und Schatten. Maße in Punkten × `scale`.
pub fn beautify(img: &RgbaImage, b: &Beautify, scale: f64) -> RgbaImage {
    let pad = b.padding * scale;
    let rad = b.corner_radius * scale;
    let sh = b.shadow_radius * scale;
    let shadow_inset = if sh > 0.0 { sh + 6.0 * scale } else { 0.0 };
    let inset = (pad + shadow_inset).round();
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    let (w, h) = ((iw + 2.0 * inset) as u32, (ih + 2.0 * inset) as u32);
    let mut out = RgbaImage::new(w, h);
    if let Some(c) = b.background {
        for (_, y, p) in out.enumerate_pixels_mut() {
            let mut c = c;
            if b.glass {
                // „Liquid Glass“ vereinfacht: sanfter heller Verlauf über der Grundfarbe.
                let t = y as f64 / h.max(1) as f64;
                for v in c.iter_mut().take(3) {
                    *v = (*v as f64 + (255.0 - *v as f64) * 0.35 * (1.0 - t)).round() as u8;
                }
            }
            *p = Rgba(c);
        }
    }
    // Schatten: weicher Abfall um das Bild herum (leicht nach unten versetzt).
    if sh > 0.0 {
        let off = 4.0 * scale;
        let (x0, y0) = (inset, inset + off);
        for (x, y, p) in out.enumerate_pixels_mut() {
            let (px, py) = (x as f64 + 0.5 - x0, y as f64 + 0.5 - y0);
            let qx = (px - iw / 2.0).abs() - (iw / 2.0 - rad.min(iw / 2.0));
            let qy = (py - ih / 2.0).abs() - (ih / 2.0 - rad.min(ih / 2.0));
            let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad.min(iw / 2.0).min(ih / 2.0);
            if d < sh {
                let t = (1.0 - (d.max(0.0) / sh)).clamp(0.0, 1.0);
                blend(p, [0, 0, 0, 255], (0.35 * t * t) as f32);
            }
        }
    }
    let ox = inset as u32;
    for (x, y, p) in img.enumerate_pixels() {
        let cov = if rad > 0.0 { rounded_cov(x as f64 + 0.5, y as f64 + 0.5, iw, ih, rad) } else { 1.0 };
        if cov <= 0.0 {
            continue;
        }
        let mut c = p.0;
        c[3] = (c[3] as f64 * cov).round() as u8;
        blend(out.get_pixel_mut(x + ox, y + ox), c, 1.0);
    }
    out
}

// ───────────────────────────── Fenster auf Milchglas ─────────────────────────────

/// Polsterung (Pixel) für den Milchglas-Hintergrund einer Fensteraufnahme (Capso: 4 % der kürzeren Seite, min. 30, + 20 für den Schatten).
pub fn frosted_padding(w: u32, h: u32) -> f64 {
    (w.min(h) as f64 * 0.04).max(30.0) + 20.0
}

/// Kastenweichzeichner (horizontal + vertikal) auf einem f32-Kanal, Radius `r`.
fn box_blur(buf: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0f32; buf.len()];
    let norm = 1.0 / (2 * r + 1) as f32;
    for y in 0..h {
        let row = &buf[y * w..(y + 1) * w];
        let at = |i: isize| row[i.clamp(0, w as isize - 1) as usize];
        let mut acc: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for x in 0..w {
            tmp[y * w + x] = acc * norm;
            acc += at(x as isize + r as isize + 1) - at(x as isize - r as isize);
        }
    }
    for x in 0..w {
        let at = |i: isize| tmp[i.clamp(0, h as isize - 1) as usize * w + x];
        let mut acc: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for y in 0..h {
            buf[y * w + x] = acc * norm;
            acc += at(y as isize + r as isize + 1) - at(y as isize - r as isize);
        }
    }
}

/// Milchglas-Hintergrund (Capso `frostedGlassBackdrop`): Sättigung 1,4, Kontrast 0,98, Gauß-Radius ~80 px.
/// `hole` = Bereich (Pixel) des Fensters selbst – wird ausgespart und aus der Umgebung aufgefüllt,
/// damit nur der Schreibtisch dahinter durchscheint.
pub fn frosted_backdrop(src: &RgbaImage, hole: Option<(u32, u32, u32, u32)>) -> RgbaImage {
    let (w, h) = src.dimensions();
    let f = 8u32;
    let (sw, sh) = ((w / f).max(1) as usize, (h / f).max(1) as usize);
    let small = image::imageops::resize(src, sw as u32, sh as u32, image::imageops::FilterType::Triangle);
    let mut ch = [vec![0f32; sw * sh], vec![0f32; sw * sh], vec![0f32; sw * sh]];
    let mut m = vec![1f32; sw * sh];
    for (x, y, p) in small.enumerate_pixels() {
        let i = y as usize * sw + x as usize;
        let inside = hole.is_some_and(|(hx, hy, hw, hh)| {
            let (px, py) = (x * f + f / 2, y * f + f / 2);
            px >= hx && px < hx + hw && py >= hy && py < hy + hh
        });
        let wgt = if inside { 0.0 } else { 1.0 };
        m[i] = wgt;
        // Sättigung/Kontrast wie CIColorControls.
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let l = 0.2125 * r + 0.7154 * g + 0.0721 * b;
        for (k, v) in [r, g, b].into_iter().enumerate() {
            let sat = l + (v - l) * 1.4;
            ch[k][i] = (((sat - 0.5) * 0.98 + 0.5).clamp(0.0, 1.0)) * wgt;
        }
    }
    // Gauß ≈ 3 Kastendurchgänge; σ = 80 px / Verkleinerung.
    let r = ((80.0 / f as f32) * 0.85).round().max(1.0) as usize;
    let all_hole = m.iter().all(|v| *v == 0.0);
    for _ in 0..3 {
        for c in ch.iter_mut() {
            box_blur(c, sw, sh, r);
        }
        if !all_hole {
            box_blur(&mut m, sw, sh, r);
        }
    }
    let mut small_out = RgbaImage::new(sw as u32, sh as u32);
    for (x, y, p) in small_out.enumerate_pixels_mut() {
        let i = y as usize * sw + x as usize;
        let n = if all_hole { 1.0 } else { m[i].max(1e-4) };
        *p = Rgba([(ch[0][i] / n * 255.0) as u8, (ch[1][i] / n * 255.0) as u8, (ch[2][i] / n * 255.0) as u8, 255]);
    }
    image::imageops::resize(&small_out, w, h, image::imageops::FilterType::Triangle)
}

/// Fenster (ohne Systemschatten) mittig auf den Milchglas-Hintergrund setzen: abgerundete Ecken (12 px),
/// weicher Schatten (Radius 20, 4 px nach unten, 25 % Schwarz). `backdrop` hat die Ausgabegröße.
pub fn frosted_window(window: &RgbaImage, backdrop: &RgbaImage) -> RgbaImage {
    let (ow, oh) = backdrop.dimensions();
    let (iw, ih) = (window.width() as f64, window.height() as f64);
    let (ox, oy) = (((ow as f64 - iw) / 2.0).max(0.0).round(), ((oh as f64 - ih) / 2.0).max(0.0).round());
    let mut out = frosted_backdrop(backdrop, Some((ox as u32, oy as u32, window.width(), window.height())));
    let (rad, sh, off) = (12.0, 20.0, 4.0);
    for (x, y, p) in out.enumerate_pixels_mut() {
        let (px, py) = (x as f64 + 0.5 - ox, y as f64 + 0.5 - oy - off);
        let qx = (px - iw / 2.0).abs() - (iw / 2.0 - rad);
        let qy = (py - ih / 2.0).abs() - (ih / 2.0 - rad);
        let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad;
        if d > 0.0 && d < sh {
            let t = 1.0 - d / sh;
            blend(p, [0, 0, 0, 255], (0.25 * t * t) as f32);
        } else if d <= 0.0 {
            blend(p, [0, 0, 0, 255], 0.25);
        }
    }
    for (x, y, p) in window.enumerate_pixels() {
        let cov = rounded_cov(x as f64 + 0.5, y as f64 + 0.5, iw, ih, rad);
        if cov <= 0.0 {
            continue;
        }
        let (tx, ty) = (x + ox as u32, y + oy as u32);
        if tx >= ow || ty >= oh {
            continue;
        }
        let mut c = p.0;
        c[3] = (c[3] as f64 * cov).round() as u8;
        blend(out.get_pixel_mut(tx, ty), c, 1.0);
    }
    out
}

// ───────────────────────────── Format & Zwischenablage ─────────────────────────────

/// PNG in Zielformat umwandeln ("png" | "jpeg"/"jpg" | "heic" | "tiff"), Qualität 1–100. Liefert neuen Pfad.
pub fn convert(src: &Path, format: &str, quality: u8) -> PathBuf {
    let (fmt, ext) = match format {
        "jpeg" | "jpg" => ("jpeg", "jpg"),
        "heic" => ("heic", "heic"),
        "tiff" => ("tiff", "tiff"),
        _ => return src.to_path_buf(),
    };
    let dst = core::temp_file(ext);
    let q = quality.clamp(1, 100).to_string();
    let mut args = vec!["-s", "format", fmt];
    if fmt != "tiff" {
        args.extend(["-s", "formatOptions", &q]);
    }
    let (s, d) = (src.to_string_lossy().to_string(), dst.to_string_lossy().to_string());
    args.extend([s.as_str(), "--out", d.as_str()]);
    if core::sh("/usr/bin/sips", &args).is_some() && dst.exists() {
        dst
    } else {
        src.to_path_buf()
    }
}

/// Dateiendung zum Ausgabeformat.
pub fn ext_for(format: &str) -> &'static str {
    match format {
        "jpeg" | "jpg" => "jpg",
        "heic" => "heic",
        _ => "png",
    }
}

/// Bild in die Zwischenablage, im gewünschten Format ("png" | "jpeg" | "tiff").
pub fn copy_image_as(path: &Path, format: &str) -> bool {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSData, NSString};
    let (conv, uti) = match format {
        "jpeg" | "jpg" => (convert(path, "jpeg", 90), "public.jpeg"),
        "tiff" => (convert(path, "tiff", 100), "public.tiff"),
        _ => return core::clipboard::copy_image(path),
    };
    let Ok(bytes) = std::fs::read(&conv) else { return core::clipboard::copy_image(path) };
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let ok = pb.setData_forType(Some(&NSData::with_bytes(&bytes)), &NSString::from_str(uti));
    // PNG zusätzlich, damit auch Apps ohne JPEG-/TIFF-Unterstützung einfügen können.
    if let Ok(png) = std::fs::read(path) {
        pb.setData_forType(Some(&NSData::with_bytes(&png)), &NSString::from_str("public.png"));
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("#FFFFFF"), Some([255, 255, 255, 255]));
        assert_eq!(parse_hex("ff000080"), Some([255, 0, 0, 128]));
        assert_eq!(parse_hex("#12"), None);
        assert_eq!(parse_hex("#GG0000"), None);
    }

    #[test]
    fn anchors() {
        assert_eq!(anchor("bottomRight", 100.0, 50.0, 20.0, 10.0, 5.0), (75.0, 35.0));
        assert_eq!(anchor("topLeft", 100.0, 50.0, 20.0, 10.0, 5.0), (5.0, 5.0));
        assert_eq!(anchor("top", 100.0, 50.0, 20.0, 10.0, 5.0), (40.0, 5.0));
        assert_eq!(anchor("left", 100.0, 50.0, 20.0, 10.0, 5.0), (5.0, 20.0));
    }

    #[test]
    fn timestamp_formats() {
        use chrono::TimeZone;
        let t = chrono::Local.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap();
        assert_eq!(timestamp_text("dateTime", t), "2026-03-04 05:06");
        assert_eq!(timestamp_text("date", t), "2026-03-04");
        assert_eq!(timestamp_text("time", t), "05:06:07");
        assert!(timestamp_text("iso8601", t).starts_with("2026-03-04T05:06:07"));
    }

    #[test]
    fn beautify_style_and_size() {
        let b = Beautify::from_value(&json!({"backgroundStyle":"solid","backgroundColor":[1,0,0,1],"padding":10,"cornerRadius":0,"shadowEnabled":false,"shadowRadius":20}));
        assert_eq!(b, Beautify { background: Some([255, 0, 0, 255]), glass: false, padding: 10.0, corner_radius: 0.0, shadow_radius: 0.0 });
        let d = Beautify::from_value(&Value::Null);
        assert_eq!((d.padding, d.corner_radius, d.shadow_radius, d.background), (40.0, 12.0, 20.0, Some([255; 4])));
        let img = RgbaImage::from_pixel(20, 10, Rgba([0, 0, 255, 255]));
        let out = beautify(&img, &b, 2.0);
        assert_eq!(out.dimensions(), (60, 50));
        assert_eq!(out.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(30, 25).0, [0, 0, 255, 255]);
        // abgerundete Ecken: Eckpixel des Bildes wird mit dem Hintergrund gemischt
        let r = Beautify { corner_radius: 5.0, ..b };
        let out = beautify(&img, &r, 1.0);
        assert_eq!(out.get_pixel(10, 10).0, [255, 0, 0, 255]);
    }

    #[test]
    fn watermark_draws_pixels() {
        let mut img = RgbaImage::from_pixel(400, 100, Rgba([0, 0, 0, 255]));
        let s = crate::settings::Settings { screenshot_timestamp_color: "#FF0000".into(), ..Default::default() };
        watermark(&mut img, &s, 1.0);
        if font().is_some() {
            assert!(img.pixels().any(|p| p.0[0] > 200), "Text sollte rot erscheinen");
            // oben links bleibt frei (Position unten rechts)
            assert!(img.enumerate_pixels().filter(|(x, y, _)| *x < 100 && *y < 40).all(|(_, _, p)| p.0 == [0, 0, 0, 255]));
        }
    }
}
