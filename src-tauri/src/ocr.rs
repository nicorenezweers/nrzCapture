//! Texterkennung (Apple Vision über `objc2-vision`). Port von Capso `OCRKit` + `OCRCoordinator` + `OCROverlayView`.
//!
//! - Instant OCR (`capture_text`): Bereich wählen → Text erkennen → Zwischenablage + Hinweis.
//! - Visual OCR (`visual`): Fenster mit Bild, hervorgehobenen Textblöcken und Textliste (Klick kopiert einen Block,
//!   „Alles kopieren“, Links öffnen).
//! - Onboarding beim ersten Aufruf (`ocrOnboardingShown`).
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::core::{self, t, Win};

/// Erkannter Textblock; `bbox` normiert (0–1), Ursprung oben links.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRegion {
    pub text: String,
    pub bbox: [f64; 4],
    pub confidence: f32,
}

/// Bereich wählen → Text erkennen → kopieren (Instant OCR).
pub fn capture_text(app: &AppHandle) {
    if !crate::settings::get().ocr_onboarding_shown {
        show_onboarding(app, Pending::Instant);
        return;
    }
    let Some(r) = crate::capture::select_region(app, crate::capture::Purpose::Ocr) else { return };
    let Some(path) = crate::capture::grab(&r, false) else {
        core::toast(app, t("Aufnahme fehlgeschlagen", "Capture failed"));
        return;
    };
    recognize_and_copy(app, &path);
}

/// Text eines Bildes erkennen (Zeilen, sortiert von oben nach unten / links nach rechts).
pub fn recognize(path: &Path) -> Result<Vec<TextRegion>, String> {
    recognize_px(path).map(|(r, _)| r)
}

/// Bild erkennen und Text in die Zwischenablage (mit Hinweis).
pub fn recognize_and_copy(app: &AppHandle, path: &Path) {
    match recognize(path) {
        Ok(regions) => {
            let sep = if crate::settings::get().ocr_keep_line_breaks { "\n" } else { " " };
            let text = join_lines(&regions, sep);
            if text.trim().is_empty() {
                core::toast(app, t("Kein Text erkannt", "No text detected"));
            } else {
                core::clipboard::copy_text(&text);
                let n = text.chars().count();
                let msg = if crate::settings::get().lang() == "de" { format!("{n} Zeichen kopiert") } else { format!("Copied {n} characters") };
                core::toast(app, &msg);
            }
        }
        Err(e) => core::toast(app, &format!("OCR: {e}")),
    }
}

/// Visual OCR: Text erkennen und im Fenster mit hervorgehobenen Blöcken zeigen (Capso `startVisualOCR`).
pub fn visual(app: &AppHandle, path: &Path) {
    if !crate::settings::get().ocr_onboarding_shown {
        show_onboarding(app, Pending::Visual(path.to_owned()));
        return;
    }
    let s = crate::settings::get();
    let (regions, (w, h)) = match recognize_px(path) {
        Ok(v) => v,
        Err(e) => {
            core::toast(app, &format!("OCR: {e}"));
            return;
        }
    };
    if regions.is_empty() {
        core::toast(app, t("Kein Text erkannt", "No text detected"));
        return;
    }
    let blocks = group_blocks(&regions, w, h, s.ocr_keep_line_breaks);
    let blocks: Vec<Value> = blocks
        .iter()
        .map(|b| {
            let links = if s.ocr_detect_links { find_links(&b.text) } else { vec![] };
            json!({ "text": b.text, "bbox": b.bbox, "confidence": b.confidence, "links": links })
        })
        .collect();
    let data = json!({ "path": path, "width": w, "height": h, "blocks": blocks, "keepLineBreaks": s.ocr_keep_line_breaks });
    let win = Win {
        min_size: Some((600.0, 400.0)),
        always_on_top: true,
        ..Win::app(t("OCR — Texterkennung", "OCR — Text Recognition"), 900.0, 550.0)
    };
    if let Some(w) = core::window(app, "ocr-visual", "ocr-visual", win, data) {
        core::set_level(&w, core::LEVEL_FLOATING);
        let _ = w.set_focus();
    }
}

// ───────────────────────────── Onboarding ─────────────────────────────

enum Pending {
    Instant,
    Visual(PathBuf),
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn show_onboarding(app: &AppHandle, then: Pending) {
    *PENDING.lock().unwrap() = Some(then);
    let win = Win { w: 700.0, h: 420.0, center: true, focus: true, shadow: true, level: Some(core::LEVEL_FLOATING), ..Default::default() };
    core::window(app, "ocr-onboarding", "ocr-onboarding", win, Value::Null);
}

fn onboarding_done(app: &AppHandle) {
    crate::settings::update(app, |s| s.ocr_onboarding_shown = true);
    if let Some(w) = app.get_webview_window("ocr-onboarding") {
        let _ = w.close();
    }
    if let Some(p) = PENDING.lock().unwrap().take() {
        let app = app.clone();
        std::thread::spawn(move || {
            // Fenster erst verschwinden lassen, damit es nicht im Auswahl-Overlay/Screenshot landet.
            std::thread::sleep(std::time::Duration::from_millis(200));
            match p {
                Pending::Instant => capture_text(&app),
                Pending::Visual(path) => visual(&app, &path),
            }
        });
    }
}

// ───────────────────────────── Vision ─────────────────────────────

/// Erkennung + Bildgröße in Pixeln (für Layout-Berechnungen in Pixeln wie bei Capso).
pub fn recognize_px(path: &Path) -> Result<(Vec<TextRegion>, (f64, f64)), String> {
    if !path.exists() {
        return Err(t("Bild nicht gefunden", "Image not found").into());
    }
    let size = image_px(path).ok_or_else(|| t("Bild kann nicht gelesen werden", "Image can't be read").to_owned())?;
    let langs = crate::settings::get().ocr_primary_language.filter(|l| !l.is_empty() && l != "auto").map(|l| vec![l]);
    let mut regions = match vision(path, langs.as_deref(), false) {
        Ok(r) => r,
        // Neural Engine gestört → schnelle Stufe, aber nur wenn sie die bevorzugten Sprachen lesen kann (CJK wird sonst Unsinn).
        Err(e) if fast_supports(&preferred_languages(), &fast_languages()) => vision(path, langs.as_deref(), true).map_err(|_| e)?,
        Err(e) => return Err(e),
    };
    sort_regions(&mut regions, size);
    Ok((regions, size))
}

fn image_px(path: &Path) -> Option<(f64, f64)> {
    use objc2_app_kit::NSImageRep;
    use objc2_foundation::NSString;
    objc2::rc::autoreleasepool(|_| {
        let rep = NSImageRep::imageRepWithContentsOfFile(&NSString::from_str(&path.to_string_lossy()))?;
        let (w, h) = (rep.pixelsWide(), rep.pixelsHigh());
        (w > 0 && h > 0).then_some((w as f64, h as f64))
    })
    .or_else(|| core::image_size(path).map(|(w, h)| (w as f64, h as f64)))
}

fn vision(path: &Path, langs: Option<&[String]>, fast: bool) -> Result<Vec<TextRegion>, String> {
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
    use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let handler = unsafe { VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &NSDictionary::new()) };
        let req = VNRecognizeTextRequest::new();
        req.setRecognitionLevel(if fast { VNRequestTextRecognitionLevel::Fast } else { VNRequestTextRecognitionLevel::Accurate });
        req.setUsesLanguageCorrection(true);
        match langs {
            Some(l) => {
                let arr = NSArray::from_retained_slice(&l.iter().map(|s| NSString::from_str(s)).collect::<Vec<_>>());
                req.setRecognitionLanguages(&arr);
                req.setAutomaticallyDetectsLanguage(false);
            }
            None => req.setAutomaticallyDetectsLanguage(true),
        }
        let r: &VNRequest = &req;
        handler.performRequests_error(&NSArray::from_slice(&[r])).map_err(|e| e.localizedDescription().to_string())?;
        let mut out = Vec::new();
        if let Some(results) = req.results() {
            for obs in results.iter() {
                let Some(c) = obs.topCandidates(1).firstObject() else { continue };
                let b = unsafe { obs.boundingBox() };
                // Vision: normiert, Ursprung unten links → oben links.
                out.push(TextRegion {
                    text: c.string().to_string(),
                    bbox: [b.origin.x, 1.0 - b.origin.y - b.size.height, b.size.width, b.size.height],
                    confidence: c.confidence(),
                });
            }
        }
        Ok(out)
    })
}

fn fast_languages() -> Vec<String> {
    use objc2_vision::{VNRecognizeTextRequest, VNRequestTextRecognitionLevel};
    objc2::rc::autoreleasepool(|_| {
        let req = VNRecognizeTextRequest::new();
        req.setRecognitionLevel(VNRequestTextRecognitionLevel::Fast);
        unsafe { req.supportedRecognitionLanguagesAndReturnError() }.map(|a| a.iter().map(|s| s.to_string()).collect()).unwrap_or_default()
    })
}

fn preferred_languages() -> Vec<String> {
    objc2_foundation::NSLocale::preferredLanguages().iter().map(|s| s.to_string()).collect()
}

/// Von Vision (genaue Stufe) unterstützte Sprachen – für die Einstellung `ocrPrimaryLanguage`.
pub fn supported_languages() -> Vec<String> {
    use objc2_vision::VNRecognizeTextRequest;
    objc2::rc::autoreleasepool(|_| {
        let req = VNRecognizeTextRequest::new();
        unsafe { req.supportedRecognitionLanguagesAndReturnError() }.map(|a| a.iter().map(|s| s.to_string()).collect()).unwrap_or_default()
    })
}

fn lang_code(id: &str) -> String {
    id.split(['-', '_']).next().unwrap_or("").to_lowercase()
}

/// True, wenn jede bevorzugte Sprache von der schnellen Stufe gelesen werden kann (Port von `fastSupports`).
pub fn fast_supports(preferred: &[String], fast: &[String]) -> bool {
    let supported: std::collections::HashSet<String> = fast.iter().map(|l| lang_code(l)).collect();
    preferred.iter().all(|p| supported.contains(&lang_code(p)))
}

// ───────────────────────────── Reine Logik ─────────────────────────────

/// Von oben nach unten, dann links nach rechts (Toleranz 10 px wie Capso).
pub fn sort_regions(regions: &mut [TextRegion], (w, h): (f64, f64)) {
    regions.sort_by(|a, b| {
        let (ay, by) = (a.bbox[1] * h, b.bbox[1] * h);
        if (ay - by).abs() < 10.0 {
            (a.bbox[0] * w).total_cmp(&(b.bbox[0] * w))
        } else {
            ay.total_cmp(&by)
        }
    });
}

/// Zeilen zu einem Text verbinden (`ocrKeepLineBreaks` → "\n", sonst " ").
pub fn join_lines(regions: &[TextRegion], sep: &str) -> String {
    regions.iter().map(|r| r.text.as_str()).filter(|t| !t.trim().is_empty()).collect::<Vec<_>>().join(sep)
}

fn is_cjk(c: char) -> bool {
    "，。！？、；：（）【】《》“”‘’」』〉》".contains(c) || matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF)
}

/// Zeilen zu Absätzen/Blöcken gruppieren: gleiche Spalte (linke Kante oder Überlappung), ähnliche Zeilenhöhe,
/// kleiner vertikaler Abstand. `keep_line_breaks` verbindet mit "\n", sonst mit Leerzeichen (CJK ohne).
pub fn group_blocks(lines: &[TextRegion], w: f64, h: f64, keep_line_breaks: bool) -> Vec<TextRegion> {
    struct B {
        r: [f64; 4], // Pixel: x, y, w, h
        lines: Vec<String>,
        line_h: f64,
        conf: f32,
        n: usize,
    }
    let px = |r: &TextRegion| [r.bbox[0] * w, r.bbox[1] * h, r.bbox[2] * w, r.bbox[3] * h];
    let mut blocks: Vec<B> = Vec::new();
    for l in lines.iter().filter(|l| !l.text.trim().is_empty()) {
        let p = px(l);
        let lh = p[3].max(1.0);
        let fit = blocks.iter().rposition(|b| {
            let last_bottom = b.r[1] + b.r[3];
            let gap = p[1] - last_bottom;
            let similar_h = (lh / b.line_h).max(b.line_h / lh) < 1.6;
            let left_aligned = (p[0] - b.r[0]).abs() < b.line_h * 1.5;
            let overlap = p[0].max(b.r[0]) < (p[0] + p[2]).min(b.r[0] + b.r[2]);
            gap > -b.line_h * 0.5 && gap < b.line_h * 0.9 && similar_h && (left_aligned || overlap)
        });
        match fit {
            Some(i) => {
                let b = &mut blocks[i];
                let (x0, y0) = (b.r[0].min(p[0]), b.r[1].min(p[1]));
                let (x1, y1) = ((b.r[0] + b.r[2]).max(p[0] + p[2]), (b.r[1] + b.r[3]).max(p[1] + p[3]));
                b.r = [x0, y0, x1 - x0, y1 - y0];
                b.line_h = (b.line_h * b.n as f64 + lh) / (b.n + 1) as f64;
                b.conf = b.conf.min(l.confidence);
                b.n += 1;
                b.lines.push(l.text.trim_end().to_owned());
            }
            None => blocks.push(B { r: p, lines: vec![l.text.trim_end().to_owned()], line_h: lh, conf: l.confidence, n: 1 }),
        }
    }
    blocks
        .into_iter()
        .map(|b| {
            let mut text = String::new();
            for (i, line) in b.lines.iter().enumerate() {
                if i > 0 {
                    if keep_line_breaks {
                        text.push('\n');
                    } else if !(text.chars().last().is_some_and(is_cjk) && line.chars().next().is_some_and(is_cjk)) {
                        text.push(' ');
                    }
                }
                text.push_str(line);
            }
            TextRegion { text, bbox: [b.r[0] / w, b.r[1] / h, b.r[2] / w, b.r[3] / h], confidence: b.conf }
        })
        .collect()
}

/// Links im Text finden (`http(s)://…`, `www.…`). `www.`-Links bekommen `https://` vorangestellt.
pub fn find_links(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in text.split_whitespace() {
        let lower = tok.to_lowercase();
        let start = ["https://", "http://", "www."].iter().filter_map(|p| lower.find(p)).min();
        let Some(i) = start else { continue };
        let url = tok[i..].trim_end_matches(['.', ',', ';', ':', ')', ']', '}', '>', '"', '\'', '!', '?', '»', '“', '”']);
        let host_part = url.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("HTTPS://").trim_start_matches("HTTP://");
        if host_part.len() < 3 || !host_part.contains('.') && !host_part.starts_with("localhost") {
            continue;
        }
        let full = if url.to_lowercase().starts_with("www.") { format!("https://{url}") } else { url.to_owned() };
        if !out.contains(&full) {
            out.push(full);
        }
    }
    out
}

/// Anzeigename einer Sprache in der UI-Sprache (über NSLocale).
pub fn language_name(code: &str) -> String {
    use objc2::AnyThread;
    use objc2_foundation::{NSLocale, NSString};
    let ui = NSLocale::initWithLocaleIdentifier(NSLocale::alloc(), &NSString::from_str(crate::settings::get().lang()));
    ui.localizedStringForLocaleIdentifier(&NSString::from_str(code)).to_string()
}

pub fn dispatch(app: &AppHandle, win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    let path = || a["path"].as_str().map(PathBuf::from).ok_or_else(|| "path fehlt".to_string());
    match cmd {
        "recognize" => Ok(serde_json::to_value(recognize(&path()?)?).unwrap()),
        "recognizeAndCopy" => {
            recognize_and_copy(app, &path()?);
            Ok(Value::Null)
        }
        "captureText" => {
            capture_text(app);
            Ok(Value::Null)
        }
        "visual" => {
            visual(app, &path()?);
            Ok(Value::Null)
        }
        "onboardingDone" => {
            onboarding_done(app);
            Ok(Value::Null)
        }
        "onboardingCancel" => {
            PENDING.lock().unwrap().take();
            let _ = win.close();
            Ok(Value::Null)
        }
        "copy" => {
            core::clipboard::copy_text(a["text"].as_str().unwrap_or(""));
            Ok(Value::Null)
        }
        "drag" => {
            let _ = win.start_dragging();
            Ok(Value::Null)
        }
        "openLink" => {
            let url = a["url"].as_str().unwrap_or("");
            if url.starts_with("http://") || url.starts_with("https://") {
                core::open(url);
            }
            Ok(Value::Null)
        }
        "languages" => {
            let mut v: Vec<(String, String)> = supported_languages().into_iter().map(|c| (language_name(&c), c)).collect();
            v.sort_by_key(|(n, _)| n.to_lowercase());
            Ok(json!(v.into_iter().map(|(name, code)| json!({ "code": code, "name": name })).collect::<Vec<_>>()))
        }
        _ => Err(format!("ocr: unbekannter Befehl {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(text: &str, x: f64, y: f64, w: f64, h: f64) -> TextRegion {
        // Pixelangaben auf ein 1000×1000-Bild normieren.
        TextRegion { text: text.into(), bbox: [x / 1000.0, y / 1000.0, w / 1000.0, h / 1000.0], confidence: 1.0 }
    }

    #[test]
    fn fast_fallback_only_for_supported_languages() {
        let fast = vec!["en-US".to_string(), "de-DE".into(), "fr-FR".into()];
        assert!(fast_supports(&["en".into(), "de-AT".into()], &fast));
        assert!(!fast_supports(&["en".into(), "zh-Hans".into()], &fast));
        assert!(fast_supports(&[], &fast));
    }

    #[test]
    fn sorts_top_to_bottom_then_left_to_right() {
        let mut v = vec![r("b", 300.0, 104.0, 50.0, 20.0), r("c", 10.0, 200.0, 50.0, 20.0), r("a", 10.0, 100.0, 50.0, 20.0)];
        sort_regions(&mut v, (1000.0, 1000.0));
        assert_eq!(v.iter().map(|r| r.text.as_str()).collect::<String>(), "abc");
    }

    #[test]
    fn joins_lines_with_separator() {
        let v = vec![r("Hallo", 0.0, 0.0, 1.0, 1.0), r(" ", 0.0, 0.0, 1.0, 1.0), r("Welt", 0.0, 0.0, 1.0, 1.0)];
        assert_eq!(join_lines(&v, "\n"), "Hallo\nWelt");
        assert_eq!(join_lines(&v, " "), "Hallo Welt");
    }

    #[test]
    fn groups_paragraphs_and_columns() {
        let lines = vec![
            r("First line of", 20.0, 10.0, 180.0, 20.0),
            r("a paragraph.", 20.0, 34.0, 160.0, 20.0),
            r("Second paragraph", 20.0, 90.0, 200.0, 20.0),
            r("Right column", 600.0, 10.0, 150.0, 20.0),
        ];
        let b = group_blocks(&lines, 1000.0, 1000.0, false);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].text, "First line of a paragraph.");
        assert!((b[0].bbox[3] - 0.044).abs() < 1e-9);
        assert_eq!(b[1].text, "Second paragraph");
        assert_eq!(b[2].text, "Right column");
        let b = group_blocks(&lines, 1000.0, 1000.0, true);
        assert_eq!(b[0].text, "First line of\na paragraph.");
    }

    #[test]
    fn cjk_lines_join_without_spaces() {
        let lines = vec![r("翻译结果应该保持", 10.0, 10.0, 140.0, 20.0), r("自然的中文阅读节奏。", 10.0, 34.0, 160.0, 20.0)];
        assert_eq!(group_blocks(&lines, 1000.0, 1000.0, false)[0].text, "翻译结果应该保持自然的中文阅读节奏。");
    }

    #[test]
    fn detects_links() {
        assert_eq!(find_links("Siehe https://example.com/a?b=1, oder www.rust-lang.org."), vec!["https://example.com/a?b=1", "https://www.rust-lang.org"]);
        assert!(find_links("keine Links hier, nur http:// und www.").is_empty());
        assert_eq!(find_links("(http://localhost:8080/x)"), vec!["http://localhost:8080/x"]);
    }
}

#[cfg(test)]
mod vision_tests {
    /// Echter Vision-Lauf: `NRZ_OCR_TEST_IMAGE=/pfad/bild.png cargo test -p nrzcapture vision_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn vision_smoke() {
        let p = std::env::var("NRZ_OCR_TEST_IMAGE").expect("NRZ_OCR_TEST_IMAGE");
        let (r, size) = super::recognize_px(std::path::Path::new(&p)).unwrap();
        println!("{size:?} {r:#?}");
        assert!(!r.is_empty());
    }
}

/// Vision-Modelle vorwärmen (erster Aufruf lädt/kompiliert das Modell, kann sehr lange dauern) – Capso `TextRecognizer.prewarm`.
/// Sinnvoll beim App-Start in einem Hintergrund-Thread.
pub fn prewarm() {
    std::thread::spawn(|| {
        let icon = Path::new("/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns");
        if icon.exists() {
            let _ = vision(icon, None, false);
        }
    });
}
