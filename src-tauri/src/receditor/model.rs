//! Reine Logik des Aufnahme-Editors – Port von Capso `EditorKit`
//! (RecordingProject, TrimRegion, ZoomSegment, RecordingEffectSegment, FrameTransform, SpringSimulator,
//! ZoomInterpolator, CursorSmoother, CursorFollowPath, CursorOverlayProvider, AutoZoomDetector, BackgroundStyle)
//! plus das gemeinsame Layout der Komposition.
//!
//! Diese Datei hängt nur von `std`, `serde` und `serde_json` ab und wird von Backend (Export) UND
//! Frontend (Vorschau, per `#[path]`) kompiliert – beide rendern so garantiert mit denselben Zahlen.
#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

// ───────────────────────────── IDs ─────────────────────────────

static ID_SEED: AtomicU64 = AtomicU64::new(0);
static ID_N: AtomicU64 = AtomicU64::new(0);

/// Startwert für IDs (z. B. aktuelle Millisekunden), damit neue IDs nicht mit geladenen kollidieren.
pub fn seed_ids(seed: u64) {
    ID_SEED.store(seed, Ordering::Relaxed);
}

pub fn new_id() -> String {
    format!("{:x}-{:x}", ID_SEED.load(Ordering::Relaxed), ID_N.fetch_add(1, Ordering::Relaxed))
}

fn smoothstep(t: f64) -> f64 {
    let c = t.clamp(0.0, 1.0);
    c * c * (3.0 - 2.0 * c)
}

fn ease_in_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

// ───────────────────────────── FrameTransform ─────────────────────────────

/// Skalierung + Fokuspunkt. `translate_x/y` tragen bei aktivem Zoom den normierten Fokus (0–1, oben links).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameTransform {
    pub scale: f64,
    pub translate_x: f64,
    pub translate_y: f64,
}

impl FrameTransform {
    pub const IDENTITY: FrameTransform = FrameTransform { scale: 1.0, translate_x: 0.0, translate_y: 0.0 };
    pub fn new(scale: f64, translate_x: f64, translate_y: f64) -> Self {
        Self { scale, translate_x, translate_y }
    }
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
    /// Fokus (normiert); bei Identität die Bildmitte.
    pub fn focus(&self) -> (f64, f64) {
        if self.is_identity() {
            (0.5, 0.5)
        } else {
            (self.translate_x, self.translate_y)
        }
    }
}

impl Default for FrameTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

// ───────────────────────────── SpringSimulator ─────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct StepResult {
    pub position: f64,
    pub velocity: f64,
}

/// Analytischer gedämpfter harmonischer Oszillator (unter-, kritisch und überdämpft).
#[derive(Clone, Copy, Debug)]
pub struct SpringSimulator {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
    pub position: f64,
    pub velocity: f64,
}

impl SpringSimulator {
    pub fn new(stiffness: f64, damping: f64, mass: f64) -> Self {
        Self { stiffness, damping, mass, position: 0.0, velocity: 0.0 }
    }

    pub fn reset(&mut self, position: f64, velocity: f64) {
        self.position = position;
        self.velocity = velocity;
    }

    /// `dt` Sekunden Richtung `target` fortschreiten (exakte Lösung, auch für große Schritte stabil).
    pub fn step(&mut self, target: f64, dt: f64) -> StepResult {
        let displacement = self.position - target;
        let velocity = self.velocity;
        if displacement.abs() < 1e-10 && velocity.abs() < 1e-10 {
            self.position = target;
            self.velocity = 0.0;
            return StepResult { position: target, velocity: 0.0 };
        }
        let omega0 = (self.stiffness / self.mass).sqrt();
        let zeta = self.damping / (2.0 * (self.stiffness * self.mass).sqrt());
        let (np, nv);
        if (zeta - 1.0).abs() < 1e-6 {
            let e = (-omega0 * dt).exp();
            let a = displacement;
            let b = velocity + omega0 * displacement;
            np = target + e * (a + b * dt);
            nv = e * (b - omega0 * (a + b * dt));
        } else if zeta < 1.0 {
            let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
            let e = (-zeta * omega0 * dt).exp();
            let a = displacement;
            let b = (velocity + zeta * omega0 * displacement) / omega_d;
            let (s, c) = (omega_d * dt).sin_cos();
            np = target + e * (a * c + b * s);
            nv = e * (velocity * c - (velocity * zeta * omega0 + displacement * omega0 * omega0) / omega_d * s);
        } else {
            let sq = (zeta * zeta - 1.0).sqrt();
            let s1 = -omega0 * (zeta + sq);
            let s2 = -omega0 * (zeta - sq);
            let a = (velocity - s2 * displacement) / (s1 - s2);
            let b = displacement - a;
            let e1 = (s1 * dt).exp();
            let e2 = (s2 * dt).exp();
            np = target + a * e1 + b * e2;
            nv = a * s1 * e1 + b * s2 * e2;
        }
        self.position = np;
        self.velocity = nv;
        StepResult { position: np, velocity: nv }
    }
}

/// Zwei unabhängige Federn für X und Y.
#[derive(Clone, Copy, Debug)]
pub struct Spring2D {
    sx: SpringSimulator,
    sy: SpringSimulator,
}

impl Spring2D {
    pub fn new(stiffness: f64, damping: f64, mass: f64) -> Self {
        Self { sx: SpringSimulator::new(stiffness, damping, mass), sy: SpringSimulator::new(stiffness, damping, mass) }
    }
    pub fn x(&self) -> f64 {
        self.sx.position
    }
    pub fn y(&self) -> f64 {
        self.sy.position
    }
    pub fn reset(&mut self, x: f64, y: f64) {
        self.sx.reset(x, 0.0);
        self.sy.reset(y, 0.0);
    }
    pub fn step(&mut self, tx: f64, ty: f64, dt: f64) -> (f64, f64) {
        (self.sx.step(tx, dt).position, self.sy.step(ty, dt).position)
    }
}

// ───────────────────────────── Cursor-Telemetrie ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CursorEventType {
    Move,
    LeftClick,
    RightClick,
    KeyDown,
    MouseUp,
}

/// Ein Cursor-Ereignis; Koordinaten normiert auf das Video (0–1, Ursprung oben links).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CursorEvent {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    #[serde(rename = "type")]
    pub kind: CursorEventType,
}

impl CursorEvent {
    pub fn new(t: f64, x: f64, y: f64, kind: CursorEventType) -> Self {
        Self { t, x, y, kind }
    }
    fn is_click(&self) -> bool {
        matches!(self.kind, CursorEventType::LeftClick | CursorEventType::RightClick)
    }
}

/// Inhalt von `<video>.cursor.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub events: Vec<CursorEvent>,
    /// Größe des Aufnahmebereichs in Punkten (falls bekannt).
    pub area_width: Option<f64>,
    pub area_height: Option<f64>,
    /// Einstellung „Mauszeiger anzeigen“ zum Aufnahmezeitpunkt (Capso `HistoryRecordingInfo.showsCursor`);
    /// steuert, ob der Editor den Cursor aus der Telemetrie zeichnet.
    #[serde(default)]
    pub user_shows_cursor: Option<bool>,
    /// Aufnahmeformat ("mp4" | "gif") → Standard-Ausgabeformat des Editors (Capso `EditorOutputFormat`).
    #[serde(default)]
    pub output_format: Option<String>,
}

fn num(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_f64)).filter(|f| f.is_finite())
}

/// Robustes Einlesen der Cursor-Datei. Unterstützt
/// `{"samples":[{t,x,y}], "clicks":[{t,x,y,button?}], "ups"|"releases":[{t}], "keys":[{t,x?,y?}]}`
/// sowie Capso-Format `{"events":[{timestamp,x,y,type}], recordingAreaWidth, recordingAreaHeight}`.
pub fn parse_telemetry(v: &Value) -> Telemetry {
    let mut events = Vec::new();
    let mut push = |item: &Value, kind: CursorEventType| {
        let Some(t) = num(item, &["t", "timestamp", "time"]) else { return };
        let x = num(item, &["x"]).unwrap_or(f64::NAN);
        let y = num(item, &["y"]).unwrap_or(f64::NAN);
        events.push(CursorEvent::new(t, x, y, kind));
    };
    let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    for s in list("samples").iter().chain(list("moves").iter()) {
        push(s, CursorEventType::Move);
    }
    for c in list("clicks") {
        let right = c.get("button").and_then(Value::as_str).map(|b| b.starts_with('r')).unwrap_or(false)
            || c.get("right").and_then(Value::as_bool).unwrap_or(false);
        push(&c, if right { CursorEventType::RightClick } else { CursorEventType::LeftClick });
    }
    for u in list("ups").iter().chain(list("releases").iter()).chain(list("mouseUps").iter()) {
        push(u, CursorEventType::MouseUp);
    }
    for k in list("keys") {
        push(&k, CursorEventType::KeyDown);
    }
    for e in list("events") {
        let kind = match e.get("type").and_then(Value::as_str).unwrap_or("move") {
            "leftClick" | "click" | "down" => CursorEventType::LeftClick,
            "rightClick" => CursorEventType::RightClick,
            "keyDown" | "key" => CursorEventType::KeyDown,
            "mouseUp" | "up" => CursorEventType::MouseUp,
            _ => CursorEventType::Move,
        };
        push(&e, kind);
    }
    events.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    // Ereignisse ohne Position (z. B. Tasten, Loslassen) übernehmen die letzte bekannte Position.
    let (mut lx, mut ly) = (0.5, 0.5);
    if let Some(first) = events.iter().find(|e| e.x.is_finite() && e.y.is_finite()) {
        lx = first.x;
        ly = first.y;
    }
    for e in &mut events {
        if e.x.is_finite() && e.y.is_finite() {
            lx = e.x.clamp(-0.5, 1.5);
            ly = e.y.clamp(-0.5, 1.5);
        }
        e.x = lx;
        e.y = ly;
    }
    Telemetry {
        events,
        area_width: num(v, &["recordingAreaWidth", "areaWidth", "width"]),
        area_height: num(v, &["recordingAreaHeight", "areaHeight", "height"]),
        user_shows_cursor: v.get("userShowsCursor").and_then(Value::as_bool),
        output_format: v.get("outputFormat").and_then(Value::as_str).map(str::to_owned),
    }
}

// ───────────────────────────── Projektmodell ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SegmentSource {
    #[default]
    Manual,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ZoomFocusMode {
    #[default]
    FollowCursor,
    Manual { x: f64, y: f64 },
}

/// Herausgeschnittener Bereich der Zeitleiste (Quellzeit in Sekunden).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrimRegion {
    #[serde(default = "new_id")]
    pub id: String,
    pub start_time: f64,
    pub end_time: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}

impl TimeRange {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

impl TrimRegion {
    pub fn new(start_time: f64, end_time: f64) -> Self {
        Self { id: new_id(), start_time, end_time }
    }
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }

    /// Verbleibende Bereiche nach dem Entfernen aller Schnitte (zusammengeführt, sortiert).
    pub fn kept_ranges(duration: f64, regions: &[TrimRegion]) -> Vec<TimeRange> {
        if duration <= 0.0 {
            return vec![];
        }
        let mut cuts: Vec<TimeRange> = regions
            .iter()
            .map(|r| TimeRange { start: r.start_time.clamp(0.0, duration), end: r.end_time.clamp(0.0, duration) })
            .filter(|r| r.end > r.start)
            .collect();
        if cuts.is_empty() {
            return vec![TimeRange { start: 0.0, end: duration }];
        }
        cuts.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        let mut merged: Vec<TimeRange> = vec![];
        for c in cuts {
            match merged.last_mut() {
                Some(last) if c.start <= last.end => last.end = last.end.max(c.end),
                _ => merged.push(c),
            }
        }
        let mut kept = vec![];
        let mut cursor = 0.0;
        for c in merged {
            if c.start > cursor {
                kept.push(TimeRange { start: cursor, end: c.start });
            }
            cursor = f64::max(cursor, c.end);
        }
        if cursor < duration {
            kept.push(TimeRange { start: cursor, end: duration });
        }
        kept
    }
}

/// Ausgabezeit (nach Schnitten) → Quellzeit.
pub fn effective_to_source(kept: &[TimeRange], t: f64) -> f64 {
    let mut acc = 0.0;
    for r in kept {
        if t < acc + r.duration() {
            return r.start + (t - acc).max(0.0);
        }
        acc += r.duration();
    }
    kept.last().map(|r| r.end).unwrap_or(t)
}

/// Quellzeit → Ausgabezeit (Zeit in Schnitten wird auf den Schnittanfang gelegt).
pub fn source_to_effective(kept: &[TimeRange], t: f64) -> f64 {
    let mut acc = 0.0;
    for r in kept {
        if t < r.start {
            return acc;
        }
        if t < r.end {
            return acc + t - r.start;
        }
        acc += r.duration();
    }
    acc
}

/// Liegt `time` in einem Schnitt, wird zum Schnittende gesprungen (EditorCoordinator.skipTrimRegions).
pub fn skip_trim_regions(regions: &[TrimRegion], time: f64, duration: f64) -> f64 {
    let mut sorted: Vec<&TrimRegion> = regions.iter().collect();
    sorted.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());
    let mut result = time;
    for r in sorted {
        if result >= r.start_time && result < r.end_time {
            result = r.end_time;
        }
    }
    result.min(duration)
}

/// Zoom-Segment (abgeleitete Sicht auf ein `RecordingEffectSegment` vom Typ Zoom).
#[derive(Clone, Debug, PartialEq)]
pub struct ZoomSegment {
    pub id: String,
    pub start_time: f64,
    pub end_time: f64,
    pub zoom_level: f64,
    pub focus_mode: ZoomFocusMode,
    pub source: SegmentSource,
}

impl ZoomSegment {
    pub fn new(start_time: f64, end_time: f64, zoom_level: f64, focus_mode: ZoomFocusMode) -> Self {
        Self { id: new_id(), start_time, end_time, zoom_level, focus_mode, source: SegmentSource::Manual }
    }
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NormalizedRect {
    pub const CENTERED_REDACTION: NormalizedRect = NormalizedRect { x: 0.2, y: 0.35, width: 0.6, height: 0.3 };
    pub fn clamped(&self, min_size: f64) -> NormalizedRect {
        let m = min_size.clamp(0.0, 1.0);
        let w = self.width.clamp(m, 1.0);
        let h = self.height.clamp(m, 1.0);
        NormalizedRect { x: self.x.clamp(0.0, 1.0 - w), y: self.y.clamp(0.0, 1.0 - h), width: w, height: h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomEffectPayload {
    pub zoom_level: f64,
    pub focus_mode: ZoomFocusMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurEffectPayload {
    pub rect: NormalizedRect,
    pub radius: f64,
}

impl Default for BlurEffectPayload {
    fn default() -> Self {
        Self { rect: NormalizedRect::CENTERED_REDACTION, radius: 18.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum EffectPayload {
    Zoom(ZoomEffectPayload),
    Blur(BlurEffectPayload),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEffectSegment {
    #[serde(default = "new_id")]
    pub id: String,
    pub start_time: f64,
    pub end_time: f64,
    #[serde(default)]
    pub source: SegmentSource,
    pub payload: EffectPayload,
}

impl RecordingEffectSegment {
    pub fn zoom(z: &ZoomSegment) -> Self {
        Self {
            id: z.id.clone(),
            start_time: z.start_time,
            end_time: z.end_time,
            source: z.source,
            payload: EffectPayload::Zoom(ZoomEffectPayload { zoom_level: z.zoom_level, focus_mode: z.focus_mode }),
        }
    }
    pub fn blur(start_time: f64, end_time: f64) -> Self {
        Self { id: new_id(), start_time, end_time, source: SegmentSource::Manual, payload: EffectPayload::Blur(BlurEffectPayload::default()) }
    }
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }
    pub fn is_zoom(&self) -> bool {
        matches!(self.payload, EffectPayload::Zoom(_))
    }
    pub fn contains(&self, t: f64) -> bool {
        t >= self.start_time && t <= self.end_time
    }
    pub fn zoom_segment(&self) -> Option<ZoomSegment> {
        match self.payload {
            EffectPayload::Zoom(z) => Some(ZoomSegment {
                id: self.id.clone(),
                start_time: self.start_time,
                end_time: self.end_time,
                zoom_level: z.zoom_level,
                focus_mode: z.focus_mode,
                source: self.source,
            }),
            _ => None,
        }
    }
}

pub fn zoom_segments(effects: &[RecordingEffectSegment]) -> Vec<ZoomSegment> {
    let mut v: Vec<ZoomSegment> = effects.iter().filter_map(|e| e.zoom_segment()).collect();
    v.sort_by(|a, b| (a.start_time, a.end_time).partial_cmp(&(b.start_time, b.end_time)).unwrap());
    v
}

/// Ersetzt alle Zoom-Effekte (z. B. durch Auto-Zoom-Vorschläge) und sortiert.
pub fn replace_zoom_segments(effects: &mut Vec<RecordingEffectSegment>, zooms: &[ZoomSegment]) {
    effects.retain(|e| !e.is_zoom());
    effects.extend(zooms.iter().map(RecordingEffectSegment::zoom));
    effects.sort_by(|a, b| (a.start_time, a.end_time).partial_cmp(&(b.start_time, b.end_time)).unwrap());
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CodableColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    #[serde(default = "one")]
    pub alpha: f64,
}

fn one() -> f64 {
    1.0
}

impl CodableColor {
    pub const fn rgb(red: f64, green: f64, blue: f64) -> Self {
        Self { red, green, blue, alpha: 1.0 }
    }
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const DARK_GRAY: Self = Self::rgb(0.2, 0.2, 0.2);
    pub const INK: Self = Self::rgb(0.102, 0.094, 0.078);
    pub const STONE: Self = Self::rgb(0.239, 0.216, 0.192);
    pub const MIST: Self = Self::rgb(0.910, 0.898, 0.875);
    pub const SAND: Self = Self::rgb(0.827, 0.796, 0.745);
    pub const DUSK: Self = Self::rgb(0.290, 0.341, 0.459);
    pub const SAGE: Self = Self::rgb(0.608, 0.659, 0.569);
    pub const CLAY: Self = Self::rgb(0.710, 0.514, 0.431);
    /// Die 7 Capso-Vorgaben (Ink, Stone, Mist, Sand, Dusk, Sage, Clay).
    pub const PRESETS: [Self; 7] = [Self::INK, Self::STONE, Self::MIST, Self::SAND, Self::DUSK, Self::SAGE, Self::CLAY];

    pub fn to_hex(&self) -> String {
        let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", c(self.red), c(self.green), c(self.blue))
    }
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
        Some(Self::rgb(p(0)?, p(2)?, p(4)?))
    }
    pub fn css(&self) -> String {
        format!(
            "rgba({},{},{},{})",
            (self.red * 255.0).round(),
            (self.green * 255.0).round(),
            (self.blue * 255.0).round(),
            self.alpha
        )
    }
    pub fn rgba8(&self) -> [u8; 4] {
        let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [c(self.red), c(self.green), c(self.blue), c(self.alpha)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundColorType {
    #[default]
    Solid,
    Gradient,
    /// Weichgezeichnete, gesättigte Kopie des Videobilds.
    LiquidGlass,
    /// Eigenes Bild (nrzCapture-Erweiterung).
    Image,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackgroundStyle {
    pub enabled: bool,
    pub color_type: BackgroundColorType,
    pub solid_color: CodableColor,
    pub gradient_from: CodableColor,
    pub gradient_to: CodableColor,
    /// Grad: 0 = links→rechts, im Uhrzeigersinn (90 = oben→unten).
    pub gradient_angle: f64,
    pub image_path: Option<String>,
    /// Innenabstand in Quellpixeln (0–80).
    pub padding: f64,
    /// Eckenradius in Quellpixeln (0–120).
    pub corner_radius: f64,
    pub shadow_enabled: bool,
    pub shadow_radius: f64,
    pub shadow_opacity: f64,
}

impl BackgroundStyle {
    pub const MAX_CORNER_RADIUS: f64 = 120.0;

    pub fn clamped_corner_radius(&self, w: f64, h: f64) -> f64 {
        let cap = w.min(h) / 2.0;
        self.corner_radius.max(0.0).min(Self::MAX_CORNER_RADIUS.min(cap))
    }

    /// Verlaufs-Start/-Ende im Einheitsquadrat (y nach unten).
    pub fn gradient_unit_points(&self) -> ((f64, f64), (f64, f64)) {
        let r = self.gradient_angle.to_radians();
        let (dx, dy) = (r.cos() * 0.5, r.sin() * 0.5);
        ((0.5 - dx, 0.5 - dy), (0.5 + dx, 0.5 + dy))
    }
}

impl Default for BackgroundStyle {
    fn default() -> Self {
        Self {
            enabled: false,
            color_type: BackgroundColorType::Solid,
            solid_color: CodableColor::DARK_GRAY,
            gradient_from: CodableColor::BLACK,
            gradient_to: CodableColor::DARK_GRAY,
            gradient_angle: 135.0,
            image_path: None,
            padding: 20.0,
            corner_radius: 12.0,
            shadow_enabled: true,
            shadow_radius: 15.0,
            shadow_opacity: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CursorSmoothingPreset {
    Snappy,
    Smooth,
    Floaty,
}

impl CursorSmoothingPreset {
    pub fn config(self) -> CursorSmoothingConfig {
        match self {
            Self::Snappy => CursorSmoothingConfig { enabled: true, stiffness: 1500.0, damping: 77.0, mass: 1.0 },
            Self::Smooth => CursorSmoothingConfig::default(),
            Self::Floaty => CursorSmoothingConfig { enabled: true, stiffness: 50.0, damping: 10.0, mass: 2.0 },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CursorSmoothingConfig {
    pub enabled: bool,
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
}

impl Default for CursorSmoothingConfig {
    fn default() -> Self {
        Self { enabled: true, stiffness: 800.0, damping: 56.0, mass: 1.0 }
    }
}

impl CursorSmoothingConfig {
    pub fn preset(&self) -> CursorSmoothingPreset {
        for p in [CursorSmoothingPreset::Snappy, CursorSmoothingPreset::Floaty] {
            let c = p.config();
            if c.stiffness == self.stiffness && c.damping == self.damping && c.mass == self.mass {
                return p;
            }
        }
        CursorSmoothingPreset::Smooth
    }
}

/// Gesamter Editor-Zustand eines Videos; als `<video>.project.json` neben dem Video gespeichert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecordingProject {
    pub id: String,
    pub source_video: String,
    pub cursor_telemetry: Option<String>,
    pub shows_cursor: bool,
    pub video_duration: f64,
    pub video_width: f64,
    pub video_height: f64,
    pub recording_area_width: f64,
    pub recording_area_height: f64,
    pub trim_regions: Vec<TrimRegion>,
    pub effect_segments: Vec<RecordingEffectSegment>,
    pub background_style: BackgroundStyle,
    pub cursor_smoothing: CursorSmoothingConfig,
    pub hides_idle_cursor: bool,
    /// Cursor-Größe als Faktor (nrzCapture-Erweiterung, 0.5–3).
    pub cursor_scale: f64,
    /// "auto" | "16:9" | "9:16" | "1:1" | "4:3" | "3:4" (nrzCapture-Erweiterung).
    pub aspect_ratio: String,
    pub created_at: String,
}

impl Default for RecordingProject {
    fn default() -> Self {
        Self {
            id: new_id(),
            source_video: String::new(),
            cursor_telemetry: None,
            shows_cursor: true,
            video_duration: 0.0,
            video_width: 1920.0,
            video_height: 1080.0,
            recording_area_width: 1920.0,
            recording_area_height: 1080.0,
            trim_regions: vec![],
            effect_segments: vec![],
            background_style: BackgroundStyle::default(),
            cursor_smoothing: CursorSmoothingConfig::default(),
            hides_idle_cursor: true,
            cursor_scale: 1.0,
            aspect_ratio: "auto".into(),
            created_at: String::new(),
        }
    }
}

impl RecordingProject {
    /// Dauer nach Abzug aller Schnitte (nie negativ).
    pub fn effective_duration(&self) -> f64 {
        TrimRegion::kept_ranges(self.video_duration, &self.trim_regions).iter().map(TimeRange::duration).sum()
    }
    pub fn kept_ranges(&self) -> Vec<TimeRange> {
        TrimRegion::kept_ranges(self.video_duration, &self.trim_regions)
    }
    pub fn zoom_segments(&self) -> Vec<ZoomSegment> {
        zoom_segments(&self.effect_segments)
    }
    pub fn effective_start(&self) -> f64 {
        self.trim_regions.iter().filter(|r| r.start_time < 0.01).map(|r| r.end_time).fold(0.0, f64::max)
    }
    pub fn effective_end(&self) -> f64 {
        let d = self.video_duration;
        self.trim_regions.iter().filter(|r| r.end_time >= d - 0.01).map(|r| r.start_time).fold(d, f64::min)
    }
    /// Schnitte innerhalb der Zeitleiste (ohne Anfangs-/End-Trim).
    pub fn timeline_cuts(&self) -> Vec<TrimRegion> {
        let d = self.video_duration;
        let mut v: Vec<TrimRegion> = self.trim_regions.iter().filter(|r| r.start_time > 0.01 && r.end_time < d - 0.01).cloned().collect();
        v.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());
        v
    }
    pub fn set_head_trim(&mut self, time: f64) {
        self.trim_regions.retain(|r| r.start_time >= 0.01);
        if time > 0.01 {
            self.trim_regions.push(TrimRegion::new(0.0, time.min(self.video_duration)));
        }
    }
    pub fn set_tail_trim(&mut self, time: f64) {
        let d = self.video_duration;
        self.trim_regions.retain(|r| r.end_time < d - 0.01);
        if time < d - 0.01 {
            self.trim_regions.push(TrimRegion::new(time.max(0.0), d));
        }
    }
    /// Effekte, Hintergrund oder Cursor müssen gerendert werden (sonst reicht der schnelle Schnitt-Export).
    pub fn needs_compositor(&self, has_telemetry: bool) -> bool {
        self.background_style.enabled || !self.effect_segments.is_empty() || (self.shows_cursor && has_telemetry) || self.aspect_ratio != "auto"
    }
    /// Cursor-Höhe in Quellpixeln (EditorCoordinator.cursorTargetHeight × Größenfaktor).
    pub fn cursor_height(&self) -> f64 {
        let display_scale = if self.recording_area_height > 0.0 { (self.video_height / self.recording_area_height).max(1.0) } else { 1.0 };
        let from_scale = 18.0 * display_scale;
        let from_video = self.video_height * 0.038;
        from_scale.max(from_video).min(48.0) * self.cursor_scale.clamp(0.25, 4.0)
    }
}

// ───────────────────────────── CursorSmoother ─────────────────────────────

/// Vorausberechnete, federgeglättete Cursor-Positionen in festen Abständen.
#[derive(Clone, Debug)]
pub struct SmoothedCursorTimeline {
    xs: Vec<f64>,
    ys: Vec<f64>,
    pub sample_interval: f64,
    pub duration: f64,
}

impl SmoothedCursorTimeline {
    pub fn sample_count(&self) -> usize {
        self.xs.len()
    }
    pub fn position(&self, time: f64) -> (f64, f64) {
        let n = self.xs.len();
        if n == 0 {
            return (0.0, 0.0);
        }
        if n == 1 {
            return (self.xs[0], self.ys[0]);
        }
        let exact = time.clamp(0.0, self.duration) / self.sample_interval;
        let lower = exact as usize;
        if lower >= n - 1 {
            return (self.xs[n - 1], self.ys[n - 1]);
        }
        let f = exact - lower as f64;
        (self.xs[lower] + f * (self.xs[lower + 1] - self.xs[lower]), self.ys[lower] + f * (self.ys[lower + 1] - self.ys[lower]))
    }
}

pub struct CursorSmoother<'a> {
    events: &'a [CursorEvent],
    config: CursorSmoothingConfig,
}

impl<'a> CursorSmoother<'a> {
    const MAX_INTERPOLATION_GAP: f64 = 0.12;

    /// `events` müssen nach Zeit sortiert sein.
    pub fn new(events: &'a [CursorEvent], config: CursorSmoothingConfig) -> Self {
        Self { events, config }
    }

    /// Rohposition: kurze Lücken linear interpoliert, lange Lücken halten die vorige Position.
    pub fn raw_position(&self, time: f64) -> (f64, f64) {
        let ev = self.events;
        if ev.is_empty() {
            return (0.0, 0.0);
        }
        if ev.len() == 1 || time <= ev[0].t {
            return (ev[0].x, ev[0].y);
        }
        let last = ev[ev.len() - 1];
        if time >= last.t {
            return (last.x, last.y);
        }
        let (mut lo, mut hi) = (0, ev.len() - 1);
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if ev[mid].t <= time {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (b, a) = (ev[lo], ev[hi]);
        let span = a.t - b.t;
        if span <= 0.0 || span > Self::MAX_INTERPOLATION_GAP {
            return (b.x, b.y);
        }
        let t = (time - b.t) / span;
        (b.x + t * (a.x - b.x), b.y + t * (a.y - b.y))
    }

    pub fn click_events(&self) -> Vec<CursorEvent> {
        self.events.iter().filter(|e| e.is_click()).copied().collect()
    }

    /// Vorwärts- und Rückwärts-Federdurchlauf (Verzögerungen heben sich auf).
    pub fn build_smoothed_timeline(&self, fps: f64, duration: f64) -> SmoothedCursorTimeline {
        let dt = 1.0 / fps;
        let count = (duration.max(0.0) / dt).floor() as usize + 1;
        let mut xs = vec![0.0; count];
        let mut ys = vec![0.0; count];
        for i in 0..count {
            let (x, y) = self.raw_position(i as f64 * dt);
            xs[i] = x;
            ys[i] = y;
        }
        if self.config.enabled {
            let c = self.config;
            let mut spring = Spring2D::new(c.stiffness, c.damping, c.mass);
            let (sx, sy) = self.raw_position(0.0);
            spring.reset(sx, sy);
            for i in 0..count {
                let (x, y) = spring.step(xs[i], ys[i], dt);
                xs[i] = x;
                ys[i] = y;
            }
            spring.reset(xs[count - 1], ys[count - 1]);
            for i in (0..count).rev() {
                let (x, y) = spring.step(xs[i], ys[i], dt);
                xs[i] = x;
                ys[i] = y;
            }
        }
        SmoothedCursorTimeline { xs, ys, sample_interval: dt, duration }
    }
}

// ───────────────────────────── CursorFollowPath ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    pub time: f64,
    pub x: f64,
    pub y: f64,
}

/// Kamera hält, wo der Cursor zur Ruhe kommt, und schwenkt weich zwischen diesen Punkten.
#[derive(Clone, Debug)]
pub struct CursorFollowPath {
    pub waypoints: Vec<Waypoint>,
}

impl CursorFollowPath {
    pub const SAMPLE_INTERVAL: f64 = 0.05;
    pub const HOLD_RADIUS: f64 = 0.2;
    pub const SETTLE_RADIUS: f64 = 0.06;
    pub const SETTLE_DURATION: f64 = 0.25;
    pub const PAN_DURATION: f64 = 0.7;

    pub fn new(timeline: &SmoothedCursorTimeline, start: f64, end: f64) -> Self {
        let mut samples = vec![];
        let mut t = start;
        while t <= end {
            let (x, y) = timeline.position(t);
            samples.push(Waypoint { time: t, x, y });
            t += Self::SAMPLE_INTERVAL;
        }
        Self { waypoints: Self::waypoints_from(&samples) }
    }

    pub fn position(&self, time: f64) -> (f64, f64) {
        let Some(first) = self.waypoints.first() else { return (0.5, 0.5) };
        let mut cur = *first;
        for next in &self.waypoints[1..] {
            let pan_start = (next.time - Self::PAN_DURATION).max(cur.time);
            if time < pan_start {
                break;
            }
            if time < next.time {
                let p = ease_in_out_cubic((time - pan_start) / (next.time - pan_start));
                return (cur.x + (next.x - cur.x) * p, cur.y + (next.y - cur.y) * p);
            }
            cur = *next;
        }
        (cur.x, cur.y)
    }

    fn dist(a: &Waypoint, b: &Waypoint) -> f64 {
        (a.x - b.x).hypot(a.y - b.y)
    }

    pub fn waypoints_from(samples: &[Waypoint]) -> Vec<Waypoint> {
        let Some(first) = samples.first() else { return vec![] };
        let mut out = vec![];
        let mut anchor = *first;
        let mut asum = (first.x, first.y, 1.0);
        let mut candidate: Option<Waypoint> = None;
        let mut csum = (0.0, 0.0, 0.0);
        for s in &samples[1..] {
            let Some(start) = candidate else {
                if Self::dist(s, &anchor) <= Self::HOLD_RADIUS {
                    asum = (asum.0 + s.x, asum.1 + s.y, asum.2 + 1.0);
                } else {
                    candidate = Some(*s);
                    csum = (s.x, s.y, 1.0);
                }
                continue;
            };
            if Self::dist(s, &start) > Self::SETTLE_RADIUS {
                candidate = Some(*s);
                csum = (s.x, s.y, 1.0);
                continue;
            }
            csum = (csum.0 + s.x, csum.1 + s.y, csum.2 + 1.0);
            if s.time - start.time < Self::SETTLE_DURATION {
                continue;
            }
            let settled = Waypoint { time: start.time, x: csum.0 / csum.2, y: csum.1 / csum.2 };
            let current = Waypoint { time: anchor.time, x: asum.0 / asum.2, y: asum.1 / asum.2 };
            if Self::dist(&settled, &current) >= Self::SETTLE_RADIUS {
                out.push(current);
                anchor = settled;
                asum = csum;
            }
            candidate = None;
        }
        out.push(Waypoint { time: anchor.time, x: asum.0 / asum.2, y: asum.1 / asum.2 });
        out
    }
}

// ───────────────────────────── ZoomInterpolator ─────────────────────────────

/// Zoom-Transform zu jedem Zeitpunkt (Rampen innerhalb der Segmente, direkte Übergänge bei Anschluss).
#[derive(Clone, Debug)]
pub struct ZoomInterpolator {
    segments: Vec<ZoomSegment>,
    follow_paths: Vec<Option<CursorFollowPath>>,
}

impl ZoomInterpolator {
    pub const RAMP_DURATION: f64 = 0.75;
    const ADJACENCY_TOLERANCE: f64 = 0.05;
    const EDGE_SNAP_RATIO: f64 = 0.25;
    const UNZOOMED: FrameTransform = FrameTransform { scale: 1.0, translate_x: 0.5, translate_y: 0.5 };

    pub fn new(segments: &[ZoomSegment], cursor_timeline: Option<&SmoothedCursorTimeline>) -> Self {
        let segments = Self::resolving_overlaps(segments);
        let follow_paths = segments
            .iter()
            .map(|s| match (s.focus_mode, cursor_timeline) {
                (ZoomFocusMode::FollowCursor, Some(tl)) => Some(CursorFollowPath::new(tl, s.start_time, s.end_time)),
                _ => None,
            })
            .collect();
        Self { segments, follow_paths }
    }

    pub fn transform(&self, time: f64, cursor: Option<(f64, f64)>) -> FrameTransform {
        let Some(index) = (0..self.segments.len()).rev().find(|&i| self.segments[i].start_time <= time && time < self.effective_end(i)) else {
            return FrameTransform::IDENTITY;
        };
        let seg = &self.segments[index];
        let ramp = Self::RAMP_DURATION.min((seg.end_time - seg.start_time) / 2.0);
        let target = self.full_transform(index, time, cursor);
        let follows_previous = index > 0 && self.connects(index - 1);
        let has_next = self.connects(index);
        if ramp > 0.0 && time < seg.start_time + ramp && (follows_previous || seg.start_time > 0.0) {
            let from = if follows_previous { self.full_transform(index - 1, time, cursor) } else { Self::UNZOOMED };
            return Self::blend(from, target, ease_in_out_cubic((time - seg.start_time) / ramp));
        }
        if ramp > 0.0 && !has_next && time > seg.end_time - ramp {
            return Self::blend(target, Self::UNZOOMED, ease_in_out_cubic((time - (seg.end_time - ramp)) / ramp));
        }
        target
    }

    /// Bei Überlappung gewinnt das später beginnende Segment; frühere werden zerteilt.
    pub fn resolving_overlaps(segments: &[ZoomSegment]) -> Vec<ZoomSegment> {
        let mut sorted = segments.to_vec();
        sorted.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());
        let mut resolved: Vec<ZoomSegment> = vec![];
        for seg in sorted {
            resolved = resolved
                .into_iter()
                .flat_map(|e| {
                    if !(e.start_time < seg.end_time && seg.start_time < e.end_time) {
                        return vec![e];
                    }
                    let mut pieces = vec![];
                    if e.start_time < seg.start_time {
                        let mut before = e.clone();
                        before.end_time = seg.start_time;
                        pieces.push(before);
                    }
                    if seg.end_time < e.end_time {
                        let mut after = e.clone();
                        after.start_time = seg.end_time;
                        pieces.push(after);
                    }
                    pieces
                })
                .collect();
            resolved.push(seg);
        }
        resolved.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());
        resolved
    }

    fn connects(&self, i: usize) -> bool {
        i + 1 < self.segments.len() && self.segments[i + 1].start_time <= self.segments[i].end_time + Self::ADJACENCY_TOLERANCE
    }

    fn effective_end(&self, i: usize) -> f64 {
        if self.connects(i) {
            self.segments[i].end_time.max(self.segments[i + 1].start_time)
        } else {
            self.segments[i].end_time
        }
    }

    fn full_transform(&self, i: usize, time: f64, cursor: Option<(f64, f64)>) -> FrameTransform {
        let seg = &self.segments[i];
        let raw = match seg.focus_mode {
            ZoomFocusMode::FollowCursor => self.follow_paths[i].as_ref().map(|p| p.position(time)).or(cursor).unwrap_or((0.5, 0.5)),
            ZoomFocusMode::Manual { x, y } => (x, y),
        };
        let (fx, fy) = Self::edge_snap(raw, seg.zoom_level);
        FrameTransform::new(seg.zoom_level, fx, fy)
    }

    fn blend(from: FrameTransform, to: FrameTransform, p: f64) -> FrameTransform {
        let scale = from.scale + (to.scale - from.scale) * p;
        let half = 0.5 / scale;
        let clamp = |v: f64| v.max(half).min(1.0 - half);
        FrameTransform::new(
            scale,
            clamp(from.translate_x + (to.translate_x - from.translate_x) * p),
            clamp(from.translate_y + (to.translate_y - from.translate_y) * p),
        )
    }

    pub fn edge_snap(c: (f64, f64), zoom: f64) -> (f64, f64) {
        (Self::edge_snap_axis(c.0, zoom), Self::edge_snap_axis(c.1, zoom))
    }

    fn edge_snap_axis(v: f64, zoom: f64) -> f64 {
        let half = 0.5 / zoom;
        let r = Self::EDGE_SNAP_RATIO;
        if v <= r {
            half
        } else if v >= 1.0 - r {
            1.0 - half
        } else {
            let t = (v - r) / (1.0 - 2.0 * r);
            half + t * (1.0 - 2.0 * half)
        }
    }
}

// ───────────────────────────── CursorOverlayProvider ─────────────────────────────

/// Klick-Stauchung, Klick-Welle und Ausblenden bei Stillstand.
#[derive(Clone, Debug)]
pub struct CursorOverlayProvider {
    clicks: Vec<CursorEvent>,
    releases: Vec<f64>,
    idle_gaps: Vec<(f64, f64)>,
}

impl CursorOverlayProvider {
    const CLICK_DURATION: f64 = 0.13;
    const CLICK_MIN_SCALE: f64 = 0.8;
    pub const IDLE_THRESHOLD: f64 = 1.5;
    pub const IDLE_FADE_DELAY: f64 = 0.1;
    pub const IDLE_FADE_DURATION: f64 = 0.3;
    pub const RIPPLE_PRESS_DURATION: f64 = 0.2;
    pub const RIPPLE_RELEASE_DURATION: f64 = 0.2;
    pub const ASSUMED_PRESS_LENGTH: f64 = 0.1;

    pub fn new(events: &[CursorEvent], hides_idle_cursor: bool) -> Self {
        let mut sorted = events.to_vec();
        sorted.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        let clicks: Vec<CursorEvent> = sorted.iter().filter(|e| e.is_click()).copied().collect();
        let ups: Vec<f64> = sorted.iter().filter(|e| e.kind == CursorEventType::MouseUp).map(|e| e.t).collect();
        let releases = clicks
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let next = clicks.get(i + 1).map(|n| n.t).unwrap_or(f64::INFINITY);
                ups.iter().copied().find(|&u| u >= c.t && u < next).unwrap_or(c.t + Self::ASSUMED_PRESS_LENGTH)
            })
            .collect();
        let mut idle_gaps = vec![];
        if hides_idle_cursor {
            let mut activity = vec![0.0];
            activity.extend(sorted.iter().filter(|e| e.kind != CursorEventType::KeyDown && e.t > 0.0).map(|e| e.t));
            for w in activity.windows(2) {
                if w[1] - w[0] >= Self::IDLE_THRESHOLD {
                    idle_gaps.push((w[0], w[1]));
                }
            }
            if let Some(&last) = activity.last() {
                idle_gaps.push((last, f64::INFINITY));
            }
        }
        Self { clicks, releases, idle_gaps }
    }

    pub fn click_scale(&self, time: f64) -> f64 {
        for c in self.clicks.iter().rev() {
            let elapsed = time - c.t;
            if elapsed < 0.0 {
                continue;
            }
            if elapsed > Self::CLICK_DURATION {
                break;
            }
            return Self::CLICK_MIN_SCALE + (1.0 - Self::CLICK_MIN_SCALE) * smoothstep(elapsed / Self::CLICK_DURATION);
        }
        1.0
    }

    /// (Radius-Anteil, Deckkraft) der Klick-Welle oder `None`.
    pub fn ripple(&self, time: f64) -> Option<(f64, f64)> {
        let i = self.clicks.iter().rposition(|c| c.t <= time)?;
        let down = self.clicks[i].t;
        let pressed = down + Self::RIPPLE_PRESS_DURATION;
        let released = self.releases[i].max(pressed);
        if time < pressed {
            return Some((0.6 + 0.4 * smoothstep((time - down) / Self::RIPPLE_PRESS_DURATION), 1.0));
        }
        if time < released {
            return Some((1.0, 1.0));
        }
        let fade = (time - released) / Self::RIPPLE_RELEASE_DURATION;
        if fade >= 1.0 {
            return None;
        }
        Some((1.0 + 0.5 * smoothstep(fade), 1.0 - smoothstep(fade)))
    }

    pub fn opacity(&self, time: f64) -> f64 {
        let Some(&(start, end)) = self.idle_gaps.iter().find(|(s, e)| *s <= time && time < *e) else { return 1.0 };
        let fade_out = (time - (start + Self::IDLE_FADE_DELAY)) / Self::IDLE_FADE_DURATION;
        let fade_in = (end - time) / Self::IDLE_FADE_DURATION;
        1.0 - smoothstep(fade_out.min(fade_in))
    }
}

// ───────────────────────────── AutoZoomDetector ─────────────────────────────

#[derive(Clone, Copy, Debug)]
enum CandidateKind {
    Click,
    ClickBurst { first: f64, last: f64, zoom: f64 },
    Typing { first: f64, last: f64, pre_roll: f64 },
    Dwell { duration: f64 },
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    kind: CandidateKind,
    center: f64,
    focus: (f64, f64),
    strength: f64,
}

/// Schlägt Zoom-Segmente aus Klicks, Verweilpunkten und Tipp-Phasen vor.
pub struct AutoZoomDetector;

impl AutoZoomDetector {
    pub const MIN_DWELL_DURATION: f64 = 0.45;
    pub const MAX_DWELL_INFLUENCE: f64 = 2.60;
    pub const DWELL_MOVE_THRESHOLD: f64 = 0.02;
    pub const MAX_DENSE_DWELL_GAP: f64 = 0.20;
    pub const MIN_SPACING: f64 = 4.00;
    pub const MERGE_GAP: f64 = 0.50;
    pub const CLICK_PRE_ROLL: f64 = 1.0;
    pub const CLICK_POST_ROLL: f64 = 2.0;
    pub const DWELL_BUFFER: f64 = 0.5;
    pub const DEFAULT_ZOOM: f64 = 1.5;
    pub const DEFAULT_DURATION: f64 = 3.0;
    pub const CLICK_BURST_GAP: f64 = 3.0;
    pub const CLICK_BURST_RADIUS: f64 = 0.25;
    pub const QUICK_CLICK_GAP: f64 = 0.5;
    pub const DENSE_BURST_SCORE: f64 = 3.0;
    pub const DENSE_BURST_ZOOM: f64 = 2.0;
    pub const MAX_SEGMENT_DURATION: f64 = 60.0;
    pub const DUPLICATE_CLICK_WINDOW: f64 = 0.05;
    pub const TYPING_GAP: f64 = 0.5;
    pub const MIN_TYPING_KEYS: usize = 4;
    pub const TYPING_PRE_ROLL: f64 = 0.3;
    pub const FOCUSING_CLICK_WINDOW: f64 = 2.0;
    pub const TYPING_POST_ROLL: f64 = 0.8;

    pub fn detect(events: &[CursorEvent], duration: f64) -> Vec<ZoomSegment> {
        if duration <= 0.0 {
            return vec![];
        }
        let mut sorted = events.to_vec();
        sorted.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        let clicks = Self::click_candidates(&sorted);
        let non_keys: Vec<CursorEvent> = sorted.iter().filter(|e| e.kind != CursorEventType::KeyDown).copied().collect();
        let dwells = Self::dwell_candidates(&non_keys);
        let typing = Self::typing_candidates(&sorted);
        let (typing, clicks) = Self::attach_focusing_clicks(typing, clicks);
        let mut all = clicks;
        all.extend(dwells);
        all.extend(typing);
        let accepted = Self::filter_by_spacing(&all, duration);
        let mut segs: Vec<ZoomSegment> = accepted.iter().filter_map(|c| Self::make_segment(c, duration)).collect();
        segs.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());
        Self::merge_overlapping(segs)
    }

    fn click_candidates(events: &[CursorEvent]) -> Vec<Candidate> {
        let mut bursts: Vec<Vec<CursorEvent>> = vec![];
        for c in events.iter().filter(|e| e.is_click()) {
            if let Some(last) = bursts.last().and_then(|b| b.last()) {
                if c.t - last.t < Self::DUPLICATE_CLICK_WINDOW {
                    continue;
                }
                if c.t - last.t <= Self::CLICK_BURST_GAP && (c.x - last.x).hypot(c.y - last.y) <= Self::CLICK_BURST_RADIUS {
                    bursts.last_mut().unwrap().push(*c);
                    continue;
                }
            }
            bursts.push(vec![*c]);
        }
        bursts
            .into_iter()
            .map(|b| {
                let first = b[0];
                if b.len() == 1 {
                    return Candidate { kind: CandidateKind::Click, center: first.t, focus: (first.x, first.y), strength: 1000.0 };
                }
                let last = b[b.len() - 1];
                let score: f64 = b.windows(2).map(|p| 1.0 + if p[1].t - p[0].t <= Self::QUICK_CLICK_GAP { 1.5 } else { 0.0 }).sum();
                let n = b.len() as f64;
                Candidate {
                    kind: CandidateKind::ClickBurst {
                        first: first.t,
                        last: last.t,
                        zoom: if score >= Self::DENSE_BURST_SCORE { Self::DENSE_BURST_ZOOM } else { Self::DEFAULT_ZOOM },
                    },
                    center: (first.t + last.t) / 2.0,
                    focus: (b.iter().map(|e| e.x).sum::<f64>() / n, b.iter().map(|e| e.y).sum::<f64>() / n),
                    strength: 1000.0 * (1.0 + score),
                }
            })
            .collect()
    }

    fn attach_focusing_clicks(typing: Vec<Candidate>, clicks: Vec<Candidate>) -> (Vec<Candidate>, Vec<Candidate>) {
        let mut remaining = clicks;
        let last_click_time = |c: &Candidate| match c.kind {
            CandidateKind::ClickBurst { last, .. } => last,
            _ => c.center,
        };
        let attached = typing
            .into_iter()
            .map(|run| {
                let CandidateKind::Typing { first, last, .. } = run.kind else { return run };
                let Some(i) = remaining.iter().rposition(|c| {
                    let t = last_click_time(c);
                    t <= first && first - t <= Self::FOCUSING_CLICK_WINDOW
                }) else {
                    return run;
                };
                let click = remaining.remove(i);
                let click_start = match click.kind {
                    CandidateKind::ClickBurst { first, .. } => first,
                    _ => click.center,
                };
                Candidate {
                    kind: CandidateKind::Typing { first: click_start, last, pre_roll: Self::CLICK_PRE_ROLL },
                    center: (click_start + last) / 2.0,
                    focus: click.focus,
                    strength: run.strength.max(click.strength),
                }
            })
            .collect();
        (attached, remaining)
    }

    fn typing_candidates(events: &[CursorEvent]) -> Vec<Candidate> {
        let mut runs: Vec<Vec<CursorEvent>> = vec![];
        for k in events.iter().filter(|e| e.kind == CursorEventType::KeyDown) {
            match runs.last_mut() {
                Some(r) if k.t - r[r.len() - 1].t <= Self::TYPING_GAP => r.push(*k),
                _ => runs.push(vec![*k]),
            }
        }
        runs.into_iter()
            .filter(|r| r.len() >= Self::MIN_TYPING_KEYS)
            .map(|r| {
                let n = r.len() as f64;
                let (first, last) = (r[0].t, r[r.len() - 1].t);
                Candidate {
                    kind: CandidateKind::Typing { first, last, pre_roll: Self::TYPING_PRE_ROLL },
                    center: (first + last) / 2.0,
                    focus: (r.iter().map(|e| e.x).sum::<f64>() / n, r.iter().map(|e| e.y).sum::<f64>() / n),
                    strength: 1000.0 + 100.0 * n,
                }
            })
            .collect()
    }

    fn is_stationary(a: &CursorEvent, b: &CursorEvent) -> bool {
        (b.x - a.x).hypot(b.y - a.y) < Self::DWELL_MOVE_THRESHOLD
    }

    fn dwell_candidates(events: &[CursorEvent]) -> Vec<Candidate> {
        if events.len() < 2 {
            return vec![];
        }
        let mut out = vec![];
        // Dichte Läufe
        let mut run_start = 0;
        for i in 1..events.len() {
            let gap = events[i].t - events[i - 1].t;
            if !Self::is_stationary(&events[i - 1], &events[i]) || gap > Self::MAX_DENSE_DWELL_GAP {
                out.extend(Self::dense_run(events, run_start, i - 1));
                run_start = i;
            }
        }
        out.extend(Self::dense_run(events, run_start, events.len() - 1));
        // Dünne Lücken an gleicher Position
        for i in 1..events.len() {
            let (p, c) = (&events[i - 1], &events[i]);
            let gap = c.t - p.t;
            if !Self::is_stationary(p, c) || gap < Self::MIN_DWELL_DURATION {
                continue;
            }
            if p.kind != CursorEventType::Move && c.kind != CursorEventType::Move {
                continue;
            }
            let d = gap.min(Self::MAX_DWELL_INFLUENCE);
            out.push(Candidate {
                kind: CandidateKind::Dwell { duration: d },
                center: (p.t + c.t) / 2.0,
                focus: ((p.x + c.x) / 2.0, (p.y + c.y) / 2.0),
                strength: d * 1000.0,
            });
        }
        out
    }

    fn dense_run(events: &[CursorEvent], s: usize, e: usize) -> Option<Candidate> {
        if s >= e {
            return None;
        }
        let dur = events[e].t - events[s].t;
        if dur < Self::MIN_DWELL_DURATION {
            return None;
        }
        let d = dur.min(Self::MAX_DWELL_INFLUENCE);
        let n = (e - s + 1) as f64;
        let run = &events[s..=e];
        Some(Candidate {
            kind: CandidateKind::Dwell { duration: d },
            center: (events[s].t + events[e].t) / 2.0,
            focus: (run.iter().map(|v| v.x).sum::<f64>() / n, run.iter().map(|v| v.y).sum::<f64>() / n),
            strength: d * 1000.0,
        })
    }

    fn filter_by_spacing(candidates: &[Candidate], duration: f64) -> Vec<Candidate> {
        let mut sorted = candidates.to_vec();
        sorted.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap().then(a.center.partial_cmp(&b.center).unwrap()));
        let span = |c: &Candidate| Self::make_segment(c, duration).map(|s| (s.start_time, s.end_time)).unwrap_or((c.center, c.center));
        let mut accepted: Vec<Candidate> = vec![];
        for c in sorted {
            let cs = span(&c);
            let conflicts = accepted.iter().any(|o| {
                let os = span(o);
                (c.center - o.center).abs() < Self::MIN_SPACING || (cs.0 < os.1 + Self::MERGE_GAP && os.0 < cs.1 + Self::MERGE_GAP)
            });
            if !conflicts {
                accepted.push(c);
            }
        }
        accepted
    }

    fn make_segment(c: &Candidate, duration: f64) -> Option<ZoomSegment> {
        let mut zoom = Self::DEFAULT_ZOOM;
        let (raw_start, raw_end) = match c.kind {
            CandidateKind::Click => (c.center - Self::CLICK_PRE_ROLL, c.center + Self::CLICK_POST_ROLL),
            CandidateKind::ClickBurst { first, last, zoom: z } => {
                zoom = z;
                let s = first - Self::CLICK_PRE_ROLL;
                (s, (last + Self::CLICK_POST_ROLL).min(s + Self::MAX_SEGMENT_DURATION))
            }
            CandidateKind::Typing { first, last, pre_roll } => {
                let s = first - pre_roll;
                (s, (last + Self::TYPING_POST_ROLL).min(s + Self::MAX_SEGMENT_DURATION))
            }
            CandidateKind::Dwell { duration: d } => {
                let half = Self::DEFAULT_DURATION.max(d + 2.0 * Self::DWELL_BUFFER) / 2.0;
                (c.center - half, c.center + half)
            }
        };
        let len = raw_end - raw_start;
        let (mut start, mut end) = (raw_start, raw_end);
        if start < 0.0 {
            start = 0.0;
            end = duration.min(len);
        } else if end > duration {
            end = duration;
            start = (end - len).max(0.0);
        }
        if end <= start {
            return None;
        }
        Some(ZoomSegment {
            id: new_id(),
            start_time: start,
            end_time: end,
            zoom_level: zoom,
            focus_mode: ZoomFocusMode::Manual { x: c.focus.0, y: c.focus.1 },
            source: SegmentSource::Auto,
        })
    }

    fn merge_overlapping(segments: Vec<ZoomSegment>) -> Vec<ZoomSegment> {
        let mut merged: Vec<ZoomSegment> = vec![];
        for next in segments {
            let Some(prev) = merged.last_mut() else {
                merged.push(next);
                continue;
            };
            if next.start_time <= prev.end_time + Self::MERGE_GAP {
                let (pd, nd) = (prev.duration(), next.duration());
                let total = pd + nd;
                let focus = |m: ZoomFocusMode| match m {
                    ZoomFocusMode::Manual { x, y } => (x, y),
                    _ => (0.5, 0.5),
                };
                let (px, py) = focus(prev.focus_mode);
                let (nx, ny) = focus(next.focus_mode);
                prev.end_time = prev.end_time.max(next.end_time);
                prev.zoom_level = prev.zoom_level.max(next.zoom_level);
                prev.focus_mode = ZoomFocusMode::Manual { x: (px * pd + nx * nd) / total, y: (py * pd + ny * nd) / total };
                prev.source = SegmentSource::Auto;
            } else {
                merged.push(next);
            }
        }
        merged
    }
}

// ───────────────────────────── Layout & Bildzustand ─────────────────────────────

/// Geometrie der Ausgabe (alle Werte in Ausgabepixeln).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub out_w: f64,
    pub out_h: f64,
    /// Videorahmen im Ausgabebild.
    pub frame_x: f64,
    pub frame_y: f64,
    pub frame_w: f64,
    pub frame_h: f64,
    /// Quellpixel → Ausgabepixel.
    pub k: f64,
    pub corner_radius: f64,
}

pub fn aspect_value(s: &str) -> Option<f64> {
    let (a, b) = s.split_once(':')?;
    let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a > 0.0 && b > 0.0).then_some(a / b)
}

fn round_up_even(v: f64) -> f64 {
    let i = v.ceil() as i64;
    (if i % 2 == 0 { i } else { i + 1 }) as f64
}

impl Layout {
    /// Layout in Quellpixeln (FrameCompositor.computeOutputSize) inkl. Seitenverhältnis.
    pub fn base(p: &RecordingProject) -> Layout {
        let (sw, sh) = (p.video_width.max(2.0), p.video_height.max(2.0));
        let bg = &p.background_style;
        let pad = if bg.enabled { bg.padding.max(0.0) } else { 0.0 };
        let (mut cw, mut ch) = if bg.enabled { (round_up_even(sw + 2.0 * pad), round_up_even(sh + 2.0 * pad)) } else { (sw, sh) };
        if let Some(r) = aspect_value(&p.aspect_ratio) {
            if cw / ch < r {
                cw = round_up_even(ch * r);
            } else {
                ch = round_up_even(cw / r);
            }
        }
        Layout {
            out_w: cw,
            out_h: ch,
            frame_x: ((cw - sw) / 2.0).max(0.0),
            frame_y: ((ch - sh) / 2.0).max(0.0),
            frame_w: sw,
            frame_h: sh,
            k: 1.0,
            corner_radius: if bg.enabled { bg.clamped_corner_radius(sw, sh) } else { 0.0 },
        }
    }

    pub fn scaled(&self, f: f64) -> Layout {
        Layout {
            out_w: self.out_w * f,
            out_h: self.out_h * f,
            frame_x: self.frame_x * f,
            frame_y: self.frame_y * f,
            frame_w: self.frame_w * f,
            frame_h: self.frame_h * f,
            k: self.k * f,
            corner_radius: self.corner_radius * f,
        }
    }

    /// Export-Layout: auf Qualitätsstufe begrenzt (nie vergrößert), Ausgabegröße gerade.
    pub fn for_export(p: &RecordingProject, quality: &str, gif: bool) -> Layout {
        let base = Self::base(p);
        let f = export_scale(base.out_w, base.out_h, quality, gif);
        let mut l = base.scaled(f);
        l.out_w = (l.out_w / 2.0).floor().max(1.0) * 2.0;
        l.out_h = (l.out_h / 2.0).floor().max(1.0) * 2.0;
        l
    }

    /// Videopunkt (normiert, oben links) → Position im Videorahmen (Ausgabepixel, relativ zum Rahmen) unter Zoom.
    pub fn place(&self, zoom: &FrameTransform, x: f64, y: f64) -> (f64, f64) {
        let (fx, fy) = zoom.focus();
        let s = zoom.scale;
        ((x - fx) * self.frame_w * s + self.frame_w / 2.0, (y - fy) * self.frame_h * s + self.frame_h / 2.0)
    }
}

/// Skalierungsfaktor für die Qualitätsstufen (MP4: maximum/1080p/720p-Box, GIF: maximale Breite 1920/1080/720).
pub fn export_scale(w: f64, h: f64, quality: &str, gif: bool) -> f64 {
    if gif {
        let max_w = match quality {
            "social" => 1080.0,
            "web" => 720.0,
            _ => 1920.0,
        };
        return (max_w / w).min(1.0);
    }
    let (bw, bh) = match quality {
        "social" => (1920.0, 1080.0),
        "web" => (1280.0, 720.0),
        _ => return 1.0,
    };
    let (bw, bh) = if h > w { (bh, bw) } else { (bw, bh) };
    (bw / w).min(bh / h).min(1.0)
}

/// GIF-Bildrate je Qualität (Capso GIFExporter), begrenzt auf 600 Bilder insgesamt.
pub fn gif_fps(quality: &str, seconds: f64) -> f64 {
    let base = match quality {
        "social" => 20.0,
        "web" => 15.0,
        _ => 25.0,
    };
    let frames = (seconds * base).ceil().max(1.0);
    if frames > 600.0 {
        600.0 / seconds
    } else {
        base
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorState {
    /// Normierte Position im Video.
    pub x: f64,
    pub y: f64,
    /// Klick-Stauchung (0.8–1).
    pub click_scale: f64,
    pub alpha: f64,
    /// (Radius-Anteil, Deckkraft) der Klick-Welle.
    pub ripple: Option<(f64, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameState {
    pub zoom: FrameTransform,
    pub cursor: Option<CursorState>,
    pub blurs: Vec<BlurEffectPayload>,
}

/// Alles, was für ein Bild zu einem Quell-Zeitpunkt nötig ist (FrameCompositor-Eingaben).
pub struct Engine {
    pub project: RecordingProject,
    timeline: Option<SmoothedCursorTimeline>,
    interp: ZoomInterpolator,
    overlay: Option<CursorOverlayProvider>,
}

impl Engine {
    pub fn new(project: RecordingProject, events: &[CursorEvent]) -> Self {
        let has = !events.is_empty();
        let zooms = project.zoom_segments();
        let timeline = (has && (project.shows_cursor || !zooms.is_empty())).then(|| {
            let cfg = if project.cursor_smoothing.enabled { project.cursor_smoothing } else { CursorSmoothingConfig { enabled: false, ..Default::default() } };
            CursorSmoother::new(events, cfg).build_smoothed_timeline(60.0, project.video_duration.max(0.0))
        });
        let interp = ZoomInterpolator::new(&zooms, timeline.as_ref());
        let overlay = (has && project.shows_cursor).then(|| CursorOverlayProvider::new(events, project.hides_idle_cursor));
        Self { project, timeline, interp, overlay }
    }

    pub fn has_cursor(&self) -> bool {
        self.overlay.is_some()
    }

    pub fn cursor_position(&self, t: f64) -> Option<(f64, f64)> {
        self.timeline.as_ref().map(|tl| tl.position(t))
    }

    /// Bildzustand zur Quellzeit `t` (Sekunden).
    pub fn frame(&self, t: f64) -> FrameState {
        let pos = self.cursor_position(t);
        let zoom = self.interp.transform(t, pos);
        let cursor = match (&self.overlay, pos) {
            (Some(o), Some((x, y))) => {
                let alpha = o.opacity(t);
                (alpha > 0.0).then(|| CursorState { x, y, click_scale: o.click_scale(t), alpha, ripple: o.ripple(t) })
            }
            _ => None,
        };
        let blurs = self
            .project
            .effect_segments
            .iter()
            .filter(|e| e.contains(t))
            .filter_map(|e| match e.payload {
                EffectPayload::Blur(b) if b.radius > 0.0 && b.rect.width > 0.0 && b.rect.height > 0.0 => Some(b),
                _ => None,
            })
            .collect();
        FrameState { zoom, cursor, blurs }
    }
}

/// Mauszeiger-Umriss (macOS-Pfeil) für Höhe 1, Spitze bei (0,0). Breite ≈ 0.62.
pub const CURSOR_ARROW: [(f64, f64); 7] = [
    (0.0, 0.0),
    (0.0, 0.84),
    (0.20, 0.655),
    (0.335, 0.97),
    (0.47, 0.915),
    (0.34, 0.61),
    (0.6, 0.61),
];

/// Weiche Ecken (FrameCompositor.continuousRoundedRectPath): Liste von Pfadbefehlen.
#[derive(Clone, Copy, Debug)]
pub enum PathCmd {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

pub fn continuous_rounded_rect(x: f64, y: f64, w: f64, h: f64, radius: f64) -> Vec<PathCmd> {
    use PathCmd::*;
    let r = radius.max(0.0).min(w.min(h) / 2.0);
    if r <= 0.0 {
        return vec![Move(x, y), Line(x + w, y), Line(x + w, y + h), Line(x, y + h), Close];
    }
    let t = (r * 1.528).min(w.min(h) / 2.0);
    let k = r * 0.67;
    let (x0, x1, y0, y1) = (x, x + w, y, y + h);
    vec![
        Move(x0 + t, y0),
        Line(x1 - t, y0),
        Cubic(x1 - k, y0, x1, y0 + k, x1, y0 + t),
        Line(x1, y1 - t),
        Cubic(x1, y1 - k, x1 - k, y1, x1 - t, y1),
        Line(x0 + t, y1),
        Cubic(x0 + k, y1, x0, y1 - k, x0, y1 - t),
        Line(x0, y0 + t),
        Cubic(x0, y0 + k, x0 + k, y0, x0 + t, y0),
        Close,
    ]
}

pub fn format_time(seconds: f64) -> String {
    let s = seconds.max(0.0);
    let mins = s as u64 / 60;
    let secs = s as u64 % 60;
    let frac = ((s - s.floor()) * 10.0) as u64;
    format!("{mins}:{secs:02}.{frac}")
}
