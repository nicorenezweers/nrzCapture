//! Brücke zum Rust-Backend: `rpc`, Events, Einstellungen, Sprache, Asset-URLs.
use leptos::{prelude::*, task::spawn_local};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
    async fn tauri_listen(event: &str, handler: &Closure<dyn Fn(JsValue)>) -> JsValue;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = emit)]
    async fn tauri_emit(event: &str, payload: JsValue) -> JsValue;
}

fn to_js(v: &Value) -> JsValue {
    v.serialize(&serde_wasm_bindgen::Serializer::json_compatible()).unwrap_or(JsValue::NULL)
}

fn from_js(v: JsValue) -> Value {
    serde_wasm_bindgen::from_value(v).unwrap_or(Value::Null)
}

/// Backend-Befehl aufrufen: `rpc("ocr", "recognize", json!({"path": p})).await`.
pub async fn rpc(m: &str, c: &str, a: Value) -> Result<Value, String> {
    let args = json!({ "m": m, "c": c, "a": a });
    match tauri_invoke("rpc", to_js(&args)).await {
        Ok(v) => Ok(from_js(v)),
        Err(e) => Err(e.as_string().unwrap_or_else(|| format!("{e:?}"))),
    }
}

/// Wie `rpc`, aber typisiert.
pub async fn rpc_as<T: DeserializeOwned>(m: &str, c: &str, a: Value) -> Result<T, String> {
    serde_json::from_value(rpc(m, c, a).await?).map_err(|e| e.to_string())
}

/// Befehl ohne auf das Ergebnis zu warten (Fehler landen in der Konsole).
pub fn fire(m: &'static str, c: &'static str, a: Value) {
    spawn_local(async move {
        if let Err(e) = rpc(m, c, a).await {
            web_sys::console::error_1(&format!("{m}.{c}: {e}").into());
        }
    });
}

/// Backend-Event abonnieren (für die Lebensdauer des Fensters).
pub fn on(event: &'static str, f: impl Fn(Value) + 'static) {
    spawn_local(async move {
        let cb = Closure::<dyn Fn(JsValue)>::new(move |e: JsValue| {
            let p = js_sys::Reflect::get(&e, &"payload".into()).unwrap_or(JsValue::NULL);
            f(from_js(p));
        });
        tauri_listen(event, &cb).await;
        cb.forget();
    });
}

/// Event an alle Fenster senden.
pub fn emit(event: &'static str, payload: Value) {
    spawn_local(async move {
        tauri_emit(event, to_js(&payload)).await;
    });
}

// ───────────────────────────── Einstellungen ─────────────────────────────

static SETTINGS: OnceLock<RwSignal<Value>> = OnceLock::new();

/// Reaktive Einstellungen (JSON, camelCase-Schlüssel wie in `src-tauri/src/settings.rs`).
/// Wird beim Start geladen und bei jedem `settings`-Event aktualisiert.
pub fn settings() -> RwSignal<Value> {
    *SETTINGS.get_or_init(|| {
        let s = RwSignal::new(Value::Null);
        spawn_local(async move {
            if let Ok(v) = rpc("settings", "get", Value::Null).await {
                s.set(v);
            }
        });
        on("settings", move |v| s.set(v));
        s
    })
}

/// Einstellungen beim Start laden (vor dem ersten Rendern), damit `t()` sofort die richtige Sprache hat.
pub async fn init_settings() {
    let s = settings();
    if let Ok(v) = rpc("settings", "get", Value::Null).await {
        s.set(v);
    }
}

/// Einzelner Wert (reaktiv, wenn innerhalb eines reaktiven Kontexts gelesen).
pub fn setting(key: &str) -> Value {
    settings().with(|s| s.get(key).cloned().unwrap_or(Value::Null))
}

pub fn setting_bool(key: &str) -> bool {
    setting(key).as_bool().unwrap_or(false)
}

pub fn setting_str(key: &str) -> String {
    setting(key).as_str().unwrap_or("").to_owned()
}

pub fn setting_f64(key: &str) -> f64 {
    setting(key).as_f64().unwrap_or(0.0)
}

/// Wert setzen (speichert sofort, alle Fenster bekommen das `settings`-Event).
pub fn set_setting(key: &str, value: Value) {
    let key = key.to_owned();
    settings().update(|s| {
        if let Some(o) = s.as_object_mut() {
            o.insert(key.clone(), value.clone());
        }
    });
    spawn_local(async move {
        let _ = rpc("settings", "set", json!({ "key": key, "value": value })).await;
    });
}

// ───────────────────────────── Sprache ─────────────────────────────

/// Effektive Sprache "de" | "en".
pub fn lang() -> &'static str {
    let l = settings().with_untracked(|s| s.get("language").and_then(|v| v.as_str()).unwrap_or("system").to_owned());
    match l.as_str() {
        "de" => "de",
        "en" => "en",
        _ => {
            let nav = window().navigator().language().unwrap_or_default();
            if nav.starts_with("de") {
                "de"
            } else {
                "en"
            }
        }
    }
}

/// Zweisprachiger Text: `t("Kopieren", "Copy")`.
pub fn t(de: &'static str, en: &'static str) -> &'static str {
    if lang() == "de" {
        de
    } else {
        en
    }
}

// ───────────────────────────── Sonstiges ─────────────────────────────

/// Lokale Datei im WebView anzeigen (`<img src=...>` / `<video src=...>`).
pub fn asset_url(path: &str) -> String {
    format!("asset://localhost/{}", String::from(js_sys::encode_uri_component(path)))
}

/// Startdaten des Fensters (vom Backend mit `core::window(..., data)` gesetzt).
pub async fn window_data() -> Value {
    rpc("core", "windowData", Value::Null).await.unwrap_or(Value::Null)
}

/// Aktuelles Fenster schließen.
pub fn close_window() {
    fire("core", "close", Value::Null);
}

/// Route des Fensters (Teil nach `#`, ohne Query).
pub fn route() -> String {
    let h = window().location().hash().unwrap_or_default();
    h.trim_start_matches('#').split('?').next().unwrap_or("").to_owned()
}

/// Synchron ins Backend-Log schreiben (funktioniert auch in einem Panic-Hook, wo keine Futures mehr laufen).
pub fn log_sync(msg: &str) {
    let f = || -> Option<()> {
        let tauri = js_sys::Reflect::get(&window(), &"__TAURI__".into()).ok()?;
        let core = js_sys::Reflect::get(&tauri, &"core".into()).ok()?;
        let invoke: js_sys::Function = js_sys::Reflect::get(&core, &"invoke".into()).ok()?.into();
        let args = to_js(&json!({ "m": "core", "c": "log", "a": { "message": msg } }));
        invoke.call2(&core, &"rpc".into(), &args).ok()?;
        Some(())
    };
    let _ = f();
}
