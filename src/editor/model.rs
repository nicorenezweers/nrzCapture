//! Annotations-Modell (Port von Capso `AnnotationKit`): Geometrie, Objekte, Hit-Tests, Dokument mit Undo/Redo,
//! Tastenkürzel, Shift-Constraints, Esc-Verhalten, Crop-Mathe, Bildtransformationen, Beautify-Einstellungen.
//! Reine Logik ohne DOM – testbar mit `cargo test -p nrzcapture-ui`.
//!
//! Alle Koordinaten sind Bildpixel (Ursprung oben links) des *Arbeitsbildes* (Original nach Drehen/Spiegeln).
use serde::{Deserialize, Serialize};

// ───────────────────────────── Geometrie ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

pub const fn pt(x: f64, y: f64) -> Pt {
    Pt { x, y }
}

impl Pt {
    pub fn dist(self, o: Pt) -> f64 {
        (self.x - o.x).hypot(self.y - o.y)
    }
    pub fn add(self, dx: f64, dy: f64) -> Pt {
        pt(self.x + dx, self.y + dy)
    }
    pub fn mid(self, o: Pt) -> Pt {
        pt((self.x + o.x) / 2.0, (self.y + o.y) / 2.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub const fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

impl Rect {
    pub fn from_points(a: Pt, b: Pt) -> Rect {
        rect(a.x.min(b.x), a.y.min(b.y), (a.x - b.x).abs(), (a.y - b.y).abs())
    }
    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }
    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }
    pub fn center(&self) -> Pt {
        pt(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn contains(&self, p: Pt) -> bool {
        p.x >= self.x && p.x <= self.max_x() && p.y >= self.y && p.y <= self.max_y()
    }
    pub fn contains_rect(&self, r: &Rect) -> bool {
        r.x >= self.x && r.y >= self.y && r.max_x() <= self.max_x() && r.max_y() <= self.max_y()
    }
    /// Wie `CGRect.insetBy` – negative Werte vergrößern.
    pub fn inset(&self, dx: f64, dy: f64) -> Rect {
        rect(self.x + dx, self.y + dy, self.w - 2.0 * dx, self.h - 2.0 * dy)
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        rect(x, y, self.max_x().max(o.max_x()) - x, self.max_y().max(o.max_y()) - y)
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (mx, my) = (self.max_x().min(o.max_x()), self.max_y().min(o.max_y()));
        if mx <= x || my <= y {
            return Rect::default();
        }
        rect(x, y, mx - x, my - y)
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    /// Wie `CGRect.integral`: nach außen auf ganze Pixel.
    pub fn integral(&self) -> Rect {
        let (x, y) = (self.x.floor(), self.y.floor());
        rect(x, y, self.max_x().ceil() - x, self.max_y().ceil() - y)
    }
    pub fn rounded(&self) -> Rect {
        let (x, y) = (self.x.round(), self.y.round());
        rect(x, y, self.max_x().round() - x, self.max_y().round() - y)
    }
    pub fn corners(&self) -> [(Handle, Pt); 4] {
        [
            (Handle::TopLeft, pt(self.x, self.y)),
            (Handle::TopRight, pt(self.max_x(), self.y)),
            (Handle::BottomLeft, pt(self.x, self.max_y())),
            (Handle::BottomRight, pt(self.max_x(), self.max_y())),
        ]
    }
}

pub fn dist_to_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    if l2 <= 0.0 {
        return p.dist(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0);
    p.dist(pt(a.x + t * dx, a.y + t * dy))
}

pub fn quad_point(t: f64, s: Pt, c: Pt, e: Pt) -> Pt {
    let u = 1.0 - t;
    pt(u * u * s.x + 2.0 * u * t * c.x + t * t * e.x, u * u * s.y + 2.0 * u * t * c.y + t * t * e.y)
}

pub fn dist_to_quad(p: Pt, s: Pt, c: Pt, e: Pt) -> f64 {
    let mut best = f64::MAX;
    let mut prev = s;
    for i in 1..=32 {
        let cur = quad_point(i as f64 / 32.0, s, c, e);
        best = best.min(dist_to_segment(p, prev, cur));
        prev = cur;
    }
    best
}

/// Punkte entlang der (ggf. gekrümmten) Strecke, `count` Segmente.
pub fn curve_points(s: Pt, c: Option<Pt>, e: Pt, count: usize) -> Vec<Pt> {
    let c = c.unwrap_or(s.mid(e));
    (0..=count).map(|i| quad_point(i as f64 / count as f64, s, c, e)).collect()
}

pub fn curve_end_angle(s: Pt, c: Option<Pt>, e: Pt) -> f64 {
    match c {
        Some(c) if e.dist(c) > 0.0 => (e.y - c.y).atan2(e.x - c.x),
        _ => (e.y - s.y).atan2(e.x - s.x),
    }
}

/// Catmull-Rom → kubische Bézier-Segmente (Capso `BezierSmoothing`): liefert (cp1, cp2, p2) je Segment.
pub fn smooth_segments(points: &[Pt]) -> Vec<(Pt, Pt, Pt)> {
    let n = points.len();
    if n < 3 {
        return points.iter().skip(1).map(|&p| (points[0], p, p)).collect();
    }
    (1..n)
        .map(|i| {
            let p0 = points[i.saturating_sub(2)];
            let p1 = points[i - 1];
            let p2 = points[i];
            let p3 = points[(i + 1).min(n - 1)];
            (pt(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0), pt(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0), p2)
        })
        .collect()
}

// ───────────────────────────── Werkzeuge ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
    Select,
    Arrow,
    Line,
    Rectangle,
    Ellipse,
    Text,
    Freehand,
    Pixelate,
    Counter,
    Highlighter,
    Spotlight,
}

impl Tool {
    pub const ALL: [Tool; 11] = [
        Tool::Select,
        Tool::Arrow,
        Tool::Line,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Text,
        Tool::Freehand,
        Tool::Pixelate,
        Tool::Counter,
        Tool::Highlighter,
        Tool::Spotlight,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Arrow => "arrow",
            Tool::Line => "line",
            Tool::Rectangle => "rectangle",
            Tool::Ellipse => "ellipse",
            Tool::Text => "text",
            Tool::Freehand => "freehand",
            Tool::Pixelate => "pixelate",
            Tool::Counter => "counter",
            Tool::Highlighter => "highlighter",
            Tool::Spotlight => "spotlight",
        }
    }

    pub fn from_id(s: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.id() == s)
    }

    /// (Deutsch, Englisch)
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            Tool::Select => ("Auswählen", "Select"),
            Tool::Arrow => ("Pfeil", "Arrow"),
            Tool::Line => ("Linie", "Line"),
            Tool::Rectangle => ("Rechteck", "Rectangle"),
            Tool::Ellipse => ("Ellipse", "Ellipse"),
            Tool::Text => ("Text", "Text"),
            Tool::Freehand => ("Zeichnen", "Draw"),
            Tool::Pixelate => ("Verpixeln / Weichzeichnen", "Pixelate / Blur"),
            Tool::Counter => ("Zähler", "Counter"),
            Tool::Highlighter => ("Textmarker", "Highlighter"),
            Tool::Spotlight => ("Spotlight", "Spotlight"),
        }
    }

    /// Capso `AnnotationToolShortcut.tool(for:)`: ein Buchstabe ohne Modifikatoren.
    pub fn for_key(key: &str) -> Option<Tool> {
        if key.chars().count() != 1 {
            return None;
        }
        Some(match key.to_lowercase().as_str() {
            "s" => Tool::Select,
            "a" => Tool::Arrow,
            "l" => Tool::Line,
            "r" => Tool::Rectangle,
            "e" | "o" => Tool::Ellipse,
            "t" => Tool::Text,
            "d" | "f" => Tool::Freehand,
            "p" | "b" => Tool::Pixelate,
            "c" | "n" => Tool::Counter,
            "h" => Tool::Highlighter,
            "g" => Tool::Spotlight,
            _ => return None,
        })
    }

    pub fn display_key(self) -> &'static str {
        match self {
            Tool::Select => "S",
            Tool::Arrow => "A",
            Tool::Line => "L",
            Tool::Rectangle => "R",
            Tool::Ellipse => "E",
            Tool::Text => "T",
            Tool::Freehand => "D",
            Tool::Pixelate => "P",
            Tool::Counter => "C",
            Tool::Highlighter => "H",
            Tool::Spotlight => "G",
        }
    }

    /// Werkzeuge, die nach dem Erstellen aktiv bleiben (sonst zurück zu „Auswählen“).
    pub fn is_sticky(self) -> bool {
        !matches!(self, Tool::Select | Tool::Text)
    }
}

/// Capso: Shift + Zentrier-Taste blockiert das Werkzeugkürzel (zentriert stattdessen das Aufziehen).
pub fn tool_for_key(key: &str, shift: bool, center_lock_key: &str) -> Option<Tool> {
    if shift && !center_lock_key.is_empty() && key.eq_ignore_ascii_case(center_lock_key) {
        return None;
    }
    Tool::for_key(key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    Square,
    Angle,
    None,
}

pub fn constraint_kind(tool: Tool) -> ConstraintKind {
    match tool {
        Tool::Rectangle | Tool::Ellipse | Tool::Pixelate | Tool::Spotlight => ConstraintKind::Square,
        Tool::Arrow | Tool::Line => ConstraintKind::Angle,
        _ => ConstraintKind::None,
    }
}

pub fn squared_end(s: Pt, e: Pt) -> Pt {
    let (dx, dy) = (e.x - s.x, e.y - s.y);
    let side = dx.abs().max(dy.abs());
    pt(s.x + if dx < 0.0 { -side } else { side }, s.y + if dy < 0.0 { -side } else { side })
}

pub fn angle_snapped_end(s: Pt, e: Pt) -> Pt {
    let (dx, dy) = (e.x - s.x, e.y - s.y);
    let len = dx.hypot(dy);
    if len <= 0.0 {
        return e;
    }
    let inc = std::f64::consts::FRAC_PI_4;
    let a = (dy.atan2(dx) / inc).round() * inc;
    pt(s.x + a.cos() * len, s.y + a.sin() * len)
}

/// Capso `AnnotationDragConstraint.constrainedDrag`: Shift → Quadrat/45°, `center_lock` → Start ist Mittelpunkt.
pub fn constrained_drag(s: Pt, e: Pt, tool: Tool, shift: bool, center_lock: bool) -> (Pt, Pt) {
    if !shift {
        return (s, e);
    }
    let c = match constraint_kind(tool) {
        ConstraintKind::Square => squared_end(s, e),
        ConstraintKind::Angle => angle_snapped_end(s, e),
        ConstraintKind::None => return (s, e),
    };
    if !center_lock {
        return (s, c);
    }
    (pt(s.x - (c.x - s.x), s.y - (c.y - s.y)), c)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeAction {
    CommitText,
    ExitCrop,
    SwitchToSelect,
    ClearSelection,
    None,
}

/// Esc schließt den Editor nie – es geht immer genau einen Schritt zurück.
pub fn escape_action(editing_text: bool, crop_mode: bool, tool: Tool, has_selection: bool) -> EscapeAction {
    if editing_text {
        EscapeAction::CommitText
    } else if crop_mode {
        EscapeAction::ExitCrop
    } else if tool != Tool::Select {
        EscapeAction::SwitchToSelect
    } else if has_selection {
        EscapeAction::ClearSelection
    } else {
        EscapeAction::None
    }
}

// ───────────────────────────── Stil ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RedactionMode {
    #[default]
    Pixelate,
    Blur,
    Solid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokePattern {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl StrokePattern {
    pub fn dash(self, lw: f64) -> Vec<f64> {
        match self {
            StrokePattern::Solid => vec![],
            StrokePattern::Dashed => vec![lw * 3.0, lw * 2.0],
            StrokePattern::Dotted => vec![0.0, (lw * 2.0).max(6.0)],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ArrowHead {
    #[default]
    Standard,
    Filled,
    Double,
    Tapered,
}

/// Farbvorgaben (Capso `AnnotationColor.allCases`): Name → sRGB.
pub const PRESET_COLORS: [(&str, (u8, u8, u8)); 8] = [
    ("red", (255, 59, 48)),
    ("orange", (255, 148, 0)),
    ("yellow", (255, 204, 0)),
    ("green", (51, 199, 89)),
    ("blue", (0, 122, 255)),
    ("purple", (176, 82, 222)),
    ("white", (255, 255, 255)),
    ("black", (0, 0, 0)),
];

/// Farbe als Name ("red") oder "#RRGGBB" → (r,g,b).
pub fn color_rgb(c: &str) -> (u8, u8, u8) {
    if let Some((_, rgb)) = PRESET_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(c)) {
        return *rgb;
    }
    let h = c.trim_start_matches('#');
    match u32::from_str_radix(h, 16) {
        Ok(v) if h.len() == 6 => ((v >> 16) as u8, (v >> 8) as u8, v as u8),
        _ => (0, 0, 0),
    }
}

pub fn color_hex(c: &str) -> String {
    let (r, g, b) = color_rgb(c);
    format!("#{r:02X}{g:02X}{b:02X}")
}

pub fn css_rgba(c: &str, a: f64) -> String {
    let (r, g, b) = color_rgb(c);
    format!("rgba({r},{g},{b},{a})")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    pub color: String,
    pub line_width: f64,
    pub opacity: f64,
    pub filled: bool,
    pub pattern: StrokePattern,
    pub arrow_head: ArrowHead,
}

impl Default for Style {
    fn default() -> Self {
        Style { color: "red".into(), line_width: 3.0, opacity: 1.0, filled: false, pattern: StrokePattern::Solid, arrow_head: ArrowHead::Standard }
    }
}

// ───────────────────────────── Objekte ─────────────────────────────

pub const COUNTER_ARROW_CLEARANCE: f64 = 6.0;
pub const SPOTLIGHT_DIM: f64 = 0.55;
pub const SPOTLIGHT_RADIUS: f64 = 8.0;
pub const TEXT_BOX_PADDING: f64 = 4.0;
pub const TEXT_LINE_HEIGHT: f64 = 1.2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Shape {
    Arrow { start: Pt, end: Pt, ctrl: Option<Pt> },
    Line { start: Pt, end: Pt, ctrl: Option<Pt> },
    Rect { rect: Rect, radius: f64 },
    Ellipse { rect: Rect },
    /// `boxed` (Capso `boxSize != nil`): `w`/`h` = Textbox, Text bricht innerhalb (Innenabstand `TEXT_BOX_PADDING`) um.
    /// Sonst (alter Stil ohne Box): `w`/`h` = gemessene Textgröße, kein Umbruch.
    Text {
        text: String,
        origin: Pt,
        font_size: f64,
        w: f64,
        h: f64,
        boxed: bool,
        fill: Option<String>,
        outline: Option<String>,
        trace: Option<String>,
    },
    Freehand { points: Vec<Pt> },
    Pixelate { rect: Rect, block: f64, mode: RedactionMode },
    Counter { center: Pt, tip: Option<Pt>, radius: f64, number: u32 },
    Spotlight { rect: Rect },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Start,
    End,
    Control,
    CounterTip,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: u64,
    pub style: Style,
    pub shape: Shape,
}

fn arrow_head_len(lw: f64) -> f64 {
    15f64.max(lw * 3.0)
}

pub fn counter_arrow_width(radius: f64, lw: f64) -> f64 {
    2f64.max(lw.min(radius * 0.28))
}

pub fn counter_has_arrow(center: Pt, tip: Option<Pt>, radius: f64) -> bool {
    tip.is_some_and(|t| t.dist(center) >= radius + COUNTER_ARROW_CLEARANCE)
}

fn path_bounds(s: Pt, e: Pt, c: Option<Pt>, pad: f64) -> Rect {
    let mut r = Rect::from_points(s, e).inset(-pad, -pad);
    if let Some(c) = c {
        r = r.union(&rect(c.x - pad, c.y - pad, 2.0 * pad, 2.0 * pad));
    }
    r
}

impl Object {
    pub fn bounds(&self) -> Rect {
        let lw = self.style.line_width;
        match &self.shape {
            Shape::Arrow { start, end, ctrl } => path_bounds(*start, *end, *ctrl, arrow_head_len(lw)),
            Shape::Line { start, end, ctrl } => path_bounds(*start, *end, *ctrl, lw.max(1.0)),
            Shape::Rect { rect, .. } | Shape::Ellipse { rect } | Shape::Pixelate { rect, .. } | Shape::Spotlight { rect } => *rect,
            Shape::Text { origin, w, h, fill, outline, trace, font_size, .. } => {
                let r = rect(origin.x, origin.y, *w, *h);
                let boxp = if fill.is_some() || outline.is_some() { TEXT_BOX_PADDING } else { 0.0 };
                let glyph = if trace.is_some() { TEXT_BOX_PADDING.max((font_size * 0.03).ceil()) } else { 0.0 };
                let p = boxp.max(glyph);
                r.inset(-p, -p)
            }
            Shape::Freehand { points } => {
                if points.is_empty() {
                    return Rect::default();
                }
                let mut r = rect(points[0].x, points[0].y, 0.0, 0.0);
                for p in points {
                    r = r.union(&rect(p.x, p.y, 0.0, 0.0));
                }
                r.inset(-lw, -lw)
            }
            Shape::Counter { center, tip, radius, .. } => {
                let c = rect(center.x - radius, center.y - radius, radius * 2.0, radius * 2.0);
                match tip {
                    Some(t) if counter_has_arrow(*center, *tip, *radius) => {
                        let hl = 10f64.max(counter_arrow_width(*radius, lw) * 3.0);
                        c.union(&rect(t.x - hl, t.y - hl, hl * 2.0, hl * 2.0))
                    }
                    _ => c,
                }
            }
        }
    }

    pub fn hit_test(&self, p: Pt, th: f64) -> bool {
        let lw = self.style.line_width;
        match &self.shape {
            Shape::Arrow { start, end, ctrl } | Shape::Line { start, end, ctrl } => {
                let d = match ctrl {
                    Some(c) => dist_to_quad(p, *start, *c, *end),
                    None => dist_to_segment(p, *start, *end),
                };
                d <= th + lw / 2.0
            }
            Shape::Rect { rect, .. } => {
                if self.style.filled {
                    return rect.inset(-th, -th).contains(p);
                }
                let o = th + lw / 2.0;
                rect.inset(-o, -o).contains(p) && !rect.inset(o, o).contains(p)
            }
            Shape::Ellipse { rect } => {
                let (rx, ry) = (rect.w / 2.0, rect.h / 2.0);
                if rx <= 0.0 || ry <= 0.0 {
                    return false;
                }
                let c = rect.center();
                let (dx, dy) = (p.x - c.x, p.y - c.y);
                let norm = dx * dx / (rx * rx) + dy * dy / (ry * ry);
                if self.style.filled {
                    norm <= 1.0 + th / rx.min(ry)
                } else {
                    (norm - 1.0).abs() <= (th + lw / 2.0) / rx.min(ry)
                }
            }
            Shape::Text { .. } | Shape::Pixelate { .. } | Shape::Spotlight { .. } => self.bounds().inset(-th, -th).contains(p),
            Shape::Freehand { points } => {
                let r = th + lw / 2.0;
                match points.len() {
                    0 => false,
                    1 => p.dist(points[0]) <= r,
                    _ => points.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= r),
                }
            }
            Shape::Counter { center, tip, radius, .. } => {
                if p.dist(*center) <= radius + th {
                    return true;
                }
                counter_has_arrow(*center, *tip, *radius) && dist_to_segment(p, *center, tip.unwrap()) <= th + counter_arrow_width(*radius, lw) / 2.0
            }
        }
    }

    pub fn move_by(&mut self, dx: f64, dy: f64) {
        let mv = |p: &mut Pt| *p = p.add(dx, dy);
        match &mut self.shape {
            Shape::Arrow { start, end, ctrl } | Shape::Line { start, end, ctrl } => {
                mv(start);
                mv(end);
                if let Some(c) = ctrl {
                    mv(c);
                }
            }
            Shape::Rect { rect, .. } | Shape::Ellipse { rect } | Shape::Pixelate { rect, .. } | Shape::Spotlight { rect } => {
                rect.x += dx;
                rect.y += dy;
            }
            Shape::Text { origin, .. } => mv(origin),
            Shape::Freehand { points } => points.iter_mut().for_each(mv),
            Shape::Counter { center, tip, .. } => {
                mv(center);
                if let Some(t) = tip {
                    mv(t);
                }
            }
        }
    }

    /// Sichtbare Griffe zum Ändern (Ecken bzw. Pfadpunkte bzw. Zählerspitze).
    pub fn handles(&self, zoom: f64) -> Vec<(Handle, Pt)> {
        match &self.shape {
            Shape::Arrow { start, end, ctrl } | Shape::Line { start, end, ctrl } => {
                let mid = match ctrl {
                    Some(c) => quad_point(0.5, *start, *c, *end),
                    None => start.mid(*end),
                };
                vec![(Handle::Start, *start), (Handle::End, *end), (Handle::Control, mid)]
            }
            Shape::Counter { center, tip, radius, .. } => {
                let t = tip.unwrap_or_else(|| pt(center.x, center.y - (radius + COUNTER_ARROW_CLEARANCE + 4.0 / zoom.max(0.1))));
                vec![(Handle::CounterTip, t)]
            }
            _ => self.bounds().corners().to_vec(),
        }
    }

    pub fn handle_at(&self, p: Pt, zoom: f64) -> Option<Handle> {
        let r = (10.0 / zoom.max(0.1)).max(9.0);
        self.handles(zoom).into_iter().find(|(_, c)| c.dist(p) <= r).map(|(h, _)| h)
    }

    /// Griff ziehen. `orig` ist der Zustand beim Drag-Start (für Skalierungen).
    pub fn drag_handle(&mut self, orig: &Object, h: Handle, p: Pt, shift: bool) {
        let ob = orig.bounds();
        match (&mut self.shape, &orig.shape) {
            (Shape::Arrow { start, end, ctrl }, _) | (Shape::Line { start, end, ctrl }, _) => match h {
                Handle::Start => *start = if shift { angle_snapped_end(*end, p) } else { p },
                Handle::End => *end = if shift { angle_snapped_end(*start, p) } else { p },
                Handle::Control => {
                    let m = start.mid(*end);
                    *ctrl = if p.dist(m) < 2.0 { None } else { Some(pt(2.0 * p.x - m.x, 2.0 * p.y - m.y)) };
                }
                _ => {}
            },
            (Shape::Counter { center, tip, radius, .. }, _) => {
                if h == Handle::CounterTip {
                    *tip = (p.dist(*center) >= *radius + COUNTER_ARROW_CLEARANCE).then_some(p);
                }
            }
            (shape, oshape) => {
                let anchor = match h {
                    Handle::TopLeft => pt(ob.max_x(), ob.max_y()),
                    Handle::TopRight => pt(ob.x, ob.max_y()),
                    Handle::BottomLeft => pt(ob.max_x(), ob.y),
                    Handle::BottomRight => pt(ob.x, ob.y),
                    _ => return,
                };
                let p = if shift { squared_end(anchor, p) } else { p };
                let mut nr = Rect::from_points(anchor, p);
                nr.w = nr.w.max(4.0);
                nr.h = nr.h.max(4.0);
                match (shape, oshape) {
                    (Shape::Rect { rect, .. }, _) | (Shape::Ellipse { rect }, _) | (Shape::Pixelate { rect, .. }, _) | (Shape::Spotlight { rect }, _) => *rect = nr,
                    (Shape::Freehand { points }, Shape::Freehand { points: op }) => {
                        let (sx, sy) = (nr.w / ob.w.max(1.0), nr.h / ob.h.max(1.0));
                        *points = op.iter().map(|q| pt(nr.x + (q.x - ob.x) * sx, nr.y + (q.y - ob.y) * sy)).collect();
                    }
                    (Shape::Text { origin, font_size, w, h: th, boxed: true, .. }, Shape::Text { origin: oo, w: ow, h: oh, .. }) => {
                        // Textbox: Ecke ziehen ändert die Box, der Text bricht neu um (Capso `TextResizeGeometry.rect`).
                        let Some(th_) = text_handle(h) else { return };
                        let r = text_resize_rect(rect(oo.x, oo.y, *ow, *oh), th_, pt(p.x - orig_handle_pt(&ob, h).x, p.y - orig_handle_pt(&ob, h).y), text_box_min(*font_size));
                        *origin = pt(r.x, r.y);
                        *w = r.w;
                        *th = r.h;
                    }
                    (Shape::Text { origin, font_size, w, h: th, .. }, Shape::Text { origin: oo, font_size: ofs, w: ow, h: oh, .. }) => {
                        // Alter Text ohne Box: Schrift skalieren, gegenüberliegende Ecke bleibt (Capso `TextResizeGeometry.fontSize/origin`).
                        let Some(th_) = text_handle(h) else { return };
                        let d = pt(p.x - orig_handle_pt(&ob, h).x, p.y - orig_handle_pt(&ob, h).y);
                        *font_size = text_resize_font_size(ob, *ofs, th_, d).min(400.0);
                        let k = *font_size / ofs;
                        *w = ow * k;
                        *th = oh * k;
                        let (padx, pady) = (oo.x - ob.x, oo.y - ob.y);
                        let size = (*w + 2.0 * padx, *th + 2.0 * pady);
                        let o = text_resize_origin(ob, size, th_);
                        *origin = pt(o.x + padx, o.y + pady);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Ecke eines Rechtecks zu einem Eckgriff.
fn orig_handle_pt(r: &Rect, h: Handle) -> Pt {
    match h {
        Handle::TopLeft => pt(r.x, r.y),
        Handle::TopRight => pt(r.max_x(), r.y),
        Handle::BottomLeft => pt(r.x, r.max_y()),
        _ => pt(r.max_x(), r.max_y()),
    }
}

fn text_handle(h: Handle) -> Option<Handle> {
    matches!(h, Handle::TopLeft | Handle::TopRight | Handle::BottomLeft | Handle::BottomRight).then_some(h)
}

/// Mindestgröße einer Textbox (Capso `minBoxSize`: 60 × max(28, Schrift + 12)).
pub fn text_box_min(font_size: f64) -> (f64, f64) {
    (60.0, 28f64.max(font_size + 12.0))
}

fn text_target_rect(b: Rect, h: Handle, d: Pt) -> Rect {
    match h {
        Handle::TopLeft => rect(b.x + d.x, b.y + d.y, b.w - d.x, b.h - d.y),
        Handle::TopRight => rect(b.x, b.y + d.y, b.w + d.x, b.h - d.y),
        Handle::BottomLeft => rect(b.x + d.x, b.y, b.w - d.x, b.h + d.y),
        _ => rect(b.x, b.y, b.w + d.x, b.h + d.y),
    }
}

/// Capso `TextResizeGeometry.rect`: neue Textbox beim Ziehen eines Eckgriffs (gegenüberliegende Kanten bleiben, Mindestgröße).
pub fn text_resize_rect(b: Rect, h: Handle, d: Pt, min: (f64, f64)) -> Rect {
    let mut r = text_target_rect(b, h, d);
    if matches!(h, Handle::TopLeft | Handle::BottomLeft) {
        r.x = r.x.min(b.max_x() - min.0);
        r.w = b.max_x() - r.x;
    } else {
        r.w = r.w.max(min.0);
    }
    if matches!(h, Handle::TopLeft | Handle::TopRight) {
        r.y = r.y.min(b.max_y() - min.1);
        r.h = b.max_y() - r.y;
    } else {
        r.h = r.h.max(min.1);
    }
    r
}

/// Capso `TextResizeGeometry.fontSize` (Text ohne Box).
pub fn text_resize_font_size(b: Rect, font_size: f64, h: Handle, d: Pt) -> f64 {
    if b.w <= 0.0 || b.h <= 0.0 {
        return font_size.max(6.0);
    }
    let t = text_target_rect(b, h, d);
    let k = (t.w.max(10.0) / b.w).max(t.h.max(10.0) / b.h);
    (font_size * k).max(6.0)
}

/// Capso `TextResizeGeometry.origin`: Ursprung, sodass die gegenüberliegende Ecke stehen bleibt.
pub fn text_resize_origin(b: Rect, size: (f64, f64), h: Handle) -> Pt {
    match h {
        Handle::TopLeft => pt(b.max_x() - size.0, b.max_y() - size.1),
        Handle::TopRight => pt(b.x, b.max_y() - size.1),
        Handle::BottomLeft => pt(b.max_x() - size.0, b.y),
        _ => pt(b.x, b.y),
    }
}

/// Aktionsknöpfe am ausgewählten Zähler (Capso `counterDeleteButtonRect` / `counterStepButtonRect`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterAction {
    Delete,
    Decrement,
    Increment,
}

/// Rechtecke der Zählerknöpfe (×, −, +) in Bildkoordinaten; `zoom` = Bildschirmpunkte pro Bildpixel.
pub fn counter_buttons(o: &Object, zoom: f64) -> Vec<(CounterAction, Rect)> {
    let Shape::Counter { center, radius, .. } = o.shape else { return vec![] };
    let s = zoom.max(0.1);
    let frame = o.bounds().inset(-6.0 / s, -6.0 / s);
    let del = 22.0 / s;
    let (size, gap) = (20.0 / s, 4.0 / s);
    let cy = center.y + radius + 8.0 / s + size / 2.0;
    let step = |inc: bool| {
        let cx = if inc { center.x + gap / 2.0 + size / 2.0 } else { center.x - gap / 2.0 - size / 2.0 };
        rect(cx - size / 2.0, cy - size / 2.0, size, size)
    };
    vec![
        (CounterAction::Delete, rect(frame.max_x() + 5.0 / s, frame.y - del / 2.0, del, del)),
        (CounterAction::Decrement, step(false)),
        (CounterAction::Increment, step(true)),
    ]
}

pub fn counter_action_at(o: &Object, p: Pt, zoom: f64) -> Option<CounterAction> {
    counter_buttons(o, zoom).into_iter().find(|(_, r)| r.contains(p)).map(|(a, _)| a)
}

/// Intelligenter Marker (Capso `highlighterSnapRect`): erste OCR-Textzeile, deren Box (vertikal um 30 % vergrößert) `p` enthält.
pub fn highlight_snap_line(lines: &[Rect], p: Pt) -> Option<Rect> {
    lines.iter().find(|r| r.inset(0.0, -r.h * 0.3).contains(p)).copied()
}

// ───────────────────────────── Dokument ─────────────────────────────

/// Drehen/Spiegeln des Originals: Arbeitsbild = Rotation^rot(Spiegel^flip(Original)), Rotation = 90° gegen den Uhrzeigersinn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Xform {
    pub rot: u8,
    pub flip: bool,
}

impl Xform {
    pub fn rotate_ccw(self) -> Xform {
        Xform { rot: (self.rot + 1) % 4, flip: self.flip }
    }
    /// Horizontal spiegeln *nach* der aktuellen Transformation (F·R^r = R^-r·F).
    pub fn flip_h(self) -> Xform {
        Xform { rot: (4 - self.rot) % 4, flip: !self.flip }
    }
    pub fn size(self, w: f64, h: f64) -> (f64, f64) {
        if self.rot % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }
    /// Affine Matrix [a,b,c,d,e,f] (Canvas `setTransform`) Original → Arbeitsbild.
    pub fn matrix(self, w: f64, h: f64) -> [f64; 6] {
        // Start: Spiegelung
        let mut m = if self.flip { [-1.0, 0.0, 0.0, 1.0, w, 0.0] } else { [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };
        let (mut cw, mut ch) = (w, h);
        for _ in 0..self.rot {
            // (X,Y) → (Y, cw − X)
            let [a, b, c, d, e, f] = m;
            m = [b, -a, d, -c, f, cw - e];
            (cw, ch) = (ch, cw);
        }
        m
    }
    pub fn apply(self, w: f64, h: f64, p: Pt) -> Pt {
        let [a, b, c, d, e, f] = self.matrix(w, h);
        pt(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }
}

/// Capso `RectTransforms.rotate90CCW`.
pub fn rect_rotate_ccw(r: Rect, old_w: f64) -> Rect {
    rect(r.y, old_w - r.max_x(), r.h, r.w)
}

pub fn rect_flip_h(r: Rect, w: f64) -> Rect {
    rect(w - r.max_x(), r.y, r.w, r.h)
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    objects: Vec<Object>,
    crop: Option<Rect>,
    xform: Xform,
    output: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    /// Originalgröße (Pixel, vor Drehen/Spiegeln).
    pub orig_w: f64,
    pub orig_h: f64,
    pub objects: Vec<Object>,
    pub selected: Option<u64>,
    pub crop: Option<Rect>,
    pub xform: Xform,
    /// Ausgabegröße nach dem Zuschneiden („Resize output“).
    pub output: Option<(u32, u32)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    next_id: u64,
    /// Kopiertes Objekt (Capso `AnnotationClipboard`, pro Editor).
    clipboard: Option<Object>,
}

impl Doc {
    pub fn new(w: f64, h: f64) -> Doc {
        Doc { orig_w: w, orig_h: h, next_id: 1, ..Default::default() }
    }
    /// Größe des Arbeitsbildes.
    pub fn image_size(&self) -> (f64, f64) {
        self.xform.size(self.orig_w, self.orig_h)
    }
    pub fn image_rect(&self) -> Rect {
        let (w, h) = self.image_size();
        rect(0.0, 0.0, w, h)
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn has_unsaved_changes(&self) -> bool {
        !self.objects.is_empty() || self.crop.is_some() || self.can_undo()
    }
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot { objects: self.objects.clone(), crop: self.crop, xform: self.xform, output: self.output }
    }
    fn apply(&mut self, s: Snapshot) {
        self.objects = s.objects;
        self.crop = s.crop;
        self.xform = s.xform;
        self.output = s.output;
        self.selected = None;
    }
    /// Zustand vor einer Änderung sichern (Capso `beginDrag`/`pushUndo`).
    pub fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        self.redo.clear();
    }
    /// Letzten Undo-Eintrag verwerfen, falls sich nichts geändert hat (z. B. Klick ohne Verschieben).
    pub fn drop_undo_if_unchanged(&mut self) {
        if self.undo.last().is_some_and(|s| *s == self.snapshot()) {
            self.undo.pop();
        }
    }
    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.apply(s);
        }
    }
    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.apply(s);
        }
    }
    pub fn add(&mut self, style: Style, shape: Shape) -> u64 {
        self.push_undo();
        let id = self.new_id();
        self.objects.push(Object { id, style, shape });
        self.selected = Some(id);
        id
    }
    pub fn remove(&mut self, id: u64) {
        self.push_undo();
        self.objects.retain(|o| o.id != id);
        if self.selected == Some(id) {
            self.selected = None;
        }
    }
    pub fn remove_selected(&mut self) -> bool {
        match self.selected {
            Some(id) => {
                self.remove(id);
                true
            }
            None => false,
        }
    }
    pub fn get(&self, id: u64) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }
    pub fn selected_object(&self) -> Option<&Object> {
        self.selected.and_then(|id| self.get(id))
    }
    pub fn object_at(&self, p: Pt, th: f64) -> Option<u64> {
        self.objects.iter().rev().find(|o| o.hit_test(p, th)).map(|o| o.id)
    }
    /// Zählerknopf ausführen (je ein Undo-Schritt; Nummer mindestens 1). `true` = geändert.
    pub fn counter_action(&mut self, id: u64, a: CounterAction) -> bool {
        let Some(Shape::Counter { number, .. }) = self.get(id).map(|o| &o.shape) else { return false };
        match a {
            CounterAction::Delete => {
                self.selected = Some(id);
                self.remove_selected()
            }
            CounterAction::Decrement if *number <= 1 => false,
            _ => {
                self.push_undo();
                if let Some(Object { shape: Shape::Counter { number, .. }, .. }) = self.get_mut(id) {
                    *number = if a == CounterAction::Increment { *number + 1 } else { *number - 1 };
                }
                true
            }
        }
    }
    pub fn next_counter_number(&self) -> u32 {
        self.objects.iter().filter_map(|o| if let Shape::Counter { number, .. } = o.shape { Some(number) } else { None }).max().unwrap_or(0) + 1
    }
    pub fn copy_selection(&mut self) -> bool {
        match self.selected_object().cloned() {
            Some(o) => {
                self.clipboard = Some(o);
                true
            }
            None => false,
        }
    }
    pub fn paste(&mut self, offset: f64) -> bool {
        let Some(mut o) = self.clipboard.clone() else { return false };
        o.move_by(offset, offset);
        self.add(o.style, o.shape);
        // Mehrfaches Einfügen versetzt weiter.
        if let Some(c) = &mut self.clipboard {
            c.move_by(offset, offset);
        }
        true
    }
    pub fn duplicate_selected(&mut self, offset: f64) -> bool {
        let Some(mut o) = self.selected_object().cloned() else { return false };
        o.move_by(offset, offset);
        self.add(o.style, o.shape);
        true
    }
    pub fn set_crop(&mut self, crop: Option<Rect>, xform: Xform, output: Option<(u32, u32)>) {
        self.push_undo();
        self.crop = crop;
        self.xform = xform;
        self.output = output;
    }
}

/// Capso `AnnotationRenderer.canvasRect`: Bild, vergrößert um Objekte, die über den Rand hinausragen.
pub const CANVAS_MARGIN: f64 = 12.0;

pub fn canvas_rect(img: Rect, objects: &[Object]) -> Rect {
    objects.iter().fold(img, |r, o| {
        let b = o.bounds();
        if img.contains_rect(&b) {
            r
        } else {
            r.union(&b.inset(-CANVAS_MARGIN, -CANVAS_MARGIN).integral())
        }
    })
}

/// Sichtbarer/Export-Bereich: Zuschnitt, sonst (vergrößerter) Bildbereich.
pub fn content_rect(doc: &Doc) -> Rect {
    match doc.crop {
        Some(c) => c.rounded().intersect(&doc.image_rect()),
        None => canvas_rect(doc.image_rect(), &doc.objects),
    }
}

// ───────────────────────────── Zuschneiden ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropPreset {
    Freeform,
    Original,
    Square,
    R4x3,
    R3x2,
    R16x9,
}

impl CropPreset {
    pub const ALL: [CropPreset; 6] = [CropPreset::Freeform, CropPreset::Original, CropPreset::Square, CropPreset::R4x3, CropPreset::R3x2, CropPreset::R16x9];
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            CropPreset::Freeform => ("Frei", "Freeform"),
            CropPreset::Original => ("Originalverhältnis", "Original Ratio"),
            CropPreset::Square => ("1 : 1 (Quadrat)", "1 : 1 (Square)"),
            CropPreset::R4x3 => ("4 : 3", "4 : 3"),
            CropPreset::R3x2 => ("3 : 2", "3 : 2"),
            CropPreset::R16x9 => ("16 : 9", "16 : 9"),
        }
    }
    /// Gespeicherter Wert (Capso `CropPreset.rawValue`, Einstellung `annotationLastCropPreset`).
    pub fn id(self) -> &'static str {
        match self {
            CropPreset::Freeform => "freeform",
            CropPreset::Original => "original",
            CropPreset::Square => "square",
            CropPreset::R4x3 => "ratio4x3",
            CropPreset::R3x2 => "ratio3x2",
            CropPreset::R16x9 => "ratio16x9",
        }
    }
    pub fn from_id(s: &str) -> CropPreset {
        CropPreset::ALL.into_iter().find(|p| p.id() == s).unwrap_or(CropPreset::Freeform)
    }
    /// Nächste/vorige Vorgabe (Capso `cyclePreset`, Taste R / ⇧R).
    pub fn cycle(self, forward: bool) -> CropPreset {
        let n = CropPreset::ALL.len();
        let i = CropPreset::ALL.iter().position(|&p| p == self).unwrap_or(0);
        CropPreset::ALL[if forward { (i + 1) % n } else { (i + n - 1) % n }]
    }
    /// Vorgabe per Ziffer 1–6 (Capso `selectPreset(byDigit:)`).
    pub fn from_digit(d: &str) -> Option<CropPreset> {
        let n: usize = d.parse().ok()?;
        (1..=CropPreset::ALL.len()).contains(&n).then(|| CropPreset::ALL[n - 1])
    }
    pub fn ratio(self, w: f64, h: f64) -> Option<f64> {
        match self {
            CropPreset::Freeform => None,
            CropPreset::Original => (h > 0.0).then(|| w / h),
            CropPreset::Square => Some(1.0),
            CropPreset::R4x3 => Some(4.0 / 3.0),
            CropPreset::R3x2 => Some(1.5),
            CropPreset::R16x9 => Some(16.0 / 9.0),
        }
    }
}

pub fn snap_value(v: f64, edges: &[f64], th: f64) -> f64 {
    if th <= 0.0 {
        return v;
    }
    let mut best = v;
    let mut bd = th;
    for &e in edges {
        let d = (v - e).abs();
        if d <= bd {
            bd = d;
            best = e;
        }
    }
    best
}

/// Capso `CropSnap.snapRect`: Kanten nahe am Bildrand auf den Rand ziehen.
pub fn snap_rect(r: Rect, w: f64, h: f64, th: f64) -> Rect {
    if th <= 0.0 {
        return r;
    }
    let x0 = snap_value(r.x, &[0.0, w], th);
    let x1 = snap_value(r.max_x(), &[0.0, w], th);
    let y0 = snap_value(r.y, &[0.0, h], th);
    let y1 = snap_value(r.max_y(), &[0.0, h], th);
    rect(x0, y0, x1 - x0, y1 - y0)
}

/// Capso `CropAreaNSView.creationRect`: neues Zuschnitt-Rechteck aufziehen (Richtung bleibt, Bildgrenzen gelten).
pub fn crop_creation_rect(s: Pt, e: Pt, ratio: Option<f64>, w: f64, h: f64) -> Rect {
    let e = pt(e.x.clamp(0.0, w), e.y.clamp(0.0, h));
    let (dx, dy) = (e.x - s.x, e.y - s.y);
    let (mut rw, mut rh) = (dx.abs(), dy.abs());
    if let Some(r) = ratio.filter(|r| *r > 0.0) {
        let avail_h = if dy < 0.0 { s.y } else { h - s.y };
        rh = rw / r;
        if rh > avail_h {
            rh = avail_h;
            rw = rh * r;
        }
    }
    let x = if dx < 0.0 { s.x - rw } else { s.x };
    let y = if dy < 0.0 { s.y - rh } else { s.y };
    rect(x, y, rw, rh)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    Move,
}

impl CropHandle {
    pub const EDGES: [CropHandle; 8] =
        [CropHandle::TopLeft, CropHandle::Top, CropHandle::TopRight, CropHandle::Right, CropHandle::BottomRight, CropHandle::Bottom, CropHandle::BottomLeft, CropHandle::Left];
    pub fn point(self, r: &Rect) -> Pt {
        let c = r.center();
        match self {
            CropHandle::TopLeft => pt(r.x, r.y),
            CropHandle::Top => pt(c.x, r.y),
            CropHandle::TopRight => pt(r.max_x(), r.y),
            CropHandle::Right => pt(r.max_x(), c.y),
            CropHandle::BottomRight => pt(r.max_x(), r.max_y()),
            CropHandle::Bottom => pt(c.x, r.max_y()),
            CropHandle::BottomLeft => pt(r.x, r.max_y()),
            CropHandle::Left => pt(r.x, c.y),
            CropHandle::Move => c,
        }
    }
    pub fn cursor(self) -> &'static str {
        match self {
            CropHandle::TopLeft | CropHandle::BottomRight => "nwse-resize",
            CropHandle::TopRight | CropHandle::BottomLeft => "nesw-resize",
            CropHandle::Top | CropHandle::Bottom => "ns-resize",
            CropHandle::Left | CropHandle::Right => "ew-resize",
            CropHandle::Move => "move",
        }
    }
}

pub fn crop_handle_at(r: &Rect, p: Pt, radius: f64) -> Option<CropHandle> {
    CropHandle::EDGES.into_iter().find(|h| h.point(r).dist(p) <= radius).or_else(|| r.contains(p).then_some(CropHandle::Move))
}

pub const MIN_CROP: f64 = 8.0;

/// Zuschnitt per Griff ändern (von `orig` aus, Zeiger um `d` verschoben), mit Seitenverhältnis, Bildgrenzen und Snap.
pub fn crop_resize(orig: Rect, h: CropHandle, d: Pt, ratio: Option<f64>, w: f64, ih: f64, snap: f64) -> Rect {
    if h == CropHandle::Move {
        let x = (orig.x + d.x).clamp(0.0, (w - orig.w).max(0.0));
        let y = (orig.y + d.y).clamp(0.0, (ih - orig.h).max(0.0));
        let r = rect(x, y, orig.w, orig.h);
        // Beim Verschieben ganzes Rechteck an Ränder schnappen.
        let sx = snap_value(r.x, &[0.0, w - r.w], snap);
        let sy = snap_value(r.y, &[0.0, ih - r.h], snap);
        return rect(sx, sy, r.w, r.h);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.max_x(), orig.max_y());
    let (left, right, top, bottom) = match h {
        CropHandle::TopLeft => (true, false, true, false),
        CropHandle::Top => (false, false, true, false),
        CropHandle::TopRight => (false, true, true, false),
        CropHandle::Right => (false, true, false, false),
        CropHandle::BottomRight => (false, true, false, true),
        CropHandle::Bottom => (false, false, false, true),
        CropHandle::BottomLeft => (true, false, false, true),
        CropHandle::Left => (true, false, false, false),
        CropHandle::Move => unreachable!(),
    };
    if left {
        x0 = snap_value((x0 + d.x).clamp(0.0, x1 - MIN_CROP), &[0.0], snap);
    }
    if right {
        x1 = snap_value((x1 + d.x).clamp(x0 + MIN_CROP, w), &[w], snap);
    }
    if top {
        y0 = snap_value((y0 + d.y).clamp(0.0, y1 - MIN_CROP), &[0.0], snap);
    }
    if bottom {
        y1 = snap_value((y1 + d.y).clamp(y0 + MIN_CROP, ih), &[ih], snap);
    }
    let Some(r) = ratio.filter(|r| *r > 0.0) else { return rect(x0, y0, x1 - x0, y1 - y0) };
    let (mut cw, mut ch) = (x1 - x0, y1 - y0);
    let corner = (left || right) && (top || bottom);
    if corner {
        if cw / ch > r {
            cw = ch * r;
        } else {
            ch = cw / r;
        }
    } else if left || right {
        ch = cw / r;
    } else {
        cw = ch * r;
    }
    // Anker bestimmen und in die Bildgrenzen einpassen.
    let ax = if left { orig.max_x() } else if right { orig.x } else { orig.center().x };
    let ay = if top { orig.max_y() } else if bottom { orig.y } else { orig.center().y };
    let max_w = if left { ax } else if right { w - ax } else { 2.0 * ax.min(w - ax) };
    let max_h = if top { ay } else if bottom { ih - ay } else { 2.0 * ay.min(ih - ay) };
    let k = (max_w / cw).min(max_h / ch).min(1.0);
    cw *= k;
    ch *= k;
    let nx = if left { ax - cw } else if right { ax } else { ax - cw / 2.0 };
    let ny = if top { ay - ch } else if bottom { ay } else { ay - ch / 2.0 };
    rect(nx, ny, cw, ch)
}

/// Größtes zentriertes Rechteck mit Seitenverhältnis innerhalb von `bounds`.
pub fn fit_ratio(bounds: Rect, ratio: f64) -> Rect {
    let (mut w, mut h) = (bounds.w, bounds.w / ratio);
    if h > bounds.h {
        h = bounds.h;
        w = h * ratio;
    }
    let c = bounds.center();
    rect(c.x - w / 2.0, c.y - h / 2.0, w, h)
}

// ───────────────────────────── Beautify ─────────────────────────────

/// Einstellungen für „Verschönern“ (Hintergrund um den Screenshot).
///
/// Gespeichert als Einstellung `beautifyStyle` (JSON, kompatibel zu Capso `BeautifySettings.Style`):
/// ```json
/// {
///   "backgroundStyle": "solid" | "gradient" | "liquidGlass" | "none",
///   "backgroundColor": [r, g, b, a],          // sRGB 0…1 (solid)
///   "gradient": [[r,g,b,a], [r,g,b,a]],       // Start/Ende (gradient, nrzCapture-Erweiterung)
///   "gradientAngle": 135,                     // Grad, 0 = von links nach rechts, im Uhrzeigersinn
///   "padding": 40,                            // Bildpunkte rund um das Bild
///   "cornerRadius": 12,                       // Eckenradius des Bildes
///   "shadowEnabled": true,
///   "shadowRadius": 20,                       // Weichheit; Schatten: schwarz 25 %, 6 px nach unten
///   "aspect": "auto" | "1:1" | "4:3" | "16:9" | "3:2" | "9:16"   // Seitenverhältnis der Ausgabe
/// }
/// ```
/// Ausgabegröße: Bild + 2·(padding + shadowInset) mit `shadowInset = shadowEnabled ? shadowRadius + 6 : 0`,
/// danach symmetrisch erweitert bis zum gewählten Seitenverhältnis. `autoBeautifyScreenshots` wendet genau diesen Stil an.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Beautify {
    #[serde(skip)]
    pub enabled: bool,
    pub background_style: String,
    pub background_color: [f64; 4],
    pub gradient: [[f64; 4]; 2],
    pub gradient_angle: f64,
    pub padding: f64,
    pub corner_radius: f64,
    pub shadow_enabled: bool,
    pub shadow_radius: f64,
    pub aspect: String,
}

impl Default for Beautify {
    fn default() -> Self {
        Beautify {
            enabled: false,
            background_style: "solid".into(),
            background_color: [1.0, 1.0, 1.0, 1.0],
            gradient: GRADIENTS[0],
            gradient_angle: 135.0,
            padding: 40.0,
            corner_radius: 12.0,
            shadow_enabled: true,
            shadow_radius: 20.0,
            aspect: "auto".into(),
        }
    }
}

/// Farbvorgaben (Capso `BeautifyPanel.presetColors`).
pub const BEAUTIFY_COLORS: [[f64; 4]; 8] = [
    [1.0, 1.0, 1.0, 1.0],
    [0.96, 0.96, 0.96, 1.0],
    [0.18, 0.18, 0.18, 1.0],
    [0.0, 0.0, 0.0, 1.0],
    [0.729, 0.902, 0.992, 1.0],
    [0.996, 0.843, 0.663, 1.0],
    [0.733, 0.969, 0.816, 1.0],
    [0.867, 0.839, 0.996, 1.0],
];

pub const GRADIENTS: [[[f64; 4]; 2]; 6] = [
    [[0.40, 0.49, 0.92, 1.0], [0.46, 0.29, 0.64, 1.0]],
    [[0.98, 0.44, 0.60, 1.0], [0.99, 0.75, 0.42, 1.0]],
    [[0.26, 0.91, 0.48, 1.0], [0.22, 0.98, 0.84, 1.0]],
    [[0.12, 0.24, 0.45, 1.0], [0.16, 0.50, 0.73, 1.0]],
    [[0.99, 0.36, 0.32, 1.0], [0.96, 0.55, 0.80, 1.0]],
    [[0.06, 0.06, 0.08, 1.0], [0.27, 0.27, 0.33, 1.0]],
];

pub const BEAUTIFY_ASPECTS: [&str; 6] = ["auto", "1:1", "4:3", "3:2", "16:9", "9:16"];

pub fn css_rgba4(c: [f64; 4]) -> String {
    format!("rgba({},{},{},{})", (c[0] * 255.0).round(), (c[1] * 255.0).round(), (c[2] * 255.0).round(), c[3])
}

pub fn parse_aspect(s: &str) -> Option<f64> {
    let (a, b) = s.split_once(':')?;
    let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a > 0.0 && b > 0.0).then(|| a / b)
}

impl Beautify {
    pub fn shadow_inset(&self) -> f64 {
        if self.shadow_enabled {
            self.shadow_radius.max(0.0) + 6.0
        } else {
            0.0
        }
    }
    pub fn outer_inset(&self) -> f64 {
        self.padding.max(0.0) + self.shadow_inset()
    }
    /// (Ausgabebreite, Ausgabehöhe, Bild-x, Bild-y) für ein Bild der Größe w×h.
    pub fn layout(&self, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let inset = self.outer_inset();
        let (mut ow, mut oh) = ((w + 2.0 * inset).round(), (h + 2.0 * inset).round());
        if let Some(r) = parse_aspect(&self.aspect) {
            if ow / oh > r {
                oh = (ow / r).round();
            } else {
                ow = (oh * r).round();
            }
        }
        (ow, oh, ((ow - w) / 2.0).round(), ((oh - h) / 2.0).round())
    }
}

// ───────────────────────────── Stil-Presets & Werkzeug-Einstellungen ─────────────────────────────

/// Capso `AnnotationStylePreset`, Liste in Einstellung `annotationStylePresets`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StylePreset {
    pub id: String,
    pub tool: String,
    pub color: String,
    pub line_width: f64,
    pub filled: bool,
    pub pattern: StrokePattern,
    pub arrow_head: ArrowHead,
    pub text_fill: bool,
    pub text_outline: bool,
    pub text_stroke: bool,
}

impl Default for StylePreset {
    fn default() -> Self {
        StylePreset {
            id: String::new(),
            tool: "arrow".into(),
            color: "red".into(),
            line_width: 3.0,
            filled: false,
            pattern: StrokePattern::Solid,
            arrow_head: ArrowHead::Standard,
            text_fill: false,
            text_outline: false,
            text_stroke: true,
        }
    }
}

/// Dauerhafte Werkzeug-Einstellungen (Capso `@AppStorage("annotation…")`), Einstellung `annotationPrefs`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolPrefs {
    pub tool: Tool,
    pub color: String,
    pub filled: bool,
    pub shape_width: f64,
    pub block_size: f64,
    pub counter_size: f64,
    pub highlighter_width: f64,
    pub text_font_size: f64,
    pub redaction_mode: RedactionMode,
    pub pattern: StrokePattern,
    pub arrow_head: ArrowHead,
    pub text_fill: bool,
    pub text_outline: bool,
    pub text_stroke: bool,
}

impl Default for ToolPrefs {
    fn default() -> Self {
        ToolPrefs {
            tool: Tool::Arrow,
            color: "red".into(),
            filled: false,
            shape_width: 3.0,
            block_size: 12.0,
            counter_size: 20.0,
            highlighter_width: 20.0,
            text_font_size: 48.0,
            redaction_mode: RedactionMode::Pixelate,
            pattern: StrokePattern::Solid,
            arrow_head: ArrowHead::Standard,
            text_fill: false,
            text_outline: false,
            text_stroke: true,
        }
    }
}

impl ToolPrefs {
    /// Wert des Größenreglers für ein Werkzeug (Bedeutung je Werkzeug verschieden).
    pub fn width_for(&self, t: Tool) -> f64 {
        match t {
            Tool::Pixelate => self.block_size,
            Tool::Counter => self.counter_size,
            Tool::Highlighter => self.highlighter_width,
            Tool::Text => self.text_font_size,
            _ => self.shape_width,
        }
    }
    pub fn set_width_for(&mut self, t: Tool, v: f64) {
        match t {
            Tool::Pixelate => self.block_size = v,
            Tool::Counter => self.counter_size = v,
            Tool::Highlighter => self.highlighter_width = v,
            Tool::Text => self.text_font_size = v,
            _ => self.shape_width = v,
        }
    }
    /// (min, max, step) des Reglers.
    pub fn width_range(t: Tool) -> (f64, f64, f64) {
        match t {
            Tool::Text => (12.0, 120.0, 1.0),
            Tool::Pixelate => (4.0, 48.0, 2.0),
            Tool::Counter => (12.0, 40.0, 1.0),
            Tool::Highlighter => (10.0, 100.0, 2.0),
            _ => (1.0, 40.0, 1.0),
        }
    }
    pub fn style_for(&self, t: Tool) -> Style {
        Style {
            color: self.color.clone(),
            line_width: self.width_for(t),
            opacity: if t == Tool::Highlighter { 0.35 } else { 1.0 },
            filled: self.filled,
            pattern: self.pattern,
            arrow_head: self.arrow_head,
        }
    }
}

// ───────────────────────────── Tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Pt, b: Pt) -> bool {
        a.dist(b) < 1e-9
    }

    #[test]
    fn tool_shortcuts() {
        assert_eq!(Tool::for_key("a"), Some(Tool::Arrow));
        assert_eq!(Tool::for_key("O"), Some(Tool::Ellipse));
        assert_eq!(Tool::for_key("b"), Some(Tool::Pixelate));
        assert_eq!(Tool::for_key("n"), Some(Tool::Counter));
        assert_eq!(Tool::for_key("g"), Some(Tool::Spotlight));
        assert_eq!(Tool::for_key("x"), None);
        assert_eq!(Tool::for_key("ab"), None);
        assert_eq!(tool_for_key("c", true, "C"), None);
        assert_eq!(tool_for_key("c", false, "C"), Some(Tool::Counter));
        for t in Tool::ALL {
            assert_eq!(Tool::for_key(t.display_key()), Some(t));
            assert_eq!(Tool::from_id(t.id()), Some(t));
        }
    }

    #[test]
    fn escape_policy_order() {
        assert_eq!(escape_action(true, true, Tool::Arrow, true), EscapeAction::CommitText);
        assert_eq!(escape_action(false, true, Tool::Arrow, true), EscapeAction::ExitCrop);
        assert_eq!(escape_action(false, false, Tool::Arrow, true), EscapeAction::SwitchToSelect);
        assert_eq!(escape_action(false, false, Tool::Select, true), EscapeAction::ClearSelection);
        assert_eq!(escape_action(false, false, Tool::Select, false), EscapeAction::None);
    }

    #[test]
    fn drag_constraints() {
        let s = pt(10.0, 10.0);
        assert_eq!(constrained_drag(s, pt(40.0, 20.0), Tool::Rectangle, true, false).1, pt(40.0, 40.0));
        assert_eq!(constrained_drag(s, pt(-20.0, 15.0), Tool::Ellipse, true, false).1, pt(-20.0, 40.0));
        let e = constrained_drag(s, pt(110.0, 15.0), Tool::Arrow, true, false).1;
        assert!((e.y - 10.0).abs() < 1e-9 && (e.x - 10.0 - 100.0f64.hypot(5.0)).abs() < 1e-9);
        let e = angle_snapped_end(pt(0.0, 0.0), pt(10.0, 9.0));
        assert!((e.x - e.y).abs() < 1e-9);
        assert_eq!(constrained_drag(s, pt(40.0, 20.0), Tool::Freehand, true, false), (s, pt(40.0, 20.0)));
        assert_eq!(constrained_drag(s, pt(40.0, 20.0), Tool::Rectangle, false, false), (s, pt(40.0, 20.0)));
        let (a, b) = constrained_drag(s, pt(20.0, 15.0), Tool::Rectangle, true, true);
        assert_eq!((a, b), (pt(0.0, 0.0), pt(20.0, 20.0)));
    }

    fn obj(shape: Shape) -> Object {
        Object { id: 1, style: Style::default(), shape }
    }

    #[test]
    fn hit_tests() {
        let r = obj(Shape::Rect { rect: rect(10.0, 10.0, 100.0, 50.0), radius: 0.0 });
        assert!(r.hit_test(pt(10.0, 30.0), 4.0));
        assert!(!r.hit_test(pt(60.0, 35.0), 4.0), "Innenraum eines leeren Rechtecks trifft nicht");
        let mut f = r.clone();
        f.style.filled = true;
        assert!(f.hit_test(pt(60.0, 35.0), 4.0));
        let e = obj(Shape::Ellipse { rect: rect(0.0, 0.0, 100.0, 100.0) });
        assert!(e.hit_test(pt(50.0, 0.0), 4.0));
        assert!(!e.hit_test(pt(50.0, 50.0), 4.0));
        let a = obj(Shape::Arrow { start: pt(0.0, 0.0), end: pt(100.0, 0.0), ctrl: None });
        assert!(a.hit_test(pt(50.0, 4.0), 4.0));
        assert!(!a.hit_test(pt(50.0, 10.0), 4.0));
        let c = obj(Shape::Arrow { start: pt(0.0, 0.0), end: pt(100.0, 0.0), ctrl: Some(pt(50.0, 100.0)) });
        assert!(c.hit_test(pt(50.0, 50.0), 4.0));
        let ctr = obj(Shape::Counter { center: pt(50.0, 50.0), tip: Some(pt(150.0, 50.0)), radius: 20.0, number: 1 });
        assert!(ctr.hit_test(pt(60.0, 55.0), 2.0));
        assert!(ctr.hit_test(pt(120.0, 51.0), 2.0));
        let fh = obj(Shape::Freehand { points: vec![pt(0.0, 0.0), pt(10.0, 10.0), pt(20.0, 0.0)] });
        assert!(fh.hit_test(pt(5.0, 5.0), 2.0));
        assert!(!fh.hit_test(pt(10.0, 0.0), 2.0));
    }

    #[test]
    fn bounds_and_move() {
        let mut a = obj(Shape::Arrow { start: pt(0.0, 0.0), end: pt(100.0, 50.0), ctrl: None });
        assert_eq!(a.bounds(), rect(-15.0, -15.0, 130.0, 80.0));
        a.move_by(5.0, 5.0);
        assert_eq!(a.bounds(), rect(-10.0, -10.0, 130.0, 80.0));
        let ctr = obj(Shape::Counter { center: pt(50.0, 50.0), tip: Some(pt(60.0, 50.0)), radius: 20.0, number: 1 });
        assert_eq!(ctr.bounds(), rect(30.0, 30.0, 40.0, 40.0), "Spitze zu nah → kein Pfeil");
    }

    #[test]
    fn handle_resize() {
        let o = obj(Shape::Rect { rect: rect(10.0, 10.0, 100.0, 50.0), radius: 0.0 });
        let mut m = o.clone();
        assert_eq!(o.handle_at(pt(111.0, 61.0), 1.0), Some(Handle::BottomRight));
        m.drag_handle(&o, Handle::BottomRight, pt(210.0, 110.0), false);
        assert_eq!(m.bounds(), rect(10.0, 10.0, 200.0, 100.0));
        m.drag_handle(&o, Handle::TopLeft, pt(150.0, 100.0), false);
        assert_eq!(m.bounds(), rect(110.0, 60.0, 40.0, 40.0));
        let l = obj(Shape::Line { start: pt(0.0, 0.0), end: pt(100.0, 0.0), ctrl: None });
        let mut m = l.clone();
        m.drag_handle(&l, Handle::Control, pt(50.0, 50.0), false);
        if let Shape::Line { start, end, ctrl: Some(c) } = m.shape {
            assert!(close(quad_point(0.5, start, c, end), pt(50.0, 50.0)));
        } else {
            panic!()
        }
    }

    #[test]
    fn document_undo_redo_clipboard() {
        let mut d = Doc::new(800.0, 600.0);
        assert!(!d.has_unsaved_changes());
        let id = d.add(Style::default(), Shape::Rect { rect: rect(0.0, 0.0, 10.0, 10.0), radius: 0.0 });
        assert_eq!(d.selected, Some(id));
        d.add(Style::default(), Shape::Counter { center: pt(5.0, 5.0), tip: None, radius: 20.0, number: d.next_counter_number() });
        assert_eq!(d.next_counter_number(), 2);
        d.undo();
        assert_eq!(d.objects.len(), 1);
        assert!(d.can_redo());
        d.redo();
        assert_eq!(d.objects.len(), 2);
        d.selected = Some(id);
        assert!(d.copy_selection());
        assert!(d.paste(12.0));
        assert_eq!(d.objects.len(), 3);
        assert_eq!(d.objects[2].bounds(), rect(12.0, 12.0, 10.0, 10.0));
        assert_ne!(d.objects[2].id, id);
        assert!(d.duplicate_selected(12.0));
        assert_eq!(d.objects.len(), 4);
        assert!(d.remove_selected());
        assert_eq!(d.objects.len(), 3);
        d.push_undo();
        d.drop_undo_if_unchanged();
        d.undo();
        assert_eq!(d.objects.len(), 4);
        // Zuschneiden ist rückgängig machbar
        d.set_crop(Some(rect(1.0, 1.0, 5.0, 5.0)), Xform::default().rotate_ccw(), None);
        assert_eq!(d.image_size(), (600.0, 800.0));
        d.undo();
        assert_eq!(d.crop, None);
        assert_eq!(d.image_size(), (800.0, 600.0));
    }

    #[test]
    fn canvas_expansion() {
        let img = rect(0.0, 0.0, 100.0, 100.0);
        let inside = obj(Shape::Rect { rect: rect(10.0, 10.0, 20.0, 20.0), radius: 0.0 });
        assert_eq!(canvas_rect(img, &[inside]), img);
        let out = obj(Shape::Rect { rect: rect(90.0, 90.0, 20.5, 20.0), radius: 0.0 });
        assert_eq!(canvas_rect(img, &[out]), rect(0.0, 0.0, 123.0, 122.0));
    }

    #[test]
    fn transforms() {
        let (w, h) = (4.0, 2.0);
        let r = Xform::default().rotate_ccw();
        // Oben rechts → oben links, oben links → unten links
        assert!(close(r.apply(w, h, pt(4.0, 0.0)), pt(0.0, 0.0)));
        assert!(close(r.apply(w, h, pt(0.0, 0.0)), pt(0.0, 4.0)));
        let f = Xform::default().flip_h();
        assert!(close(f.apply(w, h, pt(0.0, 1.0)), pt(4.0, 1.0)));
        // Spiegeln nach Drehen = Drehen, dann Spiegeln (Arbeitsbild h×w = 2×4)
        let rf = r.flip_h();
        for p in [pt(0.0, 0.0), pt(3.0, 1.0), pt(4.0, 2.0)] {
            let q = r.apply(w, h, p);
            assert!(close(rf.apply(w, h, p), pt(2.0 - q.x, q.y)), "{p:?}");
        }
        // Zweimal drehen = 180°
        let r2 = r.rotate_ccw();
        assert!(close(r2.apply(w, h, pt(1.0, 0.0)), pt(3.0, 2.0)));
        assert_eq!(Xform::default().rotate_ccw().rotate_ccw().rotate_ccw().rotate_ccw(), Xform::default());
        assert_eq!(rect_rotate_ccw(rect(1.0, 0.0, 2.0, 1.0), 4.0), rect(0.0, 1.0, 1.0, 2.0));
        assert_eq!(rect_flip_h(rect(1.0, 0.0, 2.0, 1.0), 4.0), rect(1.0, 0.0, 2.0, 1.0));
    }

    #[test]
    fn crop_presets_and_snap() {
        assert_eq!(CropPreset::Freeform.ratio(100.0, 50.0), None);
        assert_eq!(CropPreset::Original.ratio(100.0, 50.0), Some(2.0));
        assert_eq!(CropPreset::R16x9.ratio(1.0, 1.0), Some(16.0 / 9.0));
        assert_eq!(snap_value(4.0, &[0.0, 100.0], 6.0), 0.0);
        assert_eq!(snap_value(10.0, &[0.0, 100.0], 6.0), 10.0);
        assert_eq!(snap_value(4.0, &[0.0], 0.0), 4.0);
        assert_eq!(snap_rect(rect(3.0, 50.0, 95.0, 48.0), 100.0, 100.0, 6.0), rect(0.0, 50.0, 100.0, 50.0));
    }

    #[test]
    fn crop_creation() {
        let (w, h) = (1000.0, 800.0);
        assert_eq!(crop_creation_rect(pt(10.0, 20.0), pt(110.0, 70.0), None, w, h), rect(10.0, 20.0, 100.0, 50.0));
        assert_eq!(crop_creation_rect(pt(110.0, 70.0), pt(10.0, 20.0), None, w, h), rect(10.0, 20.0, 100.0, 50.0));
        assert_eq!(crop_creation_rect(pt(0.0, 0.0), pt(200.0, 5.0), Some(2.0), w, h), rect(0.0, 0.0, 200.0, 100.0));
        assert_eq!(crop_creation_rect(pt(0.0, 300.0), pt(200.0, 295.0), Some(2.0), w, h), rect(0.0, 200.0, 200.0, 100.0));
        assert_eq!(crop_creation_rect(pt(100.0, 700.0), pt(400.0, 750.0), Some(1.0), w, h), rect(100.0, 700.0, 100.0, 100.0));
        assert_eq!(crop_creation_rect(pt(900.0, 700.0), pt(1200.0, 900.0), None, w, h), rect(900.0, 700.0, 100.0, 100.0));
    }

    #[test]
    fn crop_resizing() {
        let r = rect(100.0, 100.0, 200.0, 100.0);
        assert_eq!(crop_resize(r, CropHandle::BottomRight, pt(50.0, 20.0), None, 1000.0, 800.0, 0.0), rect(100.0, 100.0, 250.0, 120.0));
        assert_eq!(crop_resize(r, CropHandle::Left, pt(-97.0, 0.0), None, 1000.0, 800.0, 6.0), rect(0.0, 100.0, 300.0, 100.0));
        assert_eq!(crop_resize(r, CropHandle::Move, pt(-500.0, 0.0), None, 1000.0, 800.0, 0.0), rect(0.0, 100.0, 200.0, 100.0));
        let q = crop_resize(r, CropHandle::BottomRight, pt(100.0, 0.0), Some(1.0), 1000.0, 800.0, 0.0);
        assert_eq!(q, rect(100.0, 100.0, 100.0, 100.0));
        let q = crop_resize(r, CropHandle::Right, pt(100.0, 0.0), Some(2.0), 1000.0, 800.0, 0.0);
        assert_eq!(q, rect(100.0, 75.0, 300.0, 150.0));
        // Seitenverhältnis bleibt auch an der Bildgrenze erhalten
        let q = crop_resize(rect(0.0, 0.0, 100.0, 100.0), CropHandle::BottomRight, pt(900.0, 900.0), Some(1.0), 1000.0, 800.0, 0.0);
        assert_eq!(q, rect(0.0, 0.0, 800.0, 800.0));
        assert_eq!(fit_ratio(rect(0.0, 0.0, 200.0, 100.0), 1.0), rect(50.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn beautify_layout_and_json() {
        let b = Beautify::default();
        assert_eq!(b.outer_inset(), 40.0 + 26.0);
        assert_eq!(b.layout(100.0, 50.0), (232.0, 182.0, 66.0, 66.0));
        let sq = Beautify { aspect: "1:1".into(), shadow_enabled: false, padding: 10.0, ..Default::default() };
        assert_eq!(sq.layout(100.0, 50.0), (120.0, 120.0, 10.0, 35.0));
        let v: Beautify = serde_json::from_str(r#"{"backgroundStyle":"liquidGlass","backgroundColor":[0,0,0,1],"padding":10,"cornerRadius":4,"shadowEnabled":false,"shadowRadius":5}"#).unwrap();
        assert_eq!(v.background_style, "liquidGlass");
        assert_eq!(v.aspect, "auto");
        let j = serde_json::to_value(&b).unwrap();
        assert!(j.get("enabled").is_none() && j["shadowRadius"] == 20.0);
    }

    #[test]
    fn colors() {
        assert_eq!(color_hex("red"), "#FF3B30");
        assert_eq!(color_rgb("#00ff00"), (0, 255, 0));
        assert_eq!(color_rgb("bogus"), (0, 0, 0));
    }

    #[test]
    fn prefs_widths() {
        let mut p = ToolPrefs::default();
        assert_eq!(p.width_for(Tool::Text), 48.0);
        p.set_width_for(Tool::Counter, 30.0);
        assert_eq!(p.width_for(Tool::Counter), 30.0);
        assert_eq!(p.width_for(Tool::Arrow), 3.0);
        assert_eq!(p.style_for(Tool::Highlighter).opacity, 0.35);
    }

    #[test]
    fn text_box_resize() {
        let b = rect(10.0, 10.0, 200.0, 40.0);
        let min = text_box_min(20.0);
        // rechts unten: breiter, Mindesthöhe
        assert_eq!(text_resize_rect(b, Handle::BottomRight, pt(50.0, -30.0), min), rect(10.0, 10.0, 250.0, 32.0));
        // links oben: rechte/untere Kante bleibt, Mindestbreite 60
        assert_eq!(text_resize_rect(b, Handle::TopLeft, pt(190.0, 0.0), min), rect(150.0, 10.0, 60.0, 40.0));
        assert_eq!(text_resize_font_size(b, 20.0, Handle::BottomRight, pt(200.0, 0.0)), 40.0);
        assert_eq!(text_resize_origin(b, (100.0, 20.0), Handle::TopLeft), pt(110.0, 30.0));
    }

    #[test]
    fn boxed_text_corner_drag_reflows() {
        let shape = Shape::Text { text: "a b c".into(), origin: pt(10.0, 10.0), font_size: 20.0, w: 200.0, h: 40.0, boxed: true, fill: None, outline: None, trace: None };
        let o = Object { id: 1, style: Style::default(), shape };
        let mut m = o.clone();
        m.drag_handle(&o, Handle::BottomRight, pt(110.0, 50.0), false);
        let Shape::Text { w, h, font_size, origin, .. } = m.shape else { panic!() };
        assert_eq!((w, h, font_size, origin), (100.0, 40.0, 20.0, pt(10.0, 10.0)));
    }

    #[test]
    fn counter_buttons_renumber() {
        let mut d = Doc::new(100.0, 100.0);
        let id = d.add(Style::default(), Shape::Counter { center: pt(50.0, 50.0), tip: None, radius: 10.0, number: 1 });
        assert!(!d.counter_action(id, CounterAction::Decrement));
        assert!(!d.can_undo() || d.objects.len() == 1);
        assert!(d.counter_action(id, CounterAction::Increment));
        assert!(matches!(d.get(id).unwrap().shape, Shape::Counter { number: 2, .. }));
        d.undo();
        assert!(matches!(d.get(id).unwrap().shape, Shape::Counter { number: 1, .. }));
        let o = d.get(id).unwrap().clone();
        let btns = counter_buttons(&o, 1.0);
        let inc = btns.iter().find(|b| b.0 == CounterAction::Increment).unwrap().1;
        assert_eq!(counter_action_at(&o, inc.center(), 1.0), Some(CounterAction::Increment));
        assert!(d.counter_action(id, CounterAction::Delete));
        assert!(d.objects.is_empty());
    }

    #[test]
    fn highlighter_snaps_to_line() {
        let lines = [rect(0.0, 100.0, 300.0, 20.0)];
        assert_eq!(highlight_snap_line(&lines, pt(10.0, 95.0)), Some(lines[0]));
        assert_eq!(highlight_snap_line(&lines, pt(10.0, 90.0)), None);
    }

    #[test]
    fn crop_preset_keys() {
        assert_eq!(CropPreset::Freeform.cycle(true), CropPreset::Original);
        assert_eq!(CropPreset::Freeform.cycle(false), CropPreset::R16x9);
        assert_eq!(CropPreset::from_digit("3"), Some(CropPreset::Square));
        assert_eq!(CropPreset::from_digit("7"), None);
        assert_eq!(CropPreset::from_id(CropPreset::R3x2.id()), CropPreset::R3x2);
        assert_eq!(CropPreset::from_id("x"), CropPreset::Freeform);
    }
}
