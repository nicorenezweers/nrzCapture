//! Screenshot-Verlauf (Bilder, GIFs, Videos) mit Aufbewahrung. Port von Capso `HistoryKit`
//! (HistoryStore, HistoryEntry, HistoryCleanup, ThumbnailGenerator, PendingHistoryCloudURLTracker)
//! und `App/Sources/History` (HistoryCoordinator: Kopieren, Sichern, Bearbeiten, PDF-Export, Ziehen, …).
//!
//! Ablage: `data_dir()/history/index.json` + je Eintrag ein Ordner `history/<id>/` mit `capture.<ext>`
//! und `thumbnail.jpg` (Videos: `thumbnail.png`).
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, Once},
    time::Duration,
};
use tauri::{AppHandle, Emitter, WebviewWindow};

use crate::core::{self, t, Win};

// ───────────────────────────── Datenmodell ─────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// Unix-Zeit in Millisekunden.
    pub created_at: i64,
    /// "screenshot" | "recording" | "gif"
    pub kind: String,
    /// Dateiname im Eintragsordner, z. B. `capture.png`.
    pub file_name: String,
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// Ursprünglicher Pfad (Exportordner / Temp-Datei).
    #[serde(default)]
    pub original_path: Option<String>,
    /// Anzeigename (ursprünglicher Dateiname).
    #[serde(default)]
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    #[serde(default)]
    pub cloud_url: Option<String>,
    /// Aufnahmemodus (Capso `HistoryCaptureMode`): "area" | "fullscreen" | "window" | "scroll" | "" (unbekannt).
    #[serde(default)]
    pub capture_mode: String,
    /// Quell-App (Capso `sourceAppName`).
    #[serde(default)]
    pub source_app_name: Option<String>,
    /// Fenstertitel (Capso `sourceWindowTitle`).
    #[serde(default)]
    pub source_window_title: Option<String>,
}

impl Entry {
    pub fn is_screenshot(&self) -> bool {
        self.kind == "screenshot"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Screenshots,
    Recordings,
    Gifs,
}

impl Filter {
    pub fn parse(s: &str) -> Self {
        match s {
            "screenshots" => Self::Screenshots,
            "recordings" => Self::Recordings,
            "gifs" => Self::Gifs,
            _ => Self::All,
        }
    }
    pub fn matches(self, e: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::Screenshots => e.kind == "screenshot",
            Self::Recordings => e.kind == "recording",
            Self::Gifs => e.kind == "gif",
        }
    }
}

/// Tage der Aufbewahrung, `None` = unbegrenzt.
pub fn retention_days(r: &str) -> Option<i64> {
    match r {
        "oneDay" => Some(1),
        "oneWeek" => Some(7),
        "twoWeeks" => Some(14),
        "oneMonth" => Some(30),
        "threeMonths" => Some(90),
        _ => None,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ───────────────────────────── Store ─────────────────────────────

/// JSON-Index + Eintragsordner. Nicht threadsicher – App-Zugriffe laufen über `with_store`.
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }
    fn index(&self) -> PathBuf {
        self.dir.join("index.json")
    }
    pub fn entry_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }
    pub fn file_path(&self, e: &Entry) -> PathBuf {
        self.entry_dir(&e.id).join(&e.file_name)
    }
    pub fn thumb_path(&self, e: &Entry) -> Option<PathBuf> {
        e.thumbnail.as_ref().map(|t| self.entry_dir(&e.id).join(t)).filter(|p| p.exists())
    }
    /// Alle Einträge, neueste zuerst.
    pub fn load(&self) -> Vec<Entry> {
        let mut v: Vec<Entry> = std::fs::read(self.index()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        v.sort_by_key(|e| std::cmp::Reverse(e.created_at));
        v
    }
    fn save(&self, v: &[Entry]) {
        let tmp = self.dir.join("index.json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(v).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, self.index());
        }
    }
    pub fn insert(&self, e: Entry) {
        let mut v = self.load();
        v.retain(|x| x.id != e.id);
        v.push(e);
        v.sort_by_key(|e| std::cmp::Reverse(e.created_at));
        self.save(&v);
    }
    pub fn update(&self, e: &Entry) -> bool {
        let mut v = self.load();
        let Some(x) = v.iter_mut().find(|x| x.id == e.id) else { return false };
        *x = e.clone();
        self.save(&v);
        true
    }
    pub fn get(&self, id: &str) -> Option<Entry> {
        self.load().into_iter().find(|e| e.id == id)
    }
    pub fn fetch_all(&self, f: Filter) -> Vec<Entry> {
        self.load().into_iter().filter(|e| f.matches(e)).collect()
    }
    /// Cloud-URL setzen/löschen; `false`, wenn die ID unbekannt ist.
    pub fn set_cloud_url(&self, id: &str, url: Option<&str>) -> bool {
        let mut v = self.load();
        let Some(e) = v.iter_mut().find(|e| e.id == id) else { return false };
        e.cloud_url = url.map(str::to_owned);
        self.save(&v);
        true
    }
    /// Eintrag samt Ordner löschen.
    pub fn delete(&self, id: &str) -> Option<Entry> {
        let mut v = self.load();
        let i = v.iter().position(|e| e.id == id)?;
        let e = v.remove(i);
        self.save(&v);
        let _ = std::fs::remove_dir_all(self.entry_dir(id));
        Some(e)
    }
    /// Alle Einträge älter als `cutoff_ms` löschen (samt Dateien), liefert die gelöschten.
    pub fn delete_older_than(&self, cutoff_ms: i64) -> Vec<Entry> {
        let (old, keep): (Vec<Entry>, Vec<Entry>) = self.load().into_iter().partition(|e| e.created_at < cutoff_ms);
        if !old.is_empty() {
            self.save(&keep);
            for e in &old {
                let _ = std::fs::remove_dir_all(self.entry_dir(&e.id));
            }
        }
        old
    }
    pub fn clear_all(&self) {
        for e in self.load() {
            let _ = std::fs::remove_dir_all(self.entry_dir(&e.id));
        }
        self.save(&[]);
    }
    /// Eintrag zu einem Pfad (Verlaufsdatei selbst oder ursprünglicher Pfad).
    pub fn find_by_path(&self, path: &Path) -> Option<Entry> {
        let p = path.to_string_lossy();
        self.load().into_iter().find(|e| self.file_path(e) == path || e.original_path.as_deref() == Some(&*p))
    }
    pub fn total_size(&self) -> u64 {
        self.load().iter().map(|e| e.file_size).sum()
    }
}

/// Retention durchsetzen (Capso `HistoryCleanup.enforce`), liefert Anzahl gelöschter Einträge.
pub fn enforce_retention(store: &Store, retention: &str, now_ms: i64) -> usize {
    match retention_days(retention) {
        Some(days) => store.delete_older_than(now_ms - days * 86_400_000).len(),
        None => 0,
    }
}

// ───────────────────────────── Ausstehende Cloud-URLs ─────────────────────────────

/// Überbrückt Uploads, die fertig werden, bevor der Verlaufseintrag geschrieben ist
/// (Capso `PendingHistoryCloudURLTracker`). Schlüssel = Quellpfad.
#[derive(Default, Debug)]
pub struct PendingCloudUrlTracker {
    pending: HashSet<String>,
    held: HashMap<String, String>,
}

impl PendingCloudUrlTracker {
    pub fn begin(&mut self, id: &str) {
        self.pending.insert(id.to_owned());
    }
    pub fn contains(&self, id: &str) -> bool {
        self.pending.contains(id)
    }
    pub fn held_url(&self, id: &str) -> Option<&str> {
        self.held.get(id).map(String::as_str)
    }
    pub fn hold(&mut self, url: &str, id: &str) -> bool {
        if !self.pending.contains(id) {
            return false;
        }
        self.held.insert(id.to_owned(), url.to_owned());
        true
    }
    pub fn finish(&mut self, id: &str) -> Option<String> {
        self.pending.remove(id);
        self.held.get(id).cloned()
    }
    pub fn complete_persistence(&mut self, id: &str) {
        self.held.remove(id);
    }
    pub fn cancel(&mut self, id: &str) {
        self.pending.remove(id);
        self.held.remove(id);
    }
}

// ───────────────────────────── App-Zustand ─────────────────────────────

static STORE_LOCK: Mutex<()> = Mutex::new(());
static PENDING: Mutex<Option<PendingCloudUrlTracker>> = Mutex::new(None);

fn history_dir() -> PathBuf {
    core::data_dir().join("history")
}

fn with_store<R>(f: impl FnOnce(&Store) -> R) -> R {
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    f(&Store::new(history_dir()))
}

fn pending<R>(f: impl FnOnce(&mut PendingCloudUrlTracker) -> R) -> R {
    f(PENDING.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default))
}

fn changed(app: &AppHandle) {
    let _ = app.emit("history-changed", ());
}

fn path_key(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn is_temp(p: &Path) -> bool {
    p.starts_with(core::temp_dir()) || p.starts_with(std::env::temp_dir())
}

fn kind_for(path: &Path, kind: &str) -> String {
    match kind {
        "screenshot" | "recording" | "gif" => kind.to_owned(),
        _ => match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
            "gif" => "gif".into(),
            "mp4" | "mov" | "m4v" | "webm" => "recording".into(),
            _ => "screenshot".into(),
        },
    }
}

/// Vorschaubild erzeugen (Capso: 640 px breit, JPEG). Videos über QuickLook (`qlmanage -t`).
fn make_thumbnail(src: &Path, dir: &Path, kind: &str) -> Option<String> {
    if kind == "recording" {
        let out = core::temp_dir().join(format!("ql-{}", core::unique_id()));
        let _ = std::fs::create_dir_all(&out);
        core::sh("qlmanage", &["-t", "-s", "640", "-o", &out.to_string_lossy(), &src.to_string_lossy()]);
        let made = std::fs::read_dir(&out).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "png"));
        let r = made.and_then(|p| std::fs::rename(&p, dir.join("thumbnail.png")).or_else(|_| std::fs::copy(&p, dir.join("thumbnail.png")).map(|_| ())).ok()).map(|_| "thumbnail.png".to_owned());
        let _ = std::fs::remove_dir_all(&out);
        return r;
    }
    let dst = dir.join("thumbnail.jpg");
    core::sh("sips", &["-Z", "640", "-s", "format", "jpeg", "-s", "formatOptions", "70", &src.to_string_lossy(), "--out", &dst.to_string_lossy()])?;
    dst.exists().then(|| "thumbnail.jpg".to_owned())
}

/// Videoabmessungen über Spotlight-Metadaten, sonst Seitenverhältnis des Vorschaubilds.
fn video_size(src: &Path, thumb: Option<&Path>) -> (u32, u32) {
    let md = |p: &Path| {
        let out = core::sh("mdls", &["-name", "kMDItemPixelWidth", "-name", "kMDItemPixelHeight", &p.to_string_lossy()])?;
        let num = |k: &str| out.lines().find(|l| l.contains(k))?.split('=').nth(1)?.trim().parse::<u32>().ok();
        Some((num("kMDItemPixelWidth")?, num("kMDItemPixelHeight")?))
    };
    md(src).or_else(|| thumb.and_then(core::image_size)).unwrap_or((0, 0))
}

/// Datei synchron in den Verlauf übernehmen (ohne Einstellungs-Prüfung).
/// Metadaten einer Aufnahme (Modus, Quell-App, Fenstertitel).
#[derive(Clone, Debug, Default)]
struct Meta {
    mode: String,
    app_name: Option<String>,
    window_title: Option<String>,
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn add_sync(app: &AppHandle, path: &Path, kind: &str) -> Option<Entry> {
    add_sync_meta(app, path, kind, Meta::default())
}

fn add_sync_meta(app: &AppHandle, path: &Path, kind: &str, meta: Meta) -> Option<Entry> {
    if !path.is_file() {
        return None;
    }
    let kind = kind_for(path, kind);
    let id = core::unique_id();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    let file_name = format!("capture.{ext}");
    let store_dir = history_dir();
    let dir = store_dir.join(&id);
    std::fs::create_dir_all(&dir).ok()?;
    let dst = dir.join(&file_name);
    // Klonen (APFS clonefile) ist billig; sonst normal kopieren.
    if core::sh("cp", &["-c", &path.to_string_lossy(), &dst.to_string_lossy()]).is_none() {
        std::fs::copy(path, &dst).ok()?;
    }
    // Begleitdateien einer Aufnahme (z. B. `<name>.cursor.json`) mitnehmen.
    if kind != "screenshot" {
        if let (Some(parent), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) {
            for f in std::fs::read_dir(parent).into_iter().flatten().flatten() {
                let n = f.file_name().to_string_lossy().into_owned();
                if n.starts_with(&format!("{stem}.")) && n.ends_with(".json") {
                    let _ = std::fs::copy(f.path(), dir.join(n.replacen(stem, "capture", 1)));
                }
            }
        }
    }
    let thumbnail = make_thumbnail(&dst, &dir, &kind);
    let (width, height) = if kind == "recording" {
        video_size(path, thumbnail.as_ref().map(|t| dir.join(t)).as_deref())
    } else {
        core::image_size(&dst).unwrap_or((0, 0))
    };
    let e = Entry {
        id,
        created_at: now_ms(),
        kind,
        file_name,
        thumbnail,
        original_path: Some(path_key(path)),
        // Temp-Dateien (noch nicht gesichert) haben keinen sprechenden Namen.
        name: if is_temp(path) { String::new() } else { path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() },
        width,
        height,
        file_size: std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0),
        cloud_url: None,
        capture_mode: meta.mode,
        source_app_name: meta.app_name,
        source_window_title: meta.window_title,
    };
    with_store(|s| s.insert(e.clone()));
    changed(app);
    Some(e)
}

/// Eintrag hinzufügen (Datei wird in den Verlaufsordner kopiert). `kind`: "screenshot" | "recording" | "gif".
/// Läuft im Hintergrund; respektiert `historyEnabled`.
pub fn add(app: &AppHandle, path: &Path, kind: &str) {
    add_meta(app, path, kind, "", None, None);
}

/// Wie `add`, zusätzlich mit Aufnahmemodus (`area` | `fullscreen` | `window` | `scroll` | ""), Quell-App und Fenstertitel
/// (Capso `HistoryEntry.captureMode/sourceAppName/sourceWindowTitle`).
pub fn add_meta(app: &AppHandle, path: &Path, kind: &str, mode: &str, app_name: Option<&str>, window_title: Option<&str>) {
    if !crate::settings::get().history_enabled || !path.is_file() {
        return;
    }
    let key = path_key(path);
    pending(|p| p.begin(&key));
    let meta = Meta { mode: mode.to_owned(), app_name: non_empty(app_name), window_title: non_empty(window_title) };
    let (app, path, kind) = (app.clone(), path.to_path_buf(), kind.to_owned());
    std::thread::spawn(move || {
        let e = add_sync_meta(&app, &path, &kind, meta);
        let held = pending(|p| p.finish(&key));
        match (e, held) {
            (Some(e), Some(url)) => {
                with_store(|s| s.set_cloud_url(&e.id, Some(&url)));
                pending(|p| p.complete_persistence(&key));
                changed(&app);
            }
            (None, _) => pending(|p| p.cancel(&key)),
            _ => {}
        }
    });
}

/// Für den Upload: Datei ist im Verlauf (oder wird gerade eingetragen)? Sonst jetzt hinzufügen.
pub fn ensure_entry(app: &AppHandle, path: &Path) {
    if !crate::settings::get().history_enabled || pending(|p| p.contains(&path_key(path))) {
        return;
    }
    if with_store(|s| s.find_by_path(path)).is_none() {
        add_sync(app, path, "");
    }
}

/// Cloud-URL nach erfolgreichem Upload im Verlaufseintrag speichern.
pub fn attach_cloud_url(app: &AppHandle, path: &Path, url: &str) {
    let key = path_key(path);
    if pending(|p| p.hold(url, &key)) {
        return;
    }
    if with_store(|s| s.find_by_path(path).map(|e| s.set_cloud_url(&e.id, Some(url)))).is_some() {
        changed(app);
    }
}

/// Bild eines Eintrags ersetzen (z. B. nach Bearbeitung im Editor): Datei austauschen, Vorschaubild neu erzeugen,
/// Größe/Abmessungen aktualisieren und Cloud-Link verwerfen (Capso `HistoryCoordinator.replaceImage`).
pub fn replace_entry_file(app: &AppHandle, id: &str, new_path: &Path) {
    if !new_path.is_file() {
        return;
    }
    let Some((mut e, old)) = entry(id).ok() else { return };
    if !e.is_screenshot() {
        return;
    }
    let dir = old.parent().map(Path::to_path_buf).unwrap_or_else(|| history_dir().join(id));
    let ext = new_path.extension().and_then(|x| x.to_str()).unwrap_or("png").to_lowercase();
    let file_name = format!("capture.{ext}");
    let dst = dir.join(&file_name);
    let tmp = dir.join(format!(".replace-{}.{ext}", core::unique_id()));
    if std::fs::copy(new_path, &tmp).is_err() || std::fs::rename(&tmp, &dst).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if dst != old {
        let _ = std::fs::remove_file(&old);
    }
    if let Some(t) = &e.thumbnail {
        let _ = std::fs::remove_file(dir.join(t));
    }
    e.thumbnail = make_thumbnail(&dst, &dir, &e.kind);
    let (w, h) = core::image_size(&dst).unwrap_or((e.width, e.height));
    e.width = w;
    e.height = h;
    e.file_name = file_name;
    e.file_size = std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
    // Das bearbeitete Bild passt nicht mehr zum hochgeladenen Objekt.
    e.cloud_url = None;
    let _ = std::fs::remove_dir_all(core::temp_dir().join("history-drag").join(id));
    with_store(|s| s.update(&e));
    changed(app);
}

/// Zeitpunkt (ms) des neuesten Verlaufseintrags – z. B. um auf eine neue Aufnahme zu warten.
pub fn latest_created_at() -> Option<i64> {
    with_store(|s| s.load().iter().map(|e| e.created_at).max())
}

/// Verlaufsfenster öffnen bzw. per Kürzel umschalten: sichtbar und nicht minimiert → ausblenden
/// (Capso `HistoryCoordinator.toggleWindow` / `HistoryWindow.toggle`).
pub fn toggle(app: &AppHandle) {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("history") {
        if w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false) {
            let _ = w.hide();
            crate::prefs::update_dock_presence(app);
            return;
        }
    }
    open(app);
}

/// Verlaufsfenster öffnen.
pub fn open(app: &AppHandle) {
    core::window(
        app,
        "history",
        "history",
        Win { min_size: Some((480.0, 400.0)), ..Win::app(t("Bildschirmfoto-Verlauf", "Screenshot History"), 760.0, 560.0) },
        Value::Null,
    );
}

/// Alte Einträge gemäß Aufbewahrung löschen. Startet beim ersten Aufruf zusätzlich eine stündliche Prüfung.
pub fn cleanup(app: &AppHandle) {
    let n = with_store(|s| enforce_retention(s, &crate::settings::get().history_retention, now_ms()));
    if n > 0 {
        changed(app);
    }
    static PERIODIC: Once = Once::new();
    PERIODIC.call_once(|| {
        let app = app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(3600));
            cleanup(&app);
        });
    });
}

// ───────────────────────────── Aktionen ─────────────────────────────

fn entry(id: &str) -> Result<(Entry, PathBuf), String> {
    with_store(|s| s.get(id).map(|e| (e.clone(), s.file_path(&e)))).ok_or_else(|| t("Eintrag nicht gefunden", "Entry not found").into())
}

fn ids(a: &Value) -> Vec<String> {
    let mut v: Vec<String> = a["ids"].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    if let Some(id) = a["id"].as_str() {
        v.push(id.to_owned());
    }
    v
}

/// Dateiname zum Sichern/Ziehen: ursprünglicher Name oder Vorlage.
fn export_name(e: &Entry) -> String {
    if !e.name.is_empty() {
        return e.name.clone();
    }
    let ext = e.file_name.rsplit('.').next().unwrap_or("png");
    let s = crate::settings::get();
    let dt = chrono::DateTime::from_timestamp_millis(e.created_at).map(|d| d.with_timezone(&chrono::Local)).unwrap_or_else(chrono::Local::now);
    let template = if e.is_screenshot() { s.screenshot_filename_template.clone() } else { "nrzCapture {timestamp}".into() };
    // Quell-App/Fenster nur bei Bildschirmfotos (Capso `saveToFile`).
    let clean = |o: &Option<String>| o.as_deref().map(|v| v.replace(['/', ':'], "-").trim().to_owned()).filter(|v| !v.is_empty() && e.is_screenshot());
    let (source, window) = (clean(&e.source_app_name), clean(&e.source_window_title));
    let name = template
        .replace("{source}", &source.as_deref().map(|a| format!(" - {a}")).unwrap_or_default())
        .replace("{app}", source.as_deref().unwrap_or(""))
        .replace("{window}", window.as_deref().unwrap_or(""))
        .replace("{timestamp}", &dt.format("%Y-%m-%d at %H.%M.%S").to_string())
        .replace("{date}", &dt.format("%Y-%m-%d").to_string())
        .replace("{time}", &dt.format("%H.%M.%S").to_string())
        .replace(['/', ':'], "-");
    format!("{}.{ext}", name.trim())
}

fn copy(app: &AppHandle, id: &str) -> Result<(), String> {
    let (e, p) = entry(id)?;
    let ok = if e.is_screenshot() { core::clipboard::copy_image(&p) } else { core::clipboard::copy_file(&p) };
    core::toast(app, if ok { t("In die Zwischenablage kopiert", "Copied to clipboard") } else { t("Kopieren fehlgeschlagen", "Copy failed") });
    Ok(())
}

fn save_to_export(app: &AppHandle, list: &[String]) -> Result<Value, String> {
    let dir = core::export_dir();
    let mut last = None;
    for id in list {
        let (e, p) = entry(id)?;
        let dst = core::unique_path(&dir, &export_name(&e));
        std::fs::copy(&p, &dst).map_err(|e| e.to_string())?;
        last = Some(dst);
    }
    if let Some(l) = &last {
        core::toast(app, t("Gesichert", "Saved"));
        return Ok(json!(l));
    }
    Ok(Value::Null)
}

/// Native „Sichern unter …“-Abfrage über AppleScript. Liefert POSIX-Pfad.
fn choose_save_path(prompt: &str, default_name: &str) -> Option<PathBuf> {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "tell application \"System Events\" to activate\nset f to choose file name with prompt \"{}\" default name \"{}\" default location (POSIX file \"{}\")\nPOSIX path of f",
        esc(prompt),
        esc(default_name),
        esc(&core::export_dir().to_string_lossy())
    );
    core::sh("osascript", &["-e", &script]).filter(|s| !s.is_empty()).map(PathBuf::from)
}

fn save_as(app: &AppHandle, id: &str) -> Result<Value, String> {
    let (e, p) = entry(id)?;
    let Some(dst) = choose_save_path(t("Sichern unter", "Save to"), &export_name(&e)) else { return Ok(Value::Null) };
    std::fs::copy(&p, &dst).map_err(|e| e.to_string())?;
    core::toast(app, t("Gesichert", "Saved"));
    Ok(json!(dst))
}

fn edit(app: &AppHandle, id: &str) -> Result<(), String> {
    let (e, p) = entry(id)?;
    match e.kind.as_str() {
        "screenshot" => crate::editor::open_with_meta(app, &p, json!({ "historyId": id })),
        "recording" => crate::receditor::open(app, &p),
        _ => core::open(&p.to_string_lossy()),
    }
    Ok(())
}

/// Ziehbare Kopie mit sprechendem Dateinamen (Capso `prepareDragFile`).
fn drag_file(e: &Entry, p: &Path) -> PathBuf {
    let dir = core::temp_dir().join("history-drag").join(&e.id);
    let _ = std::fs::create_dir_all(&dir);
    let dst = dir.join(export_name(e));
    if !dst.exists() && std::fs::hard_link(p, &dst).is_err() && std::fs::copy(p, &dst).is_err() {
        return p.to_path_buf();
    }
    dst
}

fn file_url(path: &str) -> String {
    let enc: String = path
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("file://{enc}")
}

/// Nativer Datei-Drag (ein oder mehrere Einträge) aus dem Verlaufsfenster.
fn drag(window: &WebviewWindow, list: &[String]) -> Result<(), String> {
    let mut files = vec![];
    let mut thumb = None;
    for id in list {
        let (e, p) = entry(id)?;
        if thumb.is_none() {
            thumb = with_store(|s| s.thumb_path(&e));
        }
        files.push(drag_file(&e, &p).to_string_lossy().into_owned());
    }
    let Some(first) = files.first().cloned() else { return Ok(()) };
    let url = file_url(&first);
    let xml_items: String = files.iter().map(|f| format!("<string>{}</string>", f.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"))).collect();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><array>{xml_items}</array></plist>"#
    );
    let img = thumb.unwrap_or_else(|| PathBuf::from(&first));
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        let item = drag::DragItem::Data {
            provider: Box::new(move |t| match t {
                "public.file-url" => Some(url.clone().into_bytes()),
                "NSFilenamesPboardType" => Some(plist.clone().into_bytes()),
                _ => None,
            }),
            types: vec!["public.file-url".into(), "NSFilenamesPboardType".into()],
        };
        let _ = drag::start_drag(&w, item, drag::Image::File(img), |_, _| {}, drag::Options::default());
    });
    Ok(())
}

fn delete(app: &AppHandle, list: &[String], also_cloud: bool) {
    let removed: Vec<Entry> = with_store(|s| list.iter().filter_map(|id| s.delete(id)).collect());
    changed(app);
    if also_cloud {
        let urls: Vec<String> = removed.into_iter().filter_map(|e| e.cloud_url).collect();
        if !urls.is_empty() {
            let app = app.clone();
            std::thread::spawn(move || {
                let failed = urls.iter().filter(|u| crate::share::delete_cloud_copy(u).is_err()).count();
                if failed > 0 {
                    core::toast(&app, t("Cloud-Kopie konnte nicht gelöscht werden", "Couldn't delete cloud copy"));
                }
            });
        }
    }
}

// ───────────────────────────── PDF-Export ─────────────────────────────

/// Minimales PDF aus JPEG-Seiten: (JPEG-Bytes, Pixelbreite, Pixelhöhe, Seitenbreite pt, Seitenhöhe pt).
pub fn pdf_from_jpegs(pages: &[(Vec<u8>, u32, u32, f64, f64)]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets: Vec<usize> = vec![];
    let n = pages.len();
    // Objekte: 1 Katalog, 2 Seiten, dann je Seite 3 Objekte (Page, Image, Content).
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect();
    obj(&mut out, format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).as_bytes());
    for (i, (jpeg, pw, ph, w, h)) in pages.iter().enumerate() {
        let (img, content) = (4 + i * 3, 5 + i * 3);
        obj(
            &mut out,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /XObject << /Im{i} {img} 0 R >> >> /Contents {content} 0 R >>").as_bytes(),
        );
        let mut im = format!("<< /Type /XObject /Subtype /Image /Width {pw} /Height {ph} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n", jpeg.len()).into_bytes();
        im.extend_from_slice(jpeg);
        im.extend_from_slice(b"\nendstream");
        obj(&mut out, &im);
        let cs = format!("q {w:.2} 0 0 {h:.2} 0 0 cm /Im{i} Do Q");
        obj(&mut out, format!("<< /Length {} >>\nstream\n{cs}\nendstream", cs.len()).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

/// Bilder (in Reihenfolge) als PDF schreiben; nicht lesbare Bilder werden übersprungen.
/// `false`, wenn kein Bild lesbar war (dann wird nichts geschrieben) – wie Capso `writePDF`.
pub fn write_pdf(images: &[PathBuf], dst: &Path) -> bool {
    let mut pages = vec![];
    for img in images {
        if !img.is_file() {
            continue;
        }
        let tmp = core::temp_file("jpg");
        if core::sh("sips", &["-s", "format", "jpeg", "-s", "formatOptions", "90", &img.to_string_lossy(), "--out", &tmp.to_string_lossy()]).is_none() {
            continue;
        }
        let (Ok(bytes), Some((pw, ph))) = (std::fs::read(&tmp), core::image_size(&tmp)) else { continue };
        let _ = std::fs::remove_file(&tmp);
        // Seitengröße in Punkten (wie NSImage): Pixel × 72 / DPI.
        let dpi = core::sh("sips", &["-g", "dpiWidth", &img.to_string_lossy()])
            .and_then(|o| o.lines().find(|l| l.contains("dpiWidth"))?.split(':').nth(1)?.trim().parse::<f64>().ok())
            .filter(|d| *d > 0.0)
            .unwrap_or(72.0);
        let s = 72.0 / dpi;
        pages.push((bytes, pw, ph, pw as f64 * s, ph as f64 * s));
    }
    !pages.is_empty() && std::fs::write(dst, pdf_from_jpegs(&pages)).is_ok()
}

fn export_pdf(app: &AppHandle, list: &[String]) -> Result<Value, String> {
    let mut shots: Vec<(Entry, PathBuf)> = list.iter().filter_map(|id| entry(id).ok()).filter(|(e, _)| e.is_screenshot()).collect();
    shots.sort_by_key(|(e, _)| e.created_at);
    let Some((first, _)) = shots.first() else { return Ok(Value::Null) };
    let date = chrono::DateTime::from_timestamp_millis(first.created_at).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string()).unwrap_or_default();
    let Some(mut dst) = choose_save_path(t("Als PDF exportieren", "Export as PDF"), &format!("nrzCapture {date}.pdf")) else { return Ok(Value::Null) };
    if dst.extension().is_none_or(|e| e != "pdf") {
        dst.set_extension("pdf");
    }
    let images: Vec<PathBuf> = shots.into_iter().map(|(_, p)| p).collect();
    let app = app.clone();
    std::thread::spawn(move || {
        if write_pdf(&images, &dst) {
            core::toast(&app, t("PDF exportiert", "PDF exported"));
        } else {
            core::toast(&app, t("PDF-Export fehlgeschlagen", "PDF export failed"));
        }
    });
    Ok(Value::Null)
}

// ───────────────────────────── RPC ─────────────────────────────

fn list_json(filter: Filter) -> Value {
    let s = crate::settings::get();
    with_store(|st| {
        let all = st.load();
        let count = |f: Filter| all.iter().filter(|e| f.matches(e)).count();
        let entries: Vec<Value> = all
            .iter()
            .filter(|e| filter.matches(e))
            .map(|e| {
                let mut v = serde_json::to_value(e).unwrap_or_default();
                let path = st.file_path(e);
                v["path"] = json!(path);
                v["thumbPath"] = json!(st.thumb_path(e).unwrap_or_else(|| path.clone()));
                v["uploading"] = json!(crate::share::is_uploading(&path));
                v
            })
            .collect();
        json!({
            "entries": entries,
            "counts": {
                "all": all.len(),
                "screenshots": count(Filter::Screenshots),
                "recordings": count(Filter::Recordings),
                "gifs": count(Filter::Gifs),
            },
            "totalSize": all.iter().map(|e| e.file_size).sum::<u64>(),
            "retention": s.history_retention,
            "enabled": s.history_enabled,
            "cloudConfigured": crate::share::is_configured(),
        })
    })
}

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    let list = ids(&a);
    let one = || list.first().cloned().ok_or_else(|| "id fehlt".to_string());
    match cmd {
        "list" => Ok(list_json(Filter::parse(a["filter"].as_str().unwrap_or("all")))),
        "copy" => copy(app, &one()?).map(|_| Value::Null),
        "save" => save_to_export(app, &list),
        "saveAs" => save_as(app, &one()?),
        "reveal" => {
            core::reveal(&entry(&one()?)?.1);
            Ok(Value::Null)
        }
        "edit" | "open" => edit(app, &one()?).map(|_| Value::Null),
        "pin" => {
            let (e, p) = entry(&one()?)?;
            if e.kind == "recording" {
                return Err(t("Videos können nicht angeheftet werden", "Videos can't be pinned").into());
            }
            crate::pin::pin_image(app, &p, None);
            Ok(Value::Null)
        }
        "upload" => {
            let (e, p) = entry(&one()?)?;
            if let Some(url) = e.cloud_url {
                core::clipboard::copy_text(&url);
                core::toast(app, t("Link kopiert", "Link copied"));
            } else {
                crate::share::upload_and_copy(app, &p);
            }
            Ok(Value::Null)
        }
        "copyLink" => {
            let (e, _) = entry(&one()?)?;
            let url = e.cloud_url.ok_or_else(|| t("Noch nicht hochgeladen", "Not uploaded yet").to_string())?;
            core::clipboard::copy_text(&url);
            core::toast(app, t("Link kopiert", "Link copied"));
            Ok(Value::Null)
        }
        "delete" => {
            delete(app, &list, a["cloud"].as_bool().unwrap_or(false));
            Ok(Value::Null)
        }
        "clearAll" => {
            with_store(|s| s.clear_all());
            changed(app);
            Ok(Value::Null)
        }
        "exportPdf" => export_pdf(app, &list),
        "drag" => drag(win, &list).map(|_| Value::Null),
        "quickLook" => {
            let files: Vec<String> = list.iter().filter_map(|id| entry(id).ok()).map(|(_, p)| p.to_string_lossy().into_owned()).collect();
            if !files.is_empty() {
                let _ = std::process::Command::new("qlmanage").arg("-p").args(&files).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
            }
            Ok(Value::Null)
        }
        "add" => {
            let p = a["path"].as_str().map(PathBuf::from).ok_or("path fehlt")?;
            add(app, &p, a["kind"].as_str().unwrap_or(""));
            Ok(Value::Null)
        }
        "cleanup" => {
            cleanup(app);
            Ok(Value::Null)
        }
        _ => Err(format!("history: unbekannter Befehl {cmd}")),
    }
}

// ───────────────────────────── Tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> Store {
        Store::new(std::env::temp_dir().join(format!("nrz-history-test-{}", crate::share::short_id())))
    }

    fn mk(id: &str, kind: &str, created_at: i64) -> Entry {
        Entry {
            id: id.into(),
            created_at,
            kind: kind.into(),
            file_name: "capture.png".into(),
            thumbnail: Some("thumbnail.jpg".into()),
            original_path: Some(format!("/tmp/{id}.png")),
            name: format!("{id}.png"),
            width: 100,
            height: 100,
            file_size: 1024,
            cloud_url: None,
            capture_mode: String::new(),
            source_app_name: None,
            source_window_title: None,
        }
    }

    #[test]
    fn set_cloud_url_round_trip() {
        let s = tmp_store();
        s.insert(mk("a", "screenshot", 1));
        assert_eq!(s.get("a").unwrap().cloud_url, None);
        assert!(s.set_cloud_url("a", Some("https://share.example.com/abc.png")));
        assert_eq!(s.get("a").unwrap().cloud_url.as_deref(), Some("https://share.example.com/abc.png"));
        assert!(s.set_cloud_url("a", None));
        assert_eq!(s.get("a").unwrap().cloud_url, None);
        assert!(!s.set_cloud_url("missing", Some("https://x.com/none.png")));
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn store_filters_order_and_delete() {
        let s = tmp_store();
        s.insert(mk("old", "screenshot", 10));
        s.insert(mk("vid", "recording", 30));
        s.insert(mk("gif", "gif", 20));
        assert_eq!(s.load().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["vid", "gif", "old"]);
        assert_eq!(s.fetch_all(Filter::Screenshots).len(), 1);
        assert_eq!(s.fetch_all(Filter::Recordings)[0].id, "vid");
        assert_eq!(s.fetch_all(Filter::Gifs)[0].id, "gif");
        assert_eq!(s.total_size(), 3072);
        std::fs::create_dir_all(s.entry_dir("old")).unwrap();
        assert!(s.find_by_path(Path::new("/tmp/old.png")).is_some());
        assert!(s.find_by_path(&s.entry_dir("old").join("capture.png")).is_some());
        assert_eq!(s.delete("old").map(|e| e.id), Some("old".into()));
        assert!(!s.entry_dir("old").exists());
        assert!(s.delete("old").is_none());
        s.clear_all();
        assert!(s.load().is_empty());
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn retention_cleanup() {
        let s = tmp_store();
        let day = 86_400_000;
        let now = 100 * day;
        s.insert(mk("today", "screenshot", now - 1000));
        s.insert(mk("d3", "screenshot", now - 3 * day));
        s.insert(mk("d10", "recording", now - 10 * day));
        s.insert(mk("d40", "gif", now - 40 * day));
        s.insert(mk("d100", "screenshot", now - 99 * day));
        std::fs::create_dir_all(s.entry_dir("d100")).unwrap();
        assert_eq!(enforce_retention(&s, "forever", now), 0);
        assert_eq!(enforce_retention(&s, "threeMonths", now), 1);
        assert!(!s.entry_dir("d100").exists());
        assert_eq!(enforce_retention(&s, "oneMonth", now), 1);
        assert_eq!(enforce_retention(&s, "oneWeek", now), 1);
        assert_eq!(enforce_retention(&s, "oneDay", now), 1);
        assert_eq!(s.load().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["today"]);
        assert_eq!(retention_days("forever"), None);
        assert_eq!(retention_days("threeMonths"), Some(90));
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn pending_url_finishes_with_insert() {
        let mut t = PendingCloudUrlTracker::default();
        t.begin("e");
        assert!(t.hold("https://share.example.com/fast.png", "e"));
        assert_eq!(t.finish("e").as_deref(), Some("https://share.example.com/fast.png"));
        assert!(!t.contains("e"));
        assert_eq!(t.held_url("e"), Some("https://share.example.com/fast.png"));
        t.complete_persistence("e");
        assert_eq!(t.held_url("e"), None);
    }

    #[test]
    fn pending_url_cancels_with_failed_insert() {
        let mut t = PendingCloudUrlTracker::default();
        assert!(!t.hold("https://x", "unknown"));
        t.begin("e");
        assert!(t.hold("https://share.example.com/orphan.png", "e"));
        t.cancel("e");
        assert!(!t.contains("e"));
        assert_eq!(t.finish("e"), None);
    }

    #[test]
    fn pdf_has_one_page_per_image() {
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xD9];
        let pdf = pdf_from_jpegs(&[(jpeg.clone(), 40, 20, 40.0, 20.0), (jpeg, 10, 30, 10.0, 30.0)]);
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.contains("/Count 2"));
        assert_eq!(s.matches("/Type /Page ").count(), 2);
        assert!(s.contains("/MediaBox [0 0 40.00 20.00]"));
        assert!(s.contains("/MediaBox [0 0 10.00 30.00]"));
        assert!(s.trim_end().ends_with("%%EOF"));
        // xref-Offsets zeigen auf die Objekte.
        let xref_pos: usize = s.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[xref_pos..].starts_with(b"xref"));
    }

    #[test]
    fn pdf_from_real_images_is_readable() {
        let icons = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icons");
        let dst = std::env::temp_dir().join(format!("nrz-{}.pdf", crate::share::short_id()));
        assert!(write_pdf(&[icons.join("32x32.png"), PathBuf::from("/missing.png"), icons.join("128x128.png")], &dst));
        let pdf = std::fs::read(&dst).unwrap();
        assert_eq!(String::from_utf8_lossy(&pdf).matches("/Type /Page ").count(), 2);
        // macOS kann die Datei lesen (sips öffnet PDFs über PDFKit/CoreGraphics).
        assert!(core::sh("sips", &["-g", "pixelWidth", &dst.to_string_lossy()]).is_some_and(|o| o.contains("pixelWidth")));
        let _ = std::fs::remove_file(&dst);
    }

    #[test]
    fn pdf_without_readable_images_writes_nothing() {
        let dst = std::env::temp_dir().join(format!("nrz-{}.pdf", crate::share::short_id()));
        assert!(!write_pdf(&[PathBuf::from("/nonexistent.png")], &dst));
        assert!(!dst.exists());
    }

    #[test]
    fn kinds_from_extension() {
        assert_eq!(kind_for(Path::new("a.gif"), ""), "gif");
        assert_eq!(kind_for(Path::new("a.MOV"), ""), "recording");
        assert_eq!(kind_for(Path::new("a.png"), ""), "screenshot");
        assert_eq!(kind_for(Path::new("a.png"), "gif"), "gif");
    }
}
