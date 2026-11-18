//! Frontend des Annotations-Editors (Port von Capso `AnnotationEditor` + `AnnotationKit`).
//! - `model`   reine Logik (Objekte, Hit-Tests, Undo, Crop-Mathe, Beautify-Format) – mit Unit-Tests
//! - `render`  Canvas-2D-Renderer (Anzeige und Export in voller Auflösung)
//! - `canvas`  gemeinsamer Zustand `Ed` + interaktive Zeichenfläche (Editor und Inline)
//! - `toolbar` / `beautify` / `crop` / `window`  Oberfläche
//!
//! Routen: `editor` (Editor-Fenster), `editor-beautify` (unsichtbarer Worker für `autoBeautifyScreenshots`).
pub mod beautify;
pub mod canvas;
pub mod crop;
pub mod model;
pub mod render;
pub mod toolbar;
pub mod window;

use canvas::{AnnotCanvas, Ed};
use leptos::{html, prelude::*};
use model::Tool;
use toolbar::*;

pub fn route(r: &str) -> Option<AnyView> {
    match r {
        "editor" => Some(view! { <window::EditorWindow /> }.into_any()),
        "editor-beautify" => Some(view! { <window::BeautifyWorker /> }.into_any()),
        _ => None,
    }
}

const INLINE_TOOLS: &[Tool] = &[
    Tool::Select,
    Tool::Arrow,
    Tool::Line,
    Tool::Rectangle,
    Tool::Ellipse,
    Tool::Text,
    Tool::Freehand,
    Tool::Pixelate,
    Tool::Spotlight,
    Tool::Counter,
    Tool::Highlighter,
];

/// Inline-Annotation direkt auf dem Auswahlbereich (All-in-One-Overlay, Capso „Inline annotation“).
/// Wird vom Capture-Overlay an der Position der Auswahl eingebettet (absolut positionierter Container der Größe `width`×`height` CSS-px).
/// - `image_url`: aufgenommener Bereich (asset-URL), wird als Hintergrund gezeichnet; `scale` = Pixel pro CSS-px (Retina).
/// - Zeigt eigene Werkzeugleiste (Pfeil, Rechteck, Ellipse, Text, Stift, Marker, Zähler, Verpixeln, Farben, Rückgängig)
///   und Aktionsknöpfe (Kopieren, Sichern, Abbrechen …).
/// - `on_finish(action, png_data_url)`: action = "copy" | "save" | "pin" | "share" | "editor" | "cancel";
///   `png_data_url` = gerendertes Ergebnis in voller Pixelauflösung (`data:image/png;base64,…`), `None` bei "cancel".
///
/// Hinweise für die Einbettung: Tastenkürzel (Werkzeugbuchstaben, ⌘Z, ⌘C/⏎ = Kopieren, ⌘S, ⌘P, Esc) hört die Komponente
/// selbst am `window` ab; Esc geht schrittweise zurück (Text → Werkzeug → Auswahl) und bricht erst danach ab.
/// Maus-/Zeigerereignisse innerhalb der Komponente und ihrer Leiste werden nicht weitergereicht.
/// Die data-URL kann mit `rpc("editor","writeDataUrl",{dataUrl,path})` bzw. `editor::write_data_url` gespeichert werden.
#[component]
pub fn InlineAnnotator(
    image_url: String,
    width: f64,
    height: f64,
    scale: f64,
    on_finish: Callback<(String, Option<String>)>,
) -> impl IntoView {
    let _ = scale; // Auflösung ergibt sich aus dem Bild selbst (natürliche Pixelgröße)
    let ed = Ed::new(true);
    ed.load(image_url, || {});
    let finished = StoredValue::new(false);
    let finish = move |a: &str| {
        if finished.get_value() {
            return;
        }
        if a == "cancel" {
            finished.set_value(true);
            on_finish.run(("cancel".into(), None));
            return;
        }
        let Some(data) = ed.export(false) else { return };
        finished.set_value(true);
        on_finish.run((a.to_string(), Some(data)));
    };

    let on_key = window_event_listener(leptos::ev::keydown, move |e| {
        if finished.get_value() {
            return;
        }
        if e.key() == "Escape" && !ed.escape() {
            e.prevent_default();
            finish("cancel");
            return;
        }
        if e.key() == "Escape" {
            e.prevent_default();
            return;
        }
        ed.handle_key(&e, &|a| match a {
            "saveAs" => finish("save"),
            "close" => finish("cancel"),
            a => finish(a),
        });
    });
    let on_key_up = window_event_listener(leptos::ev::keyup, move |e| ed.update_keys(&e, false));
    on_cleanup(move || {
        on_key.remove();
        on_key_up.remove();
    });

    // Leiste unter der Auswahl, sonst darüber; waagrecht im Bildschirm halten (Capso `toolbarFrame`).
    let root = NodeRef::<html::Div>::new();
    let bar = NodeRef::<html::Div>::new();
    let place = RwSignal::new((false, 0.0f64));
    Effect::new(move |_| {
        ed.prefs.track();
        ed.text.track();
        request_animation_frame(move || {
            let (Some(r), Some(b)) = (root.get_untracked(), bar.get_untracked()) else { return };
            let w = web_sys::window().unwrap();
            let (vw, vh) = (w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1e4), w.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(1e4));
            let rr = r.get_bounding_client_rect();
            let (bw, bh) = (b.offset_width() as f64, b.offset_height() as f64);
            let above = rr.bottom() + 12.0 + bh > vh - 8.0 && rr.top() - 12.0 - bh >= 8.0;
            let mut dx = (rr.width() - bw) / 2.0;
            let left = rr.left() + dx;
            if left + bw > vw - 8.0 {
                dx -= left + bw - (vw - 8.0);
            }
            if rr.left() + dx < 8.0 {
                dx = 8.0 - rr.left();
            }
            place.set((above, dx));
        });
    });

    let stop = |e: leptos::ev::PointerEvent| e.stop_propagation();
    let stop_m = |e: leptos::ev::MouseEvent| e.stop_propagation();
    view! {
        <div node_ref=root class="absolute inset-0" style=format!("width:{width}px;height:{height}px")
            on:pointerdown=stop on:mousedown=stop_m on:click=stop_m>
            <AnnotCanvas ed=ed />
            <div node_ref=bar
                class="absolute z-10 flex flex-col gap-1 whitespace-nowrap rounded-xl border border-black/10 dark:border-white/10 bg-white/90 dark:bg-zinc-800/90 px-2 py-1.5 text-zinc-800 dark:text-zinc-100 shadow-2xl backdrop-blur"
                style:left=move || format!("{}px", place.get().1)
                style:top=move || if place.get().0 { "auto".to_string() } else { format!("{}px", height + 12.0) }
                style:bottom=move || if place.get().0 { format!("{}px", height + 12.0) } else { "auto".to_string() }
                on:pointerdown=stop on:mousedown=stop_m on:click=stop_m>
                <div class="flex items-center gap-0.5">
                    <ToolButtons ed=ed tools=INLINE_TOOLS />
                    {sep()}
                    <UndoRedo ed=ed />
                    {sep()}
                    <TB title=crate::api::t("Schließen (Esc)", "Close (Esc)") icon=crate::ui::X on_click=move || finish("cancel") />
                    <TB title=crate::api::t("Im Editor öffnen", "Open in Editor") icon=I_EXTERNAL on_click=move || finish("editor") />
                    <TB title=crate::api::t("Teilen", "Share") icon=I_SHARE on_click=move || finish("share") />
                    <TB title=crate::api::t("Bild kopieren und schließen (⌘C · ⏎)", "Copy Image and Close (⌘C · ⏎)") icon=crate::ui::COPY on_click=move || finish("copy") />
                    <TB title=crate::api::t("Anheften (⌘P)", "Pin to Screen (⌘P)") icon=crate::ui::PIN on_click=move || finish("pin") />
                    <TB title=crate::api::t("Sichern (⌘S)", "Save (⌘S)") icon=I_DOWNLOAD class="!bg-blue-500 !text-white" on_click=move || finish("save") />
                </div>
                <div class="flex items-center gap-1">
                    <ColorControls ed=ed compact=true />
                    {sep()}
                    <ToolOptions ed=ed compact=true />
                </div>
            </div>
        </div>
    }
}
