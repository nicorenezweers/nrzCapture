//! ScreenCaptureKit-Aufnahme (macOS 15+): `SCStream` + `SCRecordingOutput` schreibt direkt H.264-MP4.
//! Port von Capso `ScreenRecorder` (dort AVAssetWriter; hier die einfachere SCRecordingOutput-API).
//!
//! - Bereich: Display-Filter + `sourceRect`; Fenster: `initWithDesktopIndependentWindow`; Vollbild: Display-Filter.
//! - Eigene Fenster werden über den Filter ausgeschlossen (ganze App), außer `except_windows`
//!   (Kamera-PiP, Klick-Hervorhebung, Tasten-Overlay – die sollen im Video sein).
//!   Hinweis: Seit macOS 15 respektiert SCK `NSWindowSharingType::None` nicht mehr zuverlässig – daher der Filter.
//! - Pause = aktuelle `SCRecordingOutput` entfernen (Segment wird abgeschlossen), Fortsetzen = neue Output-Datei.
//!   Beim Stopp werden die Segmente mit `export::concat` (AVMutableComposition) zusammengefügt.
//! - Mikrofon-Pegel: zusätzlicher `SCStreamOutput` für `.microphone` misst den Spitzenpegel (stummes Mikrofon erkennen);
//!   Zeitpunkte der letzten Mikrofon-/System-Audio-Puffer erkennen Aussetzer (> 5 s).
use objc2::{define_class, msg_send, rc::Retained, runtime::ProtocolObject, AllocAnyThread, DefinedClass};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_screen_capture_kit::*;
use std::{
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    time::Duration,
};

/// Was aufgenommen wird.
#[derive(Clone, Debug)]
pub struct Config {
    /// Bereich in globalen Punkten (x, y, w, h); bei `window_id` = Fensterrahmen.
    pub rect: [f64; 4],
    pub window_id: Option<u32>,
    pub fullscreen: bool,
    pub fps: i32,
    pub show_cursor: bool,
    pub system_audio: bool,
    pub mic: bool,
    pub mic_device: Option<String>,
    /// Eigene Fenster (CGWindowID), die trotz App-Ausschluss mit aufgenommen werden.
    pub except_windows: Vec<u32>,
}

/// Gemeinsamer Zustand zwischen Delegate (SCK-Threads) und Rust.
#[derive(Default)]
pub struct Shared {
    finished: (Mutex<usize>, Condvar),
    pub failed: AtomicBool,
    /// Stream vom System beendet (z. B. über das Menüleisten-Symbol „Aufnahme stoppen“).
    pub stream_stopped: AtomicBool,
    pub mic_buffers: AtomicUsize,
    /// Lautester bisher gemessener Mikrofon-Pegel (f32-Bits, linear 0…1).
    pub mic_peak: AtomicU32,
    /// Zeitpunkt (ms seit UNIX-Epoche) des letzten Mikrofon- bzw. System-Audio-Puffers, 0 = noch keiner
    /// (Capso `AudioActivityMonitor`).
    pub mic_last_ms: AtomicU64,
    pub sys_last_ms: AtomicU64,
}

/// Jetzt in ms seit UNIX-Epoche.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Shared {
    pub fn mic_peak(&self) -> f32 {
        f32::from_bits(self.mic_peak.load(Ordering::Relaxed))
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "NrzCaptureRecDelegate"]
    #[ivars = Arc<Shared>]
    struct RecDelegate;

    unsafe impl NSObjectProtocol for RecDelegate {}

    unsafe impl SCStreamDelegate for RecDelegate {
        #[unsafe(method(stream:didStopWithError:))]
        fn stream_did_stop(&self, _stream: &SCStream, error: &NSError) {
            eprintln!("SCStream gestoppt: {error:?}");
            self.ivars().stream_stopped.store(true, Ordering::Relaxed);
        }
    }

    unsafe impl SCStreamOutput for RecDelegate {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, buf: &CMSampleBuffer, ty: SCStreamOutputType) {
            if ty == SCStreamOutputType::Audio {
                self.ivars().sys_last_ms.store(now_ms(), Ordering::Relaxed);
            }
            if ty == SCStreamOutputType::Microphone {
                let s = self.ivars();
                s.mic_buffers.fetch_add(1, Ordering::Relaxed);
                s.mic_last_ms.store(now_ms(), Ordering::Relaxed);
                let p = peak_of(buf);
                if p > s.mic_peak() {
                    s.mic_peak.store(p.to_bits(), Ordering::Relaxed);
                }
            }
        }
    }

    unsafe impl SCRecordingOutputDelegate for RecDelegate {
        #[unsafe(method(recordingOutputDidFinishRecording:))]
        fn rec_finished(&self, _o: &SCRecordingOutput) {
            let (m, c) = &self.ivars().finished;
            *m.lock().unwrap() += 1;
            c.notify_all();
        }

        #[unsafe(method(recordingOutput:didFailWithError:))]
        fn rec_failed(&self, _o: &SCRecordingOutput, error: &NSError) {
            eprintln!("SCRecordingOutput Fehler: {error:?}");
            self.ivars().failed.store(true, Ordering::Relaxed);
            let (m, c) = &self.ivars().finished;
            *m.lock().unwrap() += 1;
            c.notify_all();
        }
    }
);

impl RecDelegate {
    fn new(shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(shared);
        unsafe { msg_send![super(this), init] }
    }
}

/// Spitzenpegel eines Float32-PCM-Puffers (SCK liefert Mikrofon/System-Audio als Float32).
fn peak_of(buf: &CMSampleBuffer) -> f32 {
    unsafe {
        let Some(bb) = buf.data_buffer() else { return 0.0 };
        let len = bb.data_length();
        if len < 4 {
            return 0.0;
        }
        let mut v = vec![0f32; len / 4];
        let st = bb.copy_data_bytes(0, v.len() * 4, NonNull::new_unchecked(v.as_mut_ptr().cast()));
        if st != 0 {
            return 0.0;
        }
        v.iter().filter(|x| x.is_finite()).fold(0f32, |m, x| m.max(x.abs()))
    }
}

/// Laufende Aufnahme. Die ObjC-Objekte sind thread-sicher nutzbar (SCK ruft selbst auf eigenen Queues).
pub struct Recorder {
    stream: Retained<SCStream>,
    output: Option<Retained<SCRecordingOutput>>,
    delegate: Retained<RecDelegate>,
    pub shared: Arc<Shared>,
    dir: PathBuf,
    pub segments: Vec<PathBuf>,
    pub size: (usize, usize),
}
unsafe impl Send for Recorder {}

fn shareable_content() -> Result<Retained<SCShareableContent>, String> {
    let (tx, rx) = mpsc::channel::<Result<SendPtr<SCShareableContent>, String>>();
    let block = block2::RcBlock::new(move |c: *mut SCShareableContent, e: *mut NSError| {
        let r = match unsafe { Retained::retain(c) } {
            Some(c) => Ok(SendPtr(c)),
            None => Err(unsafe { e.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "SCShareableContent fehlt".into())),
        };
        let _ = tx.send(r);
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "Zeitüberschreitung bei SCShareableContent".to_string())?.map(|p| p.0)
}

struct SendPtr<T>(Retained<T>);
unsafe impl<T> Send for SendPtr<T> {}

/// Wartet auf einen SCK-Completion-Handler mit optionalem Fehler.
fn wait_err(f: impl FnOnce(&block2::DynBlock<dyn Fn(*mut NSError)>)) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Option<String>>();
    let block = block2::RcBlock::new(move |e: *mut NSError| {
        let _ = tx.send(unsafe { e.as_ref() }.map(|e| e.localizedDescription().to_string()));
    });
    f(&block);
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(None) => Ok(()),
        Ok(Some(e)) => Err(e),
        Err(_) => Err("Zeitüberschreitung".into()),
    }
}

fn even(v: f64) -> usize {
    let v = v.ceil().max(2.0) as usize;
    v + (v & 1)
}

impl Recorder {
    /// Stream aufbauen und erste Segment-Datei starten.
    pub fn start(cfg: &Config, dir: &Path) -> Result<Self, String> {
        let content = shareable_content()?;
        let pid = std::process::id() as i32;
        unsafe {
            let windows = content.windows();
            let find_win = |id: u32| windows.iter().find(|w| w.windowID() == id);
            let [rx, ry, rw, rh] = cfg.rect;
            let (cx, cy) = (rx + rw / 2.0, ry + rh / 2.0);
            let displays = content.displays();
            let display = displays
                .iter()
                .find(|d| {
                    let f = d.frame();
                    cx >= f.origin.x && cy >= f.origin.y && cx < f.origin.x + f.size.width && cy < f.origin.y + f.size.height
                })
                .or_else(|| displays.iter().next())
                .ok_or("Kein Bildschirm gefunden")?;

            let filter: Retained<SCContentFilter> = if let Some(id) = cfg.window_id {
                let w = find_win(id).ok_or("Fenster nicht mehr vorhanden")?;
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &w)
            } else {
                let apps: Vec<_> = content.applications().iter().filter(|a| a.processID() == pid).collect();
                let except: Vec<_> = cfg.except_windows.iter().filter_map(|id| find_win(*id)).collect();
                SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &NSArray::from_retained_slice(&apps),
                    &NSArray::from_retained_slice(&except),
                )
            };
            let scale = SCShareableContent::infoForFilter(&filter).pointPixelScale() as f64;
            let sc = SCStreamConfiguration::new();
            let size = if cfg.window_id.is_some() {
                (even(rw * scale), even(rh * scale))
            } else if cfg.fullscreen {
                let f = display.frame();
                (even(f.size.width * scale), even(f.size.height * scale))
            } else {
                let f = display.frame();
                sc.setSourceRect(CGRect::new(CGPoint::new(rx - f.origin.x, ry - f.origin.y), CGSize::new(rw, rh)));
                (even(rw * scale), even(rh * scale))
            };
            sc.setWidth(size.0);
            sc.setHeight(size.1);
            sc.setMinimumFrameInterval(CMTime::new(1, cfg.fps.max(1)));
            sc.setShowsCursor(cfg.show_cursor);
            sc.setQueueDepth(6);
            sc.setCapturesAudio(cfg.system_audio);
            sc.setExcludesCurrentProcessAudio(true);
            if cfg.mic {
                sc.setCaptureMicrophone(true);
                if let Some(id) = cfg.mic_device.as_deref().filter(|s| !s.is_empty()) {
                    sc.setMicrophoneCaptureDeviceID(Some(&NSString::from_str(id)));
                }
            }

            let shared = Arc::new(Shared::default());
            let delegate = RecDelegate::new(shared.clone());
            let stream = SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &sc,
                Some(ProtocolObject::from_ref(&*delegate)),
            );
            // Bildschirm-Ausgabe registrieren (SCRecordingOutput liefert sonst je nach macOS-Version keine Bilder),
            // Mikrofon-Ausgabe für die Pegelmessung.
            let out: &ProtocolObject<dyn SCStreamOutput> = ProtocolObject::from_ref(&*delegate);
            stream.addStreamOutput_type_sampleHandlerQueue_error(out, SCStreamOutputType::Screen, None).map_err(|e| e.localizedDescription().to_string())?;
            if cfg.mic {
                let _ = stream.addStreamOutput_type_sampleHandlerQueue_error(out, SCStreamOutputType::Microphone, None);
            }
            // System-Audio-Puffer zählen (Aussetzer erkennen, Capso `AudioActivityMonitor.systemAudioBuffer`).
            if cfg.system_audio {
                let _ = stream.addStreamOutput_type_sampleHandlerQueue_error(out, SCStreamOutputType::Audio, None);
            }
            let mut rec = Recorder { stream, output: None, delegate, shared, dir: dir.to_owned(), segments: vec![], size };
            rec.add_segment()?;
            let s = rec.stream.clone();
            wait_err(|b| s.startCaptureWithCompletionHandler(Some(b)))?;
            Ok(rec)
        }
    }

    fn add_segment(&mut self) -> Result<(), String> {
        let path = self.dir.join(format!("segment-{}.mp4", self.segments.len()));
        let _ = std::fs::remove_file(&path);
        unsafe {
            let oc = SCRecordingOutputConfiguration::new();
            oc.setOutputURL(&NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())));
            if let Some(ft) = objc2_av_foundation::AVFileTypeMPEG4 {
                oc.setOutputFileType(ft);
            }
            if let Some(codec) = objc2_av_foundation::AVVideoCodecTypeH264 {
                oc.setVideoCodecType(codec);
            }
            let out = SCRecordingOutput::initWithConfiguration_delegate(SCRecordingOutput::alloc(), &oc, ProtocolObject::from_ref(&*self.delegate));
            self.stream.addRecordingOutput_error(&out).map_err(|e| e.localizedDescription().to_string())?;
            self.output = Some(out);
        }
        self.segments.push(path);
        Ok(())
    }

    /// Aktuelles Segment abschließen und warten, bis die Datei fertig geschrieben ist.
    fn close_segment(&mut self) {
        let Some(out) = self.output.take() else { return };
        let (m, c) = &self.shared.finished;
        let before = *m.lock().unwrap();
        let removed = unsafe { self.stream.removeRecordingOutput_error(&out) };
        if removed.is_ok() {
            let g = m.lock().unwrap();
            let _ = c.wait_timeout_while(g, Duration::from_secs(10), |n| *n == before);
        }
        // Leere/kaputte Segmente (z. B. sofort pausiert) verwerfen.
        if let Some(last) = self.segments.last() {
            if std::fs::metadata(last).map(|m| m.len() < 1024).unwrap_or(true) {
                let _ = std::fs::remove_file(last);
                self.segments.pop();
            }
        }
    }

    pub fn is_paused(&self) -> bool {
        self.output.is_none()
    }

    pub fn pause(&mut self) {
        self.close_segment();
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if self.output.is_some() {
            return Ok(());
        }
        self.add_segment()
    }

    /// Aufnahme beenden; liefert die fertigen Segment-Dateien (in Reihenfolge).
    pub fn stop(mut self) -> Vec<PathBuf> {
        self.close_segment();
        let s = self.stream.clone();
        let _ = wait_err(|b| unsafe { s.stopCaptureWithCompletionHandler(Some(b)) });
        std::mem::take(&mut self.segments)
    }
}

/// Mikrofone (AVCaptureDevice uniqueID + Name) für die Geräteauswahl.
pub fn microphones() -> Vec<(String, String)> {
    unsafe {
        let Some(ty) = objc2_av_foundation::AVMediaTypeAudio else { return vec![] };
        #[allow(deprecated)]
        let devs = objc2_av_foundation::AVCaptureDevice::devicesWithMediaType(ty);
        devs.iter().map(|d| (d.uniqueID().to_string(), d.localizedName().to_string())).collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn even_sizes() {
        assert_eq!(super::even(101.0), 102);
        assert_eq!(super::even(100.2), 102);
        assert_eq!(super::even(100.0), 100);
        assert_eq!(super::even(0.0), 2);
    }
}
