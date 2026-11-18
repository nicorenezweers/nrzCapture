//! Gemeinsame Helfer für alle Module: Shell, Pfade, Sprache, Fenster, Zwischenablage, Berechtigungen, Töne.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

// ───────────────────────────── Shell & Pfade ─────────────────────────────

pub fn sh(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// Echter Benutzerordner aus der Benutzerdatenbank – `$HOME` kann je nach Startweg falsch sein.
pub fn home() -> String {
    static H: Mutex<Option<String>> = Mutex::new(None);
    if let Some(h) = H.lock().unwrap().clone() {
        return h;
    }
    let h = sh("/usr/bin/dscl", &[".", "-read", &format!("/Users/{}", sh("/usr/bin/id", &["-un"]).unwrap_or_default()), "NFSHomeDirectory"])
        .and_then(|s| s.split_once(": ").map(|(_, h)| h.trim().to_owned()))
        .filter(|h| Path::new(h).is_dir())
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_default());
    *H.lock().unwrap() = Some(h.clone());
    h
}

/// `~/Library/Application Support/com.nrz.capture` (wird angelegt).
pub fn data_dir() -> PathBuf {
    let d = PathBuf::from(home()).join("Library/Application Support/com.nrz.capture");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Temporärer Arbeitsordner (Aufnahmen, die noch nicht gespeichert wurden, Vorschaubilder, …).
pub fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join("nrzcapture");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Neuer eindeutiger Pfad im Temp-Ordner, z. B. `temp_file("png")`.
pub fn temp_file(ext: &str) -> PathBuf {
    temp_dir().join(format!("{}.{ext}", unique_id()))
}

pub fn unique_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{:x}{:04x}", t.as_millis(), N.fetch_add(1, Ordering::Relaxed) & 0xffff)
}

/// Zielordner zum Speichern: Einstellung `exportLocation`, sonst macOS-Screenshot-Ort, sonst Schreibtisch.
/// Mit `screenshotMonthlyFolders` wird ein Unterordner `YYYY-MM` angehängt.
pub fn export_dir() -> PathBuf {
    let s = crate::settings::get();
    let mut dir = Some(s.export_location.clone())
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(p.replacen('~', &home(), 1)))
        .or_else(|| sh("defaults", &["read", "com.apple.screencapture", "location"]).map(|s| PathBuf::from(s.replacen('~', &home(), 1))))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from(home()).join("Desktop"));
    if s.screenshot_monthly_folders {
        dir = dir.join(chrono::Local::now().format("%Y-%m").to_string());
    }
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Dateiname aus Vorlage. Platzhalter: {source} {app} {window} {random} {timestamp} {date} {time}.
/// `source` wird unverändert für {source} eingesetzt (z. B. " - Safari" oder leer); {app}/{window} bleiben leer.
pub fn file_name(template: &str, source: &str, ext: &str) -> String {
    render_file_name(template, source, "", "", ext)
}

/// Wie `file_name`, aber mit Quell-App und Fenstertitel (Capso `FileNaming.renderTemplate`):
/// {source} = " - App" (leer ohne App), {app} = App, {window} = Fenstertitel.
pub fn file_name_with(template: &str, app: Option<&str>, window: Option<&str>, ext: &str) -> String {
    let app = sanitize_token(app);
    let window = sanitize_token(window);
    let source = if app.is_empty() { String::new() } else { format!(" - {app}") };
    render_file_name(template, &source, &app, &window, ext)
}

fn sanitize_token(v: Option<&str>) -> String {
    v.unwrap_or("").trim().chars().filter(|c| *c as u32 >= 0x20).map(|c| if matches!(c, '/' | ':') { '-' } else { c }).collect::<String>().trim().to_string()
}

fn random_token(len: usize) -> String {
    const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = vec![0u8; len];
    if getrandom::getrandom(&mut buf).is_err() {
        let n = chrono::Local::now().timestamp_nanos_opt().unwrap_or(0) as u64;
        buf.iter_mut().enumerate().for_each(|(i, b)| *b = (n >> ((i * 7) % 56)) as u8);
    }
    buf.iter().map(|b| ALPHA[*b as usize % ALPHA.len()] as char).collect()
}

fn render_file_name(template: &str, source: &str, app: &str, window: &str, ext: &str) -> String {
    let now = chrono::Local::now();
    let mut name = template
        .replace("{source}", source)
        .replace("{app}", app)
        .replace("{window}", window)
        .replace("{timestamp}", &now.format("%Y-%m-%d at %H.%M.%S").to_string())
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H.%M.%S").to_string());
    while let Some(i) = name.find("{random}") {
        name.replace_range(i..i + 8, &random_token(8));
    }
    let mut name = name.replace(['/', ':'], "-").trim().to_string();
    while name.ends_with('.') {
        name.pop();
    }
    format!("{}.{ext}", name.trim())
}

/// Freier Pfad im Ordner (hängt " 2", " 3", … an, falls vorhanden).
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..).map(|i| dir.join(format!("{stem} {i}.{ext}"))).find(|p| !p.exists()).unwrap()
}

// ───────────────────────────── Sprache ─────────────────────────────

/// Zweisprachige UI-Texte: `t("Kopieren", "Copy")`.
pub fn t<'a>(de: &'a str, en: &'a str) -> &'a str {
    if crate::settings::get().lang() == "de" {
        de
    } else {
        en
    }
}

// ───────────────────────────── Fenster ─────────────────────────────

/// Optionen für `window()`. Alle Größen in logischen Punkten.
#[derive(Clone, Debug)]
pub struct Win {
    pub title: String,
    pub w: f64,
    pub h: f64,
    pub pos: Option<(f64, f64)>,
    pub center: bool,
    pub decorations: bool,
    pub transparent: bool,
    pub shadow: bool,
    pub always_on_top: bool,
    pub resizable: bool,
    pub focus: bool,
    pub visible: bool,
    pub skip_taskbar: bool,
    pub all_spaces: bool,
    /// Optionales NSWindow-Level (z. B. `LEVEL_OVERLAY`), setzt auch „über Vollbild-Apps“.
    pub level: Option<isize>,
    pub min_size: Option<(f64, f64)>,
    /// Titelleiste transparent mit Ampel-Knöpfen über dem Inhalt (wie SwiftUI-Fenster).
    pub overlay_titlebar: bool,
}

impl Default for Win {
    fn default() -> Self {
        Self {
            title: "nrzCapture".into(),
            w: 400.0,
            h: 300.0,
            pos: None,
            center: false,
            decorations: false,
            transparent: true,
            shadow: false,
            always_on_top: true,
            resizable: false,
            focus: false,
            visible: true,
            skip_taskbar: true,
            all_spaces: true,
            level: None,
            min_size: None,
            overlay_titlebar: false,
        }
    }
}

impl Win {
    /// Normales App-Fenster (Einstellungen, Verlauf, Editor): Rahmen, Schatten, fokussiert, nicht immer oben.
    pub fn app(title: &str, w: f64, h: f64) -> Self {
        Self {
            title: title.into(),
            w,
            h,
            center: true,
            decorations: true,
            transparent: false,
            shadow: true,
            always_on_top: false,
            resizable: true,
            focus: true,
            all_spaces: false,
            overlay_titlebar: true,
            ..Default::default()
        }
    }
}

pub const LEVEL_FLOATING: isize = 3;
pub const LEVEL_POPUP: isize = 101;
pub const LEVEL_SCREENSAVER: isize = 1000;
/// Über Menüleiste & Dock (Auswahl-Overlays).
pub const LEVEL_OVERLAY: isize = 1000;

static WINDOW_DATA: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);

/// Startdaten für ein Fenster hinterlegen; das Frontend holt sie mit `rpc("core","windowData")`.
pub fn set_window_data(label: &str, v: Value) {
    WINDOW_DATA.lock().unwrap().get_or_insert_with(HashMap::new).insert(label.to_owned(), v);
}

pub fn window_data(label: &str) -> Value {
    WINDOW_DATA.lock().unwrap().as_ref().and_then(|m| m.get(label).cloned()).unwrap_or(Value::Null)
}

/// Fenster mit Route (`index.html#<route>`) öffnen oder – falls `label` existiert – zeigen.
/// `data` wird als Fensterdaten hinterlegt und zusätzlich als Event `window-data` an ein bestehendes Fenster gesendet.
pub fn window(app: &AppHandle, label: &str, route: &str, o: Win, data: Value) -> Option<WebviewWindow> {
    set_window_data(label, data.clone());
    if let Some(w) = app.get_webview_window(label) {
        use tauri::Emitter;
        let _ = w.emit_to(label, "window-data", data);
        if o.visible {
            let _ = w.show();
            if o.focus {
                let _ = w.set_focus();
            }
        }
        return Some(w);
    }
    let mut b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(format!("index.html#{route}").into()))
        .title(&o.title)
        .inner_size(o.w, o.h)
        .decorations(o.decorations)
        .transparent(o.transparent)
        .shadow(o.shadow)
        .always_on_top(o.always_on_top)
        .resizable(o.resizable)
        .focused(o.focus)
        .visible(o.visible)
        .skip_taskbar(o.skip_taskbar)
        .visible_on_all_workspaces(o.all_spaces)
        .accept_first_mouse(true);
    if o.overlay_titlebar {
        b = b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    }
    if let Some((x, y)) = o.pos {
        b = b.position(x, y);
    } else if o.center {
        b = b.center();
    }
    if let Some((w, h)) = o.min_size {
        b = b.min_inner_size(w, h);
    }
    let w = match b.build() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Fenster {label}: {e}");
            return None;
        }
    };
    if let Some(l) = o.level {
        set_level(&w, l);
    }
    #[cfg(debug_assertions)]
    eprintln!("Fenster geöffnet: {label} (#{route})");
    Some(w)
}

/// NSWindow-Level setzen und das Fenster über allen Spaces / Vollbild-Apps zeigen.
pub fn set_level(w: &WebviewWindow, level: isize) {
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        if let Ok(ptr) = w2.ns_window() {
            use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior as B};
            let nw = unsafe { &*(ptr as *const NSWindow) };
            nw.setLevel(level);
            nw.setCollectionBehavior(B::CanJoinAllSpaces | B::FullScreenAuxiliary | B::Stationary | B::IgnoresCycle);
        }
    });
}

/// Fenster bei der Bildschirmaufnahme ausblenden (eigene UI nicht mitfilmen/-fotografieren).
pub fn exclude_from_capture(w: &WebviewWindow, exclude: bool) {
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        if let Ok(ptr) = w2.ns_window() {
            use objc2_app_kit::{NSWindow, NSWindowSharingType};
            let nw = unsafe { &*(ptr as *const NSWindow) };
            nw.setSharingType(if exclude { NSWindowSharingType::None } else { NSWindowSharingType::ReadOnly });
        }
    });
}

/// Fenster auf einem Monitor platzieren und auf dessen volle Größe ziehen.
pub fn cover_monitor(w: &WebviewWindow, m: &Monitor) {
    let (s, p, sz) = (m.scale_factor(), m.position(), m.size());
    let _ = w.set_position(LogicalPosition::new(p.x as f64 / s, p.y as f64 / s));
    let _ = w.set_size(LogicalSize::new(sz.width as f64 / s, sz.height as f64 / s));
}

pub fn cursor_monitor(app: &AppHandle) -> Option<Monitor> {
    app.cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// Logische Bildschirmdaten für das Frontend / Geometrie.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    /// Arbeitsbereich (ohne Menüleiste/Dock), logisch.
    pub work: [f64; 4],
}

pub fn screens(app: &AppHandle) -> Vec<Screen> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let s = m.scale_factor();
            let wa = m.work_area();
            Screen {
                id: i as u32,
                x: m.position().x as f64 / s,
                y: m.position().y as f64 / s,
                width: m.size().width as f64 / s,
                height: m.size().height as f64 / s,
                scale: s,
                work: [wa.position.x as f64 / s, wa.position.y as f64 / s, wa.size.width as f64 / s, wa.size.height as f64 / s],
            }
        })
        .collect()
}

// ───────────────────────────── Zwischenablage ─────────────────────────────

pub mod clipboard {
    use objc2::{rc::Retained, runtime::ProtocolObject};
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardTypeTIFF, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData, NSString, NSURL};
    use std::path::{Path, PathBuf};

    fn pb() -> Retained<NSPasteboard> {
        NSPasteboard::generalPasteboard()
    }

    /// Bilddatei als Bild (PNG/TIFF je nach Datei) in die Zwischenablage.
    pub fn copy_image(path: &Path) -> bool {
        let Ok(bytes) = std::fs::read(path) else { return false };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let (bytes, ty) = if ext == "png" {
            (bytes, unsafe { NSPasteboardTypePNG })
        } else if ext == "tiff" || ext == "tif" {
            (bytes, unsafe { NSPasteboardTypeTIFF })
        } else {
            // JPEG/HEIC/WebP: über sips nach PNG wandeln, damit jede App es einfügen kann.
            let tmp = super::temp_file("png");
            super::sh("sips", &["-s", "format", "png", &path.to_string_lossy(), "--out", &tmp.to_string_lossy()]);
            match std::fs::read(&tmp) {
                Ok(b) => (b, unsafe { NSPasteboardTypePNG }),
                Err(_) => return false,
            }
        };
        let pb = pb();
        pb.clearContents();
        let data = NSData::with_bytes(&bytes);
        pb.setData_forType(Some(&data), ty)
    }

    /// Datei (beliebig, auch Video/GIF) als Datei-Referenz in die Zwischenablage – wie ⌘C im Finder.
    pub fn copy_file(path: &Path) -> bool {
        let pb = pb();
        pb.clearContents();
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let obj: Retained<ProtocolObject<dyn NSPasteboardWriting>> = ProtocolObject::from_retained(url);
        pb.writeObjects(&NSArray::from_retained_slice(&[obj]))
    }

    pub fn copy_text(text: &str) -> bool {
        let pb = pb();
        pb.clearContents();
        pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
    }

    pub fn read_text() -> Option<String> {
        pb().stringForType(unsafe { NSPasteboardTypeString }).map(|s| s.to_string())
    }

    /// Bild aus der Zwischenablage als PNG-Datei im Temp-Ordner (oder Bilddatei-Pfad, falls eine Datei kopiert wurde).
    pub fn read_image() -> Option<PathBuf> {
        let pb = pb();
        if let Some(d) = pb.dataForType(unsafe { NSPasteboardTypePNG }) {
            let p = super::temp_file("png");
            std::fs::write(&p, d.to_vec()).ok()?;
            return Some(p);
        }
        if let Some(d) = pb.dataForType(unsafe { NSPasteboardTypeTIFF }) {
            let t = super::temp_file("tiff");
            std::fs::write(&t, d.to_vec()).ok()?;
            let p = t.with_extension("png");
            super::sh("sips", &["-s", "format", "png", &t.to_string_lossy(), "--out", &p.to_string_lossy()])?;
            let _ = std::fs::remove_file(&t);
            return Some(p);
        }
        // Kopierte Bilddatei (Finder)
        let s = pb.stringForType(&NSString::from_str("public.file-url"))?.to_string();
        let path = PathBuf::from(urlencoding_decode(s.strip_prefix("file://")?));
        let ext = path.extension()?.to_str()?.to_lowercase();
        ["png", "jpg", "jpeg", "heic", "tiff", "gif", "webp", "bmp"].contains(&ext.as_str()).then_some(path)
    }

    fn urlencoding_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

// ───────────────────────────── Berechtigungen ─────────────────────────────

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub fn can_record() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// Fragt die Berechtigung „Bildschirmaufnahme“ an und öffnet sonst die passende Einstellungsseite.
pub fn ask_screen_permission() {
    if !unsafe { CGRequestScreenCaptureAccess() } {
        open_privacy("Privacy_ScreenCapture");
    }
}

pub fn has_accessibility() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Systemeinstellungen → Datenschutz, z. B. "Privacy_ScreenCapture", "Privacy_Accessibility", "Privacy_Camera", "Privacy_Microphone".
pub fn open_privacy(anchor: &str) {
    sh("open", &[&format!("x-apple.systempreferences:com.apple.preference.security?{anchor}")]);
}

// ───────────────────────────── Töne & Sonstiges ─────────────────────────────

/// Systemton abspielen (asynchron), z. B. `play_sound("/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/system/Screen Capture.aif")`.
pub fn play_sound(path: &str) {
    let p = path.to_owned();
    std::thread::spawn(move || {
        let _ = Command::new("/usr/bin/afplay").arg(p).output();
    });
}

pub const SHUTTER: &str = "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/system/Screen Capture.aif";

/// Auslöserton, falls in den Einstellungen aktiv.
pub fn shutter() {
    if crate::settings::get().play_shutter_sound {
        play_sound(SHUTTER);
    }
}

/// Im Finder zeigen.
pub fn reveal(path: &Path) {
    let _ = Command::new("open").arg("-R").arg(path).spawn();
}

/// Mit Standard-App öffnen (Datei oder URL).
pub fn open(target: &str) {
    let _ = Command::new("open").arg(target).spawn();
}

/// Kleine Hinweis-Einblendung (z. B. „Text kopiert“). Implementiert im Quick-Access-Modul.
pub fn toast(app: &AppHandle, msg: &str) {
    crate::quick::toast(app, msg);
}

/// Bildgröße in Pixeln (über `sips`, funktioniert für alle von macOS unterstützten Formate).
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    let out = sh("sips", &["-g", "pixelWidth", "-g", "pixelHeight", &path.to_string_lossy()])?;
    let num = |k: &str| out.lines().find(|l| l.contains(k))?.split(':').nth(1)?.trim().parse().ok();
    Some((num("pixelWidth")?, num("pixelHeight")?))
}

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    use serde_json::json;
    let path = || a["path"].as_str().map(PathBuf::from).ok_or_else(|| "path fehlt".to_string());
    match cmd {
        "windowData" => Ok(window_data(win.label())),
        // Frontend-Fehler/Panics im Backend-Log sichtbar machen.
        "log" => {
            if a["debug"].as_bool() == Some(true) && !cfg!(debug_assertions) {
                return Ok(Value::Null);
            }
            eprintln!("[{}] {}", win.label(), a["message"].as_str().unwrap_or(""));
            Ok(Value::Null)
        }
        "screens" => Ok(serde_json::to_value(screens(app)).unwrap()),
        "copyImage" => Ok(json!(clipboard::copy_image(&path()?))),
        "copyFile" => Ok(json!(clipboard::copy_file(&path()?))),
        "copyText" => Ok(json!(clipboard::copy_text(a["text"].as_str().unwrap_or("")))),
        "readClipboardText" => Ok(json!(clipboard::read_text())),
        "reveal" => {
            reveal(&path()?);
            Ok(Value::Null)
        }
        "open" => {
            open(a["target"].as_str().unwrap_or(""));
            Ok(Value::Null)
        }
        "toast" => {
            toast(app, a["message"].as_str().unwrap_or(""));
            Ok(Value::Null)
        }
        "imageSize" => Ok(json!(image_size(&path()?))),
        "permissions" => Ok(json!({
            "screen": can_record(),
            "accessibility": has_accessibility(),
        })),
        "askScreenPermission" => {
            ask_screen_permission();
            Ok(Value::Null)
        }
        "openPrivacy" => {
            open_privacy(a["anchor"].as_str().unwrap_or(""));
            Ok(Value::Null)
        }
        "close" => {
            let _ = win.close();
            Ok(Value::Null)
        }
        "hide" => {
            let _ = win.hide();
            Ok(Value::Null)
        }
        "version" => Ok(json!(app.package_info().version.to_string())),
        "quit" => {
            app.exit(0);
            Ok(Value::Null)
        }
        "restart" => app.restart(),
        _ => Err(format!("core: unbekannter Befehl {cmd}")),
    }
}

// ───────────────────────────── Diagnose-Protokoll ─────────────────────────────

/// Zeile ins Diagnose-Protokoll `data_dir()/logs/nrzcapture.log` schreiben – nur wenn
/// `diagnosticLoggingEnabled` an ist (Capso `DiagnosticLogger.append`). Rotiert ab 1 MB nach `nrzcapture.old.log`.
pub fn diag(category: &str, msg: &str) {
    let on = crate::settings::get_extra("diagnosticLoggingEnabled").and_then(|v| v.as_bool()).unwrap_or(false);
    if on {
        diag_write(&data_dir().join("logs").join("nrzcapture.log"), category, msg);
    }
}

/// Schreibt unabhängig von der Einstellung (für Tests/`diag`). Fehler werden ignoriert.
pub fn diag_write(file: &Path, category: &str, msg: &str) {
    use std::io::Write;
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(file).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let old = file.with_extension("old.log");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(file, &old);
    }
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{ts} [{category}] {msg}");
    }
}

// ───────────────────────────── Schlüsselbund ─────────────────────────────

/// Kennwort aus dem macOS-Schlüsselbund (generisches Kennwort, über `security`). `None` = nicht vorhanden.
pub fn keychain_get(service: &str, account: &str) -> Option<String> {
    sh("/usr/bin/security", &["find-generic-password", "-s", service, "-a", account, "-w"]).filter(|s| !s.is_empty())
}

/// Kennwort speichern (leer/Whitespace = löschen, wie Capso `KeychainHelper`). Rückgabe: Erfolg.
pub fn keychain_set(service: &str, account: &str, value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() {
        keychain_delete(service, account);
        return true;
    }
    sh("/usr/bin/security", &["add-generic-password", "-U", "-s", service, "-a", account, "-l", service, "-w", v]).is_some()
}

pub fn keychain_delete(service: &str, account: &str) {
    let _ = sh("/usr/bin/security", &["delete-generic-password", "-s", service, "-a", account]);
}

#[cfg(test)]
mod file_name_tests {
    use super::*;

    #[test]
    fn tokens() {
        let n = file_name_with("Shot{source} {app}|{window}|{random}", Some("Safari"), Some("a/b: c"), "png");
        let (head, rest) = n.split_once('|').unwrap();
        assert_eq!(head, "Shot - Safari Safari");
        let (win, rnd) = rest.split_once('|').unwrap();
        assert_eq!(win, "a-b- c");
        assert_eq!(rnd.len(), 8 + 4);
        assert_eq!(file_name_with("X{source}", None, None, "png"), "X.png");
        assert_eq!(file_name("X{source}.", " (Area)", "png"), "X (Area).png");
    }
}
