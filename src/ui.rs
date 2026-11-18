//! Gemeinsame UI-Bausteine (Leptos + Tailwind, lucide-Icons) im Stil von rust-ui.com / macOS „Liquid Glass“.
//! Module dürfen hier NEUE Komponenten/Icons am Dateiende ergänzen, bestehende aber nicht ändern.
use leptos::prelude::*;

const BTN: &str = "inline-flex items-center justify-center rounded-md size-7 shadow-sm transition-colors \
    focus-visible:outline-none [&_svg]:size-4 [&_svg]:shrink-0";

/// Kleiner runder Icon-Knopf (dunkel, für Overlays/Vorschauen).
#[component]
pub fn Button(
    #[prop(into)] title: String,
    #[prop(optional, into)] active: Signal<bool>,
    on_click: impl Fn() + 'static,
    children: Children,
) -> impl IntoView {
    let class = move || {
        let v = if active.get() { "bg-blue-500 text-white hover:bg-blue-500/90" } else { "bg-zinc-900/75 text-zinc-100 hover:bg-zinc-800 backdrop-blur" };
        format!("{BTN} {v}")
    };
    view! {
        <button title=title class=class on:click=move |e| { e.stop_propagation(); on_click() }>
            {children()}
        </button>
    }
}

/// Textknopf für Fenster (Einstellungen, Editor, Dialoge). `variant`: "primary" | "secondary" | "ghost" | "danger".
#[component]
pub fn TextButton(
    #[prop(optional, into)] variant: Option<&'static str>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] title: Option<String>,
    on_click: impl Fn() + 'static,
    children: Children,
) -> impl IntoView {
    let v = match variant.unwrap_or("secondary") {
        "primary" => "bg-blue-500 text-white hover:bg-blue-600 shadow-sm",
        "danger" => "bg-red-500 text-white hover:bg-red-600 shadow-sm",
        "ghost" => "hover:bg-black/5 dark:hover:bg-white/10",
        _ => "bg-white/80 dark:bg-white/10 border border-black/10 dark:border-white/10 hover:bg-white dark:hover:bg-white/15 shadow-sm",
    };
    view! {
        <button
            title=title
            disabled=move || disabled.get()
            class=format!("inline-flex items-center justify-center gap-1.5 rounded-md px-3 h-7 text-[13px] font-medium transition-colors disabled:opacity-40 [&_svg]:size-4 {v}")
            on:click=move |e| { e.stop_propagation(); on_click() }
        >
            {children()}
        </button>
    }
}

/// Umschalter (macOS-Switch).
#[component]
pub fn Toggle(#[prop(into)] checked: Signal<bool>, on_change: impl Fn(bool) + 'static) -> impl IntoView {
    view! {
        <button
            role="switch"
            class=move || format!(
                "relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full transition-colors {}",
                if checked.get() { "bg-blue-500" } else { "bg-black/15 dark:bg-white/20" }
            )
            on:click=move |_| on_change(!checked.get_untracked())
        >
            <span class=move || format!(
                "inline-block size-[18px] rounded-full bg-white shadow transition-transform {}",
                if checked.get() { "translate-x-[18px]" } else { "translate-x-[2px]" }
            ) />
        </button>
    }
}

/// Auswahlliste: `options` = (Wert, Anzeigename).
#[component]
pub fn Select(
    #[prop(into)] value: Signal<String>,
    options: Vec<(String, String)>,
    on_change: impl Fn(String) + 'static,
) -> impl IntoView {
    view! {
        <select
            class="h-7 rounded-md border border-black/10 dark:border-white/10 bg-white/80 dark:bg-white/10 px-2 text-[13px] outline-none focus:ring-2 focus:ring-blue-500/40"
            prop:value=move || value.get()
            on:change=move |e| on_change(event_target_value(&e))
        >
            {options.into_iter().map(|(v, l)| {
                let v2 = v.clone();
                view! { <option value=v selected=move || value.get() == v2>{l}</option> }
            }).collect_view()}
        </select>
    }
}

/// Schieberegler.
#[component]
pub fn Slider(#[prop(into)] value: Signal<f64>, min: f64, max: f64, #[prop(optional)] step: Option<f64>, on_change: impl Fn(f64) + 'static) -> impl IntoView {
    view! {
        <input type="range" min=min max=max step=step.unwrap_or(1.0)
            class="w-40 accent-blue-500"
            prop:value=move || value.get().to_string()
            on:input=move |e| on_change(event_target_value(&e).parse().unwrap_or(min)) />
    }
}

/// Textfeld.
#[component]
pub fn TextInput(
    #[prop(into)] value: Signal<String>,
    #[prop(optional, into)] placeholder: Option<String>,
    #[prop(optional)] password: bool,
    on_change: impl Fn(String) + 'static,
) -> impl IntoView {
    view! {
        <input
            type=if password { "password" } else { "text" }
            placeholder=placeholder
            class="h-7 w-full rounded-md border border-black/10 dark:border-white/10 bg-white/80 dark:bg-white/10 px-2 text-[13px] outline-none focus:ring-2 focus:ring-blue-500/40"
            prop:value=move || value.get()
            on:change=move |e| on_change(event_target_value(&e))
        />
    }
}

/// Gruppierte Karte für Einstellungen.
#[component]
pub fn Card(#[prop(optional, into)] title: Option<String>, children: Children) -> impl IntoView {
    view! {
        <section class="mb-5">
            {title.map(|t| view! { <h3 class="mb-1.5 px-1 text-[11px] font-semibold uppercase tracking-wide text-black/50 dark:text-white/50">{t}</h3> })}
            <div class="divide-y divide-black/5 dark:divide-white/10 rounded-xl border border-black/5 dark:border-white/10 bg-white/70 dark:bg-white/5 shadow-sm">
                {children()}
            </div>
        </section>
    }
}

/// Zeile in einer Karte: Bezeichnung links, Steuerelement rechts.
#[component]
pub fn Row(#[prop(into)] label: String, #[prop(optional, into)] hint: Option<String>, children: Children) -> impl IntoView {
    view! {
        <div class="flex min-h-[40px] items-center justify-between gap-4 px-3 py-2">
            <div class="min-w-0">
                <div class="text-[13px]">{label}</div>
                {hint.map(|h| view! { <div class="text-[11px] text-black/50 dark:text-white/50">{h}</div> })}
            </div>
            <div class="flex shrink-0 items-center gap-2">{children()}</div>
        </div>
    }
}

/// Tastenkürzel-Anzeige ("Alt+Shift+1" → ⌥⇧1).
pub fn shortcut_symbols(accel: &str) -> String {
    accel
        .split('+')
        .map(|p| match p.to_lowercase().as_str() {
            "cmd" | "command" | "super" | "meta" | "cmdorctrl" | "commandorcontrol" => "⌘".to_owned(),
            "shift" => "⇧".to_owned(),
            "alt" | "option" => "⌥".to_owned(),
            "ctrl" | "control" => "⌃".to_owned(),
            "escape" | "esc" => "⎋".to_owned(),
            "enter" | "return" => "↩".to_owned(),
            "space" => "Space".to_owned(),
            "backspace" => "⌫".to_owned(),
            "tab" => "⇥".to_owned(),
            k => k.trim_start_matches("key").trim_start_matches("digit").to_uppercase(),
        })
        .collect()
}

#[component]
pub fn Kbd(#[prop(into)] keys: String) -> impl IntoView {
    view! { <kbd class="rounded border border-black/10 dark:border-white/15 bg-black/5 dark:bg-white/10 px-1.5 py-0.5 font-sans text-[11px]">{shortcut_symbols(&keys)}</kbd> }
}

#[component]
pub fn Icon(#[prop(into)] d: Signal<&'static str>) -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
            stroke-linecap="round" stroke-linejoin="round" inner_html=d></svg>
    }
}

// lucide-Icons (Pfadinhalt von viewBox 0 0 24 24)
pub const X: &str = r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#;
pub const COPY: &str = r#"<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#;
pub const CHECK: &str = r#"<path d="M20 6 9 17l-5-5"/>"#;
pub const PIN: &str = r#"<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#;
pub const SAVE: &str = r#"<path d="M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7"/><path d="M7 3v4a1 1 0 0 0 1 1h7"/>"#;
pub const PENCIL: &str = r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#;
pub const SCAN_TEXT: &str = r#"<path d="M3 7V5a2 2 0 0 1 2-2h2"/><path d="M17 3h2a2 2 0 0 1 2 2v2"/><path d="M21 17v2a2 2 0 0 1-2 2h-2"/><path d="M7 21H5a2 2 0 0 1-2-2v-2"/><path d="M7 8h8"/><path d="M7 12h10"/><path d="M7 16h6"/>"#;
pub const CLOUD_UPLOAD: &str = r#"<path d="M12 13v8"/><path d="M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242"/><path d="m8 17 4-4 4 4"/>"#;
pub const LANGUAGES: &str = r#"<path d="m5 8 6 6"/><path d="m4 14 6-6 2-3"/><path d="M2 5h12"/><path d="M7 2h1"/><path d="m22 22-5-10-5 10"/><path d="M14 18h6"/>"#;
pub const TRASH: &str = r#"<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"#;
pub const FOLDER: &str = r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#;
pub const GRIP: &str = r#"<circle cx="9" cy="12" r="1"/><circle cx="9" cy="5" r="1"/><circle cx="9" cy="19" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="15" cy="5" r="1"/><circle cx="15" cy="19" r="1"/>"#;
pub const LOCK: &str = r#"<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>"#;
pub const UNLOCK: &str = r#"<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/>"#;
pub const SETTINGS: &str = r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#;
pub const CAMERA: &str = r#"<path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3z"/><circle cx="12" cy="13" r="3"/>"#;
pub const VIDEO: &str = r#"<path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/><rect x="2" y="6" width="14" height="12" rx="2"/>"#;
pub const MIC: &str = r#"<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>"#;
pub const PAUSE: &str = r#"<rect x="14" y="4" width="4" height="16" rx="1"/><rect x="6" y="4" width="4" height="16" rx="1"/>"#;
pub const PLAY: &str = r#"<polygon points="6 3 20 12 6 21 6 3"/>"#;
pub const STOP: &str = r#"<rect width="14" height="14" x="5" y="5" rx="2"/>"#;
pub const RESTART: &str = r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"#;
pub const UNDO: &str = r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>"#;
pub const REDO: &str = r#"<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5A5.5 5.5 0 0 0 4 14.5A5.5 5.5 0 0 0 9.5 20H13"/>"#;
pub const SEARCH: &str = r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#;
pub const LINK: &str = r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>"#;
pub const EXTERNAL: &str = r#"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#;
pub const CROP: &str = r#"<path d="M6 2v14a2 2 0 0 0 2 2h14"/><path d="M18 22V8a2 2 0 0 0-2-2H2"/>"#;
pub const SPARKLES: &str = r#"<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/>"#;
pub const TIMER: &str = r#"<line x1="10" x2="14" y1="2" y2="2"/><line x1="12" x2="15" y1="14" y2="11"/><circle cx="12" cy="14" r="8"/>"#;
pub const SCROLL: &str = r#"<path d="M12 5v14"/><path d="m19 12-7 7-7-7"/>"#;
pub const MONITOR: &str = r#"<rect width="20" height="14" x="2" y="3" rx="2"/><line x1="8" x2="16" y1="21" y2="21"/><line x1="12" x2="12" y1="17" y2="21"/>"#;
pub const APP_WINDOW: &str = r#"<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M10 4v4"/><path d="M2 8h20"/><path d="M6 4v4"/>"#;
pub const SQUARE_DASHED: &str = r#"<path d="M5 3a2 2 0 0 0-2 2"/><path d="M19 3a2 2 0 0 1 2 2"/><path d="M21 19a2 2 0 0 1-2 2"/><path d="M5 21a2 2 0 0 1-2-2"/><path d="M9 3h1"/><path d="M9 21h1"/><path d="M14 3h1"/><path d="M14 21h1"/><path d="M3 9v1"/><path d="M21 9v1"/><path d="M3 14v1"/><path d="M21 14v1"/>"#;
pub const HISTORY: &str = r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>"#;
pub const KEYBOARD: &str = r#"<path d="M10 8h.01"/><path d="M12 12h.01"/><path d="M14 8h.01"/><path d="M16 12h.01"/><path d="M18 8h.01"/><path d="M6 8h.01"/><path d="M7 16h10"/><path d="M8 12h.01"/><rect width="20" height="16" x="2" y="4" rx="2"/>"#;
pub const SHIELD: &str = r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>"#;
pub const INFO: &str = r#"<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>"#;
pub const IMAGE: &str = r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#;
pub const DOWNLOAD: &str = r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/>"#;
pub const MORE: &str = r#"<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>"#;
