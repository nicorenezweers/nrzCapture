//! Export über AVFoundation (Port von Capso `ExportKit`).
//!
//! - **Schnitt-Export** (keine Effekte/Hintergrund/Cursor): Behaltene Bereiche per `AVMutableComposition`
//!   aneinanderhängen und mit `AVAssetExportSession` als MP4 schreiben (CutExporter/MP4Exporter).
//! - **Kompositions-Export**: Bilder per `AVAssetReader` (BGRA) aus der Komposition lesen, mit `render::Renderer`
//!   (tiny-skia, gleiche Geometrie wie die Vorschau) zusammensetzen und per `AVAssetWriter` (H.264 + AAC, Ton aller
//!   Spuren gemischt) schreiben – bzw. als GIF (`gif`-Crate, NeuQuant-Palette pro Bild, Endlosschleife).
#![allow(deprecated)]
use objc2::{rc::Retained, runtime::AnyObject, Message};
use objc2_av_foundation::{
    AVAsset, AVAssetExportPreset1280x720, AVAssetExportPreset1920x1080, AVAssetExportPresetHighestQuality, AVAssetExportPresetPassthrough, AVAssetExportSession,
    AVAssetExportSessionStatus, AVAssetReader, AVAssetReaderAudioMixOutput, AVAssetReaderOutput, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVAssetTrack,
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor, AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeAudio, AVMediaTypeVideo,
    AVMutableComposition, AVURLAsset,
};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeRange};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferPoolCreatePixelBuffer, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSURL};
use std::{
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use tiny_skia::{IntSize, Pixmap};

use super::model::{effective_to_source, CursorEvent, Engine, Layout, RecordingProject, TimeRange};
use super::render::Renderer;
use crate::core::t;

pub struct Job {
    pub video: PathBuf,
    pub project: RecordingProject,
    pub events: Vec<CursorEvent>,
    pub gif: bool,
    /// "maximum" | "social" | "web"
    pub quality: String,
    pub dest: PathBuf,
    pub cancel: Arc<AtomicBool>,
}

fn any<T: Message>(r: Retained<T>) -> Retained<AnyObject> {
    // SAFETY: Jedes Objective-C-Objekt ist ein AnyObject.
    unsafe { Retained::cast_unchecked(r) }
}

fn n_i(v: i64) -> Retained<AnyObject> {
    any(NSNumber::new_i64(v))
}

fn n_f(v: f64) -> Retained<AnyObject> {
    any(NSNumber::new_f64(v))
}

fn n_b(v: bool) -> Retained<AnyObject> {
    any(NSNumber::new_bool(v))
}

fn s(v: &str) -> Retained<AnyObject> {
    any(NSString::from_str(v))
}

fn dict(pairs: Vec<(&str, Retained<AnyObject>)>) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let vals: Vec<&AnyObject> = pairs.iter().map(|(_, v)| &**v).collect();
    NSDictionary::from_slices(&key_refs, &vals)
}

fn file_url(p: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
}

fn cm(seconds: f64) -> CMTime {
    unsafe { CMTime::with_seconds(seconds, 600) }
}

fn secs(t: CMTime) -> f64 {
    unsafe { t.seconds() }
}

fn ns_err(e: &objc2_foundation::NSError) -> String {
    e.localizedDescription().to_string()
}

fn tracks(asset: &AVAsset, video: bool) -> Vec<Retained<AVAssetTrack>> {
    let ty = unsafe { if video { AVMediaTypeVideo } else { AVMediaTypeAudio } };
    let Some(ty) = ty else { return vec![] };
    unsafe { asset.tracksWithMediaType(ty) }.to_vec()
}

/// (Dauer s, Breite px, Höhe px) des Videos.
pub fn probe(video: &Path) -> Option<(f64, f64, f64)> {
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&file_url(video), None) };
    let d = secs(unsafe { asset.duration() });
    let tr = tracks(&asset, true).into_iter().next()?;
    let size = unsafe { tr.naturalSize() };
    let tf = unsafe { tr.preferredTransform() };
    // Hochkant gedrehte Videos (a = 0) tauschen Breite und Höhe.
    let (w, h) = if tf.a.abs() < 0.5 && tf.b.abs() > 0.5 { (size.height, size.width) } else { (size.width, size.height) };
    (d.is_finite() && w > 0.0).then_some((d, w, h))
}

/// Anzahl der Tonspuren einer Datei (System-Audio und Mikrofon landen in getrennten Spuren).
pub fn audio_track_count(video: &Path) -> usize {
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&file_url(video), None) };
    tracks(&asset, false).len()
}

/// Lineares PCM für `AVAssetReaderAudioMixOutput` (mischt alle Spuren).
fn pcm_settings() -> Retained<NSDictionary<NSString, AnyObject>> {
    dict(vec![
        ("AVFormatIDKey", n_i(0x6C70636D)), // 'lpcm'
        ("AVSampleRateKey", n_f(48000.0)),
        ("AVNumberOfChannelsKey", n_i(2)),
        ("AVLinearPCMBitDepthKey", n_i(16)),
        ("AVLinearPCMIsFloatKey", n_b(false)),
        ("AVLinearPCMIsBigEndianKey", n_b(false)),
        ("AVLinearPCMIsNonInterleaved", n_b(false)),
    ])
}

fn aac_settings() -> Retained<NSDictionary<NSString, AnyObject>> {
    dict(vec![
        ("AVFormatIDKey", n_i(0x61616320)), // 'aac '
        ("AVSampleRateKey", n_f(48000.0)),
        ("AVNumberOfChannelsKey", n_i(2)),
        ("AVEncoderBitRateKey", n_i(160_000)),
    ])
}

/// Video unverändert übernehmen und alle Tonspuren zu einer AAC-Spur mischen
/// (Capso `MP4Exporter.exportWithMergedAudio`: viele Programme lesen nur die erste Tonspur).
pub fn merge_audio_tracks(src: &Path, dest: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(dest);
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&file_url(src), None) };
    let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(&asset) }.map_err(|e| ns_err(&e))?;
    let writer = unsafe { AVAssetWriter::assetWriterWithURL_fileType_error(&file_url(dest), AVFileTypeMPEG4.unwrap()) }.map_err(|e| ns_err(&e))?;
    unsafe { writer.setShouldOptimizeForNetworkUse(true) };
    let video = tracks(&asset, true).into_iter().next().map(|vt| unsafe {
        let out = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&vt, None);
        out.setAlwaysCopiesSampleData(false);
        reader.addOutput(&out);
        let fds = vt.formatDescriptions();
        // SAFETY: Elemente von `formatDescriptions` sind CMFormatDescriptionRef (toll-free als Objekt verpackt).
        let hint = fds.firstObject().map(|o| &*(Retained::as_ptr(&o) as *const objc2_core_media::CMFormatDescription));
        let input = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings_sourceFormatHint(AVMediaTypeVideo.unwrap(), None, hint);
        input.setExpectsMediaDataInRealTime(false);
        input.setTransform(vt.preferredTransform());
        writer.addInput(&input);
        (out, input)
    });
    let audio_tracks = tracks(&asset, false);
    let audio = (!audio_tracks.is_empty()).then(|| unsafe {
        let arr = NSArray::from_retained_slice(&audio_tracks);
        let out = AVAssetReaderAudioMixOutput::assetReaderAudioMixOutputWithAudioTracks_audioSettings(&arr, Some(&pcm_settings()));
        reader.addOutput(&out);
        let input = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(AVMediaTypeAudio.unwrap(), Some(&aac_settings()));
        input.setExpectsMediaDataInRealTime(false);
        writer.addInput(&input);
        (out, input)
    });
    if !unsafe { reader.startReading() } {
        return Err(unsafe { reader.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "AVAssetReader".into()));
    }
    if !unsafe { writer.startWriting() } {
        unsafe { reader.cancelReading() };
        return Err(unsafe { writer.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "AVAssetWriter".into()));
    }
    unsafe { writer.startSessionAtSourceTime(cm(0.0)) };
    let mut video_done = video.is_none();
    let mut audio_done = audio.is_none();
    while !(video_done && audio_done) {
        if unsafe { writer.status() } == AVAssetWriterStatus::Failed {
            unsafe { reader.cancelReading() };
            return Err(unsafe { writer.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "AVAssetWriter".into()));
        }
        let mut worked = false;
        if let (false, Some((out, input))) = (video_done, video.as_ref()) {
            if unsafe { input.isReadyForMoreMediaData() } {
                worked = true;
                let o: &AVAssetReaderOutput = out;
                match unsafe { o.copyNextSampleBuffer() } {
                    Some(sb) if unsafe { input.appendSampleBuffer(&sb) } => {}
                    _ => {
                        unsafe { input.markAsFinished() };
                        video_done = true;
                    }
                }
            }
        }
        if let (false, Some((out, input))) = (audio_done, audio.as_ref()) {
            if unsafe { input.isReadyForMoreMediaData() } {
                worked = true;
                let o: &AVAssetReaderOutput = out;
                match unsafe { o.copyNextSampleBuffer() } {
                    Some(sb) if unsafe { input.appendSampleBuffer(&sb) } => {}
                    _ => {
                        unsafe { input.markAsFinished() };
                        audio_done = true;
                    }
                }
            }
        }
        if !worked {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    if unsafe { reader.status() } == AVAssetReaderStatus::Failed {
        unsafe { writer.cancelWriting() };
        return Err(unsafe { reader.error() }.map(|e| ns_err(&e)).unwrap_or_default());
    }
    let (tx, rx) = mpsc::channel::<()>();
    let block = block2::RcBlock::new(move || {
        let _ = tx.send(());
    });
    unsafe { writer.finishWritingWithCompletionHandler(&block) };
    let _ = rx.recv_timeout(Duration::from_secs(600));
    if unsafe { writer.status() } != AVAssetWriterStatus::Completed {
        return Err(unsafe { writer.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "AVAssetWriter".into()));
    }
    Ok(())
}

/// Asset mit allen behaltenen Bereichen nacheinander (oder das Original ohne Schnitte).
fn trimmed_asset(source: &AVAsset, kept: &[TimeRange], duration: f64) -> Result<Retained<AVAsset>, String> {
    let untouched = kept.len() == 1 && kept[0].start < 0.001 && (kept[0].end - duration).abs() < 0.001;
    if untouched {
        return Ok(source.retain());
    }
    if kept.is_empty() {
        return Err(t("Alles wurde herausgeschnitten.", "Everything has been cut.").into());
    }
    let comp = unsafe { AVMutableComposition::composition() };
    let mut at = 0.0;
    for r in kept {
        let range = CMTimeRange { start: cm(r.start), duration: cm(r.duration()) };
        unsafe { comp.insertTimeRange_ofAsset_atTime_error(range, source, cm(at)) }.map_err(|e| ns_err(&e))?;
        at += r.duration();
    }
    Ok(Retained::into_super(Retained::into_super(comp)))
}

/// Exportiert nach `job.dest`. `progress(0..1, stage)`.
pub fn run(job: &Job, progress: &dyn Fn(f64, &str)) -> Result<(), String> {
    progress(0.0, t("Export wird vorbereitet …", "Preparing export…"));
    let _ = std::fs::remove_file(&job.dest);
    let source: Retained<AVAsset> = Retained::into_super(unsafe { AVURLAsset::URLAssetWithURL_options(&file_url(&job.video), None) });
    let duration = secs(unsafe { source.duration() });
    let mut project = job.project.clone();
    if duration.is_finite() && duration > 0.0 {
        project.video_duration = duration;
    }
    let kept = project.kept_ranges();
    let asset = trimmed_asset(&source, &kept, project.video_duration)?;
    let has_tel = !job.events.is_empty();
    if !job.gif && !project.needs_compositor(has_tel) {
        progress(0.05, t("Schnitte werden angewendet …", "Applying trims…"));
        return cut_export(&asset, &job.dest, &job.quality, kept.len() == 1 && kept[0].start < 0.001 && (kept[0].end - project.video_duration).abs() < 0.001, &job.cancel, progress);
    }
    let fps = tracks(&source, true).first().map(|t| unsafe { t.nominalFrameRate() } as f64).filter(|f| *f >= 1.0).unwrap_or(30.0).min(240.0);
    composite_export(job, project, &asset, &kept, fps, progress)
}

fn cut_export(asset: &AVAsset, dest: &Path, quality: &str, untouched: bool, cancel: &AtomicBool, progress: &dyn Fn(f64, &str)) -> Result<(), String> {
    let preset = unsafe {
        match quality {
            "social" => AVAssetExportPreset1920x1080,
            "web" => AVAssetExportPreset1280x720,
            _ if untouched => AVAssetExportPresetPassthrough,
            _ => AVAssetExportPresetHighestQuality,
        }
    };
    let session = unsafe { AVAssetExportSession::exportSessionWithAsset_presetName(asset, preset) }.ok_or("AVAssetExportSession")?;
    unsafe {
        session.setOutputURL(Some(&file_url(dest)));
        session.setOutputFileType(AVFileTypeMPEG4);
        session.setShouldOptimizeForNetworkUse(true);
    }
    let (tx, rx) = mpsc::channel::<()>();
    let block = block2::RcBlock::new(move || {
        let _ = tx.send(());
    });
    unsafe { session.exportAsynchronouslyWithCompletionHandler(&block) };
    loop {
        match rx.recv_timeout(Duration::from_millis(150)) {
            Ok(()) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::Relaxed) {
                    unsafe { session.cancelExport() };
                }
                progress(0.05 + 0.9 * unsafe { session.progress() } as f64, t("Video wird kodiert …", "Encoding video…"));
            }
            Err(_) => break,
        }
    }
    match unsafe { session.status() } {
        AVAssetExportSessionStatus::Completed => {
            progress(1.0, t("Fertig", "Done"));
            Ok(())
        }
        AVAssetExportSessionStatus::Cancelled => Err(t("Abgebrochen", "Cancelled").into()),
        _ => Err(unsafe { session.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "Export fehlgeschlagen".into())),
    }
}

/// Liest gesperrte BGRA-Pixel in ein RGBA-Pixmap (Kanaltausch, Alpha 255).
fn read_pixels(pb: &CVPixelBuffer, into: &mut Option<Pixmap>) -> Option<()> {
    unsafe { CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) };
    let (w, h, bpr) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb), CVPixelBufferGetBytesPerRow(pb));
    let base = CVPixelBufferGetBaseAddress(pb) as *const u8;
    let ok = !base.is_null() && w > 0 && h > 0;
    if ok {
        if into.as_ref().map(|p| p.width() as usize != w || p.height() as usize != h).unwrap_or(true) {
            *into = Pixmap::from_vec(vec![0; w * h * 4], IntSize::from_wh(w as u32, h as u32)?);
        }
        let dst = into.as_mut()?.data_mut();
        for y in 0..h {
            let row = unsafe { std::slice::from_raw_parts(base.add(y * bpr), w * 4) };
            let out = &mut dst[y * w * 4..(y + 1) * w * 4];
            for (o, i) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                o[0] = i[2];
                o[1] = i[1];
                o[2] = i[0];
                o[3] = 255;
            }
        }
    }
    unsafe { CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) };
    ok.then_some(())
}

/// Schreibt das gerenderte Bild (RGBA, deckend) in einen BGRA-Pixelpuffer.
fn write_pixels(pm: &Pixmap, pb: &CVPixelBuffer) {
    unsafe { CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::empty()) };
    let (w, h, bpr) = (CVPixelBufferGetWidth(pb).min(pm.width() as usize), CVPixelBufferGetHeight(pb).min(pm.height() as usize), CVPixelBufferGetBytesPerRow(pb));
    let base = CVPixelBufferGetBaseAddress(pb) as *mut u8;
    if !base.is_null() {
        let src = pm.data();
        let sw = pm.width() as usize;
        for y in 0..h {
            let row = unsafe { std::slice::from_raw_parts_mut(base.add(y * bpr), w * 4) };
            for (o, i) in row.chunks_exact_mut(4).zip(src[y * sw * 4..(y * sw + w) * 4].chunks_exact(4)) {
                o[0] = i[2];
                o[1] = i[1];
                o[2] = i[0];
                o[3] = 255;
            }
        }
    }
    unsafe { CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::empty()) };
}

fn composite_export(job: &Job, project: RecordingProject, asset: &AVAsset, kept: &[TimeRange], fps: f64, progress: &dyn Fn(f64, &str)) -> Result<(), String> {
    let total = kept.iter().map(TimeRange::duration).sum::<f64>().max(0.001);
    let layout = Layout::for_export(&project, &job.quality, job.gif);
    let engine = Engine::new(project, &job.events);
    let mut renderer = Renderer::new(engine, layout)?;
    let (ow, oh) = (renderer.width(), renderer.height());

    let vtrack = tracks(asset, true).into_iter().next().ok_or(t("Keine Videospur gefunden", "No video track found"))?;
    let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(asset) }.map_err(|e| ns_err(&e))?;
    let vout = unsafe {
        AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(&vtrack, Some(&dict(vec![("PixelFormatType", n_i(kCVPixelFormatType_32BGRA as i64))])))
    };
    unsafe {
        vout.setAlwaysCopiesSampleData(false);
        reader.addOutput(&vout);
    }
    let audio_tracks = if job.gif { vec![] } else { tracks(asset, false) };
    let aout: Option<Retained<AVAssetReaderAudioMixOutput>> = (!audio_tracks.is_empty()).then(|| unsafe {
        let arr = NSArray::from_retained_slice(&audio_tracks);
        let o = AVAssetReaderAudioMixOutput::assetReaderAudioMixOutputWithAudioTracks_audioSettings(
            &arr,
            Some(&dict(vec![
                ("AVFormatIDKey", n_i(0x6C70636D)), // 'lpcm'
                ("AVSampleRateKey", n_f(48000.0)),
                ("AVNumberOfChannelsKey", n_i(2)),
                ("AVLinearPCMBitDepthKey", n_i(16)),
                ("AVLinearPCMIsFloatKey", n_b(false)),
                ("AVLinearPCMIsBigEndianKey", n_b(false)),
                ("AVLinearPCMIsNonInterleaved", n_b(false)),
            ])),
        );
        reader.addOutput(&o);
        o
    });
    if !unsafe { reader.startReading() } {
        return Err(unsafe { reader.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "AVAssetReader".into()));
    }
    let src_time = |te: f64| if kept.len() == 1 { kept[0].start + te } else { effective_to_source(kept, te) };
    let mut frame: Option<Pixmap> = None;
    let cancelled = || job.cancel.load(Ordering::Relaxed);
    let stage = t("Bilder werden gerendert …", "Rendering edited frames…");

    if job.gif {
        let fps_gif = super::model::gif_fps(&job.quality, total);
        let delay = (100.0 / fps_gif).round().max(2.0) as u16;
        let file = std::fs::File::create(&job.dest).map_err(|e| e.to_string())?;
        let mut enc = gif::Encoder::new(std::io::BufWriter::new(file), ow as u16, oh as u16, &[]).map_err(|e| e.to_string())?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
        let mut next = 0.0;
        while let Some(sb) = unsafe { vout.copyNextSampleBuffer() } {
            if cancelled() {
                unsafe { reader.cancelReading() };
                return Err(t("Abgebrochen", "Cancelled").into());
            }
            let te = secs(unsafe { sb.presentation_time_stamp() });
            if te + 0.5 / fps < next {
                continue;
            }
            next += 1.0 / fps_gif;
            let Some(img) = (unsafe { sb.image_buffer() }) else { continue };
            if read_pixels(&img, &mut frame).is_none() {
                continue;
            }
            let pm = renderer.render(frame.as_ref().unwrap().as_ref(), src_time(te));
            let mut rgba = pm.data().to_vec();
            let mut f = gif::Frame::from_rgba_speed(ow as u16, oh as u16, &mut rgba, 10);
            f.delay = delay;
            enc.write_frame(&f).map_err(|e| e.to_string())?;
            progress((te / total).min(0.99), stage);
        }
        if unsafe { reader.status() } == AVAssetReaderStatus::Failed {
            return Err(unsafe { reader.error() }.map(|e| ns_err(&e)).unwrap_or_default());
        }
        progress(1.0, t("Fertig", "Done"));
        return Ok(());
    }

    // MP4 über AVAssetWriter
    let writer = unsafe { AVAssetWriter::assetWriterWithURL_fileType_error(&file_url(&job.dest), AVFileTypeMPEG4.unwrap()) }.map_err(|e| ns_err(&e))?;
    let bitrate = match job.quality.as_str() {
        "web" => (ow as f64 * oh as f64 * fps * 0.08).min(5_000_000.0),
        "social" => (ow as f64 * oh as f64 * fps * 0.1).min(10_000_000.0),
        _ => (ow as f64 * oh as f64 * fps * 0.2).clamp(4_000_000.0, 60_000_000.0),
    };
    let vsettings = dict(vec![
        ("AVVideoCodecKey", s("avc1")),
        ("AVVideoWidthKey", n_i(ow as i64)),
        ("AVVideoHeightKey", n_i(oh as i64)),
        (
            "AVVideoCompressionPropertiesKey",
            any(dict(vec![
                ("AverageBitRate", n_f(bitrate)),
                ("ProfileLevel", s("H264_High_AutoLevel")),
                ("MaxKeyFrameInterval", n_i((fps * 2.0) as i64)),
            ])),
        ),
    ]);
    let vin = unsafe { AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(AVMediaTypeVideo.unwrap(), Some(&vsettings)) };
    unsafe { vin.setExpectsMediaDataInRealTime(false) };
    let adaptor = unsafe {
        AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(
            &vin,
            Some(&dict(vec![
                ("PixelFormatType", n_i(kCVPixelFormatType_32BGRA as i64)),
                ("Width", n_i(ow as i64)),
                ("Height", n_i(oh as i64)),
            ])),
        )
    };
    unsafe { writer.addInput(&vin) };
    let ain = aout.as_ref().map(|_| unsafe {
        let i = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
            AVMediaTypeAudio.unwrap(),
            Some(&dict(vec![
                ("AVFormatIDKey", n_i(0x61616320)), // 'aac '
                ("AVSampleRateKey", n_f(48000.0)),
                ("AVNumberOfChannelsKey", n_i(2)),
                ("AVEncoderBitRateKey", n_i(160_000)),
            ])),
        );
        i.setExpectsMediaDataInRealTime(false);
        writer.addInput(&i);
        i
    });
    if !unsafe { writer.startWriting() } {
        return Err(unsafe { writer.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "AVAssetWriter".into()));
    }
    unsafe { writer.startSessionAtSourceTime(cm(0.0)) };
    let fail = |msg: String| -> Result<(), String> {
        unsafe {
            reader.cancelReading();
            writer.cancelWriting();
        }
        Err(msg)
    };
    let mut video_done = false;
    let mut audio_done = ain.is_none();
    while !(video_done && audio_done) {
        if cancelled() {
            return fail(t("Abgebrochen", "Cancelled").into());
        }
        if unsafe { writer.status() } == AVAssetWriterStatus::Failed {
            return fail(unsafe { writer.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "AVAssetWriter".into()));
        }
        let mut worked = false;
        if !video_done && unsafe { vin.isReadyForMoreMediaData() } {
            worked = true;
            let vo: &AVAssetReaderOutput = &vout;
            match unsafe { vo.copyNextSampleBuffer() } {
                Some(sb) => {
                    let pts = unsafe { sb.presentation_time_stamp() };
                    let te = secs(pts);
                    if let Some(img) = unsafe { sb.image_buffer() } {
                        if read_pixels(&img, &mut frame).is_some() {
                            let pm = renderer.render(frame.as_ref().unwrap().as_ref(), src_time(te));
                            let pool = unsafe { adaptor.pixelBufferPool() }.ok_or("CVPixelBufferPool")?;
                            let mut out: *mut CVPixelBuffer = std::ptr::null_mut();
                            let rc = unsafe { CVPixelBufferPoolCreatePixelBuffer(None, &pool, NonNull::from(&mut out)) };
                            if rc != 0 || out.is_null() {
                                return fail(format!("CVPixelBufferPoolCreatePixelBuffer {rc}"));
                            }
                            // SAFETY: „Create“-Regel → wir besitzen den Puffer.
                            let out = unsafe { objc2_core_foundation::CFRetained::from_raw(NonNull::new_unchecked(out)) };
                            write_pixels(pm, &out);
                            if !unsafe { adaptor.appendPixelBuffer_withPresentationTime(&out, pts) } {
                                return fail(unsafe { writer.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "appendPixelBuffer".into()));
                            }
                        }
                    }
                    progress((te / total).min(0.99), stage);
                }
                None => {
                    unsafe { vin.markAsFinished() };
                    video_done = true;
                }
            }
        }
        if let (false, Some(ain), Some(aout)) = (audio_done, ain.as_ref(), aout.as_ref()) {
            if unsafe { ain.isReadyForMoreMediaData() } {
                worked = true;
                let next: Option<Retained<CMSampleBuffer>> = unsafe { aout.copyNextSampleBuffer() };
                match next {
                    Some(sb) => {
                        if !unsafe { ain.appendSampleBuffer(&sb) } {
                            unsafe { ain.markAsFinished() };
                            audio_done = true;
                        }
                    }
                    None => {
                        unsafe { ain.markAsFinished() };
                        audio_done = true;
                    }
                }
            }
        }
        if !worked {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    if unsafe { reader.status() } == AVAssetReaderStatus::Failed {
        return fail(unsafe { reader.error() }.map(|e| ns_err(&e)).unwrap_or_default());
    }
    progress(0.995, t("Datei wird abgeschlossen …", "Finalizing file…"));
    let (tx, rx) = mpsc::channel::<()>();
    let block = block2::RcBlock::new(move || {
        let _ = tx.send(());
    });
    unsafe { writer.finishWritingWithCompletionHandler(&block) };
    let _ = rx.recv_timeout(Duration::from_secs(120));
    if unsafe { writer.status() } != AVAssetWriterStatus::Completed {
        return Err(unsafe { writer.error() }.map(|e| ns_err(&e)).unwrap_or_else(|| "AVAssetWriter".into()));
    }
    progress(1.0, t("Fertig", "Done"));
    Ok(())
}
