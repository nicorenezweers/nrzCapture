//! Reine Auswahl-Geometrie (Port von Capso `CaptureSelectionGeometry`, `CapturePreset`,
//! `CaptureSelectionChromeLayout`). Koordinaten: logische Punkte, Ursprung oben links, y nach unten.
//! Ohne Abhängigkeiten – wird auch vom Frontend (`src/overlay.rs`, per `#[path]`) eingebunden.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }
    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }
    pub fn mid(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 <= self.max_x() && p.1 >= self.y && p.1 <= self.max_y()
    }
    pub fn standardized(&self) -> Self {
        Self::new(self.x.min(self.x + self.w), self.y.min(self.y + self.h), self.w.abs(), self.h.abs())
    }
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let mx = self.max_x().min(o.max_x());
        let my = self.max_y().min(o.max_y());
        (mx > x && my > y).then(|| Rect::new(x, y, mx - x, my - y))
    }
    pub fn inset(&self, d: f64) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn from_points(a: (f64, f64), b: (f64, f64)) -> Rect {
        Rect::new(a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
    }
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if lo > hi {
        lo
    } else {
        v.max(lo).min(hi)
    }
}

fn clamp_pt(p: (f64, f64), b: &Rect) -> (f64, f64) {
    (clamp(p.0, b.x, b.max_x()), clamp(p.1, b.y, b.max_y()))
}

// ───────────────────────────── Presets ─────────────────────────────

/// Größenvorgabe: frei, festes Seitenverhältnis oder feste Größe (Punkte).
#[derive(Clone, Debug, PartialEq)]
pub enum Preset {
    Freeform,
    Ratio(u32, u32, Option<&'static str>),
    Fixed(u32, u32, Option<&'static str>),
}

impl Preset {
    /// Kennung wie in den Einstellungen (`capturePreset`): "freeform", "16:9", "1920x1080".
    pub fn id(&self) -> String {
        match self {
            Preset::Freeform => "freeform".into(),
            Preset::Ratio(w, h, _) => format!("{w}:{h}"),
            Preset::Fixed(w, h, _) => format!("{w}x{h}"),
        }
    }

    /// Liest "freeform", "16:9", "1920x1080", "1920×1080" sowie Capso-IDs "ratio-16-9" / "fixed-1920-1080".
    pub fn parse(s: &str) -> Option<Preset> {
        let s = s.trim().to_lowercase();
        if s.is_empty() || s == "freeform" || s == "free" {
            return Some(Preset::Freeform);
        }
        let nums = |a: &str, b: &str| -> Option<(u32, u32)> {
            let w: u32 = a.trim().parse().ok()?;
            let h: u32 = b.trim().parse().ok()?;
            (w > 0 && h > 0).then_some((w, h))
        };
        if let Some(rest) = s.strip_prefix("ratio-") {
            let (a, b) = rest.split_once('-')?;
            let (w, h) = nums(a, b)?;
            return Some(Preset::Ratio(w, h, builtin_name(w, h, false)));
        }
        if let Some(rest) = s.strip_prefix("fixed-") {
            let (a, b) = rest.split_once('-')?;
            let (w, h) = nums(a, b)?;
            return Some(Preset::Fixed(w, h, builtin_name(w, h, true)));
        }
        if let Some((a, b)) = s.split_once(':') {
            let (w, h) = nums(a, b)?;
            return Some(Preset::Ratio(w, h, builtin_name(w, h, false)));
        }
        let (a, b) = s.split_once('x').or_else(|| s.split_once('×')).or_else(|| s.split_once('*'))?;
        let (w, h) = nums(a, b)?;
        Some(Preset::Fixed(w, h, builtin_name(w, h, true)))
    }

    pub fn ratio(&self) -> Option<f64> {
        match self {
            Preset::Freeform => None,
            Preset::Ratio(w, h, _) | Preset::Fixed(w, h, _) => Some(*w as f64 / *h as f64),
        }
    }

    /// Seitenverhältnis, das beim Aufziehen gilt (feste Größen werden nicht gezogen).
    pub fn drag_ratio(&self) -> Option<f64> {
        match self {
            Preset::Ratio(..) => self.ratio(),
            _ => None,
        }
    }

    pub fn fixed(&self) -> Option<(f64, f64)> {
        match self {
            Preset::Fixed(w, h, _) => Some((*w as f64, *h as f64)),
            _ => None,
        }
    }

    pub fn is_fixed(&self) -> bool {
        matches!(self, Preset::Fixed(..))
    }

    /// Anzeigename, `free` = übersetztes „Frei“.
    pub fn display_name(&self, free: &str) -> String {
        match self {
            Preset::Freeform => free.to_owned(),
            Preset::Ratio(w, h, Some(n)) => format!("{w}:{h} ({n})"),
            Preset::Ratio(w, h, None) => format!("{w}:{h}"),
            Preset::Fixed(w, h, Some(n)) => format!("{w} × {h} ({n})"),
            Preset::Fixed(w, h, None) => format!("{w} × {h}"),
        }
    }

    /// Kleines Abzeichen neben der Größenanzeige.
    pub fn badge(&self) -> Option<String> {
        match self {
            Preset::Freeform => None,
            Preset::Ratio(w, h, _) => Some(format!("{w}:{h}")),
            Preset::Fixed(..) => Some("Fixed".into()),
        }
    }
}

fn builtin_name(w: u32, h: u32, fixed: bool) -> Option<&'static str> {
    match (fixed, w, h) {
        (false, 1, 1) => Some("Square"),
        (true, 1280, 720) => Some("720p"),
        (true, 1920, 1080) => Some("1080p"),
        _ => None,
    }
}

pub fn builtin_presets() -> Vec<Preset> {
    vec![
        Preset::Freeform,
        Preset::Ratio(1, 1, Some("Square")),
        Preset::Ratio(4, 3, None),
        Preset::Ratio(16, 9, None),
        Preset::Ratio(3, 2, None),
        Preset::Fixed(512, 512, None),
        Preset::Fixed(1280, 720, Some("720p")),
        Preset::Fixed(1920, 1080, Some("1080p")),
    ]
}

/// Auswählbare Presets: eingebaute (ohne ausgeblendete, „Frei“ bleibt immer) + benutzerdefinierte.
pub fn available_presets(custom: &[String], hidden: &[String]) -> Vec<Preset> {
    let hidden: Vec<String> = hidden.iter().filter_map(|h| Preset::parse(h)).map(|p| p.id()).collect();
    let mut out: Vec<Preset> = builtin_presets().into_iter().filter(|p| *p == Preset::Freeform || !hidden.contains(&p.id())).collect();
    for c in custom {
        if let Some(p) = Preset::parse(c) {
            if !out.iter().any(|o| o.id() == p.id()) {
                out.push(p);
            }
        }
    }
    out
}

/// Nächstes/vorheriges Preset (Taste R / ⇧R).
pub fn cycle_preset(list: &[Preset], current: &str, forward: bool) -> Preset {
    if list.is_empty() {
        return Preset::Freeform;
    }
    let cur = Preset::parse(current).map(|p| p.id()).unwrap_or_default();
    let n = list.len();
    let i = list.iter().position(|p| p.id() == cur);
    let next = match (i, forward) {
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
        (None, _) => 0,
    };
    list[next].clone()
}

// ───────────────────────────── Aufziehen ─────────────────────────────

/// Wirksames Seitenverhältnis: Shift erzwingt 1:1, sonst das des Presets.
pub fn selection_ratio(preset_ratio: Option<f64>, square_lock: bool) -> Option<f64> {
    if square_lock {
        Some(1.0)
    } else {
        preset_ratio
    }
}

/// Port von `constrainedDragEnd`: Endpunkt so anpassen, dass das Rechteck `ratio` hat (größere Achse führt),
/// und innerhalb von `bounds` bleiben.
pub fn constrain_end(start: (f64, f64), raw: (f64, f64), ratio: Option<f64>, b: &Rect) -> (f64, f64) {
    let Some(ratio) = ratio.filter(|r| *r > 0.0) else { return clamp_pt(raw, b) };
    let (dx, dy) = (raw.0 - start.0, raw.1 - start.1);
    let (mut ex, mut ey);
    if dx.abs() >= dy.abs() {
        let ch = dx.abs() / ratio;
        ex = start.0 + dx;
        ey = start.1 + if dy >= 0.0 { ch } else { -ch };
    } else {
        let cw = dy.abs() * ratio;
        ex = start.0 + if dx >= 0.0 { cw } else { -cw };
        ey = start.1 + dy;
    }
    if ex < b.x {
        let ch = (start.0 - b.x) / ratio;
        ex = b.x;
        ey = start.1 + if ey >= start.1 { ch } else { -ch };
    } else if ex > b.max_x() {
        let ch = (b.max_x() - start.0) / ratio;
        ex = b.max_x();
        ey = start.1 + if ey >= start.1 { ch } else { -ch };
    }
    if ey < b.y {
        let cw = (start.1 - b.y) * ratio;
        ey = b.y;
        ex = start.0 + if ex >= start.0 { cw } else { -cw };
    } else if ey > b.max_y() {
        let cw = (b.max_y() - start.1) * ratio;
        ey = b.max_y();
        ex = start.0 + if ex >= start.0 { cw } else { -cw };
    }
    (ex, ey)
}

/// Auswahl beim Aufziehen. `centered`: Startpunkt ist die Mitte (Square-Center-Lock-Taste gehalten).
pub fn drag_rect(start: (f64, f64), raw: (f64, f64), ratio: Option<f64>, centered: bool, b: &Rect) -> Rect {
    let start = clamp_pt(start, b);
    if !centered {
        return Rect::from_points(start, constrain_end(start, raw, ratio, b));
    }
    let mut hw = (raw.0 - start.0).abs();
    let mut hh = (raw.1 - start.1).abs();
    if let Some(r) = ratio.filter(|r| *r > 0.0) {
        if hw >= hh * r {
            hh = hw / r;
        } else {
            hw = hh * r;
        }
    }
    // Symmetrisch auf den Rand begrenzen (Verhältnis bleibt erhalten).
    let max_hw = (start.0 - b.x).min(b.max_x() - start.0).max(0.0);
    let max_hh = (start.1 - b.y).min(b.max_y() - start.1).max(0.0);
    let k = [if hw > max_hw && hw > 0.0 { max_hw / hw } else { 1.0 }, if hh > max_hh && hh > 0.0 { max_hh / hh } else { 1.0 }];
    if ratio.is_some() {
        let s = k[0].min(k[1]);
        hw *= s;
        hh *= s;
    } else {
        hw *= k[0];
        hh *= k[1];
    }
    Rect::new(start.0 - hw, start.1 - hh, hw * 2.0, hh * 2.0)
}

/// Rechteck um `d` verschieben, innerhalb von `bounds`.
pub fn move_rect(r: &Rect, dx: f64, dy: f64, b: &Rect) -> Rect {
    let r = r.standardized();
    let w = r.w.min(b.w);
    let h = r.h.min(b.h);
    Rect::new(clamp(r.x + dx, b.x, b.max_x() - w), clamp(r.y + dy, b.y, b.max_y() - h), w, h)
}

/// Feste Größe um einen Mittelpunkt, innerhalb von `bounds`.
pub fn fixed_rect(w: f64, h: f64, center: (f64, f64), b: &Rect) -> Rect {
    let w = w.max(1.0).min(b.w);
    let h = h.max(1.0).min(b.h);
    move_rect(&Rect::new(center.0 - w / 2.0, center.1 - h / 2.0, w, h), 0.0, 0.0, b)
}

/// Bestehende Auswahl auf ein Seitenverhältnis bringen (um die Mitte, beim Preset-Wechsel).
pub fn fit_ratio(r: &Rect, ratio: f64, b: &Rect, min: f64) -> Rect {
    let r = r.standardized();
    if ratio <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return move_rect(&r, 0.0, 0.0, b);
    }
    let (mut w, mut h) = (r.w, r.h);
    if w / h > ratio {
        h = w / ratio;
    } else {
        w = h * ratio;
    }
    if w < min {
        w = min;
        h = w / ratio;
    }
    if h < min {
        h = min;
        w = h * ratio;
    }
    if w > b.w {
        w = b.w;
        h = w / ratio;
    }
    if h > b.h {
        h = b.h;
        w = h * ratio;
    }
    fixed_rect(w, h, r.mid(), b)
}

// ───────────────────────────── Griffe ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

pub const ALL_HANDLES: [Handle; 8] =
    [Handle::TopLeft, Handle::Top, Handle::TopRight, Handle::Right, Handle::BottomRight, Handle::Bottom, Handle::BottomLeft, Handle::Left];

impl Handle {
    /// Position des Griffs auf dem Rechteck.
    pub fn point(&self, r: &Rect) -> (f64, f64) {
        let (mx, my) = r.mid();
        match self {
            Handle::TopLeft => (r.x, r.y),
            Handle::Top => (mx, r.y),
            Handle::TopRight => (r.max_x(), r.y),
            Handle::Right => (r.max_x(), my),
            Handle::BottomRight => (r.max_x(), r.max_y()),
            Handle::Bottom => (mx, r.max_y()),
            Handle::BottomLeft => (r.x, r.max_y()),
            Handle::Left => (r.x, my),
        }
    }

    pub fn cursor(&self) -> &'static str {
        match self {
            Handle::TopLeft | Handle::BottomRight => "nwse-resize",
            Handle::TopRight | Handle::BottomLeft => "nesw-resize",
            Handle::Top | Handle::Bottom => "ns-resize",
            Handle::Left | Handle::Right => "ew-resize",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Move,
    Resize(Handle),
}

pub const MIN_EDGE_HANDLE_DIMENSION: f64 = 80.0;

/// Sichtbare Griffe: keine bei fester Größe, nur Ecken bei kleinen Auswahlen.
pub fn visible_handles(r: &Rect, fixed: bool) -> Vec<Handle> {
    if fixed {
        return vec![];
    }
    if r.w.min(r.h) < MIN_EDGE_HANDLE_DIMENSION {
        return vec![Handle::TopLeft, Handle::TopRight, Handle::BottomRight, Handle::BottomLeft];
    }
    ALL_HANDLES.to_vec()
}

fn near_corner(p: (f64, f64), c: (f64, f64), center: (f64, f64), inner: f64, outer: f64) -> bool {
    let x_in = if center.0 >= c.0 { p.0 >= c.0 } else { p.0 <= c.0 };
    let y_in = if center.1 >= c.1 { p.1 >= c.1 } else { p.1 <= c.1 };
    (p.0 - c.0).abs() <= if x_in { inner } else { outer } && (p.1 - c.1).abs() <= if y_in { inner } else { outer }
}

fn near_edge(inward: f64, inner: f64, outer: f64) -> bool {
    if inward >= 0.0 {
        inward <= inner
    } else {
        -inward <= outer
    }
}

/// Trefferprüfung (asymmetrisch: `inner` reicht in die Auswahl, `outer` nach außen).
pub fn hit_target(p: (f64, f64), r: &Rect, inner: f64, outer: f64) -> Option<Hit> {
    let r = r.standardized();
    let (inner, outer) = (inner.max(0.0), outer.max(0.0));
    let c = r.mid();
    for h in [Handle::TopLeft, Handle::TopRight, Handle::BottomRight, Handle::BottomLeft] {
        if near_corner(p, h.point(&r), c, inner, outer) {
            return Some(Hit::Resize(h));
        }
    }
    if p.0 >= r.x && p.0 <= r.max_x() {
        if near_edge(p.1 - r.y, inner, outer) {
            return Some(Hit::Resize(Handle::Top));
        }
        if near_edge(r.max_y() - p.1, inner, outer) {
            return Some(Hit::Resize(Handle::Bottom));
        }
    }
    if p.1 >= r.y && p.1 <= r.max_y() {
        if near_edge(r.max_x() - p.0, inner, outer) {
            return Some(Hit::Resize(Handle::Right));
        }
        if near_edge(p.0 - r.x, inner, outer) {
            return Some(Hit::Resize(Handle::Left));
        }
    }
    r.contains(p).then_some(Hit::Move)
}

/// Auswahl an einem Griff auf `p` ziehen. Mit `ratio` bleibt das Seitenverhältnis erhalten
/// (Ecken: gegenüberliegende Ecke fest; Kanten: um die Mitte der anderen Achse).
pub fn resize(r: &Rect, handle: Handle, p: (f64, f64), b: &Rect, min: f64, ratio: Option<f64>) -> Rect {
    let r = r.standardized();
    let p = clamp_pt(p, b);
    let min_w = min.max(1.0).min(b.w);
    let min_h = min.max(1.0).min(b.h);
    let (mut x0, mut y0, mut x1, mut y1) = (r.x, r.y, r.max_x(), r.max_y());
    let Some(ratio) = ratio.filter(|v| *v > 0.0) else {
        match handle {
            Handle::TopLeft | Handle::Left | Handle::BottomLeft => x0 = clamp(p.0, b.x, x1 - min_w),
            Handle::TopRight | Handle::Right | Handle::BottomRight => x1 = clamp(p.0, x0 + min_w, b.max_x()),
            _ => {}
        }
        match handle {
            Handle::TopLeft | Handle::Top | Handle::TopRight => y0 = clamp(p.1, b.y, y1 - min_h),
            Handle::BottomLeft | Handle::Bottom | Handle::BottomRight => y1 = clamp(p.1, y0 + min_h, b.max_y()),
            _ => {}
        }
        return Rect::new(x0, y0, x1 - x0, y1 - y0);
    };
    match handle {
        Handle::Top | Handle::Bottom => {
            let (cx, _) = r.mid();
            let anchor = if handle == Handle::Top { y1 } else { y0 };
            let max_h_v = if handle == Handle::Top { anchor - b.y } else { b.max_y() - anchor };
            let max_h_x = 2.0 * (cx - b.x).min(b.max_x() - cx) / ratio;
            let h = (p.1 - anchor).abs().max(min_h).min(max_h_v).min(max_h_x).max(1.0);
            let w = h * ratio;
            let y = if handle == Handle::Top { anchor - h } else { anchor };
            Rect::new(cx - w / 2.0, y, w, h)
        }
        Handle::Left | Handle::Right => {
            let (_, cy) = r.mid();
            let anchor = if handle == Handle::Left { x1 } else { x0 };
            let max_w_h = if handle == Handle::Left { anchor - b.x } else { b.max_x() - anchor };
            let max_w_y = 2.0 * (cy - b.y).min(b.max_y() - cy) * ratio;
            let w = (p.0 - anchor).abs().max(min_w).min(max_w_h).min(max_w_y).max(1.0);
            let h = w / ratio;
            let x = if handle == Handle::Left { anchor - w } else { anchor };
            Rect::new(x, cy - h / 2.0, w, h)
        }
        _ => {
            let left = matches!(handle, Handle::TopLeft | Handle::BottomLeft);
            let top = matches!(handle, Handle::TopLeft | Handle::TopRight);
            let ax = if left { x1 } else { x0 };
            let ay = if top { y1 } else { y0 };
            let raw = (if left { p.0.min(ax - min_w) } else { p.0.max(ax + min_w) }, if top { p.1.min(ay - min_h) } else { p.1.max(ay + min_h) });
            let end = constrain_end((ax, ay), raw, Some(ratio), b);
            Rect::from_points((ax, ay), end)
        }
    }
}

// ───────────────────────────── Platzierung ─────────────────────────────

/// Größenanzeige „1280 × 720“.
pub fn dimension_text(w: f64, h: f64) -> String {
    format!("{} × {}", (w.round() as i64).max(1), (h.round() as i64).max(1))
}

/// Position eines schwebenden Elements (Werkzeugleiste, HUD) der Größe `size` an der Auswahl:
/// bevorzugt unterhalb, sonst oberhalb, sonst innen am unteren Rand. Ergebnis immer innerhalb `b`.
pub fn place_near(sel: &Rect, size: (f64, f64), b: &Rect, gap: f64) -> (f64, f64) {
    let x = clamp(sel.mid().0 - size.0 / 2.0, b.x + 8.0, b.max_x() - size.0 - 8.0);
    let below = sel.max_y() + gap;
    if below + size.1 <= b.max_y() - 4.0 {
        return (x, below);
    }
    let above = sel.y - gap - size.1;
    if above >= b.y + 4.0 {
        return (x, above);
    }
    (x, clamp(sel.max_y() - size.1 - gap, b.y + 4.0, b.max_y() - size.1 - 4.0))
}

/// Größenlabel oberhalb der Auswahl (bzw. innen, wenn oben kein Platz ist).
pub fn dimension_label_origin(sel: &Rect, size: (f64, f64), b: &Rect) -> (f64, f64) {
    let x = clamp(sel.x, b.x, b.max_x() - size.0);
    let above = sel.y - size.1 - 8.0;
    if above >= b.y {
        return (x, above);
    }
    (clamp(sel.x + 10.0, b.x, b.max_x() - size.0), clamp(sel.y + 10.0, b.y, b.max_y() - size.1))
}

/// Oberstes Fenster unter dem Punkt (Liste von vorne nach hinten sortiert).
pub fn window_at(windows: &[(u32, Rect)], p: (f64, f64)) -> Option<usize> {
    windows.iter().position(|(_, r)| r.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Rect = Rect::new(0.0, 0.0, 1000.0, 800.0);

    #[test]
    fn preset_parse_and_id() {
        assert_eq!(Preset::parse("16:9"), Some(Preset::Ratio(16, 9, None)));
        assert_eq!(Preset::parse("ratio-1-1"), Some(Preset::Ratio(1, 1, Some("Square"))));
        assert_eq!(Preset::parse("1920x1080").unwrap().id(), "1920x1080");
        assert_eq!(Preset::parse("fixed-1280-720"), Some(Preset::Fixed(1280, 720, Some("720p"))));
        assert_eq!(Preset::parse("freeform"), Some(Preset::Freeform));
        assert_eq!(Preset::parse("0:9"), None);
        assert_eq!(Preset::parse("abc"), None);
        assert_eq!(Preset::Fixed(1920, 1080, Some("1080p")).display_name("Free"), "1920 × 1080 (1080p)");
        assert_eq!(Preset::Ratio(4, 3, None).badge().as_deref(), Some("4:3"));
    }

    #[test]
    fn preset_cycle_and_available() {
        let list = available_presets(&["21:9".into(), "16:9".into()], &["3:2".into(), "freeform".into()]);
        assert_eq!(list[0], Preset::Freeform);
        assert!(!list.iter().any(|p| p.id() == "3:2"));
        assert_eq!(list.last().unwrap().id(), "21:9");
        assert_eq!(list.iter().filter(|p| p.id() == "16:9").count(), 1);
        assert_eq!(cycle_preset(&list, "freeform", true).id(), "1:1");
        assert_eq!(cycle_preset(&list, "freeform", false).id(), "21:9");
        assert_eq!(cycle_preset(&list, "unbekannt", true), Preset::Freeform);
    }

    #[test]
    fn constrain_square_and_clamp() {
        let e = constrain_end((100.0, 100.0), (300.0, 150.0), Some(1.0), &B);
        assert_eq!(e, (300.0, 300.0));
        // nach links oben über den Rand: Verhältnis bleibt
        let e = constrain_end((100.0, 100.0), (-500.0, 50.0), Some(2.0), &B);
        let r = Rect::from_points((100.0, 100.0), e);
        assert!((r.w / r.h - 2.0).abs() < 1e-9);
        assert!(r.x >= 0.0 && r.y >= 0.0);
        // frei: nur begrenzen
        assert_eq!(constrain_end((10.0, 10.0), (2000.0, -5.0), None, &B), (1000.0, 0.0));
    }

    #[test]
    fn centered_drag() {
        let r = drag_rect((500.0, 400.0), (600.0, 450.0), None, true, &B);
        assert_eq!(r, Rect::new(400.0, 350.0, 200.0, 100.0));
        let r = drag_rect((500.0, 400.0), (600.0, 450.0), Some(1.0), true, &B);
        assert_eq!(r, Rect::new(400.0, 300.0, 200.0, 200.0));
        // am Rand symmetrisch begrenzt
        let r = drag_rect((50.0, 400.0), (300.0, 400.0), Some(1.0), true, &B);
        assert_eq!(r.x, 0.0);
        assert_eq!(r.w, 100.0);
        assert_eq!(r.h, 100.0);
    }

    #[test]
    fn move_and_fixed_stay_inside() {
        let r = move_rect(&Rect::new(900.0, 700.0, 200.0, 50.0), 500.0, 500.0, &B);
        assert_eq!(r, Rect::new(800.0, 750.0, 200.0, 50.0));
        let r = fixed_rect(1920.0, 1080.0, (10.0, 10.0), &B);
        assert_eq!(r, Rect::new(0.0, 0.0, 1000.0, 800.0));
        let r = fixed_rect(100.0, 100.0, (10.0, 10.0), &B);
        assert_eq!(r, Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn fit_ratio_keeps_center() {
        let r = fit_ratio(&Rect::new(100.0, 100.0, 400.0, 100.0), 1.0, &B, 10.0);
        assert_eq!(r.w, 400.0);
        assert_eq!(r.h, 400.0);
        // Mitte (300,150) rutscht nach unten, weil der obere Rand begrenzt (y ≥ 0).
        assert_eq!(r.mid(), (300.0, 200.0));
    }

    #[test]
    fn hit_targets() {
        let r = Rect::new(100.0, 100.0, 200.0, 200.0);
        assert_eq!(hit_target((101.0, 101.0), &r, 4.0, 8.0), Some(Hit::Resize(Handle::TopLeft)));
        assert_eq!(hit_target((95.0, 200.0), &r, 4.0, 8.0), Some(Hit::Resize(Handle::Left)));
        assert_eq!(hit_target((200.0, 298.0), &r, 4.0, 8.0), Some(Hit::Resize(Handle::Bottom)));
        assert_eq!(hit_target((307.0, 307.0), &r, 4.0, 8.0), Some(Hit::Resize(Handle::BottomRight)));
        assert_eq!(hit_target((200.0, 200.0), &r, 4.0, 8.0), Some(Hit::Move));
        assert_eq!(hit_target((50.0, 50.0), &r, 4.0, 8.0), None);
    }

    #[test]
    fn resize_free_and_ratio() {
        let r = Rect::new(100.0, 100.0, 200.0, 100.0);
        assert_eq!(resize(&r, Handle::Right, (500.0, 0.0), &B, 10.0, None), Rect::new(100.0, 100.0, 400.0, 100.0));
        assert_eq!(resize(&r, Handle::TopLeft, (50.0, 20.0), &B, 10.0, None), Rect::new(50.0, 20.0, 250.0, 180.0));
        // über die gegenüberliegende Kante hinaus → Mindestgröße
        assert_eq!(resize(&r, Handle::Left, (900.0, 0.0), &B, 10.0, None).w, 10.0);
        let q = resize(&r, Handle::Bottom, (0.0, 300.0), &B, 10.0, Some(2.0));
        assert_eq!(q.h, 200.0);
        assert_eq!(q.w, 400.0);
        assert_eq!(q.mid().0, 200.0);
        let q = resize(&r, Handle::BottomRight, (700.0, 210.0), &B, 10.0, Some(2.0));
        assert_eq!((q.x, q.y), (100.0, 100.0));
        assert!((q.w / q.h - 2.0).abs() < 1e-9);
    }

    #[test]
    fn handles_and_placement() {
        assert_eq!(visible_handles(&Rect::new(0.0, 0.0, 50.0, 300.0), false).len(), 4);
        assert_eq!(visible_handles(&Rect::new(0.0, 0.0, 100.0, 300.0), false).len(), 8);
        assert!(visible_handles(&Rect::new(0.0, 0.0, 100.0, 300.0), true).is_empty());
        let (x, y) = place_near(&Rect::new(100.0, 100.0, 200.0, 100.0), (300.0, 50.0), &B, 10.0);
        assert_eq!((x, y), (50.0, 210.0));
        let (_, y) = place_near(&Rect::new(100.0, 600.0, 200.0, 190.0), (300.0, 50.0), &B, 10.0);
        assert_eq!(y, 540.0);
        let (_, y) = place_near(&Rect::new(0.0, 0.0, 1000.0, 800.0), (300.0, 50.0), &B, 10.0);
        assert_eq!(y, 740.0);
        assert_eq!(dimension_text(1279.6, 0.2), "1280 × 1");
        let ws = [(1, Rect::new(0.0, 0.0, 100.0, 100.0)), (2, Rect::new(0.0, 0.0, 500.0, 500.0))];
        assert_eq!(window_at(&ws, (50.0, 50.0)), Some(0));
        assert_eq!(window_at(&ws, (200.0, 50.0)), Some(1));
        assert_eq!(window_at(&ws, (600.0, 50.0)), None);
    }
}
