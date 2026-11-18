//! Capture-Overlays (Port von Capso `CaptureOverlayView`, `CaptureAllInOneToolbarWindow`, `SelfTimerHUD`,
//! `ScrollCaptureHUD`):
//! - `select`: Auswahl-Overlay je Bildschirm (Standbild, Abdunklung, Fadenkreuz, Lupe, Presets, Fenstererkennung,
//!   Griffe) inkl. All-in-One-Werkzeugleiste und Inline-Annotation
//! - `selftimer`: Countdown-HUD, `scroll-hud`: Scroll-Aufnahme-Steuerung mit Vorschau, `capture-border`: Rahmen
use leptos::prelude::*;

#[path = "../src-tauri/src/capture/geom.rs"]
pub mod geom;
mod hud;
mod select;

pub fn route(r: &str) -> Option<AnyView> {
    match r {
        "select" => Some(view! { <select::Select /> }.into_any()),
        "selftimer" => Some(view! { <hud::SelfTimer /> }.into_any()),
        "scroll-hud" => Some(view! { <hud::ScrollHud /> }.into_any()),
        "capture-border" => Some(view! { <hud::Border /> }.into_any()),
        _ => None,
    }
}
