# nrzCapture – Architektur (Rust-Port von Capso)

Ziel: nrzCapture verhält sich funktional 1:1 wie [Capso](https://github.com/lzhgus/Capso) (Swift/SwiftUI, BSL 1.1), aber vollständig in Rust:
**Tauri 2** (Backend, native macOS-APIs über `objc2-*`-Crates bzw. System-CLIs) + **Leptos 0.8 CSR** (UI im WebView, Tailwind 4 über Trunk).
Capso-Quelltext als Referenz: lokal geklont unter `$CAPSO` (Pfad steht im Auftrag), dort `App/Sources/**` und `Packages/*Kit/**`.

UI-Texte zweisprachig Deutsch/Englisch: Frontend `api::t("Deutsch", "English")`, Backend `core::t("Deutsch", "English")`.
Produktname überall **nrzCapture** (nicht Capso). URL-Schema `nrzcapture://<aktion>`.

## Ordner

```
src-tauri/src/          Backend (ein Modul = eine Datei)
  main.rs               Builder, Plugins, einziger IPC-Befehl `rpc`, Updater          (Integrator)
  core.rs               Shell, Pfade, Sprache, Fenster-Helfer, Zwischenablage, Rechte  (Integrator; nur ergänzen)
  settings.rs           Alle Einstellungen (Port von AppSettings)                      (Integrator; nur ergänzen)
  actions.rs            Aktions-IDs, Menüleiste, globale Kürzel                        (Integrator)
  capture.rs scroll.rs  Screenshots, Overlay-Backend, Selbstauslöser, Scroll-Aufnahme  → Agent CAPTURE
  quick.rs pin.rs       Quick Access, Toasts, Pins                                     → Agent QUICK
  editor.rs             Annotations-Editor (Speichern/Export/Zwischenablage)           → Agent EDITOR
  record.rs camera.rs   Bildschirmaufnahme, Kamera-PiP, Klick-/Tasten-Overlays         → Agent RECORD
  receditor.rs          Aufnahme-Editor (Trim, Zoom, Cursor, Hintergrund, Export)      → Agent RECEDITOR
  ocr.rs translate.rs   Vision-OCR, Übersetzung                                        → Agent OCR
  history.rs share.rs   Verlauf, Cloud-Freigabe (R2/S3/OSS/COS)                         → Agent HISTORY
  prefs.rs              Einstellungsfenster, Über, Schreibtisch-Symbole, URLs           → Agent PREFS
src/                    Frontend (ein WASM-Bundle, Route = Fenster)
  main.rs api.rs ui.rs  Router, Backend-Brücke, gemeinsame Komponenten                 (Integrator; ui.rs nur am Ende ergänzen)
  overlay.rs            Auswahl-Overlay + All-in-One-HUD + Selbstauslöser-/Scroll-HUD  → CAPTURE (darf Unterordner overlay/ anlegen)
  quick.rs pin.rs       Quick-Access-Karten, Toast, Pin-Fenster                        → QUICK
  editor.rs (+editor/)  Annotations-Editor                                             → EDITOR
  record.rs camera.rs   Aufnahme-Leiste, Steuerung, Countdown, Rahmen, PiP, Tasten     → RECORD
  receditor.rs (+receditor/)                                                           → RECEDITOR
  ocr.rs translate.rs   Visual-OCR-Overlay, Übersetzungskarte, Eingabefenster          → OCR
  history.rs            Verlaufsfenster                                                → HISTORY
  prefs.rs (+prefs/)    Einstellungen (alle Tabs), Über                                → PREFS
```

**Regel für parallele Arbeit:** Jeder Agent ändert nur *seine* Dateien. Ausnahmen (klein halten!):
`src-tauri/Cargo.toml` / `Cargo.toml` (Abhängigkeiten ergänzen), `settings.rs` (neue Felder *anhängen*, oder besser `settings::get_extra/set_extra`),
`ui.rs` / `core.rs` (neue Helfer *am Dateiende* anhängen), `src-tauri/Info.plist` (Usage-Descriptions), `tauri.conf.json`.
Öffentliche Funktionssignaturen der Module (siehe unten) **nicht ändern** – andere Module rufen sie auf.

## IPC

Ein einziger Tauri-Befehl: `rpc(m, c, a)`. `main.rs` verteilt an `<modul>::dispatch(app, window, cmd, args) -> Result<Value, String>`.
`dispatch` läuft auf einem Blocking-Thread (darf warten). Neue Befehle = neuer `match`-Arm im eigenen Modul, sonst nichts.

Frontend (`src/api.rs`):
- `api::rpc("ocr", "recognize", json!({"path": p})).await -> Result<Value, String>`, `api::fire(m, c, a)` (ohne Warten)
- `api::on("event", |payload: Value| …)` – Backend sendet mit `app.emit_to(label, "event", payload)` / `app.emit(...)`
- `api::settings()` reaktives `RwSignal<Value>`; `api::setting_bool("key")`, `api::set_setting("key", json!(…))` (camelCase-Schlüssel)
- `api::window_data().await` – Startdaten, die das Backend beim Öffnen per `core::window(…, data)` hinterlegt hat; bei erneutem Öffnen eines bestehenden Fensters kommt zusätzlich das Event `window-data`.
- `api::asset_url(path)` für `<img>`/`<video>` lokaler Dateien.
- Inline-Annotation im Overlay: `crate::editor::InlineAnnotator` (Signatur in `src/editor.rs`, fest) – EDITOR implementiert, CAPTURE bettet ein.
- `api::route()`; Router in `src/main.rs` ruft `<modul>::route(r) -> Option<AnyView>` jedes Moduls auf.

Fenster öffnen (Backend): `core::window(app, label, route, core::Win{..}, data)`; `Win::default()` = randloses, transparentes, schwebendes Overlay; `Win::app(title, w, h)` = normales Fenster mit Ampel-Knöpfen über dem Inhalt (Inhalt oben ~28 px Platz lassen, `class="app-window"` auf das Wurzelelement für den Hintergrund). `core::set_level`, `core::cover_monitor`, `core::exclude_from_capture`, `core::screens(app)`.
Alle Fenster haben die Capability `core:default` (Events, Fenster-API im JS über `window.__TAURI__`).

## Koordinaten

Global logisch (Punkte), Ursprung oben links des Hauptbildschirms – wie Tauri. `capture::Region { x, y, width, height, screen, window_id, fullscreen }`.
`screencapture -R x,y,w,h` nutzt dieselben Koordinaten.

## Modul-Schnittstellen (fest)

```rust
// capture.rs
pub enum Intent { Save, Clipboard, Annotate, Pin, Share, Ocr, Translate }
pub enum Purpose { Screenshot, Recording, Ocr, Translate, Scroll }
pub fn area(app, Intent); fullscreen(app); window(app); all_in_one(app); last_area(app); self_timer(app);
pub fn select_region(app, Purpose) -> Option<Region>      // blockiert, zeigt Overlay
pub fn grab(&Region, cursor: bool) -> Option<PathBuf>       // PNG im Temp-Ordner
pub fn deliver(app, path, Intent, source: &str, Option<Region>) // gemeinsame Nachbearbeitung jeder Bildaufnahme
// scroll.rs
pub fn start(app)
// quick.rs
pub struct QuickItem { path, kind: "image"|"video"|"gif", saved: bool }
pub fn show(app, QuickItem); hide_for_capture(app, bool); save_all(app); close_all(app); reopen_last(app); toast(app, &str)
// pin.rs
pub fn pin_image(app, &Path, Option<[f64;4]>); pin_clipboard(app); toggle_all(app); all_hidden() -> bool
// editor.rs
pub fn open(app, &Path); open_clipboard(app); open_file_dialog(app)
// ocr.rs
pub struct TextRegion { text, bbox: [f64;4] /*normiert, oben links*/, confidence }
pub fn capture_text(app); recognize(&Path) -> Result<Vec<TextRegion>, String>; recognize_and_copy(app, &Path)
// translate.rs
pub fn capture_and_translate(app); selected_text(app); typed_text(app); translate_image(app, &Path, Option<Region>)
// record.rs / camera.rs / receditor.rs
pub fn start(app, fullscreen: bool); stop(app); is_recording() -> bool; shutdown(app)
pub fn camera::toggle(app); camera::is_open() -> bool
pub fn receditor::open(app, &Path)
// history.rs / share.rs
pub fn history::add(app, &Path, kind: &str); history::open(app); history::cleanup(app)
pub fn share::upload(app, &Path) -> Result<String, String>; share::upload_and_copy(app, &Path)
// prefs.rs
pub fn open(app, Option<&str>); about(app); toggle_desktop_icons(app); desktop_icons_hidden() -> bool;
pub fn restore_desktop_icons(); apply_system_shortcuts(bool); handle_url(app, &str)
```
Weitere `pub fn` dürfen ergänzt werden. Nach Zustandsänderungen, die das Menü betreffen (Pins, Kamera, Aufnahme, Symbole): `crate::actions::refresh_menu(app)`.

## Bauen & Prüfen

```sh
export PATH=$HOME/.cargo/bin:$PATH
cargo check -p nrzcapture                                        # Backend
cargo check -p nrzcapture-ui --target wasm32-unknown-unknown     # Frontend
trunk build                                                      # Frontend inkl. Tailwind
cargo test -p nrzcapture                                         # reine Logik-Tests (Geometrie, Stitcher, Signaturen …)
```
Bei SIGKILL von Tailwind: `codesign -f -s - ~/Library/Caches/dev.trunkrs.trunk/tailwindcss-*/tailwindcss`.
Eigenes Target-Verzeichnis pro Agent ist ok (`CARGO_TARGET_DIR`).
