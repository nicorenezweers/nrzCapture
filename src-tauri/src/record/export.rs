//! Export der Rohaufnahme: Segmente zusammenfügen (AVMutableComposition), MP4 mit Qualitätspreset
//! (AVAssetExportSession), GIF (AVAssetImageGenerator → ImageIO). Port von Capso `ExportKit` (MP4Exporter, GIFExporter).
#![allow(deprecated)] // synchrone AVAsset-APIs: wir laufen ohnehin auf einem Hintergrund-Thread
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_av_foundation::*;
use objc2_core_foundation::CGSize;
use objc2_core_media::{CMTime, CMTimeRange};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use std::{ffi::c_void, path::Path, sync::mpsc, time::Duration};

fn url(p: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
}

fn asset(p: &Path) -> Retained<AVURLAsset> {
    unsafe { AVURLAsset::URLAssetWithURL_options(&url(p), None) }
}

fn time(sec: f64) -> CMTime {
    unsafe { CMTime::with_seconds(sec, 600) }
}

/// Dauer einer Videodatei in Sekunden.
pub fn duration(p: &Path) -> f64 {
    unsafe { asset(p).duration().seconds() }
}

/// Pixelgröße der ersten Videospur.
pub fn video_size(p: &Path) -> Option<(f64, f64)> {
    unsafe {
        let a = asset(p);
        let tracks = a.tracksWithMediaType(AVMediaTypeVideo?);
        let t = tracks.iter().next()?;
        let s = t.naturalSize();
        Some((s.width, s.height))
    }
}

/// Größtes Ziel (Breite, Höhe) eines Qualitätspresets – None = Original.
pub fn preset_size(quality: &str) -> Option<(f64, f64)> {
    match quality {
        "social" => Some((1920.0, 1080.0)),
        "web" => Some((1280.0, 720.0)),
        _ => None,
    }
}

/// Passt das Video schon ins Preset? Dann reicht Passthrough (kein Neukodieren, wie Capso).
pub fn fits_preset(size: (f64, f64), preset: (f64, f64)) -> bool {
    let (long, short) = (size.0.max(size.1), size.0.min(size.1));
    long <= preset.0.max(preset.1) && short <= preset.0.min(preset.1)
}

fn run_export(a: &AVAsset, preset: &NSString, out: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(out);
    unsafe {
        let s = AVAssetExportSession::exportSessionWithAsset_presetName(a, preset).ok_or("Export-Sitzung konnte nicht erstellt werden")?;
        s.setOutputURL(Some(&url(out)));
        s.setOutputFileType(AVFileTypeMPEG4);
        s.setShouldOptimizeForNetworkUse(true);
        let (tx, rx) = mpsc::channel::<()>();
        let block = block2::RcBlock::new(move || {
            let _ = tx.send(());
        });
        s.exportAsynchronouslyWithCompletionHandler(&block);
        rx.recv_timeout(Duration::from_secs(3600)).map_err(|_| "Export: Zeitüberschreitung".to_string())?;
        if s.status() == AVAssetExportSessionStatus::Completed {
            Ok(())
        } else {
            Err(s.error().map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "Export fehlgeschlagen".into()))
        }
    }
}

/// Segmente (Pause/Fortsetzen) verlustfrei zu einer MP4 zusammenfügen. Ein Segment wird nur verschoben.
pub fn concat(segments: &[std::path::PathBuf], out: &Path) -> Result<(), String> {
    match segments {
        [] => Err("Keine Aufnahme-Daten".into()),
        [one] => std::fs::rename(one, out).or_else(|_| std::fs::copy(one, out).map(|_| ())).map_err(|e| e.to_string()),
        many => unsafe {
            let comp = AVMutableComposition::composition();
            let mut at = 0.0;
            for p in many {
                let a = asset(p);
                let d = a.duration();
                let range = CMTimeRange { start: time(0.0), duration: d };
                comp.insertTimeRange_ofAsset_atTime_error(range, &a, time(at)).map_err(|e| e.localizedDescription().to_string())?;
                at += d.seconds();
            }
            run_export(&comp, AVAssetExportPresetPassthrough, out)?;
            for p in many {
                let _ = std::fs::remove_file(p);
            }
            Ok(())
        },
    }
}

/// MP4 gemäß `exportQuality` ("maximum" | "social" | "web") schreiben. Mit mehr als einer Tonspur
/// (System-Audio + Mikrofon) werden die Spuren zu einer gemischt (Capso `MP4Exporter`, issue #55).
pub fn mp4(src: &Path, quality: &str, out: &Path) -> Result<(), String> {
    if crate::receditor::export::audio_track_count(src) > 1 {
        return crate::receditor::export::merge_audio_tracks(src, out);
    }
    let preset = preset_size(quality);
    let passthrough = match (preset, video_size(src)) {
        (None, _) => true,
        (Some(p), Some(s)) => fits_preset(s, p),
        (Some(_), None) => false,
    };
    if passthrough {
        return std::fs::copy(src, out).map(|_| ()).map_err(|e| e.to_string());
    }
    let name = unsafe { if quality == "web" { AVAssetExportPreset1280x720 } else { AVAssetExportPreset1920x1080 } };
    run_export(&asset(src), name, out)
}

/// GIF-Parameter wie Capso: (fps, maximale Breite).
pub fn gif_params(quality: &str) -> (f64, f64) {
    match quality {
        "social" => (20.0, 1080.0),
        "web" => (15.0, 720.0),
        _ => (25.0, 1920.0),
    }
}

/// Bild-Zeitpunkte: höchstens 600 Bilder insgesamt (sonst fps proportional senken).
pub fn gif_times(duration: f64, base_fps: f64) -> Vec<f64> {
    if duration <= 0.0 {
        return vec![];
    }
    let fps = if (duration * base_fps).ceil() > 600.0 { 600.0 / duration } else { base_fps };
    let n = (duration * fps).ceil().max(1.0) as usize;
    (0..n).map(|i| i as f64 / fps).filter(|t| *t < duration).collect()
}

#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    fn CGImageDestinationCreateWithURL(url: *const c_void, ty: *const c_void, count: usize, opts: *const c_void) -> *mut c_void;
    fn CGImageDestinationSetProperties(d: *mut c_void, props: *const c_void);
    fn CGImageDestinationAddImage(d: *mut c_void, img: *const c_void, props: *const c_void);
    fn CGImageDestinationFinalize(d: *mut c_void) -> bool;
    static kCGImagePropertyGIFDictionary: *const c_void;
    static kCGImagePropertyGIFLoopCount: *const c_void;
    static kCGImagePropertyGIFDelayTime: *const c_void;
    static kCGImagePropertyGIFUnclampedDelayTime: *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(p: *const c_void);
}

unsafe fn cfkey(k: *const c_void) -> &'static NSString {
    &*(k as *const NSString)
}

unsafe fn gif_dict(pairs: &[(*const c_void, Retained<NSNumber>)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<&NSString> = pairs.iter().map(|(k, _)| cfkey(*k)).collect();
    let vals: Vec<&AnyObject> = pairs.iter().map(|(_, v)| v.as_ref()).collect();
    let inner = NSDictionary::from_slices(&keys, &vals);
    NSDictionary::from_slices(&[cfkey(kCGImagePropertyGIFDictionary)], &[inner.as_ref() as &AnyObject])
}

/// Video → animiertes GIF (Endlosschleife). `progress(0…1)`.
pub fn gif(src: &Path, quality: &str, out: &Path, progress: impl Fn(f64)) -> Result<(), String> {
    let (fps, max_w) = gif_params(quality);
    let dur = duration(src);
    let times = gif_times(dur, fps);
    if times.is_empty() {
        return Err("Leeres Video".into());
    }
    let (vw, vh) = video_size(src).unwrap_or((max_w, max_w * 9.0 / 16.0));
    let scale = if vw > max_w { max_w / vw } else { 1.0 };
    let _ = std::fs::remove_file(out);
    unsafe {
        let gen = AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset(src));
        gen.setAppliesPreferredTrackTransform(true);
        gen.setMaximumSize(CGSize::new((vw * scale).round(), (vh * scale).round()));
        let tol = time(0.5 / fps);
        gen.setRequestedTimeToleranceBefore(tol);
        gen.setRequestedTimeToleranceAfter(tol);

        let u = url(out);
        let ty = NSString::from_str("com.compuserve.gif");
        let dest = CGImageDestinationCreateWithURL(Retained::as_ptr(&u).cast(), Retained::as_ptr(&ty).cast(), times.len(), std::ptr::null());
        if dest.is_null() {
            return Err("GIF-Datei konnte nicht angelegt werden".into());
        }
        let file_props = gif_dict(&[(kCGImagePropertyGIFLoopCount, NSNumber::new_i32(0))]);
        CGImageDestinationSetProperties(dest, Retained::as_ptr(&file_props).cast());
        let mut added = 0;
        for (i, t) in times.iter().enumerate() {
            let next = times.get(i + 1).copied().unwrap_or(dur.max(t + 1.0 / fps));
            let delay = (next - t).max(0.02);
            #[allow(deprecated)]
            let Ok(img) = gen.copyCGImageAtTime_actualTime_error(time(*t), std::ptr::null_mut()) else { continue };
            let fp = gif_dict(&[(kCGImagePropertyGIFDelayTime, NSNumber::new_f64(delay)), (kCGImagePropertyGIFUnclampedDelayTime, NSNumber::new_f64(delay))]);
            CGImageDestinationAddImage(dest, (&*img as *const objc2_core_graphics::CGImage).cast(), Retained::as_ptr(&fp).cast());
            added += 1;
            progress((i + 1) as f64 / times.len() as f64);
        }
        let ok = added > 0 && CGImageDestinationFinalize(dest);
        CFRelease(dest);
        if ok {
            Ok(())
        } else {
            Err("GIF-Erstellung fehlgeschlagen".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets() {
        assert_eq!(preset_size("maximum"), None);
        assert_eq!(preset_size("social"), Some((1920.0, 1080.0)));
        assert!(fits_preset((1280.0, 720.0), (1920.0, 1080.0)));
        assert!(fits_preset((720.0, 1280.0), (1920.0, 1080.0)), "Hochformat zählt nach langer Seite");
        assert!(!fits_preset((2880.0, 1800.0), (1920.0, 1080.0)));
    }

    #[test]
    fn gif_frame_times() {
        assert_eq!(gif_params("web"), (15.0, 720.0));
        let t = gif_times(2.0, 25.0);
        assert_eq!(t.len(), 50);
        assert_eq!(t[1], 0.04);
        // 100 s × 25 fps = 2500 > 600 → gedeckelt
        assert!(gif_times(100.0, 25.0).len() <= 600);
        assert!(gif_times(0.0, 25.0).is_empty());
    }
}
