//! Aufnahme-Editor (Trimmen, Schnitte, Zoom, Cursor-Glättung, Weichzeichner, Hintergrund, Export MP4/GIF).
//! Port von Capso `App/Sources/Editor` + `EditorKit` + `ExportKit`.
//!
//! Aufbau:
//! - `receditor/model.rs`  reine Logik (Projektmodell, Feder, Zoom, Cursor, Auto-Zoom, Layout) – auch vom Frontend genutzt
//! - `receditor/render.rs` Komposition eines Bildes mit tiny-skia (gleiche Geometrie wie die Canvas-Vorschau)
//! - `receditor/export.rs` AVFoundation: Schnitt-Export (AVMutableComposition + AVAssetExportSession) bzw.
//!   Frames per AVAssetReader lesen → rendern → AVAssetWriter (H.264 + AAC) oder GIF (`gif`-Crate, NeuQuant-Palette)
//!
//! RPC (`rpc("receditor", …)`): `load {video}`, `save {video, project}`, `export {video, project, format, quality, copy}`,
//! `cancel`, `chooseImage`. Events an das Editorfenster: `receditor-progress {progress, stage}`,
//! `receditor-done {path, copy}`, `receditor-error {message, cancelled}`.
pub mod export;
pub mod model;
pub mod render;
#[cfg(test)]
mod tests;

use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, WebviewWindow};

use crate::core::{self, Win};
use model::{parse_telemetry, RecordingProject, Telemetry};

static CANCEL: Mutex<Option<HashMap<String, Arc<AtomicBool>>>> = Mutex::new(None);

fn label_for(video: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    video.hash(&mut h);
    format!("receditor-{:x}", h.finish())
}

/// Editorfenster für ein Video öffnen (ein Fenster pro Video).
pub fn open(app: &AppHandle, video: &Path) {
    let mut o = Win::app(core::t("Aufnahme bearbeiten – nrzCapture", "Edit Recording – nrzCapture"), 960.0, 640.0);
    o.min_size = Some((800.0, 540.0));
    core::window(app, &label_for(video), "receditor", o, json!({ "video": video }));
}

/// `<video>.project.json`
pub fn project_path(video: &Path) -> PathBuf {
    PathBuf::from(format!("{}.project.json", video.display()))
}

/// `<video>.cursor.json` (alternativ `<name>.cursor.json`).
pub fn telemetry_path(video: &Path) -> Option<PathBuf> {
    [PathBuf::from(format!("{}.cursor.json", video.display())), video.with_extension("cursor.json")].into_iter().find(|p| p.is_file())
}

pub fn load_telemetry(video: &Path) -> Telemetry {
    telemetry_path(video)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .map(|v| parse_telemetry(&v))
        .unwrap_or_default()
}

/// Projekt laden oder neu anlegen (Dauer/Größe per AVFoundation).
pub fn load_project(video: &Path) -> Result<(RecordingProject, Telemetry), String> {
    if !video.is_file() {
        return Err(core::t("Video nicht gefunden", "Video not found").into());
    }
    let tel = load_telemetry(video);
    let probe = export::probe(video);
    let saved = std::fs::read_to_string(project_path(video)).ok().and_then(|s| serde_json::from_str::<RecordingProject>(&s).ok());
    let mut p = match saved {
        Some(p) => p,
        None => {
            let mut p = RecordingProject { source_video: video.to_string_lossy().into(), created_at: chrono::Local::now().to_rfc3339(), ..Default::default() };
            p.shows_cursor = new_project_shows_cursor(&tel);
            // Einstellung „Cursor-Glättung“ als Startwert neuer Projekte.
            p.cursor_smoothing.enabled = crate::settings::get().cursor_smoothing;
            p
        }
    };
    p.source_video = video.to_string_lossy().into();
    p.cursor_telemetry = telemetry_path(video).map(|t| t.to_string_lossy().into());
    if let Some((d, w, h)) = probe {
        p.video_duration = d;
        p.video_width = w;
        p.video_height = h;
    }
    match (tel.area_width, tel.area_height) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
            p.recording_area_width = w;
            p.recording_area_height = h;
        }
        _ => {
            // Ohne Angabe: Retina-Aufnahmen (≥ 1440 px hoch) als 2× annehmen.
            let s = if p.video_height >= 1440.0 { 2.0 } else { 1.0 };
            p.recording_area_width = p.video_width / s;
            p.recording_area_height = p.video_height / s;
        }
    }
    Ok((p, tel))
}

/// Cursor-Overlay in neuen Projekten: nur mit Telemetrie und wenn „Mauszeiger anzeigen“ beim Aufnehmen an war
/// (Capso `EditorCoordinator.shouldRenderCursorOverlay` = `showsCursor && cursorTelemetryURL != nil`).
pub fn new_project_shows_cursor(tel: &Telemetry) -> bool {
    !tel.events.is_empty() && tel.user_shows_cursor.unwrap_or(true)
}

pub fn save_project(video: &Path, p: &RecordingProject) -> Result<(), String> {
    let s = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    let path = project_path(video);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, s).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn choose_image() -> Option<String> {
    let prompt = core::t("Hintergrundbild wählen", "Choose background image");
    core::sh("osascript", &["-e", &format!("POSIX path of (choose file with prompt \"{prompt}\" of type {{\"public.image\"}})")]).filter(|s| !s.is_empty())
}

/// Sichern-Dialog (osascript `choose file name`), Standardname `Recording.<ext>` im Exportordner.
fn save_dialog(ext: &str) -> Option<PathBuf> {
    let dir = core::export_dir();
    let prompt = core::t("Aufnahme sichern", "Save Recording");
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "POSIX path of (choose file name with prompt \"{}\" default name \"Recording.{ext}\" default location (POSIX file \"{}\"))",
        esc(prompt),
        esc(&dir.to_string_lossy())
    );
    let path = core::sh("/usr/bin/osascript", &["-e", &script]).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())?;
    let mut p = PathBuf::from(path);
    if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case(ext)) != Some(true) {
        let mut s = p.into_os_string();
        s.push(format!(".{ext}"));
        p = PathBuf::from(s);
    }
    Some(p)
}

fn start_export(app: &AppHandle, label: String, a: &Value) -> Result<Value, String> {
    let video = PathBuf::from(a["video"].as_str().ok_or("video fehlt")?);
    let project: RecordingProject = serde_json::from_value(a["project"].clone()).map_err(|e| e.to_string())?;
    let gif = a["format"].as_str() == Some("gif");
    let quality = a["quality"].as_str().unwrap_or("maximum").to_owned();
    let copy = a["copy"].as_bool().unwrap_or(false);
    let ext = if gif { "gif" } else { "mp4" };
    let dest = if copy {
        core::temp_file(ext)
    } else {
        // Wie Capso (EditorPlaybackControls.exportToFile): Sichern-Dialog mit „Recording.mp4/.gif“.
        match save_dialog(ext) {
            Some(p) => p,
            None => return Ok(json!({ "cancelled": true })),
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    CANCEL.lock().unwrap().get_or_insert_with(HashMap::new).insert(label.clone(), cancel.clone());
    let app = app.clone();
    let result = json!({ "path": dest });
    std::thread::spawn(move || {
        let tel = load_telemetry(&video);
        let l2 = label.clone();
        let app2 = app.clone();
        let progress = move |p: f64, stage: &str| {
            let _ = app2.emit_to(&l2, "receditor-progress", json!({ "progress": p, "stage": stage }));
        };
        let job = export::Job { video: video.clone(), project, events: tel.events, gif, quality, dest: dest.clone(), cancel: cancel.clone() };
        match export::run(&job, &progress) {
            Ok(()) => {
                if copy {
                    core::clipboard::copy_file(&dest);
                    core::toast(&app, core::t("In die Zwischenablage kopiert", "Copied to clipboard"));
                } else {
                    core::toast(&app, core::t("Aufnahme gesichert", "Recording saved"));
                }
                let _ = app.emit_to(&label, "receditor-done", json!({ "path": dest, "copy": copy }));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&dest);
                let cancelled = cancel.load(Ordering::Relaxed);
                let _ = app.emit_to(&label, "receditor-error", json!({ "message": e, "cancelled": cancelled }));
            }
        }
        if let Some(m) = CANCEL.lock().unwrap().as_mut() {
            m.remove(&label);
        }
    });
    Ok(result)
}

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    let video = || a["video"].as_str().map(PathBuf::from).ok_or_else(|| "video fehlt".to_string());
    match cmd {
        "load" => {
            let v = video()?;
            let (p, tel) = load_project(&v)?;
            Ok(json!({ "project": p, "events": tel.events, "hasTelemetry": !tel.events.is_empty(), "outputFormat": tel.output_format }))
        }
        "save" => {
            let p: RecordingProject = serde_json::from_value(a["project"].clone()).map_err(|e| e.to_string())?;
            save_project(&video()?, &p)?;
            Ok(Value::Null)
        }
        "export" => start_export(app, win.label().to_owned(), &a),
        "cancel" => {
            if let Some(c) = CANCEL.lock().unwrap().as_ref().and_then(|m| m.get(win.label()).cloned()) {
                c.store(true, Ordering::Relaxed);
            }
            Ok(Value::Null)
        }
        "chooseImage" => Ok(json!(choose_image())),
        "reveal" => {
            core::reveal(&video()?);
            Ok(Value::Null)
        }
        _ => Err(format!("receditor: unbekannter Befehl {cmd}")),
    }
}
