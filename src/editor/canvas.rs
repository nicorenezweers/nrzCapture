//! Gemeinsamer Editor-Zustand (`Ed`) und die interaktive Zeichenfläche (Port von Capso `AnnotationCanvasNSView`,
//! `AnnotationTextEditor`, `ZoomableScrollContainer`). Wird vom Editor-Fenster und vom `InlineAnnotator` benutzt.
use super::model::*;
use super::render::{self, ACCENT};
use crate::api;
use leptos::{ev, html, prelude::*};
use serde_json::json;
use std::time::Duration;
use wasm_bindgen::{prelude::Closure, JsCast};
use web_sys::{HtmlCanvasElement, HtmlImageElement};

/// Laufende Texteingabe (Capso `AnnotationTextEditor`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextEdit {
    /// Bestehendes Objekt (wird während der Bearbeitung ausgeblendet).
    pub id: Option<u64>,
    pub origin: Pt,
    pub text: String,
    /// Von Hand gesetzte Boxbreite (Griffe); `None` = automatisch (Capso `manuallySizedWidth`).
    pub box_w: Option<f64>,
    /// Identität der Bearbeitung (Textfeld neu befüllen/fokussieren nur bei Wechsel).
    pub seq: u32,
}

#[derive(Clone, Debug, Default)]
pub enum DragState {
    #[default]
    None,
    Create { tool: Tool, start: Pt, cur: Pt, points: Vec<Pt> },
    Move { id: u64, last: Pt, moved: bool },
    Handle { id: u64, h: Handle, orig: Object },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Keys {
    pub shift: bool,
    pub center_lock: bool,
}

/// Welche Stileigenschaft geändert wurde (für die Übernahme auf das ausgewählte Objekt).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Color,
    Width,
    Filled,
    Pattern,
    ArrowHead,
    TextFx,
    Redaction,
}

/// Abbildung Bildpixel → CSS-Pixel der Zeichenfläche: `css = img * z + o`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Map {
    pub z: f64,
    pub ox: f64,
    pub oy: f64,
    pub dpr: f64,
}

impl Default for Map {
    fn default() -> Self {
        Map { z: 1.0, ox: 0.0, oy: 0.0, dpr: 1.0 }
    }
}

impl Map {
    pub fn to_img(&self, x: f64, y: f64) -> Pt {
        pt((x - self.ox) / self.z, (y - self.oy) / self.z)
    }
    pub fn to_css(&self, p: Pt) -> Pt {
        pt(p.x * self.z + self.ox, p.y * self.z + self.oy)
    }
    pub fn apply(&self, g: &web_sys::CanvasRenderingContext2d) {
        let d = self.dpr;
        let _ = g.set_transform(d * self.z, 0.0, 0.0, d * self.z, d * self.ox, d * self.oy);
    }
}

#[derive(Clone, Copy)]
pub struct Ed {
    pub doc: RwSignal<Doc>,
    pub prefs: RwSignal<ToolPrefs>,
    pub beautify: RwSignal<Beautify>,
    pub zoom: RwSignal<f64>,
    pub crop_mode: RwSignal<bool>,
    pub text: RwSignal<Option<TextEdit>>,
    pub presets: RwSignal<Vec<StylePreset>>,
    pub loaded: RwSignal<bool>,
    /// Neu zeichnen (Vorschau während des Ziehens, ohne das Dokument zu ändern).
    pub tick: RwSignal<u32>,
    /// Zoom an Fenster anpassen anfordern.
    pub fit: RwSignal<u32>,
    pub map: RwSignal<Map>,
    /// Textfeld erneut fokussieren (nach Klick in die Werkzeugleiste).
    pub refocus: RwSignal<u32>,
    pub inline: bool,
    pub src: StoredValue<Option<HtmlCanvasElement>, LocalStorage>,
    pub img: StoredValue<Option<HtmlImageElement>, LocalStorage>,
    pub drag: StoredValue<DragState>,
    pub keys: StoredValue<Keys>,
    pub center_key: StoredValue<String>,
    /// Anzeigebereich, eingefroren während eines Zieh-Vorgangs.
    last_disp: StoredValue<Option<Rect>>,
    /// OCR-Textzeilen (normiert, oben links) für den intelligenten Marker (Capso `textRegions`).
    pub text_lines: StoredValue<Vec<[f64; 4]>>,
    /// Zeile, an der der laufende Markerstrich einrastet.
    hl_snap: StoredValue<Option<Rect>>,
}

impl Ed {
    pub fn new(inline: bool) -> Ed {
        let prefs: ToolPrefs = serde_json::from_value(api::setting("annotationPrefs")).unwrap_or_default();
        let presets: Vec<StylePreset> = serde_json::from_value(api::setting("annotationStylePresets")).unwrap_or_default();
        let mut beautify: Beautify = serde_json::from_value(api::setting("beautifyStyle")).unwrap_or_default();
        beautify.enabled = false;
        let ck = api::setting_str("squareCenterLockKey");
        let ed = Ed {
            doc: RwSignal::new(Doc::new(1.0, 1.0)),
            prefs: RwSignal::new(prefs),
            beautify: RwSignal::new(beautify),
            zoom: RwSignal::new(1.0),
            crop_mode: RwSignal::new(false),
            text: RwSignal::new(None),
            presets: RwSignal::new(presets),
            loaded: RwSignal::new(false),
            tick: RwSignal::new(0),
            fit: RwSignal::new(0),
            map: RwSignal::new(Map::default()),
            refocus: RwSignal::new(0),
            inline,
            src: StoredValue::new_local(None),
            img: StoredValue::new_local(None),
            drag: StoredValue::new(DragState::None),
            keys: StoredValue::new(Keys::default()),
            center_key: StoredValue::new(if ck.is_empty() { "C".into() } else { ck }),
            last_disp: StoredValue::new(None),
            text_lines: StoredValue::new(vec![]),
            hl_snap: StoredValue::new(None),
        };
        // Werkzeug-Einstellungen verzögert speichern (Regler feuern sehr oft).
        let gen = StoredValue::new(0u32);
        Effect::new(move |prev: Option<()>| {
            let p = ed.prefs.get();
            if prev.is_none() {
                return;
            }
            let n = gen.get_value() + 1;
            gen.set_value(n);
            set_timeout(
                move || {
                    if gen.get_value() == n {
                        api::set_setting("annotationPrefs", serde_json::to_value(&p).unwrap_or_default());
                    }
                },
                Duration::from_millis(500),
            );
        });
        ed
    }

    /// Bild laden (asset- oder data-URL); bei Fehler `on_error`.
    pub fn load_or(self, url: String, on_ready: impl Fn() + 'static, on_error: impl Fn() + 'static) {
        let img = HtmlImageElement::new().unwrap();
        let onerror = Closure::<dyn Fn()>::new(on_error);
        img.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();
        self.load_img(img, url, on_ready);
    }

    /// Bild laden (asset- oder data-URL).
    pub fn load(self, url: String, on_ready: impl Fn() + 'static) {
        self.load_img(HtmlImageElement::new().unwrap(), url, on_ready);
    }

    fn load_img(self, img: HtmlImageElement, url: String, on_ready: impl Fn() + 'static) {
        img.set_cross_origin(Some("anonymous"));
        let im = img.clone();
        let onload = Closure::<dyn Fn()>::new(move || {
            let (w, h) = (im.natural_width() as f64, im.natural_height() as f64);
            self.img.set_value(Some(im.clone()));
            self.src.set_value(Some(render::build_source(&im, Xform::default())));
            self.doc.set(Doc::new(w, h));
            self.loaded.set(true);
            self.fit.update(|f| *f += 1);
            on_ready();
        });
        img.set_onload(Some(onload.as_ref().unchecked_ref()));
        onload.forget();
        img.set_src(&url);
        self.img.set_value(Some(img));
    }

    /// Quellbild neu aufbauen, wenn sich Drehen/Spiegeln geändert hat.
    pub fn sync_source(self) {
        let xf = self.doc.with_untracked(|d| d.xform);
        if let Some(img) = self.img.get_value() {
            self.src.set_value(Some(render::build_source(&img, xf)));
        }
    }

    /// Schnelle Texterkennung beim Öffnen (Capso `handleCanvasAppear` → `OCRProcessFallback.recognize`) für den Marker.
    pub fn load_text_lines(self, path: String) {
        leptos::task::spawn_local(async move {
            if let Ok(v) = api::rpc("ocr", "recognize", json!({ "path": path })).await {
                let lines = v
                    .as_array()
                    .map(|a| a.iter().filter_map(|r| serde_json::from_value::<[f64; 4]>(r["bbox"].clone()).ok()).collect())
                    .unwrap_or_default();
                self.text_lines.set_value(lines);
            }
        });
    }

    /// OCR-Zeilen in Bildpixeln (nur ohne Drehen/Spiegeln gültig).
    fn text_line_rects(self) -> Vec<Rect> {
        let (w, h, xf) = self.doc.with_untracked(|d| (d.orig_w, d.orig_h, d.xform));
        if xf != Xform::default() {
            return vec![];
        }
        self.text_lines.with_value(|l| l.iter().map(|b| rect(b[0] * w, b[1] * h, b[2] * w, b[3] * h)).collect())
    }

    /// Rechte Grenze für den automatischen Textumbruch (Bild- bzw. Zeichenflächenrand).
    fn text_right_edge(self) -> f64 {
        if self.inline {
            self.doc.with_untracked(|d| d.image_rect().max_x())
        } else {
            self.doc.with_untracked(content_rect).max_x()
        }
    }

    /// Textbox während der Eingabe (Capso `AnnotationTextEditor.resizeToFit`): automatische Breite bis zum rechten Rand
    /// (− 12 pt), mindestens 60; Höhe = umbrochener Text, mindestens max(28, Schrift + 12). Ergebnis in Bildpixeln.
    pub fn edit_box(self, te: &TextEdit, fs: f64) -> (f64, f64) {
        let z = self.map.with_untracked(|m| m.z).max(0.1);
        let (min_w, min_h) = text_box_min(fs);
        let max_auto = (self.text_right_edge() - te.origin.x - 12.0 / z).max(min_w);
        let w = match te.box_w {
            Some(b) => b.max(min_w),
            None if te.text.is_empty() => 220f64.min(max_auto),
            None => max_auto.min(min_w.max(render::measure_text(&te.text, fs).0 + 2.0 * TEXT_BOX_PADDING + 2.0 / z)),
        };
        let (_, th) = render::measure_wrapped(if te.text.is_empty() { " " } else { &te.text }, fs, (w - 2.0 * TEXT_BOX_PADDING).max(1.0));
        (w, (th + 2.0 * TEXT_BOX_PADDING).max(min_h))
    }

    pub fn tool(self) -> Tool {
        self.prefs.with(|p| p.tool)
    }

    pub fn set_tool(self, t: Tool) {
        self.commit_text();
        if self.tool_untracked() != t {
            self.doc.update(|d| d.selected = None);
        }
        self.prefs.update(|p| p.tool = t);
    }

    pub fn tool_untracked(self) -> Tool {
        self.prefs.with_untracked(|p| p.tool)
    }

    pub fn editing(self) -> bool {
        self.text.with(|t| t.is_some())
    }

    /// Stil für neue Objekte des aktuellen Werkzeugs.
    pub fn style(self) -> Style {
        self.prefs.with_untracked(|p| p.style_for(p.tool))
    }

    /// Werkzeug, dessen Reglerbedeutung gerade gilt (Text auch beim Bearbeiten).
    pub fn width_tool(self) -> Tool {
        if self.editing() {
            Tool::Text
        } else {
            self.tool()
        }
    }

    pub fn text_fx(self) -> (Option<String>, Option<String>, Option<String>) {
        self.prefs.with_untracked(|p| {
            (p.text_fill.then(|| "black".to_string()), p.text_outline.then(|| "white".to_string()), p.text_stroke.then(|| "white".to_string()))
        })
    }

    /// Capso `updateSelectedStyle` – geänderte Eigenschaft auf das ausgewählte Objekt übertragen.
    pub fn apply_to_selected(self, c: Change) {
        let p = self.prefs.get_untracked();
        let (fill, outline, trace) = self.text_fx();
        let w = p.width_for(self.width_tool_untracked());
        self.doc.update(|d| {
            let Some(id) = d.selected else { return };
            let Some(o) = d.get_mut(id) else { return };
            match c {
                Change::Color => o.style.color = p.color.clone(),
                Change::Filled => o.style.filled = p.filled,
                Change::Pattern => o.style.pattern = p.pattern,
                Change::ArrowHead => o.style.arrow_head = p.arrow_head,
                _ => {}
            }
            match (&mut o.shape, c) {
                (Shape::Pixelate { block, .. }, Change::Width) => *block = w,
                (Shape::Pixelate { mode, .. }, Change::Redaction) => *mode = p.redaction_mode,
                (Shape::Counter { radius, .. }, Change::Width) => *radius = w,
                (Shape::Text { .. }, Change::Width) => {}
                (Shape::Text { fill: f, outline: ol, trace: tr, .. }, Change::TextFx) => {
                    *f = fill.clone();
                    *ol = outline.clone();
                    *tr = trace.clone();
                }
                (_, Change::Width) => o.style.line_width = w,
                _ => {}
            }
        });
    }

    fn width_tool_untracked(self) -> Tool {
        if self.text.with_untracked(|t| t.is_some()) {
            Tool::Text
        } else {
            self.tool_untracked()
        }
    }

    pub fn redraw(self) {
        self.tick.update(|t| *t = t.wrapping_add(1));
    }

    /// Texteingabe übernehmen (Capso `commitTextEditing`).
    pub fn commit_text(self) {
        let Some(te) = self.text.get_untracked() else { return };
        self.text.set(None);
        let fs = self.prefs.with_untracked(|p| p.text_font_size);
        let (fill, outline, trace) = self.text_fx();
        let text = te.text.trim().to_string();
        let mut style = self.prefs.with_untracked(|p| p.style_for(Tool::Text));
        style.line_width = 3.0;
        let (w, h) = self.edit_box(&TextEdit { text: text.clone(), ..te.clone() }, fs);
        self.doc.update(|d| {
            if text.is_empty() {
                if let Some(id) = te.id {
                    d.remove(id);
                }
                return;
            }
            let shape = Shape::Text { text, origin: te.origin, font_size: fs, w, h, boxed: true, fill, outline, trace };
            match te.id {
                Some(id) => {
                    d.push_undo();
                    if let Some(o) = d.get_mut(id) {
                        o.shape = shape;
                        o.style.color = style.color.clone();
                    }
                    d.drop_undo_if_unchanged();
                    d.selected = Some(id);
                }
                None => {
                    d.add(style, shape);
                }
            }
        });
        if !self.tool_untracked().is_sticky() && self.tool_untracked() != Tool::Select {
            self.prefs.update(|p| p.tool = Tool::Select);
        }
    }

    /// Texteingabe beginnen – neu an `p` oder für ein bestehendes Textobjekt.
    pub fn begin_text(self, p: Pt, existing: Option<u64>) {
        self.commit_text();
        let mut origin = p;
        let mut text = String::new();
        let mut box_w = None;
        if let Some(id) = existing {
            if let Some(Object { shape: Shape::Text { text: t, origin: o, font_size, w, boxed, fill, outline, trace, .. }, style, .. }) = self.doc.with_untracked(|d| d.get(id).cloned()) {
                origin = o;
                text = t;
                // Gespeicherte Boxbreite beibehalten (alter Text ohne Box: automatisch).
                box_w = boxed.then_some(w);
                self.prefs.update(|pr| {
                    pr.text_font_size = font_size;
                    pr.color = style.color.clone();
                    pr.text_fill = fill.is_some();
                    pr.text_outline = outline.is_some();
                    pr.text_stroke = trace.is_some();
                });
            }
        } else {
            // Neuer Text: Klickpunkt ist die vertikale Mitte der ersten Zeile.
            let fs = self.prefs.with_untracked(|p| p.text_font_size);
            origin = pt(p.x, p.y - fs * TEXT_LINE_HEIGHT / 2.0);
        }
        thread_local!(static SEQ: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });
        let seq = SEQ.with(|c| {
            c.set(c.get().wrapping_add(1));
            c.get()
        });
        self.text.set(Some(TextEdit { id: existing, origin, text, box_w, seq }));
    }

    /// Capso `AnnotationEscapePolicy`. Gibt `true` zurück, wenn Esc verbraucht wurde.
    pub fn escape(self) -> bool {
        let has_sel = self.doc.with_untracked(|d| d.selected.is_some());
        match escape_action(self.text.with_untracked(|t| t.is_some()), self.crop_mode.get_untracked(), self.tool_untracked(), has_sel) {
            EscapeAction::CommitText => self.commit_text(),
            EscapeAction::ExitCrop => self.crop_mode.set(false),
            EscapeAction::SwitchToSelect => self.set_tool(Tool::Select),
            EscapeAction::ClearSelection => self.doc.update(|d| d.selected = None),
            EscapeAction::None => return false,
        }
        true
    }

    pub fn undo(self) {
        self.commit_text();
        self.doc.update(|d| d.undo());
        self.after_xform_change();
    }

    pub fn redo(self) {
        self.commit_text();
        self.doc.update(|d| d.redo());
        self.after_xform_change();
    }

    pub fn after_xform_change(self) {
        let xf = self.doc.with_untracked(|d| d.xform);
        thread_local!(static LAST: std::cell::Cell<Option<Xform>> = const { std::cell::Cell::new(None) });
        let changed = LAST.with(|l| {
            let c = l.get() != Some(xf);
            l.set(Some(xf));
            c
        });
        if changed {
            self.sync_source();
            self.fit.update(|f| *f += 1);
        }
    }

    pub fn zoom_by(self, k: f64) {
        self.zoom.update(|z| *z = (*z * k).clamp(0.1, 4.0));
    }

    /// Fertiges Bild als PNG-data-URL (volle Auflösung).
    pub fn export(self, with_beautify: bool) -> Option<String> {
        self.commit_text();
        let src = self.src.get_value()?;
        let b = self.beautify.get_untracked();
        let c = self.doc.with_untracked(|d| {
            if self.inline {
                // Inline: immer exakt der aufgenommene Bereich (keine Canvas-Vergrößerung).
                let mut d = d.clone();
                d.crop = Some(d.image_rect());
                render::render_export(&d, &src, None)
            } else {
                render::render_export(d, &src, with_beautify.then_some(&b))
            }
        });
        Some(render::png_data_url(&c))
    }

    /// Aktueller Bereich der Anzeige in Bildpixeln (inkl. Beautify-Rand).
    pub fn display_rect(self) -> Rect {
        if self.inline {
            return self.doc.with(|d| d.image_rect());
        }
        self.tick.track();
        let cr = self.doc.with(content_rect);
        let b = self.beautify.get();
        if !matches!(self.drag.get_value(), DragState::None) {
            if let Some(r) = self.last_disp.get_value() {
                return r;
            }
        }
        let r = if b.enabled {
            let (ow, oh, ix, iy) = b.layout(cr.w, cr.h);
            rect(cr.x - ix, cr.y - iy, ow, oh)
        } else {
            cr
        };
        self.last_disp.set_value(Some(r));
        r
    }

    /// Live-Vorschau des gerade aufgezogenen Objekts.
    pub fn preview_object(self) -> Option<Object> {
        let DragState::Create { tool, start, cur, points } = self.drag.get_value() else { return None };
        let k = self.keys.get_value();
        let (s, e) = constrained_drag(start, cur, tool, k.shift, k.center_lock);
        let st = self.style();
        let r = Rect::from_points(s, e);
        let shape = match tool {
            Tool::Arrow => Shape::Arrow { start: s, end: e, ctrl: None },
            Tool::Line => Shape::Line { start: s, end: e, ctrl: None },
            Tool::Rectangle => Shape::Rect { rect: r, radius: 0.0 },
            Tool::Ellipse => Shape::Ellipse { rect: r },
            Tool::Spotlight => Shape::Spotlight { rect: r },
            Tool::Pixelate => Shape::Pixelate { rect: r, block: st.line_width, mode: self.prefs.with_untracked(|p| p.redaction_mode) },
            Tool::Freehand | Tool::Highlighter => Shape::Freehand { points },
            Tool::Counter => {
                let n = self.doc.with_untracked(|d| d.next_counter_number());
                let tip = (start.dist(cur) >= st.line_width + COUNTER_ARROW_CLEARANCE).then_some(cur);
                Shape::Counter { center: start, tip, radius: st.line_width, number: n }
            }
            _ => return None,
        };
        Some(Object { id: 0, style: st, shape })
    }

    /// Objekt aus dem Ziehen erzeugen (Capso `mouseUp`).
    fn finish_create(self, tool: Tool, start: Pt, cur: Pt, points: Vec<Pt>) {
        let z = self.map.get_untracked().z;
        let k = self.keys.get_value();
        let (s, e) = constrained_drag(start, cur, tool, k.shift, k.center_lock);
        let st = self.style();
        let r = Rect::from_points(s, e);
        let min = 3.0 / z;
        let shape = match tool {
            Tool::Arrow if s.dist(e) > min => Some(Shape::Arrow { start: s, end: e, ctrl: None }),
            Tool::Line if s.dist(e) > min => Some(Shape::Line { start: s, end: e, ctrl: None }),
            Tool::Rectangle if r.w > min && r.h > min => Some(Shape::Rect { rect: r, radius: 0.0 }),
            Tool::Ellipse if r.w > min && r.h > min => Some(Shape::Ellipse { rect: r }),
            Tool::Spotlight if r.w > min && r.h > min => Some(Shape::Spotlight { rect: r }),
            Tool::Pixelate if r.w > 5.0 / z && r.h > 5.0 / z => Some(Shape::Pixelate { rect: r, block: st.line_width, mode: self.prefs.with_untracked(|p| p.redaction_mode) }),
            Tool::Freehand | Tool::Highlighter if points.len() > 1 => Some(Shape::Freehand { points }),
            Tool::Counter => {
                let tip = (start.dist(cur) >= st.line_width + COUNTER_ARROW_CLEARANCE).then_some(cur);
                let n = self.doc.with_untracked(|d| d.next_counter_number());
                Some(Shape::Counter { center: start, tip, radius: st.line_width, number: n })
            }
            _ => None,
        };
        if let Some(shape) = shape {
            self.doc.update(|d| {
                d.add(st, shape);
            });
            if !tool.is_sticky() {
                self.set_tool(Tool::Select);
            }
        }
    }

    // ───────── Zeiger ─────────

    pub fn pointer_down(self, p: Pt, dbl: bool) {
        if self.text.with_untracked(|t| t.is_some()) {
            self.commit_text();
            return;
        }
        let z = self.map.get_untracked().z;
        let th = 6.0 / z;
        let tool = self.tool_untracked();
        // Griffe des ausgewählten Objekts haben Vorrang (in jedem Werkzeug).
        if let Some(o) = self.doc.with_untracked(|d| d.selected_object().cloned()) {
            // Zählerknöpfe × / − / + (Capso `counterSelectionAction`), je ein Undo-Schritt.
            if let Some(a) = counter_action_at(&o, p, z) {
                self.doc.update(|d| {
                    d.counter_action(o.id, a);
                });
                self.redraw();
                return;
            }
            if let Some(h) = o.handle_at(p, z) {
                self.doc.update(|d| d.push_undo());
                self.drag.set_value(DragState::Handle { id: o.id, h, orig: o });
                return;
            }
        }
        match tool {
            Tool::Select => {
                let hit = self.doc.with_untracked(|d| d.object_at(p, th));
                if let Some(id) = hit {
                    let is_text = self.doc.with_untracked(|d| matches!(d.get(id).map(|o| &o.shape), Some(Shape::Text { .. })));
                    if dbl && is_text {
                        self.begin_text(p, Some(id));
                        return;
                    }
                    self.doc.update(|d| {
                        d.selected = Some(id);
                        d.push_undo();
                    });
                    self.drag.set_value(DragState::Move { id, last: p, moved: false });
                } else {
                    self.doc.update(|d| d.selected = None);
                }
            }
            Tool::Text => {
                let hit = self.doc.with_untracked(|d| d.object_at(p, th).filter(|id| matches!(d.get(*id).map(|o| &o.shape), Some(Shape::Text { .. }))));
                self.begin_text(p, hit);
            }
            t => {
                self.doc.update(|d| d.selected = None);
                // Intelligenter Marker: Start auf einer Textzeile → waagerecht auf deren Mitte (Capso `highlighterSnapRect`).
                let snap = if t == Tool::Highlighter { highlight_snap_line(&self.text_line_rects(), p) } else { None };
                self.hl_snap.set_value(snap);
                let p = self.snap_point(p);
                self.drag.set_value(DragState::Create { tool: t, start: p, cur: p, points: vec![p] });
            }
        }
        self.redraw();
    }

    /// Punkt auf die eingerastete Markerzeile legen.
    fn snap_point(self, p: Pt) -> Pt {
        match self.hl_snap.get_value() {
            Some(r) => pt(p.x, r.center().y),
            None => p,
        }
    }

    pub fn pointer_move(self, p: Pt) {
        let z = self.map.get_untracked().z;
        match self.drag.get_value() {
            DragState::None => return,
            DragState::Move { id, last, .. } => {
                let (dx, dy) = (p.x - last.x, p.y - last.y);
                self.doc.update(|d| {
                    if let Some(o) = d.get_mut(id) {
                        o.move_by(dx, dy);
                    }
                });
                self.drag.set_value(DragState::Move { id, last: p, moved: true });
            }
            DragState::Handle { id, h, orig } => {
                let shift = self.keys.get_value().shift;
                self.doc.update(|d| {
                    if let Some(o) = d.get_mut(id) {
                        o.drag_handle(&orig, h, p, shift);
                        // Textbox: Text bricht neu um; Box mindestens so hoch wie der Text.
                        if let Shape::Text { text, font_size, w, h: th, boxed: true, .. } = &mut o.shape {
                            let (_, need) = render::measure_wrapped(text, *font_size, (*w - 2.0 * TEXT_BOX_PADDING).max(1.0));
                            *th = th.max(need + 2.0 * TEXT_BOX_PADDING);
                        }
                    }
                });
            }
            DragState::Create { tool, start, mut points, .. } => {
                let p = self.snap_point(p);
                if matches!(tool, Tool::Freehand | Tool::Highlighter) && points.last().is_none_or(|l| l.dist(p) >= 1.0 / z) {
                    points.push(p);
                }
                self.drag.set_value(DragState::Create { tool, start, cur: p, points });
                self.redraw();
            }
        }
    }

    pub fn pointer_up(self, p: Pt) {
        let d = self.drag.get_value();
        self.drag.set_value(DragState::None);
        match d {
            DragState::Move { moved, .. } if !moved => self.doc.update(|d| d.drop_undo_if_unchanged()),
            DragState::Handle { .. } => self.doc.update(|d| d.drop_undo_if_unchanged()),
            DragState::Create { tool, start, mut points, .. } => {
                let p = self.snap_point(p);
                self.hl_snap.set_value(None);
                if matches!(tool, Tool::Freehand | Tool::Highlighter) && points.last() != Some(&p) {
                    points.push(p);
                }
                self.finish_create(tool, start, p, points);
            }
            _ => {}
        }
        self.redraw();
    }

    /// Mauszeiger je nach Position/Werkzeug.
    pub fn cursor_at(self, p: Pt) -> &'static str {
        let z = self.map.get_untracked().z;
        if let Some(o) = self.doc.with_untracked(|d| d.selected_object().cloned()) {
            if counter_action_at(&o, p, z).is_some() {
                return "pointer";
            }
            if let Some(h) = o.handle_at(p, z) {
                return match h {
                    Handle::TopLeft | Handle::BottomRight => "nwse-resize",
                    Handle::TopRight | Handle::BottomLeft => "nesw-resize",
                    _ => "grab",
                };
            }
        }
        match self.tool_untracked() {
            Tool::Select => {
                if self.doc.with_untracked(|d| d.object_at(p, 6.0 / z).is_some()) {
                    "move"
                } else {
                    "default"
                }
            }
            Tool::Text => "text",
            _ => "crosshair",
        }
    }

    // ───────── Tastatur ─────────

    /// Tastenkürzel des Editors (Capso `AnnotationToolShortcutMonitor`, Canvas `keyDown`, Toolbar-Shortcuts).
    /// `action` wird für Ausgabe-Aktionen aufgerufen ("copy" | "save" | "saveAs" | "pin" | "close").
    pub fn handle_key(self, e: &web_sys::KeyboardEvent, action: &dyn Fn(&str)) {
        let key = e.key();
        let cmd = e.meta_key() || e.ctrl_key();
        let shift = e.shift_key();
        self.update_keys(e, true);
        let editing = self.text.with_untracked(|t| t.is_some());
        if self.crop_mode.get_untracked() {
            if key == "Escape" {
                e.prevent_default();
                self.crop_mode.set(false);
            }
            return;
        }
        if editing {
            match key.as_str() {
                "Escape" => {
                    e.prevent_default();
                    self.commit_text();
                }
                "Enter" if cmd => {
                    e.prevent_default();
                    self.commit_text();
                }
                _ => {}
            }
            return;
        }
        // Eingabefelder der Leisten (Regler, Auswahl, Zahlen) behalten ihre Tasten.
        let in_field = e
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "SELECT" | "TEXTAREA"));
        if in_field && key != "Escape" && !cmd {
            return;
        }
        let lower = key.to_lowercase();
        let consumed = match (cmd, shift, lower.as_str()) {
            (true, false, "z") => {
                self.undo();
                true
            }
            (true, true, "z") => {
                self.redo();
                true
            }
            (true, false, "c") => {
                // Objekt kopieren; ohne Auswahl → Bild kopieren und schließen.
                if !self.doc.try_update(|d| d.copy_selection()).unwrap_or(false) {
                    action("copy");
                }
                true
            }
            (true, true, "c") => {
                action("copy");
                true
            }
            (true, false, "v") => {
                let off = 12.0 / self.map.get_untracked().z;
                self.doc.update(|d| {
                    d.paste(off);
                });
                true
            }
            (true, false, "d") => {
                let off = 12.0 / self.map.get_untracked().z;
                self.doc.update(|d| {
                    d.duplicate_selected(off);
                });
                true
            }
            (true, false, "s") => {
                action("save");
                true
            }
            (true, true, "s") => {
                action("saveAs");
                true
            }
            (true, false, "p") => {
                action("pin");
                true
            }
            (true, _, "+" | "=") => {
                self.zoom_by(1.25);
                true
            }
            (true, _, "-") => {
                self.zoom_by(1.0 / 1.25);
                true
            }
            (true, _, "0") => {
                self.fit.update(|f| *f += 1);
                true
            }
            (false, _, "escape") => self.escape(),
            (false, false, "enter") => {
                action("copy");
                true
            }
            (false, _, "backspace" | "delete") => {
                self.doc.update(|d| {
                    d.remove_selected();
                });
                true
            }
            (false, _, _) if !e.alt_key() => {
                if let Some(t) = tool_for_key(&key, shift, &self.center_key.get_value()) {
                    self.set_tool(t);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if consumed {
            e.prevent_default();
        }
    }

    /// Shift / Zentrier-Taste live verfolgen (Vorschau rastet sofort ein/aus).
    pub fn update_keys(self, e: &web_sys::KeyboardEvent, down: bool) {
        let mut k = self.keys.get_value();
        k.shift = e.shift_key();
        if e.key().eq_ignore_ascii_case(&self.center_key.get_value()) && !e.meta_key() && !e.ctrl_key() && !e.alt_key() {
            k.center_lock = down;
        }
        self.keys.set_value(k);
        if !matches!(self.drag.get_value(), DragState::None) {
            self.redraw();
        }
    }

    pub fn save_presets(self) {
        api::set_setting("annotationStylePresets", serde_json::to_value(self.presets.get_untracked()).unwrap_or(json!([])));
    }

    pub fn save_current_as_preset(self) {
        let p = self.prefs.get_untracked();
        let t = self.width_tool_untracked();
        let preset = StylePreset {
            id: format!("{:x}", (js_sys::Math::random() * 1e15) as u64),
            tool: t.id().into(),
            color: p.color.clone(),
            line_width: p.width_for(t),
            filled: p.filled,
            pattern: p.pattern,
            arrow_head: p.arrow_head,
            text_fill: p.text_fill,
            text_outline: p.text_outline,
            text_stroke: p.text_stroke,
        };
        self.presets.update(|l| {
            l.retain(|x| StylePreset { id: preset.id.clone(), ..x.clone() } != preset);
            l.push(preset);
        });
        self.save_presets();
    }

    pub fn apply_preset(self, pr: &StylePreset) {
        let t = Tool::from_id(&pr.tool).unwrap_or(Tool::Arrow);
        if t != self.tool_untracked() {
            self.doc.update(|d| d.selected = None);
        }
        self.commit_text();
        self.prefs.update(|p| {
            p.tool = t;
            p.color = pr.color.clone();
            p.filled = pr.filled;
            p.pattern = pr.pattern;
            p.arrow_head = pr.arrow_head;
            p.text_fill = pr.text_fill;
            p.text_outline = pr.text_outline;
            p.text_stroke = pr.text_stroke;
            p.set_width_for(t, pr.line_width);
        });
    }

    /// Ganze Szene zeichnen.
    pub fn paint(self, g: &web_sys::CanvasRenderingContext2d, cw: f64, ch: f64) {
        let m = self.map.get_untracked();
        let _ = g.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        g.clear_rect(0.0, 0.0, cw, ch);
        let Some(src) = self.src.get_value() else { return };
        m.apply(g);
        let editing = self.text.get_untracked().and_then(|t| t.id);
        let preview = self.preview_object();
        self.doc.with_untracked(|d| {
            let cr = content_rect(d);
            let b = self.beautify.get_untracked();
            if b.enabled && !self.inline {
                let (ow, oh, ix, iy) = b.layout(cr.w, cr.h);
                render::draw_beautify_bg(g, &b, rect(cr.x - ix, cr.y - iy, ow, oh), &src);
                render::begin_beautify_image(g, &b, cr, m.z * m.dpr);
                render::draw_doc(g, &src, d, editing, preview.as_ref());
                g.restore();
            } else {
                if !self.inline {
                    // Schachbrett, wo die Zeichenfläche über das Bild hinausgeht (Capso `drawTransparencyChecker`).
                    let css = |r: Rect| {
                        let o = m.to_css(pt(r.x, r.y));
                        rect(o.x, o.y, r.w * m.z, r.h * m.z)
                    };
                    render::draw_checker(g, m.dpr, css(cr), css(d.image_rect()));
                    m.apply(g);
                }
                g.save();
                if d.crop.is_some() || self.inline {
                    g.begin_path();
                    g.rect(cr.x, cr.y, cr.w, cr.h);
                    g.clip();
                }
                render::draw_doc(g, &src, d, editing, preview.as_ref());
                g.restore();
            }
            if let Some(o) = d.selected_object() {
                if editing != Some(o.id) {
                    render::draw_selection(g, o, m.z);
                }
            }
        });
    }
}

// ───────────────────────────── Zeichenfläche ─────────────────────────────

/// Scrollbare, zoombare Zeichenfläche. `margin` = Rand um den Inhalt in CSS-px.
#[component]
pub fn AnnotCanvas(ed: Ed, #[prop(optional)] margin: f64) -> impl IntoView {
    let outer = NodeRef::<html::Div>::new();
    let scroller = NodeRef::<html::Div>::new();
    let canvas_ref = NodeRef::<html::Canvas>::new();
    let vp = RwSignal::new((0.0f64, 0.0f64));
    let scroll = RwSignal::new((0.0f64, 0.0f64));
    let cursor = RwSignal::new("default");

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

    // An Fenster anpassen (Capso `fitToWindow`: nie größer als 100 %).
    let fitted = StoredValue::new(None::<u32>);
    Effect::new(move |_| {
        let n = ed.fit.get();
        let (w, h) = vp.get();
        if w <= 0.0 || !ed.loaded.get() || fitted.get_value() == Some(n) {
            return;
        }
        fitted.set_value(Some(n));
        let d = untrack(|| ed.display_rect());
        let z = ((w - 2.0 * margin) / d.w).min((h - 2.0 * margin) / d.h);
        ed.zoom.set(if ed.inline { z } else { z.min(1.0).max(0.05) });
    });

    // Abbildung berechnen
    let content = Memo::new(move |_| ed.display_rect());
    let spacer = move || {
        let d = content.get();
        let z = ed.zoom.get();
        (d.w * z + 2.0 * margin, d.h * z + 2.0 * margin)
    };
    Effect::new(move |_| {
        let d = content.get();
        let z = ed.zoom.get();
        let (vw, vh) = vp.get();
        let (sl, st) = scroll.get();
        let ox = margin.max((vw - d.w * z) / 2.0) - sl - d.x * z;
        let oy = margin.max((vh - d.h * z) / 2.0) - st - d.y * z;
        let dpr = web_sys::window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0);
        ed.map.set(Map { z, ox, oy, dpr });
    });

    // Zeichnen (einmal pro Frame)
    let pending = StoredValue::new(false);
    Effect::new(move |_| {
        ed.doc.track();
        ed.tick.track();
        ed.text.track();
        ed.beautify.track();
        ed.prefs.track();
        let m = ed.map.get();
        let (vw, vh) = vp.get();
        let Some(c) = canvas_ref.get() else { return };
        let (pw, ph) = ((vw * m.dpr).round() as u32, (vh * m.dpr).round() as u32);
        if c.width() != pw || c.height() != ph {
            c.set_width(pw);
            c.set_height(ph);
        }
        if pending.get_value() {
            return;
        }
        pending.set_value(true);
        request_animation_frame(move || {
            pending.set_value(false);
            if let Some(c) = canvas_ref.get_untracked() {
                let g = render::ctx(&c);
                ed.paint(&g, c.width() as f64, c.height() as f64);
            }
        });
    });

    // Zoom-Änderung: Punkt unter dem Zeiger festhalten
    let zoom_at = move |k: f64, cx: f64, cy: f64| {
        let m = ed.map.get_untracked();
        let p = m.to_img(cx, cy);
        ed.zoom_by(k);
        let z = ed.zoom.get_untracked();
        let d = content.get_untracked();
        let (vw, vh) = vp.get_untracked();
        let sl = (p.x - d.x) * z + margin.max((vw - d.w * z) / 2.0) - cx;
        let st = (p.y - d.y) * z + margin.max((vh - d.h * z) / 2.0) - cy;
        request_animation_frame(move || {
            if let Some(s) = scroller.get_untracked() {
                s.set_scroll_left(sl.max(0.0) as i32);
                s.set_scroll_top(st.max(0.0) as i32);
            }
        });
    };

    // Trackpad-Pinch (WebKit GestureEvent)
    Effect::new(move |_| {
        let Some(s) = scroller.get() else { return };
        let last = StoredValue::new(1.0f64);
        let start = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
            e.prevent_default();
            last.set_value(1.0);
        });
        let change = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
            e.prevent_default();
            let sc = js_sys::Reflect::get(&e, &"scale".into()).ok().and_then(|v| v.as_f64()).unwrap_or(1.0);
            let k = sc / last.get_value();
            last.set_value(sc);
            let (vw, vh) = vp.get_untracked();
            zoom_at(k, vw / 2.0, vh / 2.0);
        });
        let _ = s.add_event_listener_with_callback("gesturestart", start.as_ref().unchecked_ref());
        let _ = s.add_event_listener_with_callback("gesturechange", change.as_ref().unchecked_ref());
        start.forget();
        change.forget();
    });

    let local = move |e: &web_sys::MouseEvent| -> Pt {
        let r = outer.get_untracked().map(|o| o.get_bounding_client_rect());
        let (x, y) = r.map(|r| (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top())).unwrap_or_default();
        ed.map.get_untracked().to_img(x, y)
    };

    let on_down = move |e: ev::PointerEvent| {
        if e.button() != 0 {
            return;
        }
        e.prevent_default();
        if let Some(s) = scroller.get_untracked() {
            let _ = s.focus();
            let _ = s.set_pointer_capture(e.pointer_id());
        }
        let p = local(&e);
        ed.keys.update_value(|k| k.shift = e.shift_key());
        ed.pointer_down(p, e.detail() >= 2);
    };
    let on_move = move |e: ev::PointerEvent| {
        let p = local(&e);
        ed.keys.update_value(|k| k.shift = e.shift_key());
        if matches!(ed.drag.get_value(), DragState::None) {
            cursor.set(ed.cursor_at(p));
        } else {
            ed.pointer_move(p);
        }
    };
    let on_up = move |e: ev::PointerEvent| {
        if matches!(ed.drag.get_value(), DragState::None) {
            return;
        }
        let p = local(&e);
        ed.pointer_up(p);
    };
    // Doppelklick (PointerEvent.detail ist in WebKit 0) → Text bearbeiten
    let on_dbl = move |e: ev::MouseEvent| {
        if ed.text.with_untracked(|t| t.is_some()) {
            return;
        }
        let p = local(&e);
        if ed.tool_untracked() == Tool::Select {
            let id = ed.doc.with_untracked(|d| d.object_at(p, 6.0 / ed.map.get_untracked().z).filter(|id| matches!(d.get(*id).map(|o| &o.shape), Some(Shape::Text { .. }))));
            if id.is_some() {
                ed.drag.set_value(DragState::None);
                ed.doc.update(|d| d.drop_undo_if_unchanged());
                ed.begin_text(p, id);
            }
        }
    };
    let on_wheel = move |e: ev::WheelEvent| {
        if e.ctrl_key() || e.meta_key() {
            e.prevent_default();
            let r = outer.get_untracked().map(|o| o.get_bounding_client_rect());
            let (x, y) = r.map(|r| (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top())).unwrap_or_default();
            zoom_at((-e.delta_y() * 0.01).exp(), x, y);
        }
    };

    view! {
        <div node_ref=outer class="relative size-full overflow-hidden">
            <div node_ref=scroller tabindex="-1"
                class="absolute inset-0 overflow-auto outline-none"
                style:cursor=move || cursor.get()
                on:scroll=move |_| { if let Some(s) = scroller.get_untracked() { scroll.set((s.scroll_left() as f64, s.scroll_top() as f64)) } }
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_up
                on:dblclick=on_dbl
                on:wheel=on_wheel
            >
                <div style:width=move || format!("{}px", spacer().0) style:height=move || format!("{}px", spacer().1)></div>
            </div>
            <canvas node_ref=canvas_ref class="pointer-events-none absolute left-0 top-0"
                style:width=move || format!("{}px", vp.get().0) style:height=move || format!("{}px", vp.get().1)></canvas>
            <TextEditorOverlay ed=ed />
        </div>
    }
}

/// Textfeld über der Zeichenfläche (Capso `AnnotationTextEditor`): Textbox mit automatischem Umbruch am rechten Rand,
/// vier Eckgriffe zum Ändern der Boxbreite während der Eingabe (Capso `liveTextHandleCenters` + `TextResizeGeometry`).
#[component]
fn TextEditorOverlay(ed: Ed) -> impl IntoView {
    let ta = NodeRef::<html::Textarea>::new();
    // Identität der Bearbeitung: beim Wechsel Wert setzen und fokussieren.
    let key = Memo::new(move |_| ed.text.with(|t| t.as_ref().map(|t| (t.id, t.seq))));
    Effect::new(move |_| {
        ed.refocus.track();
        if key.get().is_none() {
            return;
        }
        let text = ed.text.with_untracked(|t| t.as_ref().map(|t| t.text.clone()).unwrap_or_default());
        request_animation_frame(move || {
            if let Some(t) = ta.get_untracked() {
                if t.value() != text {
                    t.set_value(&text);
                    let len = text.encode_utf16().count() as u32;
                    let _ = t.set_selection_range(len, len);
                }
                let _ = t.focus();
            }
        });
    });
    // (Position in CSS-px, Boxgröße in CSS-px, Schriftgröße in CSS-px, Zoom)
    let geo = Memo::new(move |_| {
        let m = ed.map.get();
        ed.doc.track();
        let fs_img = ed.prefs.with(|p| p.text_font_size);
        let Some(te) = ed.text.get() else { return (pt(0.0, 0.0), (0.0, 0.0), fs_img * m.z, m.z) };
        let (w, h) = ed.edit_box(&te, fs_img);
        (m.to_css(te.origin), (w * m.z, h * m.z), fs_img * m.z, m.z)
    });
    // Griff ziehen: (Griff, Startpunkt CSS, ursprüngliche Box in Bildpixeln)
    let resizing = StoredValue::new(None::<(Handle, Pt, Rect)>);
    let handle_view = move |h: Handle| {
        let pos = move || {
            let (p, (w, hh), _, _) = geo.get();
            let (x, y) = match h {
                Handle::TopLeft => (p.x, p.y),
                Handle::TopRight => (p.x + w, p.y),
                Handle::BottomLeft => (p.x, p.y + hh),
                _ => (p.x + w, p.y + hh),
            };
            (x - 4.5, y - 4.5)
        };
        let cursor = if matches!(h, Handle::TopLeft | Handle::BottomRight) { "nwse-resize" } else { "nesw-resize" };
        view! {
            <div class="absolute size-[9px] rounded-[2px] border-[1.5px] border-[#0a84ff] bg-white"
                style:left=move || format!("{}px", pos().0) style:top=move || format!("{}px", pos().1) style:cursor=cursor
                on:pointerdown=move |e: ev::PointerEvent| {
                    e.prevent_default();
                    e.stop_propagation();
                    if let Some(el) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let _ = el.set_pointer_capture(e.pointer_id());
                    }
                    let Some(te) = ed.text.get_untracked() else { return };
                    let fs = ed.prefs.with_untracked(|p| p.text_font_size);
                    let (w, hh) = ed.edit_box(&te, fs);
                    resizing.set_value(Some((h, pt(e.client_x() as f64, e.client_y() as f64), rect(te.origin.x, te.origin.y, w, hh))));
                }
                on:pointermove=move |e: ev::PointerEvent| {
                    let Some((h, start, orig)) = resizing.get_value() else { return };
                    let z = ed.map.with_untracked(|m| m.z).max(0.01);
                    let d = pt((e.client_x() as f64 - start.x) / z, (e.client_y() as f64 - start.y) / z);
                    let fs = ed.prefs.with_untracked(|p| p.text_font_size);
                    let r = text_resize_rect(orig, h, d, text_box_min(fs));
                    ed.text.update(|t| if let Some(t) = t {
                        t.origin = pt(r.x, r.y);
                        t.box_w = Some(r.w);
                    });
                }
                on:pointerup=move |_| { resizing.set_value(None); ed.refocus.update(|r| *r += 1); }
                on:pointercancel=move |_| resizing.set_value(None)
            ></div>
        }
    };
    view! {
        <textarea node_ref=ta spellcheck="false"
            class="absolute resize-none overflow-hidden whitespace-pre-wrap break-words m-0 border-0 outline-none"
            style:display=move || if key.get().is_some() { "block" } else { "none" }
            style:left=move || format!("{}px", geo.get().0.x)
            style:top=move || format!("{}px", geo.get().0.y)
            style:width=move || format!("{}px", geo.get().1.0)
            style:height=move || format!("{}px", geo.get().1.1)
            style:padding=move || format!("{}px", TEXT_BOX_PADDING * geo.get().3)
            style:box-sizing="border-box"
            style:overflow-wrap="anywhere"
            style:font=move || render::text_font(geo.get().2)
            style:line-height=TEXT_LINE_HEIGHT.to_string()
            style:color=move || ed.prefs.with(|p| css_rgba(&p.color, 1.0))
            style:background=move || if ed.prefs.with(|p| p.text_fill) { "rgba(0,0,0,0.5)" } else { "transparent" }
            style:outline=move || if ed.prefs.with(|p| p.text_outline) { "2px solid white" } else { "1px dashed rgba(10,132,255,.9)" }
            style:border-radius="4px"
            style:paint-order="stroke fill"
            style:-webkit-text-stroke=move || if ed.prefs.with(|p| p.text_stroke) { format!("{}px white", (geo.get().2 * 0.03).max(0.5)) } else { "0".into() }
            style:caret-color=ACCENT
            on:input=move |e| {
                let v = event_target_value(&e);
                ed.text.update(|t| if let Some(t) = t { t.text = v });
            }
            on:pointerdown=|e| e.stop_propagation()
        ></textarea>
        <Show when=move || key.get().is_some()>
            {handle_view(Handle::TopLeft)}
            {handle_view(Handle::TopRight)}
            {handle_view(Handle::BottomLeft)}
            {handle_view(Handle::BottomRight)}
        </Show>
    }
}
