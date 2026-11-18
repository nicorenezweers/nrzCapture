//! nrzCapture-Frontend: ein WASM-Bundle für alle Fenster. Die Route (`index.html#<route>`) wählt die Ansicht.
//! Jedes Modul exportiert `route(r) -> Option<AnyView>` und behandelt nur seine eigenen Routen.
#![allow(dead_code)]
mod api;
mod camera;
mod editor;
mod history;
mod ocr;
mod overlay;
mod pin;
mod prefs;
mod quick;
mod receditor;
mod record;
mod translate;
mod ui;

use leptos::{prelude::*, task::spawn_local};

fn router(r: &str) -> AnyView {
    quick::route(r)
        .or_else(|| overlay::route(r))
        .or_else(|| pin::route(r))
        .or_else(|| editor::route(r))
        .or_else(|| ocr::route(r))
        .or_else(|| translate::route(r))
        .or_else(|| record::route(r))
        .or_else(|| camera::route(r))
        .or_else(|| receditor::route(r))
        .or_else(|| history::route(r))
        .or_else(|| prefs::route(r))
        .unwrap_or_else(|| ().into_any())
}

fn main() {
    console_error_panic_hook_lite();
    let _ = any_spawner::Executor::init_wasm_bindgen();
    spawn_local(async {
        api::init_settings().await;
        let r = api::route();
        // Fenster-Hintergrund: transparente Overlays vs. normale App-Fenster entscheidet jede Ansicht selbst.
        api::fire("core", "log", serde_json::json!({ "message": format!("gerendert: #{r}"), "debug": true }));
        leptos::mount::mount_to_body(move || router(&r));
    });
}

/// Panics in der Konsole sichtbar machen (ohne zusätzliche Abhängigkeit).
fn console_error_panic_hook_lite() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC {} ({})", info, api::route());
        web_sys::console::error_1(&msg.clone().into());
        api::log_sync(&msg);
    }));
    // JS-Fehler ebenfalls ins Backend-Log.
    let cb = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::ErrorEvent)>::new(|e: web_sys::ErrorEvent| {
        api::fire("core", "log", serde_json::json!({ "message": format!("JS-Fehler: {} ({}:{})", e.message(), e.filename(), e.lineno()) }));
    });
    let _ = window().add_event_listener_with_callback("error", wasm_bindgen::JsCast::unchecked_ref(cb.as_ref()));
    cb.forget();
}
