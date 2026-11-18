//! Editor-Fenster (Port von Capso `AnnotationEditorView` / `AnnotationEditorWindow` / `AnnotationEditorCloseGuard`).
//! Route `editor`; Startdaten `{path, name, capture, meta}` vom Backend (`editor::open_with_meta`).
use super::beautify::BeautifyPanel;
use super::canvas::{AnnotCanvas, Ed};
use super::crop::CropView;
use super::model::Tool;
use super::render;
use super::toolbar::*;
use crate::api::{self, t};
use crate::ui::{Icon, TextButton};
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

const EDITOR_TOOLS: &[Tool] = &[
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

/// Ausgabe-Aktion ausführen. Schließt das Fenster nach Kopieren/Sichern/Anheften/Teilen (wie Capso).
fn run_action(ed: Ed, action: &str, busy: RwSignal<bool>) {
    if busy.get_untracked() || ed.crop_mode.get_untracked() {
        return;
    }
    let action = action.to_string();
    if action == "close" {
        request_close(ed);
        return;
    }
    let Some(data) = ed.export(true) else { return };
    busy.set(true);
    spawn_local(async move {
        let res = api::rpc("editor", &action, json!({ "dataUrl": data })).await;
        busy.set(false);
        match res {
            Ok(v) => {
                let done = match action.as_str() {
                    "save" | "saveAs" => v["saved"].as_bool().unwrap_or(false),
                    _ => true,
                };
                if done {
                    api::fire("editor", "forceClose", Value::Null);
                }
            }
            Err(e) => api::fire("core", "toast", json!({ "message": e })),
        }
    });
}

thread_local!(static CLOSE_ASK: std::cell::Cell<Option<RwSignal<bool>>> = const { std::cell::Cell::new(None) });

/// Schließen mit Schutz vor Verlust ungesicherter Änderungen.
fn request_close(ed: Ed) {
    ed.commit_text();
    if ed.doc.with_untracked(|d| d.has_unsaved_changes()) {
        if let Some(s) = CLOSE_ASK.with(|c| c.get()) {
            s.set(true);
            return;
        }
    }
    api::fire("editor", "forceClose", Value::Null);
}

#[component]
pub fn EditorWindow() -> impl IntoView {
    let ed = Ed::new(false);
    let show_beautify = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let ask_close = RwSignal::new(false);
    CLOSE_ASK.with(|c| c.set(Some(ask_close)));
    let name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);

    spawn_local(async move {
        let d = api::window_data().await;
        let path = d["path"].as_str().unwrap_or("").to_string();
        name.set(d["name"].as_str().unwrap_or("").to_string());
        if path.is_empty() {
            error.set(Some(t("Kein Bild", "No image").into()));
            return;
        }
        // Textzeilen für den intelligenten Marker im Hintergrund erkennen (Capso: OCR beim Öffnen).
        ed.load_text_lines(path.clone());
        // Erst asset-URL (schnell); falls das Bild nicht lädt, data-URL über das Backend (z. B. HEIC).
        ed.load_or(api::asset_url(&path), || {}, move || {
            let p = path.clone();
            spawn_local(async move {
                match api::rpc("editor", "readDataUrl", json!({ "path": p })).await {
                    Ok(v) => ed.load(v.as_str().unwrap_or("").to_string(), || {}),
                    Err(e) => error.set(Some(e)),
                }
            });
        });
    });

    // Ungesicherte Änderungen ans Backend melden (Schließen-Schutz beim roten Knopf / ⌘W).
    Effect::new(move |prev: Option<bool>| {
        let dirty = ed.doc.with(|d| d.has_unsaved_changes());
        if prev != Some(dirty) {
            api::fire("editor", "setDirty", json!({ "dirty": dirty }));
        }
        dirty
    });
    api::on("editor-close-request", move |_| request_close(ed));

    let action = move |a: &str| run_action(ed, a, busy);
    let on_key = window_event_listener(leptos::ev::keydown, move |e| {
        if ask_close.get_untracked() {
            match e.key().as_str() {
                "Escape" => ask_close.set(false),
                "Enter" => api::fire("editor", "forceClose", Value::Null),
                _ => {}
            }
            return;
        }
        if (e.meta_key() || e.ctrl_key()) && e.key().eq_ignore_ascii_case("w") {
            e.prevent_default();
            request_close(ed);
            return;
        }
        ed.handle_key(&e, &|a| run_action(ed, a, busy));
    });
    let on_key_up = window_event_listener(leptos::ev::keyup, move |e| ed.update_keys(&e, false));
    on_cleanup(move || {
        on_key.remove();
        on_key_up.remove();
    });

    // Ziehen aus dem Editor: Datei beim Überfahren vorbereiten (Capso `prepareDragFile`).
    let drag_file = RwSignal::new(None::<(u64, String)>);
    let doc_ver = Memo::new(move |_| {
        ed.doc.track();
        ed.beautify.track();
        js_sys::Date::now() as u64
    });
    let prepare_drag = move || {
        let v = doc_ver.get_untracked();
        if drag_file.with_untracked(|d| d.as_ref().is_some_and(|(dv, _)| *dv == v)) || ed.editing() {
            return;
        }
        let Some(data) = ed.export(true) else { return };
        spawn_local(async move {
            if let Ok(p) = api::rpc("editor", "prepareDrag", json!({ "dataUrl": data })).await {
                if let Some(p) = p.as_str() {
                    drag_file.set(Some((v, p.to_string())));
                }
            }
        });
    };

    let zoom_pct = move || format!("{}%", (ed.zoom.get() * 100.0).round() as i64);
    let size_label = move || {
        let r = ed.doc.with(super::model::content_rect);
        let (w, h) = ed.doc.with(|d| d.output).filter(|_| ed.doc.with(|d| d.crop.is_some())).map(|(w, h)| (w as f64, h as f64)).unwrap_or((r.w, r.h));
        format!("{} × {} px", w.round() as i64, h.round() as i64)
    };

    view! {
        <div class="app-window flex h-screen flex-col overflow-hidden">
            // Titelzeile (Ampel-Knöpfe liegen darüber); Ziehen bewegt das Fenster.
            <div class="flex h-7 shrink-0 items-center justify-center text-[12px] font-medium text-zinc-500"
                on:mousedown=move |e| { if e.button() == 0 { api::fire("editor", "startWindowDrag", Value::Null) } }
                on:dblclick=move |_| api::fire("editor", "zoomWindow", Value::Null)>
                {move || name.get()}
            </div>
            <Show when=move || !ed.crop_mode.get() fallback=move || view! { <CropView ed=ed /> }>
                // Werkzeugleiste
                <div class="flex shrink-0 flex-wrap items-center gap-x-1 gap-y-1 border-b border-black/10 dark:border-white/10 bg-white/70 dark:bg-zinc-800/70 px-2 py-1.5">
                    <ToolButtons ed=ed tools=EDITOR_TOOLS />
                    {sep()}
                    <ToolOptions ed=ed />
                    {sep()}
                    <ColorControls ed=ed />
                    {sep()}
                    <PresetsMenu ed=ed />
                    <TB title=t("Zuschneiden", "Crop") icon=I_CROP on_click=move || { ed.commit_text(); ed.doc.update(|d| d.selected = None); ed.crop_mode.set(true); } />
                    <TB title=t("Verschönern", "Beautify") icon=I_SPARKLES active=show_beautify on_click=move || show_beautify.update(|b| *b = !*b) />
                    {sep()}
                    <UndoRedo ed=ed />
                    <div class="flex-1"></div>
                    <TB title=t("Schließen", "Close") icon=crate::ui::X class="hover:!bg-red-500/15 hover:!text-red-500" on_click=move || request_close(ed) />
                    <button title=move || if ed.editing() { t("Erst Textbearbeitung abschließen", "Finish text editing before dragging") } else { t("Bearbeitetes Bild ziehen", "Drag Edited Image") }
                        class=format!("{BTN} {BTN_IDLE} cursor-grab")
                        on:mouseenter=move |_| prepare_drag()
                        on:mousedown=move |e| {
                            e.prevent_default();
                            if e.button() != 0 || ed.editing() { return; }
                            match drag_file.get_untracked() {
                                Some((v, p)) if v == doc_ver.get_untracked() => api::fire("editor", "drag", json!({ "path": p })),
                                _ => prepare_drag(),
                            }
                        }>
                        <Icon d=I_MOVE />
                    </button>
                    <TB title=t("Bild kopieren und schließen (⌘C · ⏎ · ⇧⌘C)", "Copy Image and Close (⌘C · ⏎ · ⌘⇧C)") icon=crate::ui::COPY on_click=move || action("copy") />
                    <TB title=t("Teilen (Cloud-Link kopieren)", "Share (copy cloud link)") icon=I_SHARE on_click=move || action("share") />
                    <TB title=t("Anheften (⌘P)", "Pin (⌘P)") icon=crate::ui::PIN on_click=move || action("pin") />
                    <button title=move || t("Sichern (⌘S) · Sichern unter … (⇧⌘S)", "Save (⌘S) · Save As… (⇧⌘S)")
                        class=format!("{BTN} bg-blue-500 text-white hover:bg-blue-600")
                        on:mousedown=|e| e.prevent_default()
                        on:click=move |e| { if e.alt_key() { action("saveAs") } else { action("save") } }>
                        <Icon d=I_DOWNLOAD />
                    </button>
                </div>
                <Show when=move || show_beautify.get()>
                    <BeautifyPanel ed=ed />
                </Show>
                // Zeichenfläche
                <div class="relative min-h-0 flex-1 bg-[#1f1f1f]">
                    <AnnotCanvas ed=ed margin=24.0 />
                    <Show when=move || error.get().is_some()>
                        <div class="absolute inset-0 grid place-items-center text-zinc-400">{move || error.get()}</div>
                    </Show>
                </div>
                // Zoomleiste
                <div class="flex h-8 shrink-0 items-center gap-1 border-t border-black/10 dark:border-white/10 bg-white/70 dark:bg-zinc-800/70 px-2 text-[12px]">
                    <TB title=format!("{} (⌘−)", t("Verkleinern", "Zoom Out")) icon=I_MINUS on_click=move || ed.zoom_by(1.0 / 1.25) />
                    <button class="w-12 tabular-nums text-zinc-600 dark:text-zinc-300" title=move || format!("{} (⌘0)", t("An Fenster anpassen", "Fit to Window"))
                        on:click=move |_| ed.fit.update(|f| *f += 1)>{zoom_pct}</button>
                    <TB title=format!("{} (⌘+)", t("Vergrößern", "Zoom In")) icon=I_PLUS on_click=move || ed.zoom_by(1.25) />
                    <button class="rounded px-2 py-0.5 text-zinc-500 hover:bg-black/5 dark:hover:bg-white/10" on:click=move |_| ed.zoom.set(1.0)>"100%"</button>
                    <div class="flex-1"></div>
                    <span class="tabular-nums text-zinc-500">{size_label}</span>
                </div>
            </Show>
            // Schließen-Schutz (Capso `AnnotationEditorCloseGuard`)
            <Show when=move || ask_close.get()>
                <div class="fixed inset-0 z-50 grid place-items-center bg-black/30">
                    <div class="w-80 rounded-xl bg-white dark:bg-zinc-800 p-5 text-center shadow-2xl">
                        <div class="mb-1 text-[14px] font-semibold">{move || t("Änderungen verwerfen?", "Discard your edits?")}</div>
                        <div class="mb-4 text-[12px] text-zinc-500">{move || t("Deine Anmerkungen wurden nicht gesichert und gehen verloren.", "Your annotations haven't been saved and will be lost.")}</div>
                        <div class="flex justify-center gap-2">
                            <TextButton on_click=move || ask_close.set(false)>{move || t("Abbrechen", "Cancel")}</TextButton>
                            <TextButton variant="danger" on_click=move || api::fire("editor", "forceClose", Value::Null)>{move || t("Verwerfen", "Discard")}</TextButton>
                        </div>
                    </div>
                </div>
            </Show>
            <Show when=move || busy.get()>
                <div class="pointer-events-none fixed bottom-10 right-3 rounded-md bg-zinc-900/80 px-2 py-1 text-[11px] text-white">{move || t("Bitte warten …", "Working…")}</div>
            </Show>
        </div>
    }
}

/// Unsichtbares Fenster für `autoBeautifyScreenshots`: Bild laden, mit gespeichertem Stil verschönern, Datei schreiben.
#[component]
pub fn BeautifyWorker() -> impl IntoView {
    spawn_local(async move {
        let d = api::window_data().await;
        let path = d["path"].as_str().unwrap_or("").to_string();
        let out = d["out"].as_str().unwrap_or("").to_string();
        let job = d["job"].clone();
        let fail = move |job: Value| api::fire("editor", "beautifyDone", json!({ "job": job, "path": null }));
        let ed = Ed::new(false);
        let job2 = job.clone();
        let done = move || {
            ed.beautify.update(|b| b.enabled = true);
            let Some(src) = ed.src.get_value() else { return };
            let b = ed.beautify.get_untracked();
            let c = render::beautify_canvas(&src, &b);
            let data = render::png_data_url(&c);
            let (job, out) = (job2.clone(), out.clone());
            spawn_local(async move {
                let ok = api::rpc("editor", "writeDataUrl", json!({ "dataUrl": data, "path": out })).await;
                api::fire("editor", "beautifyDone", json!({ "job": job, "path": ok.ok() }));
            });
        };
        if path.is_empty() {
            fail(job);
            return;
        }
        match api::rpc("editor", "readDataUrl", json!({ "path": path })).await {
            Ok(v) => ed.load(v.as_str().unwrap_or("").to_string(), done),
            Err(_) => fail(job),
        }
    });
}
