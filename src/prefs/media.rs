//! Tabs „Screenshot“, „Aufnahme“, „Quick Access“, „Export“, „OCR & Übersetzung“.
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

use super::widgets::*;
use crate::{api, api::t, ui};

/// Eingebaute Vorgaben (Capso `CapturePreset.allBuiltins`) im Einstellungsformat "W:H" / "WxH".
const BUILTIN_PRESETS: &[(&str, &str)] = &[
    ("1:1", "1:1 (Square)"),
    ("4:3", "4:3"),
    ("16:9", "16:9"),
    ("3:2", "3:2"),
    ("512x512", "512 × 512"),
    ("1280x720", "1280 × 720 (720p)"),
    ("1920x1080", "1920 × 1080 (1080p)"),
];

fn preset_label(p: &str) -> String {
    if p == "freeform" {
        return t("Frei", "Freeform").to_owned();
    }
    BUILTIN_PRESETS.iter().find(|(v, _)| *v == p).map(|(_, l)| l.replace("Square", t("Quadrat", "Square"))).unwrap_or_else(|| p.replace('x', " × "))
}

fn str_list(key: &str) -> Vec<String> {
    api::setting(key).as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

pub fn screenshot() -> impl IntoView {
    let ts_on = sb("screenshotTimestampEnabled");
    let presets_on = sb("capturePresetsEnabled");
    let copy_on = sb("screenshotAutoCopy");
    view! {
        <ui::Card title=t("Nach der Aufnahme", "After Capture")>
            {toggle_row(t("Vorschau zeigen", "Show Preview"), t("Miniatur mit Schnellaktionen (Quick Access)", "Display thumbnail with quick actions"), "screenshotShowPreview")}
            {toggle_row(t("In Zwischenablage kopieren", "Copy to Clipboard"), t("Screenshot sofort kopieren", "Copy screenshot immediately"), "screenshotAutoCopy")}
            <Show when=move || copy_on.get()>
                {select_row(t("Inhalt der Zwischenablage", "Clipboard Content"), t("Bilddaten kopieren oder den gespeicherten Pfad", "Copy image data, or the saved file path"), "screenshotClipboardContent",
                    opts(&[("image", t("Bild", "Image")), ("filePath", t("Dateipfad", "File Path"))]))}
            </Show>
            {select_row(t("Format in der Zwischenablage", "Clipboard Format"), t("Gilt für jedes Kopieren eines Screenshots", "Used whenever a screenshot is copied"), "screenshotClipboardFormat",
                opts(&[("png", "PNG"), ("jpeg", "JPEG"), ("tiff", "TIFF")]))}
            {toggle_row(t("Automatisch sichern", "Auto Save"), t("Automatisch als Datei speichern", "Save to file automatically"), "screenshotAutoSave")}
        </ui::Card>

        <ui::Card title=t("Aufnahme", "Capture")>
            {toggle_row(t("Mauszeiger zeigen", "Show Cursor"), t("Zeiger in Screenshots aufnehmen", "Include the mouse pointer in screenshots"), "screenshotShowsCursor")}
            {toggle_row(t("Fensterschatten aufnehmen", "Capture Window Shadow"), t("Schatten bei Fensteraufnahmen einschließen", "Include shadow in window captures"), "captureWindowShadow")}
            {toggle_row(t("Lupe zeigen", "Show Magnifier"), t("Vergrößerung neben dem Fadenkreuz bei der Auswahl", "Magnifier next to the crosshair while selecting"), "showMagnifier")}
        </ui::Card>

        <ui::Card title=t("Zeitstempel", "Timestamp")>
            {toggle_row(t("Zeitstempel hinzufügen", "Add Timestamp"), t("Aufnahmezeit in Screenshots zeichnen", "Draw the capture time onto screenshots"), "screenshotTimestampEnabled")}
            <Show when=move || ts_on.get()>
                {select_row(t("Position", "Position"), "", "screenshotTimestampPosition", opts(&[
                    ("bottomRight", t("Unten rechts", "Bottom Right")), ("bottomLeft", t("Unten links", "Bottom Left")),
                    ("topRight", t("Oben rechts", "Top Right")), ("topLeft", t("Oben links", "Top Left"))]))}
                {select_row(t("Format", "Format"), "", "screenshotTimestampFormat", opts(&[
                    ("dateTime", t("Datum & Uhrzeit", "Date & Time")), ("date", t("Datum", "Date")), ("time", t("Uhrzeit", "Time")), ("iso", "ISO 8601")]))}
                <ui::Row label=t("Farbe", "Color") hint="">
                    <input type="color" class="h-7 w-10 cursor-pointer rounded border border-black/10 bg-transparent dark:border-white/10"
                        prop:value=move || api::setting_str("screenshotTimestampColor")
                        on:change=move |e| api::set_setting("screenshotTimestampColor", json!(event_target_value(&e).to_uppercase()))/>
                </ui::Row>
                {slider_row(t("Schriftgröße", "Font Size"), "", "screenshotTimestampFontSize", 8.0, 48.0, 1.0, " pt", true)}
            </Show>
        </ui::Card>

        <ui::Card title=t("Verschönern", "Beautify")>
            {toggle_row(t("Jeden Screenshot verschönern", "Beautify Every Screenshot"), t("Nutzt Hintergrund, Abstand, Ecken und Schatten, die zuletzt im Editor eingestellt wurden", "Uses the background, padding, corners and shadow last set in the editor's Beautify panel"), "autoBeautifyScreenshots")}
            <ui::Row label=t("Stil zurücksetzen", "Reset Style") hint=t("Verschönern-Stil auf Standard", "Reset Beautify style to default")>
                <ui::TextButton on_click=move || api::set_setting("beautifyStyle", Value::Null)>{t("Zurücksetzen", "Reset")}</ui::TextButton>
            </ui::Row>
        </ui::Card>

        <ui::Card title=t("Selbstauslöser", "Self-Timer")>
            {stepper_row(t("Standarddauer", "Default Duration"), t("Verzögerung bis zum Auslösen", "Delay before the screenshot fires"), "selfTimerSeconds", 1, 60, " s")}
            {toggle_row(t("Tick-Ton", "Tick Sound"), t("Jede Sekunde des Countdowns einen Ton abspielen", "Play a sound on each second of the countdown"), "selfTimerTickSound")}
            <ui::Row label=t("HUD-Position zurücksetzen", "Reset HUD Position") hint="">
                <ui::TextButton on_click=move || api::set_setting("selfTimerHudPosition", Value::Null)>{t("Zurücksetzen", "Reset")}</ui::TextButton>
            </ui::Row>
        </ui::Card>

        <ui::Card title=t("Aufnahme-Vorgaben", "Capture Presets")>
            {toggle_row(t("Aufnahme-Vorgaben", "Capture Presets"), t("Bereichsauswahl auf Seitenverhältnisse oder feste Größen beschränken", "Constrain area capture to preset aspect ratios or fixed sizes"), "capturePresetsEnabled")}
            <Show when=move || presets_on.get()>
                <Presets/>
            </Show>
        </ui::Card>
    }
}

#[component]
fn Presets() -> impl IntoView {
    let kind = RwSignal::new("ratio".to_string());
    let w = RwSignal::new(String::new());
    let h = RwSignal::new(String::new());
    let err = RwSignal::new(false);
    let default_opts = Signal::derive(move || {
        let hidden = str_list("hiddenBuiltinPresets");
        let mut o = vec![("freeform".to_string(), preset_label("freeform"))];
        o.extend(BUILTIN_PRESETS.iter().filter(|(v, _)| !hidden.iter().any(|x| x == v)).map(|(v, _)| (v.to_string(), preset_label(v))));
        o.extend(str_list("customCapturePresets").into_iter().map(|p| (p.clone(), preset_label(&p))));
        o
    });
    let add = move || {
        let (a, b) = (w.get_untracked().trim().parse::<u32>(), h.get_untracked().trim().parse::<u32>());
        let (Ok(a), Ok(b)) = (a, b) else { return err.set(true) };
        if a == 0 || b == 0 || (kind.get_untracked() == "fixed" && (a > 16384 || b > 16384)) {
            return err.set(true);
        }
        let p = if kind.get_untracked() == "fixed" { format!("{a}x{b}") } else { format!("{a}:{b}") };
        let mut list = str_list("customCapturePresets");
        if !list.contains(&p) && !BUILTIN_PRESETS.iter().any(|(v, _)| *v == p) {
            list.push(p);
            api::set_setting("customCapturePresets", json!(list));
        }
        err.set(false);
        w.set(String::new());
        h.set(String::new());
    };
    view! {
        <ui::Row label=t("Standard-Vorgabe", "Default Preset") hint=t("Beim Start einer Bereichsauswahl", "Used when an area selection starts")>
            {move || view! { <ui::Select value=ss("capturePreset") options=default_opts.get() on_change=move |v: String| api::set_setting("capturePreset", json!(v))/> }}
        </ui::Row>
        {BUILTIN_PRESETS.iter().map(|(v, _)| {
            let v: &'static str = v;
            let visible = Signal::derive(move || !str_list("hiddenBuiltinPresets").iter().any(|x| x == v));
            view! {
                <ui::Row label=preset_label(v) hint=t("Eingebaut", "Built-in")>
                    <ui::Toggle checked=visible on_change=move |on| {
                        let mut list = str_list("hiddenBuiltinPresets");
                        list.retain(|x| x != v);
                        if !on { list.push(v.to_string()); }
                        api::set_setting("hiddenBuiltinPresets", json!(list));
                    }/>
                </ui::Row>
            }
        }).collect_view()}
        {move || str_list("customCapturePresets").into_iter().map(|p| {
            let p2 = p.clone();
            view! {
                <ui::Row label=preset_label(&p) hint=t("Eigene", "Custom")>
                    <ui::TextButton variant="ghost" title=t("Entfernen", "Remove").to_string() on_click=move || {
                        let mut list = str_list("customCapturePresets");
                        list.retain(|x| *x != p2);
                        api::set_setting("customCapturePresets", json!(list));
                        if api::setting_str("capturePreset") == p2 { api::set_setting("capturePreset", json!("freeform")); }
                    }><ui::Icon d=ui::TRASH/></ui::TextButton>
                </ui::Row>
            }
        }).collect_view()}
        <div class="flex flex-wrap items-center gap-2 px-3 py-2.5">
            <span class="text-[13px]">{t("Eigene Vorgabe hinzufügen", "Add Custom Preset")}</span>
            <div class="flex-1"></div>
            <ui::Select value=kind options=opts(&[("ratio", t("Seitenverhältnis", "Aspect Ratio")), ("fixed", t("Feste Größe", "Fixed Size"))]) on_change=move |v: String| kind.set(v)/>
            <div class="w-16"><ui::TextInput value=w placeholder=t("B", "W") on_change=move |v: String| w.set(v)/></div>
            <span class="text-black/40">{move || if kind.get() == "fixed" { "×" } else { ":" }}</span>
            <div class="w-16"><ui::TextInput value=h placeholder=t("H", "H") on_change=move |v: String| h.set(v)/></div>
            <ui::TextButton variant="primary" on_click=add>{t("Hinzufügen", "Add")}</ui::TextButton>
        </div>
        <Show when=move || err.get()>
            <div class="px-3 pb-2 text-[11px] text-red-500">{t("Bitte gültige positive Zahlen eingeben (feste Größe ≤ 16384 px).", "Enter valid positive numbers (fixed size ≤ 16384 px).")}</div>
        </Show>
    }
}

pub fn recording() -> impl IntoView {
    let mics = RwSignal::new(Vec::<(String, String)>::new());
    let cams = RwSignal::new(Vec::<(String, String)>::new());
    let load = |cmd: &'static str, sig: RwSignal<Vec<(String, String)>>| {
        spawn_local(async move {
            if let Ok(Value::Array(a)) = api::rpc("prefs", cmd, Value::Null).await {
                sig.set(a.iter().map(|d| (d["id"].as_str().unwrap_or("").to_owned(), d["name"].as_str().unwrap_or("").to_owned())).collect());
            }
        })
    };
    load("microphones", mics);
    load("cameras", cams);
    let mic_on = sb("recordingMicEnabled");
    let device_select = move |key: &'static str, list: RwSignal<Vec<(String, String)>>| {
        move || {
            let mut o = vec![(String::new(), t("Systemstandard", "System Default").to_owned())];
            o.extend(list.get());
            view! { <ui::Select value=ss(key) options=o on_change=move |v: String| api::set_setting(key, if v.is_empty() { Value::Null } else { json!(v) })/> }
        }
    };
    view! {
        <ui::Card title=t("Mauszeiger", "Cursor")>
            {toggle_row(t("Mauszeiger zeigen", "Show Cursor"), "", "showCursor")}
            {toggle_row(t("Klicks hervorheben", "Highlight Clicks"), t("Puls-Effekt beim Mausklick", "Radial pulse on mouse click"), "highlightClicks")}
            {toggle_row(t("Tastendrücke zeigen", "Show Key Presses"), t("Einblendung gedrückter Tasten während der Aufnahme. Braucht Eingabeüberwachung.", "KeyCastr-style overlay of keys while recording. Requires Input Monitoring."), "showKeyPresses")}
            <ui::Row label=t("Position der Tasten-Einblendung", "Key Overlay Position") hint=t("Auf Standard zurücksetzen", "Reset to default")>
                <ui::TextButton on_click=move || api::set_setting("keyPressOverlayOffset", Value::Null)>{t("Zurücksetzen", "Reset")}</ui::TextButton>
            </ui::Row>
            {toggle_row(t("Zeigerglättung", "Cursor Smoothing"), t("Bézier-Interpolation gegen Zittern", "Bézier interpolation to reduce jitter"), "cursorSmoothing")}
        </ui::Card>

        <ui::Card title=t("Verhalten", "Behavior")>
            {toggle_row(t("Countdown zeigen", "Show Countdown"), t("3-Sekunden-Countdown vor der Aufnahme", "3-second countdown before recording"), "showCountdown")}
            {toggle_row(t("Schreibtisch-Symbole ausblenden", "Hide Desktop Icons"), t("Symbole während der Aufnahme mit dem Hintergrundbild abdecken", "Cover desktop icons with the wallpaper while recording"), "hideDesktopIconsWhileRecording")}
            {toggle_row(t("Editor nach der Aufnahme öffnen", "Open Editor After Recording"), t("Schneiden und Effekte hinzufügen vor dem Export", "Edit, trim, and add effects before exporting"), "openEditorAfterRecording")}
            {toggle_row(t("In Zwischenablage kopieren", "Copy to Clipboard"), t("Fertige Aufnahme als MP4- oder GIF-Datei kopieren", "Copy the finished recording as an MP4 or GIF file"), "recordingAutoCopy")}
            {toggle_row(t("Bildschirm abdunkeln", "Dim Screen While Recording"), t("Bereich außerhalb der Aufnahme abdunkeln", "Dim everything outside the recording area"), "dimScreenWhileRecording")}
            {toggle_row(t("Fenster statt Bereich wählen", "Select Window by Default"), t("Aufnahme startet mit Fensterauswahl", "Recording starts with window selection"), "recordingSelectsWindow")}
            {toggle_row(t("Letzten Aufnahmebereich merken", "Remember Last Recording Area"), "", "rememberLastRecordingArea")}
            {toggle_row(t("Bei wenig Speicherplatz warnen", "Warn About Low Disk Space"), "", "warnAboutLowDiskSpace")}
        </ui::Card>

        <ui::Card title=t("Audio", "Audio")>
            {toggle_row(t("Systemton aufnehmen", "Record System Audio"), "", "recordingSystemAudio")}
            {toggle_row(t("Mikrofon aufnehmen", "Record Microphone"), "", "recordingMicEnabled")}
            <Show when=move || mic_on.get()>
                <ui::Row label=t("Mikrofon", "Microphone") hint="">
                    {device_select("recordingMicDeviceId", mics)}
                    <ui::TextButton variant="ghost" title=t("Liste aktualisieren", "Refresh list").to_string() on_click=move || load("microphones", mics)><ui::Icon d=ui::RESTART/></ui::TextButton>
                </ui::Row>
                {toggle_row(t("Bei stummem Mikrofon warnen", "Warn About Muted Microphone"), "", "warnAboutMutedMicrophone")}
            </Show>
        </ui::Card>

        <ui::Card title=t("Kamera-PiP", "Camera PiP")>
            {toggle_row(t("Kamera einblenden", "Show Camera"), t("Kamerabild während der Aufnahme", "Camera overlay while recording"), "cameraEnabled")}
            <ui::Row label=t("Kamera", "Camera") hint="">
                {device_select("cameraDeviceId", cams)}
            </ui::Row>
            {select_row(t("Form", "Shape"), "", "cameraShape", opts(&[("circle", t("Kreis", "Circle")), ("square", t("Quadrat", "Square")), ("landscape", t("Quer", "Landscape")), ("portrait", t("Hoch", "Portrait"))]))}
            {select_row(t("Größe", "Size"), "", "cameraSize", opts(&[("small", t("Klein", "Small")), ("medium", t("Mittel", "Medium")), ("large", t("Groß", "Large"))]))}
            {slider_row(t("Eigene Größe", "Custom Size"), t("0 = Größe oben verwenden", "0 = use size above"), "cameraCustomSizePt", 0.0, 600.0, 10.0, " pt", true)}
            {toggle_row(t("Spiegeln", "Mirror"), "", "cameraMirror")}
            {toggle_row(t("Bei Hover ausblenden", "Fade on Hover"), t("Deckend, bis der Zeiger darüber ist – dann fast durchsichtig. Im Vollbild immer deckend.", "Solid until you hover — then nearly transparent so you can see behind it. Stays solid in fullscreen."), "cameraPipFadeOnHover")}
            {toggle_row(t("Klicks durchlassen", "Click Through PiP"), t("Klicks auf die kleine Kameraansicht gehen an das Fenster dahinter. Nie im Vollbild.", "Clicks pass through to whatever is behind the small camera view. Never applies in fullscreen."), "cameraPipClickThrough")}
        </ui::Card>

        <ui::Card title=t("Format", "Format")>
            {select_row(t("Standard-Aufnahmeformat", "Default Recording Format"), "", "recordingFormat", opts(&[("mp4", "MP4"), ("gif", "GIF")]))}
        </ui::Card>
    }
}

/// Quick-Access-Aktionen (Capso `QuickAccessActionKind`). `quickAccessActions` = Liste sichtbarer IDs in Reihenfolge; `null` = Standard.
const QA_ACTIONS: &[&str] = &["drag", "copy", "save", "upload", "annotate", "pin", "ocr", "translate"];
const QA_DEFAULT: &[&str] = &["drag", "copy", "save", "upload", "annotate", "pin"];

fn qa_label(id: &str) -> &'static str {
    match id {
        "drag" => t("Ziehen", "Drag"),
        "copy" => t("Kopieren", "Copy"),
        "save" => t("Sichern", "Save"),
        "upload" => t("Hochladen", "Upload"),
        "annotate" => t("Bearbeiten", "Annotate"),
        "pin" => t("Anheften", "Pin"),
        "ocr" => t("Text erkennen", "Recognize Text"),
        _ => t("Übersetzen", "Translate"),
    }
}

fn qa_visible() -> Vec<String> {
    match api::setting("quickAccessActions") {
        Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
        _ => QA_DEFAULT.iter().map(|s| s.to_string()).collect(),
    }
}

pub fn quick_access() -> impl IntoView {
    let auto = sb("quickAccessAutoClose");
    let pos_btn = move |v: &'static str, label: &'static str| {
        view! {
            <button
                class=move || format!("flex-1 rounded-lg border px-3 py-2 text-[12px] transition-colors {}",
                    if api::setting_str("quickAccessPosition") == v { "border-blue-500 bg-blue-500/10 text-blue-600 dark:text-blue-300" } else { "border-black/10 hover:bg-black/5 dark:border-white/10 dark:hover:bg-white/10" })
                on:click=move |_| api::set_setting("quickAccessPosition", json!(v))
            >{label}</button>
        }
    };
    let move_action = move |id: String, delta: i32| {
        let mut v = qa_visible();
        if let Some(i) = v.iter().position(|x| *x == id) {
            let j = i as i32 + delta;
            if j >= 0 && (j as usize) < v.len() {
                v.swap(i, j as usize);
                api::set_setting("quickAccessActions", json!(v));
            }
        }
    };
    view! {
        <ui::Card title=t("Position", "Position")>
            <div class="flex gap-2 px-3 py-3">
                {pos_btn("bottomLeft", t("↙ Unten links", "↙ Bottom Left"))}
                {pos_btn("centerScreen", t("◎ Mitte", "◎ Center"))}
                {pos_btn("bottomRight", t("↘ Unten rechts", "↘ Bottom Right"))}
            </div>
        </ui::Card>
        <ui::Card title=t("Automatisch schließen", "Auto-Close")>
            {toggle_row(t("Vorschau automatisch schließen", "Auto-Close Preview"), t("Nach Zeitablauf ausblenden", "Dismiss after timeout"), "quickAccessAutoClose")}
            <Show when=move || auto.get()>
                {num_select_row(t("Schließen nach", "Close After"), "", "quickAccessAutoCloseInterval", &[(5, "5 s"), (10, "10 s"), (15, "15 s"), (30, "30 s")])}
            </Show>
            {toggle_row(t("Nach dem Ziehen schließen", "Close After Dragging"), t("Vorschau schließen, sobald sie in einer anderen App abgelegt wurde", "Close the preview once it's dropped into another app"), "quickAccessCloseAfterDrag")}
        </ui::Card>
        <ui::Card title=t("Aktionen", "Actions")>
            {QA_ACTIONS.iter().map(|id| {
                let id: &'static str = id;
                let on = Signal::derive(move || qa_visible().iter().any(|x| x == id));
                view! {
                    <ui::Row label=qa_label(id) hint=if QA_DEFAULT.contains(&id) { "" } else { t("Sonst im „Mehr“-Menü", "Otherwise in the “More” menu") }>
                        <Show when=move || on.get()>
                            <ui::TextButton variant="ghost" title=t("Nach oben", "Move up").to_string() on_click=move || move_action(id.to_string(), -1)>"↑"</ui::TextButton>
                            <ui::TextButton variant="ghost" title=t("Nach unten", "Move down").to_string() on_click=move || move_action(id.to_string(), 1)>"↓"</ui::TextButton>
                        </Show>
                        <ui::Toggle checked=on on_change=move |v| {
                            let mut list = qa_visible();
                            list.retain(|x| x != id);
                            if v { list.push(id.to_string()); }
                            api::set_setting("quickAccessActions", json!(list));
                        }/>
                    </ui::Row>
                }
            }).collect_view()}
            <ui::Row label=t("Reihenfolge", "Order") hint="">
                <span class="text-[11px] text-black/50 dark:text-white/50">{move || qa_visible().iter().map(|s| qa_label(s)).collect::<Vec<_>>().join(" · ")}</span>
                <ui::TextButton on_click=move || api::set_setting("quickAccessActions", Value::Null)>{t("Standard", "Default")}</ui::TextButton>
            </ui::Row>
        </ui::Card>
    }
}

/// Dateinamen-Vorlage mit Live-Vorschau.
fn template_row(label: &'static str, key: &'static str, kind: &'static str) -> impl IntoView {
    let value = RwSignal::new(api::setting_str(key));
    let preview = RwSignal::new(String::new());
    let refresh = move |tpl: String| {
        spawn_local(async move {
            if let Ok(v) = api::rpc("prefs", "filenamePreview", json!({"template": tpl, "kind": kind})).await {
                preview.set(v.as_str().unwrap_or("").to_owned());
            }
        })
    };
    refresh(value.get_untracked());
    // Format-Wechsel ändert die Endung der Vorschau.
    Effect::new(move |_| {
        let _ = (api::setting_str("screenshotFormat"), api::setting_str("recordingFormat"));
        refresh(value.get_untracked());
    });
    let insert = move |ph: &'static str| {
        let v = format!("{}{ph}", value.get_untracked());
        value.set(v.clone());
        api::set_setting(key, json!(v.clone()));
        refresh(v);
    };
    view! {
        <div class="px-3 py-2.5">
            <div class="mb-1.5 text-[13px]">{label}</div>
            <input type="text"
                class="h-7 w-full rounded-md border border-black/10 bg-white/80 px-2 font-mono text-[12px] outline-none focus:ring-2 focus:ring-blue-500/40 dark:border-white/10 dark:bg-white/10"
                prop:value=move || value.get()
                on:input=move |e| { let v = event_target_value(&e); value.set(v.clone()); refresh(v); }
                on:change=move |e| api::set_setting(key, json!(event_target_value(&e)))/>
            <div class="mt-1.5 flex flex-wrap items-center gap-1">
                {["{timestamp}", "{date}", "{time}", "{source}"].into_iter().filter(|p| kind == "screenshot" || *p != "{source}").map(|p| view! {
                    <button class="rounded bg-black/5 px-1.5 py-0.5 font-mono text-[11px] hover:bg-black/10 dark:bg-white/10 dark:hover:bg-white/15" on:click=move |_| insert(p)>{p}</button>
                }).collect_view()}
                <div class="flex-1"></div>
                <span class="truncate text-[11px] text-black/50 dark:text-white/50">{move || format!("{} {}", t("Vorschau:", "Preview:"), preview.get())}</span>
            </div>
        </div>
    }
}

pub fn export() -> impl IntoView {
    let dir = RwSignal::new(String::new());
    let load_dir = move || {
        spawn_local(async move {
            if let Ok(v) = api::rpc("prefs", "exportDir", Value::Null).await {
                dir.set(v.as_str().unwrap_or("").to_owned());
            }
        })
    };
    load_dir();
    let choose = move || {
        spawn_local(async move {
            if let Ok(Value::String(p)) = api::rpc("prefs", "chooseFolder", json!({"current": dir.get_untracked()})).await {
                api::set_setting("exportLocation", json!(p));
                set_timeout(load_dir, std::time::Duration::from_millis(200));
            }
        })
    };
    let lossy = Signal::derive(move || api::setting_str("screenshotFormat") != "png");
    view! {
        <ui::Card title=t("Qualität", "Quality")>
            {select_row(t("Exportqualität", "Export Quality"), t("Gilt für Video- und GIF-Exporte", "Applies to video and GIF exports"), "exportQuality",
                opts(&[("maximum", t("Maximal", "Maximum")), ("social", t("Social Media", "Social")), ("web", t("Web", "Web"))]))}
            {select_row(t("Screenshot-Format", "Screenshot Format"), t("HEIC erzeugt die kleinsten Dateien; PNG ist verlustfrei", "HEIC produces the smallest files; PNG is lossless"), "screenshotFormat",
                opts(&[("png", "PNG"), ("jpeg", "JPEG"), ("heic", "HEIC")]))}
            <Show when=move || lossy.get()>
                {slider_row(t("Screenshot-Qualität", "Screenshot Quality"), "", "screenshotQuality", 30.0, 100.0, 1.0, " %", true)}
            </Show>
        </ui::Card>

        <ui::Card title=t("Speicherort", "Save Location")>
            <ui::Row label=t("Exportieren nach", "Export To") hint="">
                <span class="max-w-[220px] truncate text-[12px] text-black/55 dark:text-white/55" title=move || dir.get()>{move || dir.get()}</span>
                <ui::TextButton on_click=choose>{t("Wählen …", "Choose…")}</ui::TextButton>
                <ui::TextButton variant="ghost" title=t("Im Finder zeigen", "Show in Finder").to_string() on_click=move || api::fire("prefs", "revealExportDir", Value::Null)><ui::Icon d=ui::FOLDER/></ui::TextButton>
            </ui::Row>
            <Show when=move || !api::setting_str("exportLocation").is_empty()>
                <ui::Row label=t("Standard verwenden", "Use Default") hint=t("macOS-Screenshot-Ort bzw. Schreibtisch", "macOS screenshot location or Desktop")>
                    <ui::TextButton on_click=move || { api::set_setting("exportLocation", json!("")); set_timeout(load_dir, std::time::Duration::from_millis(200)); }>{t("Zurücksetzen", "Reset")}</ui::TextButton>
                </ui::Row>
            </Show>
            {toggle_row(t("Monatsordner", "Monthly Screenshot Folders"), t("Screenshots in Unterordnern jjjj-MM speichern", "Save screenshots into yyyy-MM subfolders"), "screenshotMonthlyFolders")}
        </ui::Card>

        <ui::Card title=t("Dateinamen", "File Names")>
            {template_row(t("Screenshot-Dateiname", "Screenshot File Name"), "screenshotFilenameTemplate", "screenshot")}
            {template_row(t("Aufnahme-Dateiname", "Recording File Name"), "recordingFilenameTemplate", "recording")}
        </ui::Card>

        <ui::Card title=t("Geöffnete Bilder", "Opened Images")>
            {select_row(t("Beim Sichern eines geöffneten Bildes", "When Saving an Opened Image"), t("Gilt für Bilder aus dem Finder oder dem Öffnen-Dialog", "Applies to images opened from Finder or the Open Image panel"), "openedImageSaveBehavior",
                opts(&[("ask", t("Jedes Mal fragen", "Ask Every Time")), ("overwrite", t("Original überschreiben", "Overwrite Original")), ("copy", t("Neue Kopie sichern", "Save New Copy"))]))}
        </ui::Card>
    }
}

const LANGS: &[(&str, &str, &str)] = &[
    ("en", "Englisch", "English"),
    ("de", "Deutsch", "German"),
    ("fr", "Französisch", "French"),
    ("es", "Spanisch", "Spanish"),
    ("it", "Italienisch", "Italian"),
    ("pt", "Portugiesisch", "Portuguese"),
    ("nl", "Niederländisch", "Dutch"),
    ("pl", "Polnisch", "Polish"),
    ("sv", "Schwedisch", "Swedish"),
    ("da", "Dänisch", "Danish"),
    ("fi", "Finnisch", "Finnish"),
    ("no", "Norwegisch", "Norwegian"),
    ("cs", "Tschechisch", "Czech"),
    ("tr", "Türkisch", "Turkish"),
    ("ru", "Russisch", "Russian"),
    ("uk", "Ukrainisch", "Ukrainian"),
    ("ar", "Arabisch", "Arabic"),
    ("hi", "Hindi", "Hindi"),
    ("ja", "Japanisch", "Japanese"),
    ("ko", "Koreanisch", "Korean"),
    ("zh-Hans", "Chinesisch (vereinfacht)", "Chinese (Simplified)"),
    ("zh-Hant", "Chinesisch (traditionell)", "Chinese (Traditional)"),
];

const OCR_LANGS: &[(&str, &str)] = &[
    ("en-US", "English"),
    ("de-DE", "Deutsch"),
    ("fr-FR", "Français"),
    ("it-IT", "Italiano"),
    ("es-ES", "Español"),
    ("pt-BR", "Português"),
    ("nl-NL", "Nederlands"),
    ("ru-RU", "Русский"),
    ("uk-UA", "Українська"),
    ("ja-JP", "日本語"),
    ("ko-KR", "한국어"),
    ("zh-Hans", "简体中文"),
    ("zh-Hant", "繁體中文"),
];

pub fn ocr() -> impl IntoView {
    let provider = ss("translationProvider");
    let delay = Signal::derive(move || api::setting_str("translationAutoDismiss") == "afterDelay");
    let test_msg = RwSignal::new(None::<(bool, String)>);
    // Anbieter-Einstellungen des aktuellen Anbieters (Modell/Endpunkt je Anbieter, API-Schlüssel im Schlüsselbund).
    let info = RwSignal::new(Value::Null);
    let key_input = RwSignal::new(String::new());
    Effect::new(move |_| {
        let id = provider.get();
        test_msg.set(None);
        spawn_local(async move {
            if let Ok(v) = api::rpc("translate", "providerSettings", json!({"provider": id})).await {
                key_input.set(v["apiKey"].as_str().unwrap_or("").to_owned());
                info.set(v);
            }
        });
    });
    let flag = move |k: &'static str| Signal::derive(move || info.with(|i| i[k].as_bool().unwrap_or(false)));
    let (needs_key, has_endpoint, has_model) = (flag("requiresApiKey"), flag("supportsEndpoint"), flag("supportsModel"));
    // Wert eines anbieterspezifischen Schlüssels; ohne gespeicherten Wert der Standard des Anbieters.
    let field = move |key_field: &'static str, default_field: &'static str| {
        Signal::derive(move || {
            let (key, default) = info.with(|i| (i[key_field].as_str().unwrap_or("").to_owned(), i[default_field].as_str().unwrap_or("").to_owned()));
            if key.is_empty() { return String::new(); }
            api::setting(&key).as_str().map(str::to_owned).unwrap_or(default)
        })
    };
    let set_field = move |key_field: &'static str, v: String| {
        let key = info.with_untracked(|i| i[key_field].as_str().unwrap_or("").to_owned());
        if !key.is_empty() { api::set_setting(&key, json!(v.trim())) }
    };
    let save_key = move |v: String| {
        key_input.set(v.clone());
        spawn_local(async move {
            test_msg.set(Some(match api::rpc("translate", "setApiKey", json!({"value": v})).await {
                Ok(m) => (true, m.as_str().unwrap_or("").to_owned()),
                Err(e) => (false, e),
            }));
        });
    };
    let test = move || {
        test_msg.set(Some((true, t("Teste …", "Testing…").to_owned())));
        spawn_local(async move {
            let r = api::rpc("translate", "testConnection", json!({"apiKey": key_input.get_untracked()})).await;
            test_msg.set(Some(match r {
                Ok(_) => (true, t("Verbindung erfolgreich", "Connection succeeded").to_owned()),
                Err(e) => (false, e),
            }));
        });
    };
    let langs: Vec<(String, String)> = LANGS.iter().map(|(c, de, en)| (c.to_string(), t(de, en).to_string())).collect();
    let mut ocr_langs = vec![(String::new(), t("Automatisch", "Auto").to_string())];
    ocr_langs.extend(OCR_LANGS.iter().map(|(c, l)| (c.to_string(), l.to_string())));
    view! {
        <ui::Card title=t("Texterkennung", "Text Recognition")>
            {toggle_row(t("Zeilenumbrüche behalten", "Keep Line Breaks"), t("Ursprüngliche Zeilenstruktur erhalten", "Preserve original line structure"), "ocrKeepLineBreaks")}
            {toggle_row(t("Links erkennen", "Detect Links"), t("URLs automatisch erkennen und verlinken", "Auto-detect and linkify URLs"), "ocrDetectLinks")}
            {select_row_ext(t("Hauptsprache", "Primary Language"), t("Automatisch erkennen, wenn nicht gesetzt", "Auto-detect if unset"), "ocrPrimaryLanguage", ocr_langs, true)}
        </ui::Card>

        <ui::Card title=t("Übersetzung", "Translation")>
            {select_row(t("Zielsprache", "Target Language"), t("Sprache, in die übersetzt wird", "Language to translate text into"), "translationTargetLanguage", langs)}
            {select_row(t("Anbieter", "Provider"), "", "translationProvider", opts(&[
                ("apple", t("Apple (auf dem Gerät)", "Apple (On-Device)")),
                ("openAICompatible", "OpenAI-kompatibel / OpenAI-compatible"),
                ("deepSeek", "DeepSeek"),
                ("openRouter", "OpenRouter"),
                ("deepL", "DeepL"),
                ("googleCloud", "Google Cloud Translation"),
                ("custom", t("Eigener Endpunkt", "Custom Endpoint")),
            ]))}
            <Show when=move || provider.get() == "apple">
                <div class="px-3 pt-2">
                    {banner("warn", t(
                        "Apple Translation (auf dem Gerät) ist nur aus Swift nutzbar und steht in nrzCapture nicht zur Verfügung. Wähle einen Cloud-Anbieter und trage seinen API-Schlüssel ein.",
                        "Apple Translation (on-device) is Swift-only and isn't available in nrzCapture. Choose a cloud provider and enter its API key.").to_owned())}
                </div>
            </Show>
            <Show when=move || needs_key.get()>
                <ui::Row label=t("API-Schlüssel", "API Key") hint=t("Im Schlüsselbund gespeichert", "Stored in Keychain")>
                    <div class="w-60">
                        <ui::TextInput value=key_input placeholder="sk-…" password=true on_change=save_key/>
                    </div>
                </ui::Row>
                <Show when=move || has_endpoint.get()>
                    <ui::Row label=t("Endpunkt", "Endpoint") hint=t("Leer lassen für den Standard des Anbieters", "Leave empty to use the provider default")>
                        <div class="w-64">
                            <ui::TextInput value=field("endpointKey", "defaultEndpoint") placeholder="https://…" on_change=move |v| set_field("endpointKey", v)/>
                        </div>
                    </ui::Row>
                </Show>
                <Show when=move || has_model.get()>
                    <ui::Row label=t("Modell", "Model") hint=t("Leer lassen für den Standard des Anbieters", "Leave empty to use the provider default")>
                        <div class="w-60">
                            <ui::TextInput value=field("modelKey", "defaultModel") placeholder="" on_change=move |v| set_field("modelKey", v)/>
                        </div>
                    </ui::Row>
                </Show>
                <ui::Row label=t("Verbindung", "Connection") hint=t("Aktuelle Anbieter-Einstellungen prüfen", "Verify the current provider settings")>
                    <ui::TextButton on_click=test>{t("Verbindung testen", "Test Connection")}</ui::TextButton>
                </ui::Row>
            </Show>
            {move || test_msg.get().map(|(ok, m)| view! {
                <div class=if ok { "px-4 py-2 text-[12px] text-green-600" } else { "px-4 py-2 text-[12px] text-red-500" }>{m}</div>
            })}
            {toggle_row(t("Übersetzung automatisch kopieren", "Auto-Copy Translation"), t("Ergebnis automatisch in die Zwischenablage", "Copy result to clipboard automatically"), "translationAutoCopy")}
            {toggle_row(t("Originaltext zeigen", "Show Original Text"), t("Quelltext neben der Übersetzung anzeigen", "Display source text alongside translation"), "translationShowOriginal")}
            {select_row(t("Position der Karte", "Card Position"), "", "translationCardPosition", opts(&[
                ("belowSelection", t("Unter der Auswahl", "Below Selection")), ("centerScreen", t("Bildschirmmitte", "Center of Screen")), ("rememberLast", t("Letzte Position", "Remember Last"))]))}
            {select_row(t("Automatisch schließen", "Auto-Dismiss"), t("Wann die Übersetzungskarte schließt", "When to close the translation panel"), "translationAutoDismiss", opts(&[
                ("manual", t("Manuell", "Manual")), ("clickOutside", t("Klick daneben", "Click outside")), ("afterDelay", t("Nach Verzögerung", "After delay"))]))}
            <Show when=move || delay.get()>
                {slider_row(t("Verzögerung", "Delay"), "", "translationAutoDismissDelay", 2.0, 60.0, 1.0, " s", true)}
            </Show>
        </ui::Card>

        <ui::Card title=t("Einführung", "Onboarding")>
            <ui::Row label=t("Einführungen erneut zeigen", "Show Onboarding Again") hint=t("OCR- und Übersetzungs-Hinweise beim nächsten Mal anzeigen", "Show OCR and translation tips next time")>
                <ui::TextButton on_click=move || { api::set_setting("ocrOnboardingShown", json!(false)); api::set_setting("translationOnboardingShown", json!(false)); }>{t("Zurücksetzen", "Reset")}</ui::TextButton>
            </ui::Row>
        </ui::Card>
    }
}
