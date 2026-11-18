//! Einstellungs-Panel rechts (Port von Capso `EditorSettingsPanel`): Hintergrund, Seitenverhältnis, Effekte
//! (Auswahl bearbeiten, Zooms vorschlagen), Cursor, Export-Format/Qualität.
use super::{model::*, timeline::AddEffectMenu, Ed, Sel};
use crate::{
    api::{self, t},
    ui::{Select, TextButton, Toggle},
};
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};
use std::time::Duration;

fn label(s: &'static str) -> impl IntoView {
    view! { <div class="px-1 pb-1.5 pt-4 text-[11px] font-semibold uppercase tracking-wide text-black/45 dark:text-white/45">{s}</div> }
}

fn card(children: impl IntoView) -> impl IntoView {
    view! {
        <div class="divide-y divide-black/5 rounded-xl border border-black/5 bg-white/70 shadow-sm dark:divide-white/10 dark:border-white/10 dark:bg-white/5">
            {children}
        </div>
    }
}

fn toggle_row(name: &'static str, get: impl Fn() -> bool + Send + Sync + 'static, set: impl Fn(bool) + 'static) -> impl IntoView {
    view! {
        <div class="flex min-h-[38px] items-center justify-between gap-3 px-3.5 py-2">
            <span>{name}</span>
            <Toggle checked=Signal::derive(get) on_change=set />
        </div>
    }
}

fn picker_row(name: &'static str, value: impl Fn() -> String + Send + Sync + 'static, options: Vec<(&'static str, &'static str)>, set: impl Fn(String) + 'static) -> impl IntoView {
    view! {
        <div class="flex min-h-[38px] items-center justify-between gap-3 px-3.5 py-2">
            <span>{name}</span>
            <Select value=Signal::derive(value) options=options.into_iter().map(|(v, l)| (v.to_owned(), l.to_owned())).collect() on_change=set />
        </div>
    }
}

/// Schieberegler über volle Breite (Capso `verticalSliderRow`).
fn slider_row(
    name: &'static str,
    value: impl Fn() -> f64 + Send + Sync + Copy + 'static,
    min: f64,
    max: f64,
    step: f64,
    unit: &'static str,
    precision: usize,
    set: impl Fn(f64) + 'static,
) -> impl IntoView {
    view! {
        <div class="flex flex-col gap-1 px-3.5 py-2">
            <div class="flex items-center justify-between">
                <span>{name}</span>
                <span class="font-mono text-[11px] text-black/45 dark:text-white/45">{move || format!("{:.*}{unit}", precision, value())}</span>
            </div>
            <input type="range" class="w-full accent-blue-500" min=min max=max step=step
                prop:value=move || value().to_string()
                on:input=move |e| set(event_target_value(&e).parse().unwrap_or(min)) />
        </div>
    }
}

fn color_input(value: impl Fn() -> CodableColor + Send + Sync + 'static, set: impl Fn(CodableColor) + 'static) -> impl IntoView {
    view! {
        <input type="color" class="h-6 w-9 cursor-pointer rounded border border-black/10 bg-transparent p-0 dark:border-white/15"
            prop:value=move || value().to_hex()
            on:input=move |e| if let Some(c) = CodableColor::from_hex(&event_target_value(&e)) { set(c) } />
    }
}

#[component]
pub fn SettingsPanel(ed: Ed) -> impl IntoView {
    let p = ed.project;
    let bg = move || p.with(|p| p.background_style.clone());
    let set_bg = move |f: Box<dyn FnOnce(&mut BackgroundStyle)>| p.update(|p| f(&mut p.background_style));

    view! {
        <div class="flex w-[280px] shrink-0 flex-col overflow-y-auto px-3 pb-6">
            <div class="pt-1 text-[15px] font-semibold">{t("Effekte", "Effects")}</div>

            // ───────── Hintergrund ─────────
            {label(t("Hintergrund", "Background"))}
            {card(view! {
                {toggle_row(t("Aktiviert", "Enabled"), move || bg().enabled, move |v| set_bg(Box::new(move |b| b.enabled = v)))}
                <Show when=move || bg().enabled>
                    {picker_row(t("Stil", "Style"),
                        move || serde_json::to_value(bg().color_type).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default(),
                        vec![("solid", t("Einfarbig", "Solid")), ("gradient", t("Verlauf", "Gradient")), ("liquidGlass", "Liquid Glass"), ("image", t("Bild", "Image"))],
                        move |v| if let Ok(ct) = serde_json::from_value::<BackgroundColorType>(Value::String(v)) {
                            set_bg(Box::new(move |b| b.color_type = ct));
                            if ct == BackgroundColorType::Image && bg().image_path.is_none() { choose_image(ed) }
                        })}
                    <Show when=move || bg().color_type == BackgroundColorType::Solid>
                        <ColorPresets ed=ed />
                    </Show>
                    <Show when=move || bg().color_type == BackgroundColorType::Gradient>
                        <div class="flex items-center justify-between gap-3 px-3.5 py-2">
                            <span>{t("Farben", "Colors")}</span>
                            <div class="flex items-center gap-2">
                                {color_input(move || bg().gradient_from, move |c| set_bg(Box::new(move |b| b.gradient_from = c)))}
                                <span class="text-black/30 dark:text-white/30">"→"</span>
                                {color_input(move || bg().gradient_to, move |c| set_bg(Box::new(move |b| b.gradient_to = c)))}
                            </div>
                        </div>
                        {slider_row(t("Winkel", "Angle"), move || bg().gradient_angle, 0.0, 360.0, 1.0, "°", 0, move |v| set_bg(Box::new(move |b| b.gradient_angle = v)))}
                    </Show>
                    <Show when=move || bg().color_type == BackgroundColorType::Image>
                        <div class="flex items-center justify-between gap-3 px-3.5 py-2">
                            <span class="min-w-0 truncate text-[12px] text-black/50 dark:text-white/50">
                                {move || bg().image_path.and_then(|p| p.rsplit('/').next().map(str::to_owned)).unwrap_or_else(|| t("Kein Bild", "No image").into())}
                            </span>
                            <TextButton on_click=move || choose_image(ed)>{t("Wählen …", "Choose…")}</TextButton>
                        </div>
                    </Show>
                    {slider_row(t("Abstand", "Padding"), move || bg().padding, 0.0, 80.0, 1.0, " px", 0, move |v| set_bg(Box::new(move |b| b.padding = v)))}
                    {slider_row(t("Eckenradius", "Corner Radius"), move || bg().corner_radius, 0.0, BackgroundStyle::MAX_CORNER_RADIUS, 1.0, " px", 0, move |v| set_bg(Box::new(move |b| b.corner_radius = v)))}
                    {toggle_row(t("Schatten", "Shadow"), move || bg().shadow_enabled, move |v| set_bg(Box::new(move |b| b.shadow_enabled = v)))}
                    <Show when=move || bg().shadow_enabled>
                        {slider_row(t("Schattenradius", "Shadow Radius"), move || bg().shadow_radius, 0.0, 30.0, 1.0, " px", 0, move |v| set_bg(Box::new(move |b| b.shadow_radius = v)))}
                        {slider_row(t("Schattendeckkraft", "Shadow Opacity"), move || bg().shadow_opacity, 0.0, 1.0, 0.05, "", 2, move |v| set_bg(Box::new(move |b| b.shadow_opacity = v)))}
                    </Show>
                </Show>
                {picker_row(t("Seitenverhältnis", "Aspect Ratio"), move || p.with(|p| p.aspect_ratio.clone()),
                    vec![("auto", t("Original", "Original")), ("16:9", "16:9"), ("9:16", "9:16"), ("1:1", "1:1"), ("4:3", "4:3"), ("3:4", "3:4")],
                    move |v| p.update(|p| p.aspect_ratio = v))}
            })}

            // ───────── Effekte ─────────
            <EffectsSection ed=ed />

            // ───────── Cursor ─────────
            {label(t("Cursor", "Cursor"))}
            <Show when=move || ed.has_tel.get() fallback=move || card(view! {
                <div class="px-3.5 py-2.5 text-[12px] text-black/50 dark:text-white/50">
                    {t("Für diese Aufnahme gibt es keine Cursor-Daten.", "No cursor data was recorded for this video.")}
                </div>
            })>
                {card(view! {
                    {toggle_row(t("Cursor anzeigen", "Show Cursor"), move || p.with(|p| p.shows_cursor), move |v| p.update(|p| p.shows_cursor = v))}
                    {slider_row(t("Größe", "Size"), move || p.with(|p| p.cursor_scale), 0.5, 3.0, 0.1, "×", 1, move |v| p.update(|p| p.cursor_scale = v))}
                    {toggle_row(t("Glättung", "Smoothing"), move || p.with(|p| p.cursor_smoothing.enabled), move |v| p.update(|p| p.cursor_smoothing.enabled = v))}
                    <Show when=move || p.with(|p| p.cursor_smoothing.enabled)>
                        {picker_row(t("Stil", "Style"),
                            move || serde_json::to_value(p.with(|p| p.cursor_smoothing.preset())).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default(),
                            vec![("snappy", t("Direkt", "Snappy")), ("smooth", t("Weich", "Smooth")), ("floaty", t("Schwebend", "Floaty"))],
                            move |v| if let Ok(pr) = serde_json::from_value::<CursorSmoothingPreset>(Value::String(v)) { p.update(|p| p.cursor_smoothing = pr.config()) })}
                    </Show>
                })}
                <div class="h-2" />
                {card(toggle_row(t("Bei Stillstand ausblenden", "Hide when idle"), move || p.with(|p| p.hides_idle_cursor), move |v| p.update(|p| p.hides_idle_cursor = v)))}
            </Show>

            // ───────── Export ─────────
            {label(t("Export", "Export"))}
            {card(view! {
                {picker_row(t("Format", "Format"), move || ed.format.get(), vec![("mp4", "MP4"), ("gif", "GIF")], move |v| {
                    api::set_setting("recordingFormat", json!(v));
                    ed.format.set(v);
                })}
                {picker_row(t("Qualität", "Quality"), move || ed.quality.get(),
                    vec![("maximum", t("Maximal", "Maximum")), ("social", t("Social (1080p)", "Social (1080p)")), ("web", t("Web (720p)", "Web (720p)"))],
                    move |v| {
                        api::set_setting("exportQuality", json!(v));
                        ed.quality.set(v);
                    })}
            })}
        </div>
    }
}

fn choose_image(ed: Ed) {
    spawn_local(async move {
        if let Ok(Value::String(path)) = api::rpc("receditor", "chooseImage", Value::Null).await {
            ed.project.update(|p| {
                p.background_style.image_path = Some(path);
                p.background_style.color_type = BackgroundColorType::Image;
            });
        }
    });
}

/// 2×4-Raster: 7 Capso-Vorgaben + eigene Farbe.
#[component]
fn ColorPresets(ed: Ed) -> impl IntoView {
    let p = ed.project;
    let current = move || p.with(|p| p.background_style.solid_color);
    let names = ["Ink", "Stone", "Mist", "Sand", "Dusk", "Sage", "Clay"];
    view! {
        <div class="grid grid-cols-4 gap-2 px-3.5 py-2.5">
            {CodableColor::PRESETS.iter().zip(names).map(|(c, name)| {
                let c = *c;
                view! {
                    <button title=name
                        class=move || format!("h-7 rounded-md border border-black/10 shadow-sm dark:border-white/15 {}", if current() == c { "ring-2 ring-blue-500 ring-offset-1" } else { "" })
                        style=format!("background:{}", c.css())
                        on:click=move |_| p.update(|p| p.background_style.solid_color = c) />
                }
            }).collect_view()}
            <label title=t("Eigene Farbe", "Custom")
                class=move || format!("relative flex h-7 cursor-pointer items-center justify-center rounded-md border border-dashed border-black/25 text-[10px] dark:border-white/25 {}",
                    if CodableColor::PRESETS.contains(&current()) { "" } else { "ring-2 ring-blue-500 ring-offset-1" })
                style=move || if CodableColor::PRESETS.contains(&current()) { String::new() } else { format!("background:{}", current().css()) }>
                {t("Eigene", "Custom")}
                <input type="color" class="absolute inset-0 cursor-pointer opacity-0"
                    prop:value=move || current().to_hex()
                    on:input=move |e| if let Some(c) = CodableColor::from_hex(&event_target_value(&e)) { p.update(|p| p.background_style.solid_color = c) } />
            </label>
        </div>
    }
}

#[derive(Clone, Copy, PartialEq)]
enum AutoRun {
    Idle,
    Found(usize),
    Empty,
}

#[component]
fn EffectsSection(ed: Ed) -> impl IntoView {
    let p = ed.project;
    let run = RwSignal::new(AutoRun::Idle);
    let count = move || p.with(|p| p.effect_segments.len() + p.timeline_cuts().len());
    view! {
        {label(t("Zeitleiste", "Timeline"))}
        {card(view! {
            <div class="flex items-center justify-between gap-2 px-3.5 py-2">
                <AddEffectMenu ed=ed />
                <span class="rounded-full bg-black/5 px-2 py-0.5 text-[11px] text-black/50 dark:bg-white/10 dark:text-white/50">{count}</span>
            </div>
            {move || match ed.sel.get() {
                None => view! {
                    <div class="px-3.5 py-2.5 text-[12px] text-black/45 dark:text-white/45">
                        {t("„Effekt hinzufügen“ oder Doppelklick auf eine Effektspur.", "Use Add Effect or double-click an effects lane.")}
                    </div>
                }.into_any(),
                Some(Sel::Cut(_)) => selected_cut(ed).into_any(),
                Some(Sel::Effect(_)) => selected_effect(ed).into_any(),
            }}
            <Show when=move || ed.has_tel.get()>
                <div class="flex flex-col gap-1 px-3.5 py-2.5">
                    <div class="flex items-center justify-between">
                        <span>{t("Zooms vorschlagen", "Suggest zooms")}</span>
                        <button class="flex items-center gap-1 rounded-[5px] bg-purple-500/12 px-2.5 py-1 text-[11px] font-medium text-purple-600 hover:bg-purple-500/20 dark:text-purple-300"
                            on:click=move |_| {
                                let n = ed.auto_zoom();
                                run.set(if n > 0 { AutoRun::Found(n) } else { AutoRun::Empty });
                                if n == 0 {
                                    set_timeout(move || if run.get_untracked() == AutoRun::Empty { run.set(AutoRun::Idle) }, Duration::from_secs(3));
                                }
                            }>
                            "✦ "{t("Erkennen", "Detect")}
                        </button>
                    </div>
                    <span class="text-[11px] text-black/45 dark:text-white/45">
                        {move || match run.get() {
                            AutoRun::Empty => t("Keine klaren Momente gefunden – mehr klicken beim Aufnehmen.", "No clear moments detected — try recording with more clicks.").to_owned(),
                            AutoRun::Found(n) => format!("{n} {}", t("Zooms vorgeschlagen.", "zooms suggested.")),
                            AutoRun::Idle => t("Analysiert Klicks, Pausen und Tippen.", "Analyzes clicks + pauses.").to_owned(),
                        }}
                    </span>
                </div>
            </Show>
        })}
    }
}

fn selected_cut(ed: Ed) -> impl IntoView {
    let cut = move || ed.selected_cut();
    view! {
        <div class="flex items-center justify-between gap-2 px-3.5 py-2">
            <div class="flex flex-col">
                <span class="text-[11px] text-black/40 dark:text-white/40">{t("Ausgewählter Schnitt", "Selected Cut")}</span>
                <span class="font-mono text-[12px]">{move || cut().map(|c| format!("{} – {}  ({:.1} s {})", format_time(c.start_time), format_time(c.end_time), c.duration(), t("entfernt", "removed"))).unwrap_or_default()}</span>
            </div>
            <TextButton variant="ghost" on_click=move || ed.remove_selected()>{t("Löschen", "Delete")}</TextButton>
        </div>
    }
}

fn selected_effect(ed: Ed) -> impl IntoView {
    let seg = move || ed.selected_effect();
    let id = move || seg().map(|s| s.id).unwrap_or_default();
    let zoom = move || seg().and_then(|s| match s.payload {
        EffectPayload::Zoom(z) => Some(z),
        _ => None,
    });
    let blur = move || seg().and_then(|s| match s.payload {
        EffectPayload::Blur(b) => Some(b),
        _ => None,
    });
    let set_zoom = move |f: Box<dyn FnOnce(&mut ZoomEffectPayload)>| {
        ed.update_effect(&id(), move |s| {
            if let EffectPayload::Zoom(z) = &mut s.payload {
                f(z);
            }
            s.source = SegmentSource::Manual;
        })
    };
    view! {
        <div class="flex items-center justify-between px-3.5 pt-2">
            <span class="text-[11px] text-black/40 dark:text-white/40">
                {move || format!("{} {}", t("Ausgewählt:", "Selected"), if zoom().is_some() { t("Zoom", "Zoom") } else { t("Weichzeichner", "Blur") })}
                {move || seg().map(|s| format!("  {} – {}", format_time(s.start_time), format_time(s.end_time)))}
            </span>
            <TextButton variant="ghost" on_click=move || ed.remove_selected()>{t("Löschen", "Delete")}</TextButton>
        </div>
        <Show when=move || zoom().is_some()>
            {slider_row(t("Zoomstufe", "Zoom Level"), move || zoom().map(|z| z.zoom_level).unwrap_or(1.5), 1.25, 5.0, 0.25, "×", 2,
                move |v| set_zoom(Box::new(move |z| z.zoom_level = v.clamp(1.25, 5.0))))}
            {picker_row(t("Fokus", "Focus"),
                move || match zoom().map(|z| z.focus_mode) {
                    Some(ZoomFocusMode::Manual { x, y }) if (x - 0.5).abs() < 1e-9 && (y - 0.5).abs() < 1e-9 => "center".into(),
                    Some(ZoomFocusMode::Manual { .. }) => "manual".into(),
                    _ => "follow".into(),
                },
                vec![("follow", t("Cursor folgen", "Follow Cursor")), ("center", t("Mitte", "Center")), ("manual", t("Punkt wählen", "Pick Point"))],
                move |v| {
                    let mode = match v.as_str() {
                        "center" => ZoomFocusMode::Manual { x: 0.5, y: 0.5 },
                        "manual" => match zoom().map(|z| z.focus_mode) {
                            Some(m @ ZoomFocusMode::Manual { .. }) => m,
                            _ => ZoomFocusMode::Manual { x: 0.5001, y: 0.5 },
                        },
                        _ => ZoomFocusMode::FollowCursor,
                    };
                    set_zoom(Box::new(move |z| z.focus_mode = mode));
                    // In das Segment springen, damit der Fokus in der Vorschau sichtbar ist.
                    if let Some(s) = ed.selected_effect() {
                        let tm = ed.time.get_untracked();
                        if tm < s.start_time || tm > s.end_time {
                            ed.seek((s.start_time + s.end_time) / 2.0);
                        }
                    }
                })}
        </Show>
        <Show when=move || blur().is_some()>
            {slider_row(t("Stärke", "Blur"), move || blur().map(|b| b.radius).unwrap_or(18.0), 2.0, 48.0, 1.0, " px", 0,
                move |v| ed.update_effect(&id(), move |s| if let EffectPayload::Blur(b) = &mut s.payload { b.radius = v }))}
            <div class="px-3.5 pb-2 text-[11px] text-black/45 dark:text-white/45">
                {t("Bereich in der Vorschau verschieben und an der Ecke skalieren.", "Drag the region in the preview; resize from the corner.")}
            </div>
        </Show>
    }
}
