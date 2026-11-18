//! Werkzeugleiste (Port von Capso `AnnotationToolbar`, `AnnotationColorControls`, Stil-Presets).
use super::canvas::{Change, Ed};
use super::model::*;
use crate::api::{self, t};
use crate::ui::Icon;
use leptos::prelude::*;
use serde_json::json;

// lucide-Icons
pub const I_SELECT: &str = r#"<path d="M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z"/>"#;
pub const I_ARROW: &str = r#"<path d="M7 7h10v10"/><path d="M7 17 17 7"/>"#;
pub const I_LINE: &str = r#"<path d="M5 19 19 5"/>"#;
pub const I_RECT: &str = r#"<rect width="18" height="14" x="3" y="5" rx="1"/>"#;
pub const I_ELLIPSE: &str = r#"<circle cx="12" cy="12" r="9"/>"#;
pub const I_PENCIL: &str = r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#;
pub const I_EYE_OFF: &str = r#"<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/>"#;
pub const I_COUNTER: &str = r#"<circle cx="12" cy="12" r="10"/><path d="M12 16V8l-2 2"/>"#;
pub const I_HIGHLIGHTER: &str = r#"<path d="m9 11-6 6v3h9l3-3"/><path d="m22 12-4.6 4.6a2 2 0 0 1-2.8 0l-5.2-5.2a2 2 0 0 1 0-2.8L14 4"/>"#;
pub const I_SPOTLIGHT: &str = r#"<path d="M18 6c0 2-2 2-2 4v10a2 2 0 0 1-2 2h-4a2 2 0 0 1-2-2V10c0-2-2-2-2-4V2h12z"/><line x1="6" x2="18" y1="6" y2="6"/><line x1="12" x2="12" y1="12" y2="12"/>"#;
pub const I_CROP: &str = r#"<path d="M6 2v14a2 2 0 0 0 2 2h14"/><path d="M18 22V8a2 2 0 0 0-2-2H2"/>"#;
pub const I_SPARKLES: &str = r#"<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/>"#;
pub const I_UNDO: &str = r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>"#;
pub const I_REDO: &str = r#"<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5A5.5 5.5 0 0 0 4 14.5A5.5 5.5 0 0 0 9.5 20H13"/>"#;
pub const I_BOOKMARK: &str = r#"<path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z"/>"#;
pub const I_ROTATE: &str = r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"#;
pub const I_FLIP: &str = r#"<path d="M8 3H5a2 2 0 0 0-2 2v14c0 1.1.9 2 2 2h3"/><path d="M16 3h3a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-3"/><path d="M12 20v2"/><path d="M12 14v2"/><path d="M12 8v2"/><path d="M12 2v2"/>"#;
pub const I_MOVE: &str = r#"<path d="M12 2v20"/><path d="m15 19-3 3-3-3"/><path d="m19 9 3 3-3 3"/><path d="M2 12h20"/><path d="m5 9-3 3 3 3"/><path d="m9 5 3-3 3 3"/>"#;
pub const I_SHARE: &str = r#"<path d="M4 12v8a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8"/><path d="m16 6-4-4-4 4"/><path d="M12 2v13"/>"#;
pub const I_DOWNLOAD: &str = r#"<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>"#;
pub const I_EXTERNAL: &str = r#"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#;
pub const I_PLUS: &str = r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#;
pub const I_MINUS: &str = r#"<path d="M5 12h14"/>"#;
pub const I_FILL: &str = r#"<rect width="16" height="16" x="4" y="4" rx="2" fill="currentColor"/>"#;
pub const I_NOFILL: &str = r#"<rect width="16" height="16" x="4" y="4" rx="2"/>"#;
pub const I_CHEVRON: &str = r#"<path d="m6 9 6 6 6-6"/>"#;
pub const I_PIPETTE: &str = r#"<path d="m2 22 1-1h3l9-9"/><path d="M3 21v-3l9-9"/><path d="m15 6 3.4-3.4a2.1 2.1 0 1 1 3 3L18 9l.4.4a2.1 2.1 0 1 1-3 3l-3.8-3.8a2.1 2.1 0 1 1 3-3l.4.4Z"/>"#;

pub fn tool_icon(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => I_SELECT,
        Tool::Arrow => I_ARROW,
        Tool::Line => I_LINE,
        Tool::Rectangle => I_RECT,
        Tool::Ellipse => I_ELLIPSE,
        Tool::Text => "",
        Tool::Freehand => I_PENCIL,
        Tool::Pixelate => I_EYE_OFF,
        Tool::Counter => I_COUNTER,
        Tool::Highlighter => I_HIGHLIGHTER,
        Tool::Spotlight => I_SPOTLIGHT,
    }
}

pub fn tool_title(tool: Tool) -> String {
    let (de, en) = tool.label();
    format!("{} ({})", t(de, en), tool.display_key())
}

pub const BTN: &str = "inline-flex shrink-0 items-center justify-center rounded-md size-7 transition-colors [&_svg]:size-[15px] disabled:opacity-35";
pub const BTN_IDLE: &str = "text-zinc-700 dark:text-zinc-200 hover:bg-black/5 dark:hover:bg-white/10";
pub const BTN_ON: &str = "bg-blue-500 text-white";

/// Kleiner Werkzeugknopf, der dem Textfeld den Fokus nicht stiehlt.
#[component]
pub fn TB(
    #[prop(into)] title: String,
    #[prop(optional, into)] active: Signal<bool>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] icon: Option<&'static str>,
    #[prop(optional, into)] label: Option<&'static str>,
    #[prop(optional, into)] class: Option<&'static str>,
    on_click: impl Fn() + 'static,
) -> impl IntoView {
    let extra = class.unwrap_or("");
    view! {
        <button title=title disabled=move || disabled.get()
            class=move || format!("{BTN} {} {extra}", if active.get() { BTN_ON } else { BTN_IDLE })
            on:mousedown=|e| e.prevent_default()
            on:click=move |e| { e.stop_propagation(); on_click() }>
            {icon.map(|d| view! { <Icon d=d /> })}
            {label.map(|l| view! { <span class="text-[13px] font-semibold leading-none">{l}</span> })}
        </button>
    }
}

pub fn sep() -> impl IntoView {
    view! { <div class="mx-1 h-5 w-px shrink-0 bg-black/10 dark:bg-white/15"></div> }
}

/// Werkzeugknöpfe.
#[component]
pub fn ToolButtons(ed: Ed, tools: &'static [Tool]) -> impl IntoView {
    tools
        .iter()
        .map(|&tool| {
            let active = Signal::derive(move || ed.tool() == tool);
            if tool == Tool::Text {
                view! { <TB title=tool_title(tool) active=active label="Aa" on_click=move || ed.set_tool(tool) /> }.into_any()
            } else {
                view! { <TB title=tool_title(tool) active=active icon=tool_icon(tool) on_click=move || ed.set_tool(tool) /> }.into_any()
            }
        })
        .collect_view()
}

/// Größenregler + werkzeugabhängige Optionen (Capso `toolOptions`).
#[component]
pub fn ToolOptions(ed: Ed, #[prop(optional)] compact: bool) -> impl IntoView {
    let wt = Memo::new(move |_| ed.width_tool());
    let width = move || ed.prefs.with(|p| p.width_for(wt.get()));
    let range = move || ToolPrefs::width_range(wt.get());
    let title = move || {
        let w = width() as i64;
        match wt.get() {
            Tool::Text => format!("{}: {w}", t("Schriftgröße", "Font Size")),
            Tool::Pixelate => format!("{}: {w}", t("Blockgröße", "Block Size")),
            Tool::Counter => format!("{}: {w}", t("Zählergröße", "Counter Size")),
            Tool::Highlighter => format!("{}: {w}", t("Markerbreite", "Highlighter Width")),
            _ => format!("{}: {w}", t("Linienstärke", "Line Width")),
        }
    };
    let set_width = move |v: f64| {
        ed.prefs.update(|p| p.set_width_for(wt.get_untracked(), v));
        ed.apply_to_selected(Change::Width);
    };
    let sel_cls = "h-6 rounded-md border border-black/10 dark:border-white/15 bg-white/80 dark:bg-white/10 px-1 text-[12px] outline-none";
    let show_pattern = move || matches!(wt.get(), Tool::Arrow | Tool::Line | Tool::Rectangle | Tool::Ellipse);
    // Auswahlwerkzeug: Linienstärke und Füllung gelten für das ausgewählte Objekt (Capso `strokeGroup` mit `.select`).
    let show_fill = move || matches!(wt.get(), Tool::Rectangle | Tool::Ellipse | Tool::Select);
    let show_text = move || wt.get() == Tool::Text;
    let redaction = move || ed.prefs.with(|p| p.redaction_mode);
    let show_width = move || wt.get() != Tool::Spotlight && !(wt.get() == Tool::Pixelate && redaction() == RedactionMode::Solid);
    let txt_toggle = move |label: (&'static str, &'static str), help: (&'static str, &'static str), get: fn(&ToolPrefs) -> bool, set: fn(&mut ToolPrefs)| {
        view! {
            <button title=move || t(help.0, help.1)
                class=move || format!("h-6 rounded-md px-1.5 text-[11px] font-medium {}", if ed.prefs.with(get) { BTN_ON } else { BTN_IDLE })
                on:mousedown=|e| e.prevent_default()
                on:click=move |_| { ed.prefs.update(set); ed.apply_to_selected(Change::TextFx); ed.refocus.update(|r| *r += 1); }>
                {move || t(label.0, label.1)}
            </button>
        }
    };
    view! {
        <div class="flex shrink-0 items-center gap-1.5">
            <Show when=move || wt.get() == Tool::Pixelate>
                <select class=sel_cls title=move || t("Schwärzungsart", "Redaction Mode")
                    on:change=move |e| {
                        let m = match event_target_value(&e).as_str() { "blur" => RedactionMode::Blur, "solid" => RedactionMode::Solid, _ => RedactionMode::Pixelate };
                        ed.prefs.update(|p| p.redaction_mode = m);
                        ed.apply_to_selected(Change::Redaction);
                    }>
                    <option value="pixelate" selected=move || redaction() == RedactionMode::Pixelate>{move || t("Verpixeln", "Pixelate")}</option>
                    <option value="blur" selected=move || redaction() == RedactionMode::Blur>{move || t("Weichzeichnen", "Blur")}</option>
                    <option value="solid" selected=move || redaction() == RedactionMode::Solid>{move || t("Füllen", "Solid")}</option>
                </select>
            </Show>
            <Show when=show_width>
                <input type="range" class=if compact { "w-20 accent-blue-500" } else { "w-24 accent-blue-500" }
                    title=title
                    min=move || range().0 max=move || range().1 step=move || range().2
                    prop:value=move || width().to_string()
                    on:input=move |e| { if let Ok(v) = event_target_value(&e).parse::<f64>() { set_width(v) } }
                    on:change=move |_| ed.refocus.update(|r| *r += 1) />
                <span class="w-6 text-right text-[11px] tabular-nums text-zinc-500">{move || (width() as i64).to_string()}</span>
            </Show>
            <Show when=show_pattern>
                <select class=sel_cls title=move || t("Linienmuster", "Stroke Pattern")
                    on:change=move |e| {
                        let v = match event_target_value(&e).as_str() { "dashed" => StrokePattern::Dashed, "dotted" => StrokePattern::Dotted, _ => StrokePattern::Solid };
                        ed.prefs.update(|p| p.pattern = v);
                        ed.apply_to_selected(Change::Pattern);
                    }>
                    <option value="solid" selected=move || ed.prefs.with(|p| p.pattern == StrokePattern::Solid)>"———"</option>
                    <option value="dashed" selected=move || ed.prefs.with(|p| p.pattern == StrokePattern::Dashed)>"– – –"</option>
                    <option value="dotted" selected=move || ed.prefs.with(|p| p.pattern == StrokePattern::Dotted)>"· · · ·"</option>
                </select>
            </Show>
            <Show when=move || wt.get() == Tool::Arrow>
                <select class=sel_cls title=move || t("Pfeilstil", "Arrow Style")
                    on:change=move |e| {
                        let v = match event_target_value(&e).as_str() { "filled" => ArrowHead::Filled, "double" => ArrowHead::Double, "tapered" => ArrowHead::Tapered, _ => ArrowHead::Standard };
                        ed.prefs.update(|p| p.arrow_head = v);
                        ed.apply_to_selected(Change::ArrowHead);
                    }>
                    <option value="standard" selected=move || ed.prefs.with(|p| p.arrow_head == ArrowHead::Standard)>{move || t("→ Offen", "→ Open")}</option>
                    <option value="filled" selected=move || ed.prefs.with(|p| p.arrow_head == ArrowHead::Filled)>{move || t("➤ Gefüllt", "➤ Filled")}</option>
                    <option value="double" selected=move || ed.prefs.with(|p| p.arrow_head == ArrowHead::Double)>{move || t("↔ Doppelt", "↔ Double")}</option>
                    <option value="tapered" selected=move || ed.prefs.with(|p| p.arrow_head == ArrowHead::Tapered)>{move || t("➽ Spitz zulaufend", "➽ Tapered")}</option>
                </select>
            </Show>
            <Show when=show_fill>
                <TB title=t("Form füllen", "Fill Shape") active=Signal::derive(move || ed.prefs.with(|p| p.filled))
                    icon=I_FILL
                    on_click=move || { ed.prefs.update(|p| p.filled = !p.filled); ed.apply_to_selected(Change::Filled); } />
            </Show>
            <Show when=show_text>
                {txt_toggle(("Füllung", "Fill"), ("Text-Hintergrund", "Text Fill"), |p| p.text_fill, |p| p.text_fill = !p.text_fill)}
                {txt_toggle(("Rahmen", "Outline"), ("Textrahmen", "Text Box Outline"), |p| p.text_outline, |p| p.text_outline = !p.text_outline)}
                {txt_toggle(("Kontur", "Trace"), ("Textkontur", "Text Trace"), |p| p.text_stroke, |p| p.text_stroke = !p.text_stroke)}
            </Show>
        </div>
    }
}

/// Farbvorgaben + eigene Farbe (Systemfarbpalette inkl. Pipette) + Hex (Klick kopiert).
#[component]
pub fn ColorControls(ed: Ed, #[prop(optional)] compact: bool) -> impl IntoView {
    let size = if compact { "size-[15px]" } else { "size-[17px]" };
    let set = move |c: String| {
        ed.prefs.update(|p| p.color = c);
        ed.apply_to_selected(Change::Color);
        ed.refocus.update(|r| *r += 1);
    };
    let swatches = PRESET_COLORS
        .iter()
        .map(|(name, _)| {
            let name = *name;
            let selected = move || ed.prefs.with(|p| p.color == name);
            view! {
                <button title=name class=move || format!("grid place-items-center rounded-full p-[2px] {}", if selected() { "ring-2 ring-blue-500" } else { "" })
                    on:mousedown=|e| e.prevent_default()
                    on:click=move |_| set(name.to_string())>
                    <span class=format!("block {size} rounded-full border border-black/25") style:background=css_rgba(name, 1.0)></span>
                </button>
            }
        })
        .collect_view();
    view! {
        <div class="flex shrink-0 items-center gap-[3px]">
            {swatches}
            <label title=move || t("Eigene Farbe", "Custom Color") class="relative ml-0.5 grid size-6 cursor-pointer place-items-center rounded-full border border-black/25"
                style:background="conic-gradient(red, yellow, lime, cyan, blue, magenta, red)">
                <span class="block size-3 rounded-full border border-white/80" style:background=move || ed.prefs.with(|p| css_rgba(&p.color, 1.0))></span>
                <input type="color" class="absolute inset-0 cursor-pointer opacity-0"
                    prop:value=move || ed.prefs.with(|p| color_hex(&p.color).to_lowercase())
                    on:input=move |e| set(event_target_value(&e).to_uppercase()) />
            </label>
            <Show when=move || !compact>
                // Pipette (Capso `pickScreenColor` → NSColorSampler)
                <button title=move || t("Farbe vom Bildschirm wählen", "Pick Color From Screen")
                    class="ml-1 grid size-6 place-items-center rounded-full bg-black/5 dark:bg-white/10 text-zinc-700 dark:text-zinc-200 hover:bg-black/10 dark:hover:bg-white/15 [&_svg]:size-[13px]"
                    on:mousedown=|e| e.prevent_default()
                    on:click=move |_| leptos::task::spawn_local(async move {
                        if let Ok(v) = api::rpc("editor", "pickColor", serde_json::Value::Null).await {
                            if let Some(c) = v.as_str() {
                                set(c.to_string());
                            }
                        }
                    })>
                    <Icon d=I_PIPETTE />
                </button>
                <button title=move || t("Hex-Wert kopieren", "Copy Hex Value")
                    class="ml-1 h-6 w-[62px] rounded border border-black/10 dark:border-white/15 bg-black/5 dark:bg-white/5 font-mono text-[11px] text-zinc-500"
                    on:mousedown=|e| e.prevent_default()
                    on:click=move |_| api::fire("core", "copyText", json!({"text": ed.prefs.with_untracked(|p| color_hex(&p.color))}))>
                    {move || ed.prefs.with(|p| color_hex(&p.color))}
                </button>
            </Show>
        </div>
    }
}

/// Stil-Presets (Capso `presetsMenu`).
#[component]
pub fn PresetsMenu(ed: Ed) -> impl IntoView {
    let open = RwSignal::new(false);
    let item = "flex w-full items-center gap-2 rounded px-2 py-1 text-left text-[12px] hover:bg-blue-500 hover:text-white";
    let ptitle = |p: &StylePreset| {
        let tool = Tool::from_id(&p.tool).unwrap_or(Tool::Arrow);
        let (de, en) = tool.label();
        format!("{} · {} pt", t(de, en), p.line_width.round() as i64)
    };
    view! {
        <div class="relative">
            <TB title=t("Stil-Vorlagen", "Style Presets") icon=I_BOOKMARK active=open on_click=move || open.update(|o| *o = !*o) />
            <Show when=move || open.get()>
                <div class="fixed inset-0 z-40" on:click=move |_| open.set(false)></div>
                <div class="absolute left-0 top-8 z-50 w-56 rounded-lg border border-black/10 dark:border-white/10 bg-white dark:bg-zinc-800 p-1 shadow-xl">
                    <For each=move || ed.presets.get() key=|p| p.id.clone() let:p>
                        {
                            let pc = p.clone();
                            let title = ptitle(&p);
                            view! {
                                <div class="group flex items-center">
                                    <button class=item on:mousedown=|e| e.prevent_default() on:click=move |_| { ed.apply_preset(&pc); open.set(false); }>
                                        <span class="block size-3 rounded-full border border-black/25" style:background=css_rgba(&p.color, 1.0)></span>
                                        {title}
                                    </button>
                                    <button title=move || t("Entfernen", "Remove") class="px-1.5 text-zinc-400 opacity-0 hover:text-red-500 group-hover:opacity-100"
                                        on:click={ let id = p.id.clone(); move |_| { ed.presets.update(|l| l.retain(|x| x.id != id)); ed.save_presets(); } }>"×"</button>
                                </div>
                            }
                        }
                    </For>
                    <Show when=move || !ed.presets.with(|l| l.is_empty())>
                        <div class="my-1 h-px bg-black/10 dark:bg-white/10"></div>
                    </Show>
                    <button class=item on:mousedown=|e| e.prevent_default() on:click=move |_| { ed.save_current_as_preset(); open.set(false); }>
                        {move || t("Aktuellen Stil sichern", "Save Current Style")}
                    </button>
                </div>
            </Show>
        </div>
    }
}

/// Rückgängig / Wiederholen.
#[component]
pub fn UndoRedo(ed: Ed) -> impl IntoView {
    view! {
        <TB title=format!("{} (⌘Z)", t("Rückgängig", "Undo")) icon=I_UNDO disabled=Signal::derive(move || !ed.doc.with(|d| d.can_undo())) on_click=move || ed.undo() />
        <TB title=format!("{} (⇧⌘Z)", t("Wiederholen", "Redo")) icon=I_REDO disabled=Signal::derive(move || !ed.doc.with(|d| d.can_redo())) on_click=move || ed.redo() />
    }
}
