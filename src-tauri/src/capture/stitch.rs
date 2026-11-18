//! Scroll-Aufnahme: Bilder zu einer langen Seite zusammensetzen.
//! Port von Capso `ScrollStitcher` + `HeaderDetector`; die Versatz-Erkennung (in Capso Vision
//! `VNTranslationalImageRegistrationRequest`) ist hier eine Zeilen-Überlappungssuche.
use image::RgbaImage;
use std::collections::HashMap;

/// Höchstens dieser Anteil der Höhe wird als feste Kopf-/Fußzeile betrachtet.
const MAX_SCAN_PERCENT: f64 = 0.20;
/// Spalten-Stichproben pro Zeile.
const SAMPLE_COLUMNS: u32 = 20;
/// Toleranz je Farbkanal (Subpixel-Rendering).
const TOLERANCE: i32 = 2;

fn px_close(a: &[u8], b: &[u8]) -> bool {
    (a[0] as i32 - b[0] as i32).abs() <= TOLERANCE && (a[1] as i32 - b[1] as i32).abs() <= TOLERANCE && (a[2] as i32 - b[2] as i32).abs() <= TOLERANCE
}

/// Anzahl identischer Zeilen oben (`from_bottom = false`, fixe Kopfzeile) bzw. unten (fixe Fußzeile).
pub fn unchanged_rows(a: &RgbaImage, b: &RgbaImage, from_bottom: bool) -> u32 {
    let w = a.width().min(b.width());
    let h = a.height().min(b.height());
    let max_rows = (h as f64 * MAX_SCAN_PERCENT) as u32;
    if w <= SAMPLE_COLUMNS || max_rows == 0 {
        return 0;
    }
    let step = (w / SAMPLE_COLUMNS).max(1);
    let mut n = 0;
    for scanned in 0..max_rows {
        let row = if from_bottom { h - 1 - scanned } else { scanned };
        let same = (0..w).step_by(step as usize).all(|x| px_close(&a.get_pixel(x, row).0, &b.get_pixel(x, row).0));
        if !same {
            break;
        }
        n = scanned + 1;
    }
    n
}

/// Feste Bereiche (Kopf-/Fußzeile in Pixelzeilen), aus zwei aufeinanderfolgenden Bildern ermittelt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedRegions {
    pub header: u32,
    pub footer: u32,
}

impl FixedRegions {
    pub fn detect(a: &RgbaImage, b: &RgbaImage) -> Self {
        Self { header: unchanged_rows(a, b, false), footer: unchanged_rows(a, b, true) }
    }
}

/// Signatur einer Zeile (gerundete Stichproben, damit kleine Rauschunterschiede nicht stören).
fn row_hash(img: &RgbaImage, y: u32) -> u64 {
    let w = img.width();
    let step = (w / 64).max(1);
    let mut h: u64 = 0xcbf29ce484222325;
    for x in (0..w).step_by(step as usize) {
        let p = img.get_pixel(x, y).0;
        for c in &p[..3] {
            h ^= (*c >> 3) as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Mittlere Abweichung der Zeilen `a[y + d]` ↔ `b[y]` im Bereich `y0..y1` (Stichproben).
fn score(a: &RgbaImage, b: &RgbaImage, d: u32, y0: u32, y1: u32) -> f64 {
    let w = a.width().min(b.width());
    let xs = (w / 48).max(1) as usize;
    let rows = (y1.saturating_sub(y0 + d)).max(1);
    let ys = (rows / 160).max(1) as usize;
    let (mut sum, mut n) = (0u64, 0u64);
    for y in (y0..y1.saturating_sub(d)).step_by(ys) {
        for x in (0..w).step_by(xs) {
            let p = a.get_pixel(x, y + d).0;
            let q = b.get_pixel(x, y).0;
            sum += (p[0] as i32 - q[0] as i32).unsigned_abs() as u64 + (p[1] as i32 - q[1] as i32).unsigned_abs() as u64 + (p[2] as i32 - q[2] as i32).unsigned_abs() as u64;
            n += 1;
        }
    }
    if n == 0 {
        f64::MAX
    } else {
        sum as f64 / n as f64
    }
}

/// Um wie viele Pixelzeilen ist `later` gegenüber `earlier` nach unten gescrollt?
/// (Inhalt wandert nach oben: `later[y] == earlier[y + d]`.) `None`, wenn keine sichere Überlappung gefunden wurde.
pub fn vertical_offset(earlier: &RgbaImage, later: &RgbaImage, fixed: FixedRegions) -> Option<u32> {
    if earlier.dimensions() != later.dimensions() {
        return None;
    }
    let h = earlier.height();
    let y0 = fixed.header;
    let y1 = h.saturating_sub(fixed.footer);
    if y1 <= y0 + 8 {
        return None;
    }
    let area = y1 - y0;
    // Mindestens so viele Zeilen müssen überlappen.
    let min_overlap = (area / 8).max(4);
    let max_d = area - min_overlap;

    // Kandidaten über markante Zeilen (Zeilen, die sich von der Nachbarzeile unterscheiden).
    let ha: Vec<u64> = (y0..y1).map(|y| row_hash(earlier, y)).collect();
    let hb: Vec<u64> = (y0..y1).map(|y| row_hash(later, y)).collect();
    let mut pos: HashMap<u64, Vec<u32>> = HashMap::new();
    for (i, v) in ha.iter().enumerate() {
        pos.entry(*v).or_default().push(i as u32);
    }
    let mut votes: HashMap<u32, u32> = HashMap::new();
    for i in 1..hb.len() {
        if hb[i] == hb[i - 1] {
            continue;
        }
        if let Some(ps) = pos.get(&hb[i]) {
            if ps.len() > 8 {
                continue;
            }
            for &p in ps {
                if p as usize > i {
                    let d = p - i as u32;
                    if d <= max_d {
                        *votes.entry(d).or_default() += 1;
                    }
                }
            }
        }
    }
    let mut cands: Vec<(u32, u32)> = votes.into_iter().collect();
    cands.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut best: Option<(u32, f64)> = None;
    for &(d, _) in cands.iter().take(6) {
        let s = score(earlier, later, d, y0, y1);
        if best.is_none_or(|(_, bs)| s < bs) {
            best = Some((d, s));
        }
    }
    // Rückfall: grobe Vollsuche (z. B. bei leicht abweichendem Rendering).
    if best.is_none_or(|(_, s)| s > 6.0) {
        let step = (area / 400).max(1);
        let mut coarse: Option<(u32, f64)> = None;
        for d in (1..=max_d).step_by(step as usize) {
            let s = score(earlier, later, d, y0, y1);
            if coarse.is_none_or(|(_, bs)| s < bs) {
                coarse = Some((d, s));
            }
        }
        if let Some((cd, _)) = coarse {
            for d in cd.saturating_sub(step).max(1)..=(cd + step).min(max_d) {
                let s = score(earlier, later, d, y0, y1);
                if best.is_none_or(|(_, bs)| s < bs) {
                    best = Some((d, s));
                }
            }
        }
    }
    let (d, s) = best?;
    // Kein Versatz besser als „unverändert“? Dann nicht gescrollt.
    let still = score(earlier, later, 0, y0, y1);
    (s <= 6.0 && s < still).then_some(d)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StitchResult {
    Stitched(u32),
    NoChange,
    AlignmentFailed,
}

/// Setzt Bilder schrittweise zu einer langen Seite zusammen. Zeilen einer festen Fußzeile werden beim
/// Scrollen nie angehängt; die Fußzeile des letzten Bildes kommt am Ende einmal unten dran.
#[derive(Default)]
pub struct Stitcher {
    body: Option<RgbaImage>,
    last: Option<RgbaImage>,
    footer: u32,
}

impl Stitcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_initial(&mut self, frame: RgbaImage) {
        self.body = Some(frame.clone());
        self.last = Some(frame);
        self.footer = 0;
    }

    pub fn last_frame(&self) -> Option<&RgbaImage> {
        self.last.as_ref()
    }

    /// Untere `h` Zeilen jedes Bildes aus dem Körper herausnehmen.
    pub fn set_footer(&mut self, h: u32) {
        let Some(body) = &self.body else { return };
        if h == 0 || h >= body.height() {
            return;
        }
        self.footer = h;
        self.body = Some(image::imageops::crop_imm(body, 0, 0, body.width(), body.height() - h).to_image());
    }

    pub fn total_height(&self) -> u32 {
        self.body.as_ref().map(|b| b.height() + self.footer).unwrap_or(0)
    }

    /// Neues Bild anhängen, das `offset` Zeilen weiter unten liegt.
    pub fn stitch(&mut self, frame: RgbaImage, offset: u32) -> StitchResult {
        let Some(body) = &self.body else {
            self.set_initial(frame);
            return StitchResult::Stitched(0);
        };
        if offset == 0 {
            return StitchResult::NoChange;
        }
        let bottom = frame.height().saturating_sub(self.footer);
        if offset >= bottom || frame.width() != body.width() {
            return StitchResult::AlignmentFailed;
        }
        let rows = image::imageops::crop_imm(&frame, 0, bottom - offset, frame.width(), offset).to_image();
        self.body = Some(stack(body, &rows));
        self.last = Some(frame);
        StitchResult::Stitched(offset)
    }

    /// Fertige Seite inklusive fester Fußzeile.
    pub fn merged(&self) -> Option<RgbaImage> {
        let body = self.body.as_ref()?;
        match (&self.last, self.footer) {
            (Some(last), f) if f > 0 => {
                let footer = image::imageops::crop_imm(last, 0, last.height() - f, last.width(), f).to_image();
                Some(stack(body, &footer))
            }
            _ => Some(body.clone()),
        }
    }
}

fn stack(top: &RgbaImage, bottom: &RgbaImage) -> RgbaImage {
    let mut out = RgbaImage::new(top.width(), top.height() + bottom.height());
    image::imageops::replace(&mut out, top, 0, 0);
    image::imageops::replace(&mut out, bottom, 0, top.height() as i64);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// Lange „Seite“ mit unregelmäßigem Inhalt (Pseudozufall pro Zeile, Blöcke, Leerzeilen).
    fn page(w: u32, h: u32) -> RgbaImage {
        let mut s: u32 = 12345;
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            let blank = (y / 7) % 5 == 0;
            for x in 0..w {
                let v = if blank { 250 } else { ((s >> 8) as u8).wrapping_add((x * 7 / (1 + y % 13)) as u8) };
                img.put_pixel(x, y, Rgba([v, v.wrapping_mul(3), 255 - v, 255]));
            }
        }
        img
    }

    fn with_bars(content: &RgbaImage, header: u32, footer: u32) -> RgbaImage {
        let mut f = content.clone();
        for y in 0..header {
            for x in 0..f.width() {
                f.put_pixel(x, y, Rgba([10, 20, 200, 255]));
            }
        }
        for y in f.height() - footer..f.height() {
            for x in 0..f.width() {
                f.put_pixel(x, y, Rgba([200, 20, 10, 255]));
            }
        }
        f
    }

    /// Sichtbarer Ausschnitt: fixer Kopf (`header` Zeilen), Inhalt ab `scroll`, fixer Fuß.
    fn frame(p: &RgbaImage, scroll: u32, h: u32, header: u32, footer: u32) -> RgbaImage {
        let visible = image::imageops::crop_imm(p, 0, scroll, p.width(), h).to_image();
        with_bars(&visible, header, footer)
    }

    #[test]
    fn detects_fixed_bars() {
        let p = page(120, 2000);
        let a = frame(&p, 0, 300, 30, 20);
        let b = frame(&p, 90, 300, 30, 20);
        let f = FixedRegions::detect(&a, &b);
        assert_eq!(f, FixedRegions { header: 30, footer: 20 });
        assert_eq!(unchanged_rows(&a, &a, false), 60); // max. 20 % der Höhe
    }

    #[test]
    fn finds_offsets() {
        let p = page(160, 3000);
        for d in [3u32, 17, 80, 150, 199] {
            let a = frame(&p, 400, 300, 30, 20);
            let b = frame(&p, 400 + d, 300, 30, 20);
            assert_eq!(vertical_offset(&a, &b, FixedRegions { header: 30, footer: 20 }), Some(d), "d={d}");
        }
        let a = frame(&p, 400, 300, 0, 0);
        assert_eq!(vertical_offset(&a, &a, FixedRegions::default()), None);
        // zurückgescrollt → kein positiver Versatz
        let b = frame(&p, 350, 300, 0, 0);
        assert_eq!(vertical_offset(&a, &b, FixedRegions::default()), None);
    }

    #[test]
    fn stitches_page_with_header_and_footer() {
        let (header, footer, h) = (30u32, 20u32, 300u32);
        let p = page(100, 2000);
        let mut st = Stitcher::new();
        let mut scroll = 0;
        st.set_initial(frame(&p, scroll, h, header, footer));
        let mut fixed: Option<FixedRegions> = None;
        for step in [60u32, 120, 45, 200, 90] {
            scroll += step;
            let next = frame(&p, scroll, h, header, footer);
            let prev = st.last_frame().unwrap().clone();
            let f = *fixed.get_or_insert_with(|| {
                let f = FixedRegions::detect(&prev, &next);
                st.set_footer(f.footer);
                f
            });
            let d = vertical_offset(&prev, &next, f).expect("Versatz");
            assert_eq!(d, step);
            assert_eq!(st.stitch(next, d), StitchResult::Stitched(step));
        }
        let out = st.merged().unwrap();
        assert_eq!(out.height(), h + scroll);
        assert_eq!(st.total_height(), h + scroll);
        // Erwartung: Kopf + durchgehender Inhalt + Fuß
        let expected = with_bars(&image::imageops::crop_imm(&p, 0, 0, 100, h + scroll).to_image(), header, footer);
        assert!(out.pixels().zip(expected.pixels()).all(|(a, b)| a == b));
    }

    #[test]
    fn stitch_edge_cases() {
        let p = page(50, 500);
        let mut st = Stitcher::new();
        assert_eq!(st.stitch(frame(&p, 0, 100, 0, 0), 0), StitchResult::Stitched(0));
        assert_eq!(st.stitch(frame(&p, 0, 100, 0, 0), 0), StitchResult::NoChange);
        assert_eq!(st.stitch(frame(&p, 0, 100, 0, 0), 100), StitchResult::AlignmentFailed);
        assert_eq!(st.total_height(), 100);
    }
}
