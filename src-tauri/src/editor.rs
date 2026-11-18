//! Annotations-Editor (Pfeile, Formen, Text, Zähler, Verpixeln, Zuschneiden, Verschönern). Port von Capso `AnnotationEditor` + `AnnotationKit`.
//! Das Zeichnen passiert im WebView (Route `editor`, Canvas 2D); hier: Fenster, Speichern/Export, Zwischenablage, Ziehen, Schließen-Schutz.
//!
//! RPC-Befehle (`rpc("editor", cmd, args)`), Bilddaten immer als PNG-data-URL (`dataUrl`):
//! - `copy {dataUrl}`            → Zwischenablage gemäß `screenshotClipboardContent` (Bild im `screenshotClipboardFormat` oder Dateipfad)
//! - `save {dataUrl}`            → Aufnahme: Exportordner; geöffnete Datei: gemäß `openedImageSaveBehavior` (ask/overwrite/copy) → `{saved, path}`
//! - `saveAs {dataUrl}`          → Dateidialog → `{saved, path}`
//! - `pin {dataUrl}` / `share {dataUrl}` → `pin::pin_image` (an der Position des Editorfensters) / `share::upload_and_copy`
//! - `pickColor`                 → Farbe vom Bildschirm (NSColorSampler) → `"#RRGGBB"` oder `null`
//! - `writeDataUrl {dataUrl, path}` → Datei schreiben (Format nach Endung: png/jpg/jpeg/heic/tiff) → Pfad
//! - `readDataUrl {path}`        → Bild als data-URL (HEIC/TIFF werden nach PNG gewandelt)
//! - `prepareDrag {dataUrl}` → Pfad, `drag {path}` → natives Ziehen aus dem Fenster
//!
//! Startdaten des Fensters: `{path, name, capture, meta}`; `meta` = `{sourceApp, windowTitle, capturedAt (Unix-ms), historyId}` (alle optional).
//! - `setDirty {dirty}`, `forceClose`, `startWindowDrag`, `zoomWindow`, `openFile`, `openClipboard`, `beautifyDone {job, path}`
use crate::core::{self, t, Win};
use base64::Engine;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{mpsc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WindowEvent};

static DIRTY: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static JOBS: Mutex<Option<HashMap<String, mpsc::Sender<Option<PathBuf>>>>> = Mutex::new(None);

fn set_dirty(label: &str, dirty: bool) {
    let mut g = DIRTY.lock().unwrap();
    let s = g.get_or_insert_with(HashSet::new);
    if dirty {
        s.insert(label.to_owned());
    } else {
        s.remove(label);
    }
}

fn is_dirty(label: &str) -> bool {
    DIRTY.lock().unwrap().as_ref().is_some_and(|s| s.contains(label))
}

/// Menüleisten-App nach vorne holen (Fenster/Dialoge sonst hinter anderen Apps).
fn activate(app: &AppHandle) {
    let _ = app.run_on_main_thread(|| {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            #[allow(deprecated)]
            objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        }
    });
}

/// Bild im Editor öffnen (jedes Mal ein neues Fenster mit eindeutigem Label).
pub fn open(app: &AppHandle, path: &Path) {
    open_with_meta(app, path, Value::Null);
}

/// Wie `open`, mit Aufnahme-Metadaten für Dateinamen und Verlauf (Capso `AnnotationEditorWindow(sourceAppName:sourceWindowTitle:captureDate:)`).
/// `meta`: `{sourceApp, windowTitle, capturedAt (Unix-ms), historyId}` – alle Felder optional.
/// Mit `historyId` aktualisiert „Sichern“ diesen Verlaufseintrag statt einen neuen anzulegen.
pub fn open_with_meta(app: &AppHandle, path: &Path, meta: Value) {
    let label = format!("editor-{}", core::unique_id());
    let (pw, ph) = core::image_size(path).unwrap_or((1200, 800));
    let screen = core::cursor_monitor(app);
    let scale = screen.as_ref().map(|m| m.scale_factor()).unwrap_or(2.0);
    let (sw, sh) = screen
        .as_ref()
        .map(|m| {
            let wa = m.work_area();
            (wa.size.width as f64 / scale, wa.size.height as f64 / scale)
        })
        .unwrap_or((1440.0, 900.0));
    // Fenstergröße an das Bild (in Punkten) anpassen, innerhalb von 90 % des Bildschirms.
    let chrome_h = 28.0 + 46.0 + 32.0 + 48.0;
    let w = (pw as f64 / scale + 48.0).clamp(860.0_f64.min(sw * 0.9), sw * 0.9);
    let h = (ph as f64 / scale + chrome_h).clamp(520.0_f64.min(sh * 0.9), sh * 0.9);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let capture = path.starts_with(core::temp_dir());
    let win = Win { min_size: Some((720.0, 420.0)), ..Win::app(&format!("nrzCapture – {name}"), w, h) };
    activate(app);
    let Some(w) = core::window(app, &label, "editor", win, json!({ "path": path, "name": name, "capture": capture, "meta": meta })) else { return };
    let (app2, l) = (app.clone(), label.clone());
    w.on_window_event(move |e| match e {
        WindowEvent::CloseRequested { api, .. } if is_dirty(&l) => {
            api.prevent_close();
            let _ = app2.emit_to(l.as_str(), "editor-close-request", ());
        }
        WindowEvent::Destroyed => set_dirty(&l, false),
        _ => {}
    });
}

/// Bild aus der Zwischenablage im Editor öffnen.
pub fn open_clipboard(app: &AppHandle) {
    match core::clipboard::read_image() {
        Some(p) => open(app, &p),
        None => core::toast(app, t("Kein Bild in der Zwischenablage", "No image on the clipboard")),
    }
}

/// Dateiauswahl → Editor (läuft in einem eigenen Thread, blockiert also nicht).
pub fn open_file_dialog(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let prompt = t("Bild zum Bearbeiten wählen", "Choose an image to annotate").replace('"', "'");
        let script = format!(
            "tell me to activate\nset fs to choose file of type {{\"public.image\"}} with prompt \"{prompt}\" with multiple selections allowed\nset out to \"\"\nrepeat with f in fs\nset out to out & POSIX path of f & linefeed\nend repeat\nreturn out"
        );
        if let Some(out) = core::sh("osascript", &["-e", &script]) {
            for p in out.lines().map(str::trim).filter(|l| !l.is_empty()) {
                open(&app, Path::new(p));
            }
        }
    });
}

/// Auto-Beautify: Bild mit dem gespeicherten `beautifyStyle` verschönern (Format siehe Frontend `editor::model::Beautify`).
/// Rendert in einem unsichtbaren WebView-Fenster; blockiert bis zu 20 s. **Nicht auf dem Main-Thread aufrufen.**
/// Ergebnis: neue PNG-Datei im Temp-Ordner (oder `None`).
pub fn beautify_file(app: &AppHandle, path: &Path) -> Option<PathBuf> {
    let job = core::unique_id();
    let out = core::temp_file("png");
    let (tx, rx) = mpsc::channel();
    JOBS.lock().unwrap().get_or_insert_with(HashMap::new).insert(job.clone(), tx);
    let label = format!("editor-beautify-{job}");
    let win = Win { w: 16.0, h: 16.0, visible: false, focus: false, ..Default::default() };
    core::window(app, &label, "editor-beautify", win, json!({ "path": path, "out": out, "job": job }))?;
    let res = rx.recv_timeout(Duration::from_secs(20)).ok().flatten();
    JOBS.lock().unwrap().as_mut().map(|m| m.remove(&job));
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.destroy();
    }
    res.filter(|p| p.exists())
}

// ───────────────────────────── Dateien ─────────────────────────────

fn decode_data_url(data_url: &str) -> Result<Vec<u8>, String> {
    let b64 = data_url.split_once(";base64,").map(|(_, b)| b).unwrap_or(data_url);
    base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| format!("data-URL: {e}"))
}

/// Endung für das eingestellte Bildformat (`screenshotFormat`).
pub fn format_ext() -> &'static str {
    match crate::settings::get().screenshot_format.as_str() {
        "jpeg" | "jpg" => "jpg",
        "heic" => "heic",
        _ => "png",
    }
}

/// PNG-data-URL als Datei schreiben; Format nach Endung (png/jpg/jpeg/heic/tiff), Qualität aus `screenshotQuality`.
pub fn write_data_url(data_url: &str, path: &Path) -> Result<PathBuf, String> {
    let bytes = decode_data_url(data_url)?;
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    let fmt = match ext.as_str() {
        "jpg" | "jpeg" => "jpeg",
        "heic" | "heif" => "heic",
        "tif" | "tiff" => "tiff",
        _ => "",
    };
    if fmt.is_empty() {
        std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
        return Ok(path.to_owned());
    }
    let tmp = core::temp_file("png");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    let q = crate::settings::get().screenshot_quality.clamp(1, 100).to_string();
    let (src, dst) = (tmp.to_string_lossy().into_owned(), path.to_string_lossy().into_owned());
    let mut args = vec!["-s", "format", fmt];
    if fmt != "tiff" {
        args.extend(["-s", "formatOptions", q.as_str()]);
    }
    args.extend([src.as_str(), "--out", dst.as_str()]);
    let ok = core::sh("sips", &args).is_some();
    let _ = std::fs::remove_file(&tmp);
    if ok && path.exists() {
        Ok(path.to_owned())
    } else {
        Err(t("Bild konnte nicht gesichert werden", "Couldn't save image").into())
    }
}

/// Bild als data-URL lesen (für das WebView, falls das Asset-Protokoll scheitert).
fn read_data_url(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let (bytes, mime) = match ext.as_str() {
        "png" => (std::fs::read(path).map_err(|e| e.to_string())?, "image/png"),
        "jpg" | "jpeg" => (std::fs::read(path).map_err(|e| e.to_string())?, "image/jpeg"),
        "gif" => (std::fs::read(path).map_err(|e| e.to_string())?, "image/gif"),
        "webp" => (std::fs::read(path).map_err(|e| e.to_string())?, "image/webp"),
        _ => {
            let tmp = core::temp_file("png");
            core::sh("sips", &["-s", "format", "png", &path.to_string_lossy(), "--out", &tmp.to_string_lossy()]).ok_or("sips")?;
            let b = std::fs::read(&tmp).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&tmp);
            (b, "image/png")
        }
    };
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Dateiname aus der Vorlage (Capso `FileNaming.renderTemplate`): `{date}` `{time}` `{timestamp}` `{source}` (" - App")
/// `{app}` `{window}` `{random}`; Datum = Aufnahmezeitpunkt (`date_ms`, sonst jetzt).
pub fn meta_file_name(template: &str, app_name: &str, window: &str, date_ms: Option<i64>, ext: &str) -> String {
    use chrono::TimeZone;
    let clean = |s: &str| s.trim().replace(['/', ':'], "-").trim().to_string();
    let (app_name, window) = (clean(app_name), clean(window));
    let date = date_ms.and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single()).unwrap_or_else(chrono::Local::now);
    let (d, tm) = (date.format("%Y-%m-%d").to_string(), date.format("%H.%M.%S").to_string());
    let mut name = template
        .replace("{timestamp}", &format!("{d} at {tm}"))
        .replace("{date}", &d)
        .replace("{time}", &tm)
        .replace("{source}", &if app_name.is_empty() { String::new() } else { format!(" - {app_name}") })
        .replace("{app}", &app_name)
        .replace("{window}", &window);
    while let Some(i) = name.find("{random}") {
        let r: String = core::unique_id().chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).take(8).collect();
        name.replace_range(i..i + 8, &r);
    }
    let name = name.replace(['/', ':'], "-");
    let name = name.trim().trim_end_matches('.').trim();
    format!("{}.{ext}", if name.is_empty() { "Screenshot" } else { name })
}

/// Metadaten des Editorfensters (`open_with_meta`).
fn meta(win: &WebviewWindow) -> Value {
    core::window_data(win.label())["meta"].clone()
}

/// Dateiname für dieses Editorfenster (Quell-App, Fenstertitel, Aufnahmedatum aus den Metadaten).
fn file_name_for(win: &WebviewWindow, ext: &str) -> String {
    let m = meta(win);
    let tpl = crate::settings::get().screenshot_filename_template;
    meta_file_name(&tpl, m["sourceApp"].as_str().unwrap_or(""), m["windowTitle"].as_str().unwrap_or(""), m["capturedAt"].as_i64(), ext)
}

/// Als neue Datei im Exportordner sichern (Dateiname aus `screenshotFilenameTemplate`).
/// Aus dem Verlauf geöffnet (`historyId`): Verlaufseintrag auf die neue Datei umstellen statt neu anlegen.
fn save_new(app: &AppHandle, win: &WebviewWindow, data_url: &str) -> Result<PathBuf, String> {
    let name = file_name_for(win, format_ext());
    let path = write_data_url(data_url, &core::unique_path(&core::export_dir(), &name))?;
    match meta(win)["historyId"].as_str().filter(|s| !s.is_empty()) {
        Some(id) => crate::history::replace_entry_file(app, id, &path),
        None => crate::history::add(app, &path, "screenshot"),
    }
    core::toast(app, t("Gesichert", "Saved"));
    Ok(path)
}

fn esc_as(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Capso `promptSaveChoice`: "copy" | "overwrite" | None (Abbrechen).
fn ask_save_choice(app: &AppHandle, name: &str) -> Option<&'static str> {
    activate(app);
    let (copy, over, cancel) = (t("Als neue Kopie sichern", "Save as New Copy"), t("Original überschreiben", "Overwrite Original"), t("Abbrechen", "Cancel"));
    let msg = format!(
        "{}\n\n{}",
        t("Änderungen an „{}“ sichern?", "Save changes to \"{}\"?").replace("{}", name),
        t("Du kannst die Originaldatei überschreiben oder deine Anmerkungen als neue Datei sichern.", "You can overwrite the original file or save your annotations as a new file.")
    );
    let script = format!(
        "tell me to activate\ndisplay dialog \"{}\" buttons {{\"{}\", \"{}\", \"{}\"}} default button 3 cancel button 1 with title \"nrzCapture\" with icon note\nreturn button returned of result",
        esc_as(&msg),
        esc_as(cancel),
        esc_as(over),
        esc_as(copy)
    );
    let out = core::sh("osascript", &["-e", &script])?;
    if out == over {
        Some("overwrite")
    } else if out == copy {
        Some("copy")
    } else {
        None
    }
}

fn save(app: &AppHandle, win: &WebviewWindow, data_url: &str) -> Result<Value, String> {
    let d = core::window_data(win.label());
    let orig = d["path"].as_str().map(PathBuf::from);
    let from_history = d["meta"]["historyId"].as_str().is_some_and(|s| !s.is_empty());
    let capture = d["capture"].as_bool().unwrap_or(true) || from_history;
    let path = match orig.filter(|p| !capture && p.exists()) {
        None => save_new(app, win, data_url)?,
        Some(orig) => {
            let behavior = crate::settings::get().opened_image_save_behavior;
            let choice = match behavior.as_str() {
                "overwrite" => Some("overwrite"),
                "copy" => Some("copy"),
                _ => ask_save_choice(app, &orig.file_name().unwrap_or_default().to_string_lossy()),
            };
            match choice {
                Some("overwrite") => {
                    let p = write_data_url(data_url, &orig).map_err(|_| t("Datei konnte nicht überschrieben werden", "Couldn't overwrite file").to_string())?;
                    core::toast(app, t("Gesichert", "Saved"));
                    p
                }
                Some(_) => save_new(app, win, data_url)?,
                None => return Ok(json!({ "saved": false })),
            }
        }
    };
    Ok(json!({ "saved": true, "path": path }))
}

fn save_as(app: &AppHandle, win: &WebviewWindow, data_url: &str) -> Result<Value, String> {
    activate(app);
    let d = core::window_data(win.label());
    let ext = format_ext();
    let default = match d["path"].as_str().filter(|_| !d["capture"].as_bool().unwrap_or(true)) {
        Some(p) => Path::new(p).with_extension(ext).file_name().unwrap_or_default().to_string_lossy().into_owned(),
        None => file_name_for(win, ext),
    };
    let dir = core::export_dir();
    let script = format!(
        "tell me to activate\nset f to choose file name with prompt \"{}\" default name \"{}\" default location POSIX file \"{}\"\nreturn POSIX path of f",
        esc_as(t("Sichern unter", "Save As")),
        esc_as(&default),
        esc_as(&dir.to_string_lossy())
    );
    let Some(out) = core::sh("osascript", &["-e", &script]) else { return Ok(json!({ "saved": false })) };
    let mut path = PathBuf::from(out.trim());
    if path.extension().is_none() {
        path.set_extension(ext);
    }
    let path = write_data_url(data_url, &path)?;
    crate::history::add(app, &path, "screenshot");
    core::toast(app, t("Gesichert", "Saved"));
    Ok(json!({ "saved": true, "path": path }))
}

/// Temporäre Datei mit lesbarem Namen (für Ziehen/Teilen/Kopieren als Pfad).
fn named_temp(win: &WebviewWindow, data_url: &str, ext: &str) -> Result<PathBuf, String> {
    let dir = core::temp_dir().join(format!("edit-{}", core::unique_id()));
    write_data_url(data_url, &dir.join(file_name_for(win, ext)))
}

/// Fenster während des Herausziehens unsichtbar und mausdurchlässig machen (Capso `hideDuringExternalDrag`/`showAfterExternalDrag`).
/// Muss auf dem Main-Thread laufen.
fn set_drag_hidden(w: &WebviewWindow, hidden: bool) {
    if let Ok(ptr) = w.ns_window() {
        let nw = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
        nw.setAlphaValue(if hidden { 0.0 } else { 1.0 });
        nw.setIgnoresMouseEvents(hidden);
    }
}

/// Farbe vom Bildschirm wählen (Capso `NSColorSampler`, „Pick Color From Screen“). Blockiert bis zur Auswahl; `None` bei Abbruch.
/// **Nicht auf dem Main-Thread aufrufen.**
pub fn pick_screen_color(app: &AppHandle) -> Option<String> {
    let (tx, rx) = mpsc::channel::<Option<String>>();
    let _ = app.run_on_main_thread(move || {
        use objc2_app_kit::{NSColor, NSColorSampler, NSColorSpace};
        let sampler = NSColorSampler::new();
        let tx = Mutex::new(Some(tx));
        let block = block2::RcBlock::new(move |c: *mut NSColor| {
            let hex = unsafe { c.as_ref() }.and_then(|c| c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())).map(|c| {
                let v = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
                format!("#{:02X}{:02X}{:02X}", v(c.redComponent()), v(c.greenComponent()), v(c.blueComponent()))
            });
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(hex);
            }
        });
        unsafe { sampler.showSamplerWithSelectionHandler(&block) };
    });
    rx.recv().ok().flatten()
}

/// Pin-Rahmen: Bild (Pixel) eingepasst und zentriert im Editorfenster (Capso `onPin(rendered, window.frame)`).
fn pin_frame(win: &WebviewWindow, img_px: (u32, u32)) -> Option<[f64; 4]> {
    let scale = win.scale_factor().ok()?;
    let pos = win.outer_position().ok()?.to_logical::<f64>(scale);
    let size = win.outer_size().ok()?.to_logical::<f64>(scale);
    Some(fit_into([pos.x, pos.y, size.width, size.height], img_px.0 as f64 / scale, img_px.1 as f64 / scale))
}

/// Größe `w`×`h` (nie vergrößert) eingepasst und zentriert in `frame`.
fn fit_into(frame: [f64; 4], w: f64, h: f64) -> [f64; 4] {
    let k = (frame[2] / w.max(1.0)).min(frame[3] / h.max(1.0)).min(1.0);
    let (nw, nh) = (w * k, h * k);
    [frame[0] + (frame[2] - nw) / 2.0, frame[1] + (frame[3] - nh) / 2.0, nw, nh]
}

fn drag(app: &AppHandle, window: &WebviewWindow, path: PathBuf) {
    let thumb = path.with_file_name(".drag-thumb.png");
    core::sh("sips", &["-Z", "256", "-s", "format", "png", &path.to_string_lossy(), "--out", &thumb.to_string_lossy()]);
    let ps = path.to_string_lossy().into_owned();
    let enc: String = ps
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let url = format!("file://{enc}");
    let xml = ps.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><array><string>{xml}</string></array></plist>"#
    );
    let (w, app) = (window.clone(), app.clone());
    let _ = window.run_on_main_thread(move || {
        let item = drag::DragItem::Data {
            provider: Box::new(move |t| match t {
                "public.file-url" => Some(url.clone().into_bytes()),
                "NSFilenamesPboardType" => Some(plist.clone().into_bytes()),
                _ => None,
            }),
            types: vec!["public.file-url".into(), "NSFilenamesPboardType".into()],
        };
        let img = if thumb.exists() { thumb } else { path.clone() };
        let label = w.label().to_owned();
        let w2 = w.clone();
        let started = drag::start_drag(
            &w,
            item,
            drag::Image::File(img),
            move |res, _| {
                set_drag_hidden(&w2, false);
                let _ = app.emit_to(label.as_str(), "editor-dropped", matches!(res, drag::DragResult::Dropped));
            },
            drag::Options::default(),
        );
        if started.is_ok() {
            set_drag_hidden(&w, true);
        }
    });
}

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    let data = || a["dataUrl"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| "dataUrl fehlt".to_string());
    let path = || a["path"].as_str().map(PathBuf::from).ok_or_else(|| "path fehlt".to_string());
    match cmd {
        "copy" => {
            // Capso `ScreenshotClipboard.copy(content:imageFormat:)`
            let s = crate::settings::get();
            let ok = if s.screenshot_clipboard_content == "filePath" {
                let p = named_temp(win, data()?, format_ext())?;
                core::clipboard::copy_file(&p)
            } else {
                let tmp = write_data_url(data()?, &core::temp_file("png"))?;
                crate::capture::post::copy_image_as(&tmp, &s.screenshot_clipboard_format)
            };
            if !ok {
                return Err(t("Kopieren fehlgeschlagen", "Copy failed").into());
            }
            core::toast(app, t("In die Zwischenablage kopiert", "Copied to clipboard"));
            Ok(json!(true))
        }
        "save" => save(app, win, data()?),
        "saveAs" => save_as(app, win, data()?),
        "pin" => {
            let p = write_data_url(data()?, &core::temp_file("png"))?;
            let frame = core::image_size(&p).and_then(|px| pin_frame(win, px));
            crate::pin::pin_image(app, &p, frame);
            Ok(json!(true))
        }
        "pickColor" => Ok(json!(pick_screen_color(app))),
        "share" => {
            let p = named_temp(win, data()?, format_ext())?;
            crate::share::upload_and_copy(app, &p);
            Ok(json!(true))
        }
        "writeDataUrl" => Ok(json!(write_data_url(data()?, &path()?)?)),
        "readDataUrl" => Ok(json!(read_data_url(&path()?)?)),
        "prepareDrag" => Ok(json!(named_temp(win, data()?, format_ext())?)),
        "drag" => {
            drag(app, win, path()?);
            Ok(Value::Null)
        }
        "setDirty" => {
            set_dirty(win.label(), a["dirty"].as_bool().unwrap_or(false));
            Ok(Value::Null)
        }
        "forceClose" => {
            set_dirty(win.label(), false);
            let _ = win.destroy();
            Ok(Value::Null)
        }
        "startWindowDrag" => {
            let _ = win.start_dragging();
            Ok(Value::Null)
        }
        "zoomWindow" => {
            let _ = if win.is_maximized().unwrap_or(false) { win.unmaximize() } else { win.maximize() };
            Ok(Value::Null)
        }
        "openFile" => {
            open_file_dialog(app);
            Ok(Value::Null)
        }
        "openClipboard" => {
            open_clipboard(app);
            Ok(Value::Null)
        }
        "beautifyDone" => {
            let job = a["job"].as_str().unwrap_or("");
            if let Some(tx) = JOBS.lock().unwrap().as_ref().and_then(|m| m.get(job)) {
                let _ = tx.send(a["path"].as_str().map(PathBuf::from));
            }
            Ok(Value::Null)
        }
        _ => Err(format!("editor: unbekannter Befehl {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_url_decoding() {
        assert_eq!(decode_data_url("data:image/png;base64,aGFsbG8=").unwrap(), b"hallo");
        assert_eq!(decode_data_url("aGFsbG8=").unwrap(), b"hallo");
        assert!(decode_data_url("data:image/png;base64,***").is_err());
    }

    #[test]
    fn file_names_from_meta() {
        use chrono::TimeZone;
        let ms = chrono::Local.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap().timestamp_millis();
        assert_eq!(meta_file_name("Screenshot {timestamp}{source}", "Safari", "", Some(ms), "png"), "Screenshot 2026-03-04 at 05.06.07 - Safari.png");
        assert_eq!(meta_file_name("Screenshot {date}{source}", "", "", Some(ms), "jpg"), "Screenshot 2026-03-04.jpg");
        assert_eq!(meta_file_name("{app} – {window}", "Mail", "a/b: c", Some(ms), "png"), "Mail – a-b- c.png");
        let r = meta_file_name("x{random}", "", "", None, "png");
        assert_eq!(r.len(), "x".len() + 8 + 4);
        assert!(!r.contains('{'));
    }

    #[test]
    fn pin_fit() {
        assert_eq!(fit_into([100.0, 50.0, 800.0, 600.0], 400.0, 300.0), [300.0, 200.0, 400.0, 300.0]);
        assert_eq!(fit_into([0.0, 0.0, 800.0, 600.0], 1600.0, 600.0), [0.0, 150.0, 800.0, 300.0]);
    }

    #[test]
    fn applescript_escaping() {
        assert_eq!(esc_as(r#"a"b\c"#), r#"a\"b\\c"#);
    }
}
