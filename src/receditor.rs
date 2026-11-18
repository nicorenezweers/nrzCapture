//! Aufnahme-Editor (Frontend) – Port von Capso `RecordingEditorView` + `EditorCoordinator`.
//! Layout wie Capso: links Vorschau (Live-Komposition auf `<canvas>` aus `<video>`), darunter Wiedergabe-Steuerung
//! und Zeitleiste (Trim-Griffe, Schnitte, Zoom-, Weichzeichner-Spur), rechts das Einstellungs-Panel (280 px).
//! Die Logik (Projektmodell, Zoom, Cursor, Auto-Zoom, Layout) teilt sich das Frontend mit dem Backend-Export.
#[path = "../src-tauri/src/receditor/model.rs"]
pub mod model;
mod panel;
mod preview;
mod timeline;

use crate::api::{self, t};
use leptos::{ev, prelude::*, task::spawn_local};
use model::*;
use serde_json::{json, Value};
use std::time::Duration;

pub fn route(r: &str) -> Option<AnyView> {
    (r == "receditor").then(|| view! { <RecEditor /> }.into_any())
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sel {
    Effect(String),
    Cut(String),
}

/// Gemeinsamer Editor-Zustand (Copy, an alle Teilansichten weitergereicht).
#[derive(Clone, Copy)]
pub struct Ed {
    pub video: RwSignal<String>,
    pub loaded: RwSignal<bool>,
    pub project: RwSignal<RecordingProject>,
    pub events: StoredValue<Vec<CursorEvent>>,
    pub has_tel: RwSignal<bool>,
    pub engine: StoredValue<Option<Engine>>,
    /// Aktuelle Quellzeit (s).
    pub time: RwSignal<f64>,
    pub playing: RwSignal<bool>,
    pub sel: RwSignal<Option<Sel>>,
    /// (Fortschritt 0–1, Status) während eines Exports.
    pub export: RwSignal<Option<(f64, String)>>,
    pub export_copy: RwSignal<bool>,
    pub format: RwSignal<String>,
    pub quality: RwSignal<String>,
    pub error: RwSignal<Option<String>>,
    pub video_ref: NodeRef<leptos::html::Video>,
    pub canvas_ref: NodeRef<leptos::html::Canvas>,
    /// Vorschau neu zeichnen.
    pub dirty: StoredValue<bool>,
    /// Layout der Vorschau in CSS-Pixeln (für Overlays).
    pub view_layout: RwSignal<Option<Layout>>,
}

impl Ed {
    fn new() -> Self {
        let s = |k: &str, d: &str| {
            let v = api::setting_str(k);
            if v.is_empty() {
                d.to_owned()
            } else {
                v
            }
        };
        Self {
            video: RwSignal::new(String::new()),
            loaded: RwSignal::new(false),
            project: RwSignal::new(RecordingProject::default()),
            events: StoredValue::new(vec![]),
            has_tel: RwSignal::new(false),
            engine: StoredValue::new(None),
            time: RwSignal::new(0.0),
            playing: RwSignal::new(false),
            sel: RwSignal::new(None),
            export: RwSignal::new(None),
            export_copy: RwSignal::new(false),
            format: RwSignal::new(s("recordingFormat", "mp4")),
            quality: RwSignal::new(s("exportQuality", "maximum")),
            error: RwSignal::new(None),
            video_ref: NodeRef::new(),
            canvas_ref: NodeRef::new(),
            dirty: StoredValue::new(true),
            view_layout: RwSignal::new(None),
        }
    }

    pub fn duration(&self) -> f64 {
        self.project.with_untracked(|p| p.video_duration)
    }

    pub fn mark_dirty(&self) {
        self.dirty.set_value(true);
    }

    /// Springt zur Zeit `t` (Schnitte werden übersprungen, wie EditorCoordinator.seek).
    pub fn seek(&self, t: f64) {
        let d = self.duration();
        let t = self.project.with_untracked(|p| skip_trim_regions(&p.trim_regions, t.clamp(0.0, d), d));
        if let Some(v) = self.video_ref.get_untracked() {
            v.set_current_time(t);
        }
        self.time.set(t);
        self.mark_dirty();
    }

    pub fn play(&self) {
        let Some(v) = self.video_ref.get_untracked() else { return };
        let (start, end) = self.project.with_untracked(|p| (p.effective_start(), p.effective_end()));
        if self.time.get_untracked() >= end - 0.05 || self.time.get_untracked() < start {
            self.seek(start);
        }
        let _ = v.play();
        self.playing.set(true);
    }

    pub fn pause(&self) {
        if let Some(v) = self.video_ref.get_untracked() {
            let _ = v.pause();
        }
        self.playing.set(false);
    }

    pub fn toggle_play(&self) {
        if self.playing.get_untracked() {
            self.pause()
        } else {
            self.play()
        }
    }

    pub fn selected_effect(&self) -> Option<RecordingEffectSegment> {
        let Some(Sel::Effect(id)) = self.sel.get() else { return None };
        self.project.with(|p| p.effect_segments.iter().find(|e| e.id == id).cloned())
    }

    pub fn selected_cut(&self) -> Option<TrimRegion> {
        let Some(Sel::Cut(id)) = self.sel.get() else { return None };
        self.project.with(|p| p.trim_regions.iter().find(|r| r.id == id).cloned())
    }

    pub fn update_effect(&self, id: &str, f: impl FnOnce(&mut RecordingEffectSegment)) {
        self.project.update(|p| {
            if let Some(e) = p.effect_segments.iter_mut().find(|e| e.id == id) {
                f(e);
            }
        });
    }

    pub fn add_cut(&self, at: f64) {
        let d = self.duration();
        let len = 1.0f64.min(d - at);
        if len <= 0.2 {
            return;
        }
        let r = TrimRegion::new(at, at + len);
        let id = r.id.clone();
        self.project.update(|p| p.trim_regions.push(r));
        self.sel.set(Some(Sel::Cut(id)));
        if self.time.get_untracked() >= at && self.time.get_untracked() < at + len {
            self.seek(at + len);
        }
    }

    pub fn add_zoom(&self, at: f64) {
        let d = self.duration();
        let len = 3.0f64.min(d - at);
        if len <= 0.5 {
            return;
        }
        let z = ZoomSegment::new(at, at + len, 1.5, ZoomFocusMode::FollowCursor);
        let seg = RecordingEffectSegment::zoom(&z);
        let id = seg.id.clone();
        self.project.update(|p| p.effect_segments.push(seg));
        self.sel.set(Some(Sel::Effect(id)));
    }

    pub fn add_blur(&self, at: f64) {
        let d = self.duration();
        let len = 3.0f64.min(d - at);
        if len <= 0.5 {
            return;
        }
        let seg = RecordingEffectSegment::blur(at, at + len);
        let id = seg.id.clone();
        self.project.update(|p| p.effect_segments.push(seg));
        self.sel.set(Some(Sel::Effect(id)));
    }

    pub fn remove_selected(&self) {
        match self.sel.get_untracked() {
            Some(Sel::Effect(id)) => self.project.update(|p| p.effect_segments.retain(|e| e.id != id)),
            Some(Sel::Cut(id)) => self.project.update(|p| p.trim_regions.retain(|r| r.id != id)),
            None => return,
        }
        self.sel.set(None);
    }

    /// Auto-Zoom (EditorCoordinator.autoZoom): ersetzt bisherige Vorschläge, behält manuelle Zooms.
    pub fn auto_zoom(&self) -> usize {
        let d = self.duration();
        let suggested = self.events.with_value(|e| AutoZoomDetector::detect(e, d));
        let n = suggested.len();
        self.project.update(|p| {
            let mut zooms: Vec<ZoomSegment> = p.zoom_segments().into_iter().filter(|z| z.source != SegmentSource::Auto).collect();
            zooms.extend(suggested);
            zooms.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());
            replace_zoom_segments(&mut p.effect_segments, &zooms);
        });
        if let Some(Sel::Effect(id)) = self.sel.get_untracked() {
            if !self.project.with_untracked(|p| p.effect_segments.iter().any(|e| e.id == id)) {
                self.sel.set(None);
            }
        }
        n
    }

    pub fn start_export(&self, copy: bool) {
        if self.export.get_untracked().is_some() {
            return;
        }
        self.pause();
        let ed = *self;
        ed.export_copy.set(copy);
        ed.export.set(Some((0.0, t("Export wird vorbereitet …", "Preparing export…").into())));
        let args = json!({
            "video": ed.video.get_untracked(),
            "project": ed.project.get_untracked(),
            "format": ed.format.get_untracked(),
            "quality": ed.quality.get_untracked(),
            "copy": copy,
        });
        spawn_local(async move {
            let _ = api::rpc("receditor", "save", json!({ "video": ed.video.get_untracked(), "project": ed.project.get_untracked() })).await;
            match api::rpc("receditor", "export", args).await {
                // Sichern-Dialog abgebrochen
                Ok(v) if v["cancelled"].as_bool() == Some(true) => ed.export.set(None),
                Ok(_) => {}
                Err(e) => {
                    ed.export.set(None);
                    ed.error.set(Some(e));
                }
            }
        });
    }
}

fn save_later(ed: Ed, gen: StoredValue<u64>) {
    gen.update_value(|g| *g += 1);
    let mine = gen.get_value();
    set_timeout(
        move || {
            if gen.get_value() != mine || !ed.loaded.get_untracked() {
                return;
            }
            let args = json!({ "video": ed.video.get_untracked(), "project": ed.project.get_untracked() });
            spawn_local(async move {
                let _ = api::rpc("receditor", "save", args).await;
            });
        },
        Duration::from_millis(400),
    );
}

/// Wiedergabe-Schleife: Schnitte überspringen, Zeit nachführen, Vorschau zeichnen.
fn frame_loop(ed: Ed) {
    request_animation_frame(move || {
        if let Some(v) = ed.video_ref.get_untracked() {
            if ed.playing.get_untracked() {
                let t = v.current_time();
                let (d, end, start, skip) = ed.project.with_untracked(|p| (p.video_duration, p.effective_end(), p.effective_start(), skip_trim_regions(&p.trim_regions, t, p.video_duration)));
                if skip >= end - 0.01 || v.ended() || t >= d {
                    ed.pause();
                    ed.seek(start.min(end));
                } else {
                    if skip > t + 0.01 {
                        v.set_current_time(skip);
                    }
                    ed.time.set(skip);
                }
                ed.mark_dirty();
            }
        }
        if ed.dirty.get_value() {
            ed.dirty.set_value(false);
            preview::draw(ed);
        }
        frame_loop(ed);
    });
}

#[component]
fn RecEditor() -> impl IntoView {
    let ed = Ed::new();
    model::seed_ids(js_sys::Date::now() as u64);
    let gen = StoredValue::new(0u64);

    // Laden
    spawn_local(async move {
        let data = api::window_data().await;
        let video = data["video"].as_str().unwrap_or_default().to_owned();
        ed.video.set(video.clone());
        match api::rpc("receditor", "load", json!({ "video": video })).await {
            Ok(v) => {
                let events: Vec<CursorEvent> = serde_json::from_value(v["events"].clone()).unwrap_or_default();
                ed.has_tel.set(!events.is_empty());
                // Mit „GIF“ aufgenommen → Editor exportiert standardmäßig GIF (Capso EditorOutputFormat).
                if let Some(f) = v["outputFormat"].as_str().filter(|f| *f == "gif" || *f == "mp4") {
                    ed.format.set(f.to_owned());
                }
                ed.events.set_value(events);
                if let Ok(p) = serde_json::from_value::<RecordingProject>(v["project"].clone()) {
                    let start = p.effective_start();
                    ed.project.set(p);
                    ed.time.set(start);
                }
                ed.loaded.set(true);
            }
            Err(e) => ed.error.set(Some(e)),
        }
    });

    // Engine neu aufbauen + speichern, sobald sich das Projekt ändert
    Effect::new(move |_| {
        let p = ed.project.get();
        if !ed.loaded.get() {
            return;
        }
        let engine = ed.events.with_value(|e| Engine::new(p, e));
        ed.engine.set_value(Some(engine));
        ed.mark_dirty();
        save_later(ed, gen);
    });
    Effect::new(move |_| {
        ed.sel.track();
        ed.time.track();
        ed.mark_dirty();
    });

    // Export-Events
    api::on("receditor-progress", move |v: Value| {
        if ed.export.get_untracked().is_some() {
            ed.export.set(Some((v["progress"].as_f64().unwrap_or(0.0), v["stage"].as_str().unwrap_or("").to_owned())));
        }
    });
    api::on("receditor-done", move |_v: Value| {
        ed.export.set(None);
        // Wie Capso (EditorPlaybackControls): nach Sichern und nach Kopieren schließt der Editor.
        api::close_window();
    });
    api::on("receditor-error", move |v: Value| {
        ed.export.set(None);
        if !v["cancelled"].as_bool().unwrap_or(false) {
            ed.error.set(Some(v["message"].as_str().unwrap_or("Export fehlgeschlagen").to_owned()));
        }
    });

    // Tastatur: Leertaste = Wiedergabe, ⌫ = Auswahl löschen, Esc = Auswahl aufheben, ←/→ = 1/10 s
    let keys = window_event_listener(ev::keydown, move |e| {
        let tag = e.target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()).map(|el| el.tag_name()).unwrap_or_default();
        if tag == "INPUT" || tag == "SELECT" || tag == "TEXTAREA" || ed.export.get_untracked().is_some() {
            return;
        }
        match e.key().as_str() {
            " " => {
                e.prevent_default();
                ed.toggle_play();
            }
            "Backspace" | "Delete" => ed.remove_selected(),
            "Escape" => ed.sel.set(None),
            "ArrowLeft" => ed.seek(ed.time.get_untracked() - if e.shift_key() { 1.0 } else { 0.1 }),
            "ArrowRight" => ed.seek(ed.time.get_untracked() + if e.shift_key() { 1.0 } else { 0.1 }),
            _ => {}
        }
    });
    let resize = window_event_listener(ev::resize, move |_| ed.mark_dirty());
    on_cleanup(move || {
        keys.remove();
        resize.remove();
    });
    frame_loop(ed);

    view! {
        <div class="app-window flex h-screen select-none flex-col overflow-hidden text-[13px]">
            <div class="h-7 shrink-0" />
            <div class="flex min-h-0 flex-1">
                <div class="flex min-w-0 flex-1 flex-col">
                    <div class="relative min-h-0 flex-1 p-4">
                        <preview::Preview ed=ed />
                        {move || ed.export.get().map(|(p, stage)| view! { <ExportOverlay ed=ed progress=p stage=stage /> })}
                        {move || ed.error.get().map(|e| view! {
                            <div class="absolute inset-x-4 top-4 flex items-center justify-between gap-3 rounded-lg bg-red-500/90 px-3 py-2 text-white shadow">
                                <span class="truncate">{e}</span>
                                <button class="text-white/80 hover:text-white" on:click=move |_| ed.error.set(None)>"✕"</button>
                            </div>
                        })}
                    </div>
                    <div class="h-px shrink-0 bg-black/10 dark:bg-white/10" />
                    <div class="flex h-[226px] shrink-0 flex-col gap-2 px-4 py-3">
                        <timeline::PlaybackControls ed=ed />
                        <timeline::Timeline ed=ed />
                    </div>
                </div>
                <div class="w-px shrink-0 bg-black/10 dark:bg-white/10" />
                <panel::SettingsPanel ed=ed />
            </div>
        </div>
    }
}

#[component]
fn ExportOverlay(ed: Ed, progress: f64, stage: String) -> impl IntoView {
    view! {
        <div class="absolute inset-0 flex items-center justify-center bg-black/25">
            <div class="flex w-[260px] flex-col items-center gap-3 rounded-2xl bg-white/90 px-5 py-4 shadow-xl backdrop-blur dark:bg-zinc-800/90">
                <div class="h-1.5 w-[220px] overflow-hidden rounded-full bg-black/10 dark:bg-white/15">
                    <div class="h-full rounded-full bg-blue-500 transition-[width]" style=format!("width:{:.1}%", progress * 100.0) />
                </div>
                <div class="text-[13px] font-medium">{stage}" "{format!("{:.0} %", progress * 100.0)}</div>
                <div class="text-center text-[11px] text-black/50 dark:text-white/50">
                    {move || if ed.export_copy.get() {
                        t("Das Ergebnis wird in die Zwischenablage kopiert.", "The result will be copied to the clipboard.")
                    } else {
                        t("Der Editor schließt, sobald der Export fertig ist.", "The editor will close when the export is finished.")
                    }}
                </div>
                <crate::ui::TextButton on_click=move || api::fire("receditor", "cancel", Value::Null)>{t("Abbrechen", "Cancel")}</crate::ui::TextButton>
            </div>
        </div>
    }
}
