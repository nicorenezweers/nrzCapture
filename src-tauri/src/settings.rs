//! Persistente Einstellungen (Port von Capso `AppSettings`).
//! Gespeichert als JSON in `<AppSupport>/com.nrz.capture/settings.json`.
//! Frontend: `rpc("settings","get")` / `rpc("settings","set",{key,value})`, Event `settings` bei jeder Änderung.
//! Module, die zusätzliche Schlüssel brauchen, nutzen `extra` (beliebige JSON-Werte, `get_extra`/`set_extra`).
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, sync::RwLock};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StoredSelection {
    /// Rechteck in globalen logischen Koordinaten (Punkte, Ursprung oben links des Hauptbildschirms).
    Area { x: f64, y: f64, width: f64, height: f64, screen: u32 },
    Fullscreen { screen: u32 },
    Window { window_id: u32 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    // Allgemein
    pub language: String, // "system" | "de" | "en"
    pub start_at_login: bool,
    pub play_shutter_sound: bool,
    pub show_menu_bar_icon: bool,
    pub automation_urls_enabled: bool,
    pub automation_urls_asked: bool,
    pub replace_system_shortcuts: bool, // ⌘⇧3/4/5 an nrzCapture umleiten
    pub update_check: String,           // Prüfabstand: "daily" | "weekly" | "monthly" (Capso `UpdateCheckFrequency`)
    pub export_location: String,        // leer = Schreibtisch / macOS-Screenshot-Ort
    pub opened_image_save_behavior: String, // "ask" | "overwrite" | "copy"

    // Screenshot
    pub screenshot_format: String,         // "png" | "jpeg" | "heic"
    pub screenshot_quality: u8,            // 30–100 (verlustbehaftete Formate, Capso `qualityPercentRange`)
    pub screenshot_show_preview: bool,     // Quick Access zeigen
    pub screenshot_auto_copy: bool,
    pub screenshot_clipboard_format: String,  // "png" | "jpeg" | "tiff"
    pub screenshot_clipboard_content: String, // "image" | "filePath"
    pub screenshot_auto_save: bool,
    pub screenshot_monthly_folders: bool,
    pub screenshot_filename_template: String, // Platzhalter: {source} {timestamp} {date} {time} {counter}
    pub recording_filename_template: String,
    pub screenshot_shows_cursor: bool,
    pub auto_beautify_screenshots: bool,
    pub beautify_style: Value,
    pub screenshot_timestamp_enabled: bool,
    pub screenshot_timestamp_position: String, // "bottomRight" | "bottomLeft" | "topRight" | "topLeft"
    pub screenshot_timestamp_format: String,   // "dateTime" | "date" | "time" | "iso"
    pub screenshot_timestamp_color: String,
    pub screenshot_timestamp_font_size: u32,
    pub capture_window_shadow: bool,
    pub show_magnifier: bool,
    pub last_capture_selection: Option<StoredSelection>,
    pub self_timer_seconds: u32, // 1–60 (Capso `selfTimerDurationRange`)
    pub self_timer_tick_sound: bool,
    pub self_timer_hud_position: Option<(f64, f64)>,
    pub capture_presets_enabled: bool,
    pub capture_preset: String, // "freeform" | "1:1" | "4:3" | "16:9" | "1920x1080" | …
    pub custom_capture_presets: Vec<String>,
    pub hidden_builtin_presets: Vec<String>,
    pub square_center_lock_key: String, // Standard "C"

    // Quick Access
    pub quick_access_position: String, // "bottomLeft" | "centerScreen" | "bottomRight"
    pub quick_access_close_after_drag: bool,
    pub quick_access_auto_close: bool,
    pub quick_access_auto_close_interval: u32, // Sekunden
    pub quick_access_actions: Value,           // Layout der Aktionsknöpfe

    // Aufnahme
    pub recording_format: String, // "mp4" | "gif"
    pub export_quality: String,   // "maximum" | "social" | "web"
    pub show_cursor: bool,
    pub highlight_clicks: bool,
    pub show_key_presses: bool,
    pub key_press_overlay_offset: Option<(f64, f64)>,
    pub cursor_smoothing: bool,
    pub dim_screen_while_recording: bool,
    pub show_countdown: bool,
    pub recording_selects_window: bool,
    pub remember_last_recording_area: bool,
    pub last_recording_area: Option<StoredSelection>,
    pub recording_system_audio: bool,
    pub recording_mic_enabled: bool,
    pub recording_mic_device_id: Option<String>,
    pub warn_about_muted_microphone: bool,
    pub hide_desktop_icons_while_recording: bool,
    pub warn_about_low_disk_space: bool,
    pub open_editor_after_recording: bool,
    pub recording_auto_copy: bool,

    // Kamera-PiP
    pub camera_enabled: bool,
    pub camera_device_id: Option<String>,
    pub camera_shape: String, // "circle" | "square" | "landscape" | "portrait"
    pub camera_size: String,  // "small" | "medium" | "large"
    pub camera_mirror: bool,
    pub camera_custom_size_pt: f64,
    pub camera_pip_fade_on_hover: bool,
    pub camera_pip_click_through: bool,

    // Verlauf
    pub history_enabled: bool,
    pub history_retention: String, // "oneWeek" | "twoWeeks" | "oneMonth" | "unlimited" (Capso `HistoryRetention`)

    // OCR
    pub ocr_keep_line_breaks: bool,
    pub ocr_detect_links: bool,
    pub ocr_primary_language: Option<String>,
    pub ocr_onboarding_shown: bool,

    // Übersetzung
    pub translation_target_language: String,
    pub translation_provider: String, // "apple" | "openAICompatible" | "deepSeek" | "openRouter" | "deepL" | "googleCloud" | "custom"
    /// Veraltet: Modell/Endpunkt liegen je Anbieter in `translationProvider.<id>.model/endpoint` (extra),
    /// der API-Schlüssel im Schlüsselbund (`translate::migrate_settings`).
    pub translation_provider_model: String,
    pub translation_provider_endpoint: String,
    pub translation_api_key: String,
    pub translation_auto_copy: bool,
    pub translation_show_original: bool,
    pub translation_card_position: String, // "belowSelection" | "centerScreen" | "rememberLast"
    pub translation_auto_dismiss: String,  // "manual" | "clickOutside" | "afterDelay"
    pub translation_auto_dismiss_delay: f64,
    pub translation_onboarding_shown: bool,

    // Cloud-Freigabe (R2 / S3 / Aliyun OSS / Tencent COS)
    pub cloud_share_provider: Option<String>,
    pub cloud_share_url_prefix: Option<String>,
    pub cloud_share_account_id: Option<String>,
    pub cloud_share_bucket: Option<String>,
    pub cloud_share_region: Option<String>,
    pub cloud_share_endpoint: Option<String>,
    pub cloud_share_path_prefix: Option<String>,
    /// Veraltet: Zugangsdaten liegen im Schlüsselbund (`share::migrate_keys`); nur noch für Assistent-Tests/Altbestand.
    pub cloud_share_access_key: Option<String>,
    pub cloud_share_secret_key: Option<String>,
    pub cloud_share_auto_upload: bool,
    pub cloud_share_copy_link_on_start: bool,

    /// Automatisch nach Updates suchen (Capso „Automatically Check for Updates“).
    pub update_auto_check: bool,
    /// Updates automatisch laden und installieren (Capso „Automatically Install Updates“).
    pub update_auto_install: bool,

    /// Tastenkürzel: Aktions-ID → Tauri-Accelerator ("Alt+Shift+1"), leer = keins.
    pub shortcuts: BTreeMap<String, String>,

    /// Freie Zusatzschlüssel für Module.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub fn default_shortcuts() -> BTreeMap<String, String> {
    // Nur noch Screenshots: Bereich, Vollbild, Fenster.
    [
        ("captureArea", "Alt+Shift+1"),
        ("captureFullscreen", "Alt+Shift+2"),
        ("captureWindow", "Alt+Shift+3"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "system".into(),
            start_at_login: false,
            play_shutter_sound: true,
            show_menu_bar_icon: true,
            automation_urls_enabled: false,
            automation_urls_asked: false,
            replace_system_shortcuts: false,
            update_check: "daily".into(),
            export_location: String::new(),
            opened_image_save_behavior: "ask".into(),

            screenshot_format: "png".into(),
            screenshot_quality: 85,
            screenshot_show_preview: true,
            screenshot_auto_copy: true,
            screenshot_clipboard_format: "png".into(),
            screenshot_clipboard_content: "image".into(),
            screenshot_auto_save: true,
            screenshot_monthly_folders: false,
            screenshot_filename_template: "nrzCapture Screenshot{source} {timestamp}".into(),
            recording_filename_template: "nrzCapture Recording {timestamp}".into(),
            screenshot_shows_cursor: false,
            auto_beautify_screenshots: false,
            beautify_style: Value::Null,
            screenshot_timestamp_enabled: false,
            screenshot_timestamp_position: "bottomRight".into(),
            screenshot_timestamp_format: "dateTime".into(),
            screenshot_timestamp_color: "#FFFFFF".into(),
            screenshot_timestamp_font_size: 14,
            capture_window_shadow: true,
            show_magnifier: false,
            last_capture_selection: None,
            self_timer_seconds: 5,
            self_timer_tick_sound: true,
            self_timer_hud_position: None,
            capture_presets_enabled: true,
            capture_preset: "freeform".into(),
            custom_capture_presets: vec![],
            hidden_builtin_presets: vec![],
            square_center_lock_key: "C".into(),

            quick_access_position: "bottomLeft".into(),
            quick_access_close_after_drag: true,
            quick_access_auto_close: false,
            quick_access_auto_close_interval: 5,
            quick_access_actions: Value::Null,

            recording_format: "mp4".into(),
            export_quality: "maximum".into(),
            show_cursor: true,
            highlight_clicks: true,
            show_key_presses: false,
            key_press_overlay_offset: None,
            cursor_smoothing: true,
            dim_screen_while_recording: false,
            show_countdown: true,
            recording_selects_window: false,
            remember_last_recording_area: false,
            last_recording_area: None,
            recording_system_audio: true,
            recording_mic_enabled: false,
            recording_mic_device_id: None,
            warn_about_muted_microphone: true,
            hide_desktop_icons_while_recording: false,
            warn_about_low_disk_space: true,
            open_editor_after_recording: false,
            recording_auto_copy: false,

            camera_enabled: false,
            camera_device_id: None,
            camera_shape: "circle".into(),
            camera_size: "medium".into(),
            camera_mirror: true,
            camera_custom_size_pt: 0.0,
            camera_pip_fade_on_hover: false,
            camera_pip_click_through: false,

            history_enabled: true,
            history_retention: "oneMonth".into(),

            ocr_keep_line_breaks: true,
            ocr_detect_links: true,
            ocr_primary_language: None,
            ocr_onboarding_shown: false,

            translation_target_language: default_translation_target(),
            translation_provider: "apple".into(),
            translation_provider_model: String::new(),
            translation_provider_endpoint: String::new(),
            translation_api_key: String::new(),
            translation_auto_copy: true,
            translation_show_original: true,
            translation_card_position: "centerScreen".into(),
            translation_auto_dismiss: "manual".into(),
            translation_auto_dismiss_delay: 10.0,
            translation_onboarding_shown: false,

            cloud_share_provider: None,
            cloud_share_url_prefix: None,
            cloud_share_account_id: None,
            cloud_share_bucket: None,
            cloud_share_region: None,
            cloud_share_endpoint: None,
            cloud_share_path_prefix: None,
            cloud_share_access_key: None,
            cloud_share_secret_key: None,
            cloud_share_auto_upload: false,
            cloud_share_copy_link_on_start: false,

            update_auto_check: true,
            update_auto_install: false,

            shortcuts: default_shortcuts(),
            extra: Map::new(),
        }
    }
}

impl Settings {
    /// Wie Capso `isCloudShareConfigured`: Anbieter, URL-Präfix, Bucket und Konto-ID (R2) bzw. Region
    /// (Zugangsdaten liegen im Schlüsselbund).
    pub fn is_cloud_share_configured(&self) -> bool {
        let ok = |o: &Option<String>| o.as_deref().is_some_and(|s| !s.trim().is_empty());
        let id_ok = match self.cloud_share_provider.as_deref() {
            Some("r2") => ok(&self.cloud_share_account_id),
            Some("s3" | "tencentCOS" | "aliyunOSS") => ok(&self.cloud_share_region),
            _ => false,
        };
        id_ok && ok(&self.cloud_share_url_prefix) && ok(&self.cloud_share_bucket)
    }
    /// Effektive UI-Sprache: "de" oder "en".
    pub fn lang(&self) -> &'static str {
        match self.language.as_str() {
            "de" => "de",
            "en" => "en",
            _ if system_language().starts_with("de") => "de",
            _ => "en",
        }
    }
}

/// Bevorzugte Systemsprache als Kürzel ("de", "en", …).
pub fn system_language() -> String {
    crate::core::sh("defaults", &["read", "-g", "AppleLanguages"])
        .and_then(|s| s.split('"').nth(1).map(|l| l.split(['-', '_']).next().unwrap_or("en").to_owned()))
        .unwrap_or_else(|| "en".into())
}

/// Standard-Zielsprache der Übersetzung aus `AppleLocale` (Capso `systemDefaultLanguage`), einmal ermittelt.
pub fn default_translation_target() -> String {
    static T: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    T.get_or_init(|| crate::translate::default_target(&crate::core::sh("defaults", &["read", "-g", "AppleLocale"]).unwrap_or_else(|| "en_US".into()))).clone()
}

/// Alte bzw. ungültige Werte auf Capsos Wertebereiche bringen.
pub fn normalize(s: &mut Settings) {
    // Update: früher "never" statt eigenem Schalter.
    if s.update_check == "never" {
        s.update_auto_check = false;
        s.update_check = "daily".into();
    }
    if !matches!(s.update_check.as_str(), "daily" | "weekly" | "monthly") {
        s.update_check = "daily".into();
    }
    s.history_retention = match s.history_retention.as_str() {
        "oneDay" | "oneWeek" => "oneWeek",
        "twoWeeks" => "twoWeeks",
        "forever" | "unlimited" => "unlimited",
        _ => "oneMonth",
    }
    .into();
    s.self_timer_seconds = s.self_timer_seconds.clamp(1, 60);
    s.screenshot_quality = s.screenshot_quality.clamp(30, 100);
}

static SETTINGS: RwLock<Option<Settings>> = RwLock::new(None);

fn file() -> std::path::PathBuf {
    crate::core::data_dir().join("settings.json")
}

/// Aktuelle Einstellungen (Kopie).
pub fn get() -> Settings {
    if let Some(s) = SETTINGS.read().unwrap().as_ref() {
        return s.clone();
    }
    let s: Settings = std::fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut s = s;
    normalize(&mut s);
    // Neue Standard-Kürzel ergänzen, ohne Benutzerwerte zu überschreiben.
    for (k, v) in default_shortcuts() {
        s.shortcuts.entry(k).or_insert(v);
    }
    *SETTINGS.write().unwrap() = Some(s.clone());
    s
}

/// Ändern + speichern + Event `settings` an alle Fenster + interne Reaktionen (Kürzel, Autostart, …).
pub fn update(app: &AppHandle, f: impl FnOnce(&mut Settings)) {
    let mut s = get();
    f(&mut s);
    save(app, s);
}

fn save(app: &AppHandle, mut s: Settings) {
    normalize(&mut s);
    let _ = std::fs::write(file(), serde_json::to_vec_pretty(&s).unwrap_or_default());
    *SETTINGS.write().unwrap() = Some(s.clone());
    let v = serde_json::to_value(&s).unwrap_or_default();
    let _ = app.emit("settings", &v);
    crate::actions::settings_changed(app, &s);
}

pub fn get_extra(key: &str) -> Option<Value> {
    get().extra.get(key).cloned()
}

pub fn set_extra(app: &AppHandle, key: &str, v: Value) {
    update(app, |s| {
        s.extra.insert(key.to_owned(), v);
    });
}

/// Einzelnen Schlüssel (camelCase, wie im JSON) setzen.
pub fn set_key(app: &AppHandle, key: &str, value: Value) -> Result<(), String> {
    let mut v = serde_json::to_value(get()).map_err(|e| e.to_string())?;
    v.as_object_mut().unwrap().insert(key.to_owned(), value);
    let s: Settings = serde_json::from_value(v).map_err(|e| e.to_string())?;
    save(app, s);
    Ok(())
}

pub fn dispatch(app: &AppHandle, cmd: &str, a: Value) -> Result<Value, String> {
    match cmd {
        "get" => serde_json::to_value(get()).map_err(|e| e.to_string()),
        "set" => {
            let key = a["key"].as_str().ok_or("key fehlt")?;
            set_key(app, key, a["value"].clone())?;
            Ok(Value::Null)
        }
        // Mehrere Schlüssel auf einmal: {values:{key:value,…}}
        "patch" => {
            let mut v = serde_json::to_value(get()).map_err(|e| e.to_string())?;
            if let Some(o) = a["values"].as_object() {
                for (k, val) in o {
                    v[k] = val.clone();
                }
            }
            let s: Settings = serde_json::from_value(v).map_err(|e| e.to_string())?;
            save(app, s);
            Ok(Value::Null)
        }
        "reset" => {
            save(app, Settings::default());
            Ok(Value::Null)
        }
        "defaults" => Ok(json!({ "shortcuts": default_shortcuts() })),
        _ => Err(format!("settings: unbekannter Befehl {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_migrates_legacy_values() {
        let mut s = Settings { update_check: "never".into(), history_retention: "forever".into(), self_timer_seconds: 0, screenshot_quality: 5, ..Settings::default() };
        normalize(&mut s);
        assert!(!s.update_auto_check);
        assert_eq!(s.update_check, "daily");
        assert_eq!(s.history_retention, "unlimited");
        assert_eq!(s.self_timer_seconds, 1);
        assert_eq!(s.screenshot_quality, 30);
        let mut s = Settings { history_retention: "threeMonths".into(), self_timer_seconds: 99, update_check: "monthly".into(), ..Settings::default() };
        normalize(&mut s);
        assert_eq!(s.history_retention, "oneMonth");
        assert_eq!(s.self_timer_seconds, 60);
        assert_eq!(s.update_check, "monthly");
        assert!(s.update_auto_check);
    }

    #[test]
    fn capso_defaults() {
        let s = Settings::default();
        assert_eq!(s.translation_provider, "apple");
        assert_eq!(s.translation_card_position, "centerScreen");
        assert_eq!(s.translation_auto_dismiss, "manual");
        assert_eq!(s.quick_access_position, "bottomLeft");
        assert!(!s.cloud_share_copy_link_on_start);
        assert!(s.update_auto_check && !s.update_auto_install);
    }

    #[test]
    fn cloud_configured_like_capso() {
        let mut s = Settings { cloud_share_provider: Some("r2".into()), cloud_share_bucket: Some("b".into()), cloud_share_url_prefix: Some("https://x.dev".into()), ..Settings::default() };
        assert!(!s.is_cloud_share_configured());
        s.cloud_share_account_id = Some("abc".into());
        assert!(s.is_cloud_share_configured());
        s.cloud_share_provider = Some("s3".into());
        assert!(!s.is_cloud_share_configured());
        s.cloud_share_region = Some("us-east-1".into());
        assert!(s.is_cloud_share_configured());
    }

    #[test]
    fn diag_write_appends_and_rotates() {
        let dir = std::env::temp_dir().join(format!("nrz-diag-{}", crate::core::unique_id()));
        let f = dir.join("nrzcapture.log");
        crate::core::diag_write(&f, "App", "hello");
        let txt = std::fs::read_to_string(&f).unwrap();
        assert!(txt.contains("[App] hello"));
        std::fs::write(&f, vec![b'x'; 1_000_001]).unwrap();
        crate::core::diag_write(&f, "App", "after");
        assert!(dir.join("nrzcapture.old.log").exists());
        assert!(std::fs::read_to_string(&f).unwrap().contains("after"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn update_intervals() {
        assert_eq!(crate::update_interval("daily"), 86_400);
        assert_eq!(crate::update_interval("weekly"), 604_800);
        assert_eq!(crate::update_interval("monthly"), 2_592_000);
    }
}
