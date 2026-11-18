//! Verlaufsfenster (Port von Capso `HistoryView` / `HistoryItemView`): Raster mit Vorschaubildern,
//! Filter, Suche, Datumsgruppen, Mehrfachauswahl, Kontextmenü, Ziehen, PDF-Export, Cloud-Upload.
use crate::{
    api::{self, t},
    ui::*,
};
use leptos::{ev, prelude::*, task::spawn_local};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use wasm_bindgen::JsCast;

pub fn route(r: &str) -> Option<AnyView> {
    (r == "history").then(|| view! { <History /> }.into_any())
}

// lucide-Icons, nur hier verwendet
const DOWNLOAD: &str = r#"<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>"#;
const LINK: &str = r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>"#;
const CLOUD_CHECK: &str = r#"<path d="m17 15-5.5 5.5L9 18"/><path d="M5 17.743A7 7 0 1 1 15.71 10h1.79a4.5 4.5 0 0 1 1.5 8.742"/>"#;
const FILE_TEXT: &str = r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#;
const HISTORY: &str = r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>"#;
const SEARCH: &str = r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#;
const IMAGE: &str = r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#;
const SPINNER: &str = r#"<path d="M21 12a9 9 0 1 1-6.219-8.56"/>"#;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    created_at: f64,
    kind: String,
    #[serde(default)]
    name: String,
    width: u32,
    height: u32,
    file_size: f64,
    #[serde(default)]
    cloud_url: Option<String>,
    path: String,
    thumb_path: String,
    #[serde(default)]
    uploading: bool,
    /// "area" | "fullscreen" | "window" | "scroll" | "" (ältere Einträge)
    #[serde(default)]
    capture_mode: String,
    #[serde(default)]
    source_app_name: Option<String>,
    #[serde(default)]
    source_window_title: Option<String>,
}

impl Item {
    fn is_screenshot(&self) -> bool {
        self.kind == "screenshot"
    }
    /// Titel wie Capso `HistoryItemView.displayName`: Quell-App, sonst Aufnahmemodus.
    fn display_name(&self) -> String {
        if let Some(a) = self.source_app_name.as_deref().filter(|a| !a.trim().is_empty()) {
            return a.to_owned();
        }
        if self.is_screenshot() {
            match self.capture_mode.as_str() {
                "area" => return t("Bereichsaufnahme", "Area Capture").to_owned(),
                "fullscreen" => return t("Vollbild", "Fullscreen").to_owned(),
                "window" => return t("Fenster", "Window").to_owned(),
                "scroll" => return t("Scroll-Aufnahme", "Scrolling Capture").to_owned(),
                _ => {}
            }
        }
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match self.kind.as_str() {
            "recording" => t("Aufnahme", "Recording"),
            "gif" => "GIF",
            _ => t("Bildschirmfoto", "Screenshot"),
        }
        .to_owned()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct Counts {
    all: usize,
    screenshots: usize,
    recordings: usize,
    gifs: usize,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ListData {
    entries: Vec<Item>,
    counts: Counts,
    total_size: f64,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    cloud_configured: bool,
}

#[derive(Clone, Copy)]
struct Ctx {
    data: RwSignal<ListData>,
    filter: RwSignal<&'static str>,
    query: RwSignal<String>,
    selection: RwSignal<HashSet<String>>,
    anchor: RwSignal<Option<String>>,
    uploading: RwSignal<HashSet<String>>,
    confirm: RwSignal<Option<Vec<String>>>,
    confirm_clear: RwSignal<bool>,
    menu: RwSignal<Option<(f64, f64, String)>>,
    visible: Memo<Vec<Item>>,
}

fn load(data: RwSignal<ListData>) {
    spawn_local(async move {
        if let Ok(v) = api::rpc_as::<ListData>("history", "list", json!({ "filter": "all" })).await {
            data.set(v);
        }
    });
}

fn act(cmd: &'static str, id: &str) {
    api::fire("history", cmd, json!({ "id": id }));
}

fn act_many(cmd: &'static str, ids: Vec<String>) {
    api::fire("history", cmd, json!({ "ids": ids }));
}

// ───────────────────────────── Zeit & Größe ─────────────────────────────

fn ymd(ms: f64) -> (u32, u32, u32) {
    let d = js_sys::Date::new(&ms.into());
    (d.get_full_year(), d.get_month(), d.get_date())
}

const MONTHS_DE: [&str; 12] = ["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."];
const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS_DE: [&str; 7] = ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"];
const DAYS_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Gruppenname wie Capso: Heute, Gestern, Wochentag (< 7 Tage), sonst Datum.
fn group_label(ms: f64, now: f64) -> String {
    let de = api::lang() == "de";
    if ymd(ms) == ymd(now) {
        return t("Heute", "Today").into();
    }
    if ymd(ms) == ymd(now - 86_400_000.0) {
        return t("Gestern", "Yesterday").into();
    }
    let d = js_sys::Date::new(&ms.into());
    if (now - ms) / 86_400_000.0 < 7.0 {
        let w = d.get_day() as usize % 7;
        return if de { DAYS_DE[w] } else { DAYS_EN[w] }.into();
    }
    let (y, m, day) = ymd(ms);
    if de {
        format!("{day}. {} {y}", MONTHS_DE[m as usize % 12])
    } else {
        format!("{} {day}, {y}", MONTHS_EN[m as usize % 12])
    }
}

fn relative_time(ms: f64, now: f64) -> String {
    let s = ((now - ms) / 1000.0).max(0.0) as u64;
    let de = api::lang() == "de";
    match s {
        0..=59 => t("gerade eben", "just now").into(),
        60..=3599 => if de { format!("vor {} Min.", s / 60) } else { format!("{} min ago", s / 60) },
        3600..=86_399 => if de { format!("vor {} Std.", s / 3600) } else { format!("{} hr ago", s / 3600) },
        _ => {
            let d = s / 86_400;
            if de {
                format!("vor {d} {}", if d == 1 { "Tag" } else { "Tagen" })
            } else {
                format!("{d} {} ago", if d == 1 { "day" } else { "days" })
            }
        }
    }
}

fn format_bytes(b: f64) -> String {
    let (v, u) = if b < 1e3 {
        return format!("{b} B");
    } else if b < 1e6 {
        (format!("{:.0}", b / 1e3), "KB")
    } else if b < 1e9 {
        (format!("{:.1}", b / 1e6), "MB")
    } else {
        (format!("{:.2}", b / 1e9), "GB")
    };
    let v = if api::lang() == "de" { v.replace('.', ",") } else { v };
    format!("{v} {u}")
}

fn retention_label(r: &str) -> &'static str {
    match r {
        "oneDay" => t("Aufbewahrung: 1 Tag", "Keeping 1 day"),
        "oneWeek" => t("Aufbewahrung: 1 Woche", "Keeping 1 week"),
        "twoWeeks" => t("Aufbewahrung: 2 Wochen", "Keeping 2 weeks"),
        "threeMonths" => t("Aufbewahrung: 3 Monate", "Keeping 3 months"),
        "forever" | "unlimited" => t("Aufbewahrung: unbegrenzt", "Keeping forever"),
        _ => t("Aufbewahrung: 1 Monat", "Keeping 1 month"),
    }
}

// ───────────────────────────── Auswahl ─────────────────────────────

impl Ctx {
    fn ordered_selection(&self) -> Vec<String> {
        let sel = self.selection.get_untracked();
        self.visible.get_untracked().into_iter().filter(|i| sel.contains(&i.id)).map(|i| i.id).collect()
    }

    /// Ausgewählte IDs, falls `id` dazugehört, sonst nur `id` (für Ziehen/Kontextmenü).
    fn targets(&self, id: &str) -> Vec<String> {
        if self.selection.with_untracked(|s| s.contains(id)) {
            self.ordered_selection()
        } else {
            vec![id.to_owned()]
        }
    }

    fn select(&self, id: &str, meta: bool, shift: bool) {
        let ids: Vec<String> = self.visible.get_untracked().into_iter().map(|i| i.id).collect();
        if meta {
            self.selection.update(|s| {
                if !s.remove(id) {
                    s.insert(id.to_owned());
                }
            });
            self.anchor.set(Some(id.to_owned()));
        } else if let (true, Some(a)) = (shift, self.anchor.get_untracked()) {
            if let (Some(from), Some(to)) = (ids.iter().position(|x| *x == a), ids.iter().position(|x| x == id)) {
                let (lo, hi) = (from.min(to), from.max(to));
                self.selection.set(ids[lo..=hi].iter().cloned().collect());
            }
        } else {
            self.selection.set([id.to_owned()].into());
            self.anchor.set(Some(id.to_owned()));
        }
    }

    fn move_selection(&self, step: isize, extend: bool) {
        let ids: Vec<String> = self.visible.get_untracked().into_iter().map(|i| i.id).collect();
        if ids.is_empty() {
            return;
        }
        let Some(cur) = self.anchor.get_untracked().and_then(|a| ids.iter().position(|x| *x == a)) else {
            self.selection.set([ids[0].clone()].into());
            self.anchor.set(Some(ids[0].clone()));
            return;
        };
        let next = ids[(cur as isize + step).clamp(0, ids.len() as isize - 1) as usize].clone();
        if extend {
            self.selection.update(|s| {
                s.insert(next.clone());
            });
        } else {
            self.selection.set([next.clone()].into());
        }
        self.anchor.set(Some(next.clone()));
        if let Some(el) = document().get_element_by_id(&format!("h-{next}")) {
            el.scroll_into_view_with_bool(false);
        }
    }

    fn ask_delete(&self, ids: Vec<String>) {
        if !ids.is_empty() {
            self.confirm.set(Some(ids));
        }
    }
}

fn columns() -> isize {
    let w = window().inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(760.0);
    (((w - 32.0 + 12.0) / (165.0 + 12.0)).floor() as isize).max(1)
}

// ───────────────────────────── Fenster ─────────────────────────────

#[component]
fn History() -> impl IntoView {
    let data = RwSignal::new(ListData::default());
    let filter = RwSignal::new("all");
    let query = RwSignal::new(String::new());
    let visible = Memo::new(move |_| {
        let f = filter.get();
        let q = query.get().trim().to_lowercase();
        let now = js_sys::Date::now();
        data.with(|d| {
            d.entries
                .iter()
                .filter(|i| match f {
                    "screenshots" => i.kind == "screenshot",
                    "recordings" => i.kind == "recording",
                    "gifs" => i.kind == "gif",
                    _ => true,
                })
                .filter(|i| {
                    q.is_empty()
                        || i.display_name().to_lowercase().contains(&q)
                        || group_label(i.created_at, now).to_lowercase().contains(&q)
                        || format!("{}×{}", i.width, i.height).contains(&q)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    let ctx = Ctx {
        data,
        filter,
        query,
        selection: RwSignal::new(HashSet::new()),
        anchor: RwSignal::new(None),
        uploading: RwSignal::new(HashSet::new()),
        confirm: RwSignal::new(None),
        confirm_clear: RwSignal::new(false),
        menu: RwSignal::new(None),
        visible,
    };
    provide_context(ctx);

    load(data);
    api::on("history-changed", move |_| load(data));
    api::on("window-data", move |_| load(data));
    api::on("share-state", move |v: Value| {
        let path = v["path"].as_str().unwrap_or("").to_owned();
        match v["state"].as_str() {
            Some("uploading") => ctx.uploading.update(|s| {
                s.insert(path);
            }),
            _ => {
                ctx.uploading.update(|s| {
                    s.remove(&path);
                });
                load(data);
            }
        }
    });
    // Relative Zeiten aktuell halten.
    let tick = RwSignal::new(0u32);
    let _ = set_interval_with_handle(move || tick.update(|n| *n += 1), std::time::Duration::from_secs(30));

    let _keys = window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| {
        let in_input = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA"));
        if ctx.confirm.get_untracked().is_some() || ctx.confirm_clear.get_untracked() {
            if e.key() == "Escape" {
                ctx.confirm.set(None);
                ctx.confirm_clear.set(false);
            }
            return;
        }
        if in_input {
            if e.key() == "Escape" {
                ctx.query.set(String::new());
            }
            return;
        }
        let meta = e.meta_key();
        let sel = ctx.ordered_selection();
        match e.key().as_str() {
            "ArrowLeft" => ctx.move_selection(-1, e.shift_key()),
            "ArrowRight" => ctx.move_selection(1, e.shift_key()),
            "ArrowUp" => ctx.move_selection(-columns(), e.shift_key()),
            "ArrowDown" => ctx.move_selection(columns(), e.shift_key()),
            " " if !sel.is_empty() => act_many("quickLook", sel),
            "Backspace" | "Delete" if !sel.is_empty() => ctx.ask_delete(sel),
            "Enter" if sel.len() == 1 => act("edit", &sel[0]),
            "Escape" => {
                ctx.menu.set(None);
                ctx.selection.set(HashSet::new());
            }
            "a" if meta => ctx.selection.set(ctx.visible.get_untracked().into_iter().map(|i| i.id).collect()),
            "c" if meta && sel.len() == 1 => act("copy", &sel[0]),
            "f" if meta => {
                if let Some(el) = document().get_element_by_id("history-search").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) {
                    let _ = el.focus();
                }
            }
            _ => return,
        }
        e.prevent_default();
    });
    on_cleanup(move || drop(_keys));

    let groups = Memo::new(move |_| {
        tick.track();
        let now = js_sys::Date::now();
        let mut out: Vec<(String, Vec<Item>)> = vec![];
        for i in visible.get() {
            let k = group_label(i.created_at, now);
            match out.last_mut() {
                Some((g, l)) if *g == k => l.push(i),
                _ => out.push((k, vec![i])),
            }
        }
        out
    });

    view! {
        <div class="app-window flex h-screen flex-col" on:click=move |_| ctx.menu.set(None)>
            <div data-tauri-drag-region class="flex h-7 shrink-0 items-center justify-center text-[13px] font-semibold text-black/70 dark:text-white/70">
                <span data-tauri-drag-region>{t("Bildschirmfoto-Verlauf", "Screenshot History")}</span>
            </div>
            <FilterBar />
            <div class="h-px shrink-0 bg-black/10 dark:bg-white/10" />
            <div class="min-h-0 flex-1 overflow-y-auto" on:click=move |_| ctx.selection.set(HashSet::new())>
                <Show when=move || !visible.with(Vec::is_empty) fallback=move || view! { <EmptyState /> }>
                    <div class="flex flex-col gap-4 p-4">
                        <For each=move || groups.get() key=|(k, l)| (k.clone(), l.iter().map(|i| (i.id.clone(), i.cloud_url.clone(), i.file_size as u64)).collect::<Vec<_>>()) let:(g)>
                            <Section title=g.0 entries=g.1 tick=tick />
                        </For>
                    </div>
                </Show>
            </div>
            <div class="h-px shrink-0 bg-black/10 dark:bg-white/10" />
            <StatusBar />
            <ContextMenu />
            <DeleteDialog />
        </div>
    }
}

#[component]
fn FilterBar() -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let btn = move |label: &'static str, key: &'static str, count: fn(&Counts) -> usize| {
        let active = move || ctx.filter.get() == key;
        view! {
            <button
                class=move || format!(
                    "flex items-center gap-1 rounded-[5px] px-3 py-[3px] text-[12px] font-medium transition-colors {}",
                    if active() { "bg-white text-black shadow-sm dark:bg-white/15 dark:text-white" } else { "text-black/55 hover:text-black/80 dark:text-white/55 dark:hover:text-white/80" }
                )
                on:click=move |_| { ctx.filter.set(key); ctx.selection.set(HashSet::new()); }
            >
                {label}
                <span class=move || format!("text-[10px] font-semibold {}", if active() { "opacity-60" } else { "opacity-40" })>
                    {move || ctx.data.with(|d| count(&d.counts))}
                </span>
            </button>
        }
    };
    view! {
        <div class="flex shrink-0 items-center gap-2 px-4 py-2">
            <div class="flex gap-px rounded-md bg-black/5 p-0.5 dark:bg-white/10">
                {btn(t("Alle", "All"), "all", |c| c.all)}
                {btn(t("Bildschirmfotos", "Screenshots"), "screenshots", |c| c.screenshots)}
                {btn(t("Aufnahmen", "Recordings"), "recordings", |c| c.recordings)}
                {btn("GIFs", "gifs", |c| c.gifs)}
            </div>
            <div class="flex-1" />
            <div class="relative w-44">
                <span class="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-black/40 dark:text-white/40 [&_svg]:size-3.5"><Icon d=SEARCH /></span>
                <input id="history-search" type="search" placeholder=t("Suchen", "Search")
                    class="h-7 w-full rounded-md border border-black/10 bg-white/80 pl-7 pr-2 text-[12px] outline-none focus:ring-2 focus:ring-blue-500/40 dark:border-white/10 dark:bg-white/10"
                    prop:value=move || ctx.query.get()
                    on:input=move |e| ctx.query.set(event_target_value(&e)) />
            </div>
            <button title=t("Verlauf löschen", "Clear History")
                class="inline-flex size-7 items-center justify-center rounded-md text-black/50 hover:bg-black/5 hover:text-black/80 disabled:opacity-30 dark:text-white/50 dark:hover:bg-white/10 dark:hover:text-white/80 [&_svg]:size-3.5"
                disabled=move || ctx.data.with(|d| d.entries.is_empty())
                on:click=move |_| ctx.confirm_clear.set(true)>
                <Icon d=TRASH />
            </button>
        </div>
    }
}

#[component]
fn Section(title: String, entries: Vec<Item>, tick: RwSignal<u32>) -> impl IntoView {
    let count = entries.len();
    let shots: Vec<String> = entries.iter().filter(|i| i.is_screenshot()).map(|i| i.id.clone()).collect();
    let has_shots = !shots.is_empty();
    view! {
        <section class="flex flex-col gap-2.5">
            <div class="flex items-center gap-2.5">
                <span class="text-[11px] font-semibold uppercase tracking-wide text-black/50 dark:text-white/50">{title}</span>
                <div class="h-px flex-1 bg-black/10 dark:bg-white/10" />
                <span class="text-[11px] text-black/35 dark:text-white/35">{count}</span>
                {has_shots.then(|| {
                    let shots = shots.clone();
                    view! {
                        <button title=t("Bildschirmfotos als PDF exportieren", "Export Screenshots as PDF")
                            class="text-black/45 hover:text-black/80 dark:text-white/45 dark:hover:text-white/80 [&_svg]:size-3.5"
                            on:click=move |e| { e.stop_propagation(); act_many("exportPdf", shots.clone()) }>
                            <Icon d=FILE_TEXT />
                        </button>
                    }
                })}
            </div>
            <div class="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(165px,1fr))]">
                {entries.into_iter().map(|i| view! { <Card item=i tick=tick /> }).collect_view()}
            </div>
        </section>
    }
}

#[component]
fn HoverButton(#[prop(into)] title: String, d: &'static str, on_click: impl Fn() + 'static) -> impl IntoView {
    view! {
        <button title=title
            class="inline-flex size-[30px] items-center justify-center rounded-[7px] border border-white/15 bg-black/55 text-white/90 backdrop-blur hover:bg-black/70 [&_svg]:size-3.5"
            on:click=move |e| { e.stop_propagation(); on_click() }>
            <Icon d=d />
        </button>
    }
}

#[component]
fn Card(item: Item, tick: RwSignal<u32>) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let id = item.id.clone();
    let selected = {
        let id = id.clone();
        Memo::new(move |_| ctx.selection.with(|s| s.contains(&id)))
    };
    let uploading = {
        let (p, u) = (item.path.clone(), item.uploading);
        Memo::new(move |_| u || ctx.uploading.with(|s| s.contains(&p)))
    };
    // Modus-Badge wie Capso `HistoryItemView.modeBadge`.
    let (badge, color) = match (item.kind.as_str(), item.capture_mode.as_str()) {
        ("recording", _) => ("Video", "bg-red-500/85"),
        ("gif", _) => ("GIF", "bg-orange-500/85"),
        (_, "area") => (t("Bereich", "Area"), "bg-blue-500/85"),
        (_, "fullscreen") => (t("Voll", "Full"), "bg-blue-500/85"),
        (_, "window") => (t("Fenster", "Window"), "bg-blue-500/85"),
        (_, "scroll") => ("Scroll", "bg-blue-500/85"),
        _ => (t("Bild", "Image"), "bg-blue-500/85"),
    };
    let created = item.created_at;
    let time = move || {
        tick.track();
        relative_time(created, js_sys::Date::now())
    };
    let dims = if item.width > 0 { format!("{} × {}", item.width, item.height) } else { String::new() };
    let src = api::asset_url(&item.thumb_path);
    let has_cloud = item.cloud_url.is_some();
    let cloud_ok = ctx.data.with_untracked(|d| d.cloud_configured);
    let (id_c, id_s, id_e, id_u, id_l) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    let (id_click, id_dbl, id_drag, id_menu) = (id.clone(), id.clone(), id.clone(), id.clone());
    view! {
        <div id=format!("h-{id}")
            class=move || format!(
                "group relative flex flex-col overflow-hidden rounded-lg border bg-black/[0.03] transition-all duration-150 hover:scale-[1.02] hover:bg-black/[0.06] hover:shadow-lg dark:bg-white/[0.03] dark:hover:bg-white/[0.06] {}",
                if selected.get() { "border-blue-500 ring-2 ring-blue-500" } else { "border-black/5 dark:border-white/5" }
            )
            draggable="true"
            on:click=move |e| { e.stop_propagation(); ctx.menu.set(None); ctx.select(&id_click, e.meta_key(), e.shift_key()) }
            on:dblclick=move |_| act("edit", &id_dbl)
            on:dragstart=move |e| { e.prevent_default(); act_many("drag", ctx.targets(&id_drag)) }
            on:contextmenu=move |e| {
                e.prevent_default();
                e.stop_propagation();
                if !selected.get_untracked() { ctx.select(&id_menu, false, false) }
                ctx.menu.set(Some((e.client_x() as f64, e.client_y() as f64, id_menu.clone())));
            }
        >
            <div class="relative aspect-[16/10] w-full overflow-hidden rounded-md bg-black/5 dark:bg-white/5">
                <img src=src draggable="false" loading="lazy" class="size-full object-contain"
                    on:error=move |e| { if let Some(el) = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok()) { let _ = el.style().set_property("visibility", "hidden"); } } />
                <span class=format!("absolute right-1.5 top-1.5 rounded px-1.5 py-px text-[9px] font-semibold uppercase tracking-wide text-white {color}")>{badge}</span>
                <div class="absolute inset-0 flex items-center justify-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                    <HoverButton title=t("In die Zwischenablage kopieren", "Copy to Clipboard") d=COPY on_click=move || act("copy", &id_c) />
                    <HoverButton title=t("Sichern", "Save") d=DOWNLOAD on_click=move || act("save", &id_s) />
                    <HoverButton title=t("Bearbeiten", "Edit") d=PENCIL on_click=move || act("edit", &id_e) />
                    {move || if has_cloud {
                        let id = id_l.clone();
                        Some(view! { <HoverButton title=t("Cloud-Link kopieren", "Copy cloud link") d=LINK on_click=move || act("copyLink", &id) /> }.into_any())
                    } else if uploading.get() {
                        Some(view! {
                            <span class="inline-flex size-[30px] items-center justify-center rounded-[7px] bg-black/55 text-white [&_svg]:size-3.5 [&_svg]:animate-spin"><Icon d=SPINNER /></span>
                        }.into_any())
                    } else if cloud_ok {
                        let id = id_u.clone();
                        Some(view! { <HoverButton title=t("In die Cloud hochladen", "Upload to cloud") d=CLOUD_UPLOAD on_click=move || act("upload", &id) /> }.into_any())
                    } else {
                        None
                    }}
                </div>
            </div>
            <div class="flex flex-col gap-0.5 px-2 py-[7px]">
                <div class="flex items-center gap-1">
                    <span class="min-w-0 flex-1 truncate text-[11.5px] font-medium" title=item.source_window_title.clone().unwrap_or_default()>{item.display_name()}</span>
                    {has_cloud.then(|| view! {
                        <span title=t("In die Cloud hochgeladen", "Uploaded to cloud") class="text-blue-500/70 [&_svg]:size-3"><Icon d=CLOUD_CHECK /></span>
                    })}
                </div>
                <div class="flex items-center gap-1 text-[10.5px] text-black/40 dark:text-white/40">
                    <span>{time}</span>
                    {(!dims.is_empty()).then(|| view! { <span class="size-0.5 rounded-full bg-current" /><span>{dims.clone()}</span> })}
                </div>
            </div>
        </div>
    }
}

#[component]
fn ContextMenu() -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    move || {
        let (x, y, id) = ctx.menu.get()?;
        let item = ctx.data.with_untracked(|d| d.entries.iter().find(|i| i.id == id).cloned())?;
        let targets = ctx.targets(&id);
        let multi = targets.len() > 1;
        let cloud_ok = ctx.data.with_untracked(|d| d.cloud_configured);
        let (w, h) = (
            window().inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(760.0),
            window().inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(560.0),
        );
        let (x, y) = (x.min(w - 216.0).max(4.0), y.min(h - 330.0).max(4.0));
        let entry = move |label: &'static str, f: Box<dyn Fn()>| {
            view! {
                <button class="flex w-full items-center rounded px-2.5 py-1 text-left text-[13px] hover:bg-blue-500 hover:text-white"
                    on:click=move |e| { e.stop_propagation(); ctx.menu.set(None); f() }>{label}</button>
            }
        };
        let sep = || view! { <div class="mx-2 my-1 h-px bg-black/10 dark:bg-white/10" /> };
        let id0 = id.clone();
        let mk = |cmd: &'static str| {
            let id = id0.clone();
            Box::new(move || act(cmd, &id)) as Box<dyn Fn()>
        };
        let t_save = targets.clone();
        let t_pdf = targets.clone();
        let t_ql = targets.clone();
        let t_del = targets.clone();
        let any_shot = ctx.data.with_untracked(|d| d.entries.iter().any(|i| targets.contains(&i.id) && i.is_screenshot()));
        Some(view! {
            <div class="fixed z-50 w-[210px] rounded-lg border border-black/10 bg-white/95 p-1 text-black shadow-xl backdrop-blur dark:border-white/10 dark:bg-zinc-800/95 dark:text-white"
                style=format!("left:{x}px;top:{y}px")
                on:click=|e| e.stop_propagation()>
                {(!multi).then(|| view! {
                    {entry(t("In die Zwischenablage kopieren", "Copy to Clipboard"), mk("copy"))}
                })}
                {entry(if multi { t("Alle sichern", "Save All") } else { t("Sichern", "Save") }, Box::new(move || act_many("save", t_save.clone())))}
                {(!multi).then(|| view! {
                    {entry(t("Sichern unter …", "Save to…"), mk("saveAs"))}
                    {entry(t("Bearbeiten", "Edit"), mk("edit"))}
                })}
                {(!multi && item.kind != "recording").then(|| entry(t("Anheften", "Pin to Screen"), mk("pin")))}
                {entry(t("Übersicht", "Quick Look"), Box::new(move || act_many("quickLook", t_ql.clone())))}
                {any_shot.then(|| entry(t("Als PDF exportieren", "Export as PDF"), Box::new(move || act_many("exportPdf", t_pdf.clone()))))}
                {(!multi).then(|| {
                    if item.cloud_url.is_some() {
                        Some(entry(t("Cloud-Link kopieren", "Copy Cloud Link"), mk("copyLink")))
                    } else if cloud_ok {
                        Some(entry(t("In die Cloud hochladen", "Upload to Cloud"), mk("upload")))
                    } else {
                        None
                    }
                })}
                {sep()}
                {(!multi).then(|| entry(t("Im Finder zeigen", "Show in Finder"), mk("reveal")))}
                {(!multi).then(sep)}
                <button class="flex w-full items-center rounded px-2.5 py-1 text-left text-[13px] text-red-600 hover:bg-red-500 hover:text-white dark:text-red-400"
                    on:click=move |e| { e.stop_propagation(); ctx.menu.set(None); ctx.ask_delete(t_del.clone()) }>
                    {if multi { t("Auswahl aus dem Verlauf löschen", "Delete Selection from History") } else { t("Aus dem Verlauf löschen", "Delete from History") }}
                </button>
            </div>
        })
    }
}

#[component]
fn DeleteDialog() -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let also_cloud = RwSignal::new(true);
    move || {
        let clear = ctx.confirm_clear.get();
        let ids = ctx.confirm.get();
        if !clear && ids.is_none() {
            return None;
        }
        let ids = ids.unwrap_or_default();
        let n = if clear { ctx.data.with_untracked(|d| d.entries.len()) } else { ids.len() };
        let has_cloud = ctx.data.with_untracked(|d| d.entries.iter().any(|i| (clear || ids.contains(&i.id)) && i.cloud_url.is_some()));
        also_cloud.set(true);
        let title = match (clear, n) {
            (true, _) => t("Gesamten Verlauf löschen?", "Clear entire history?").to_owned(),
            (_, 1) => t("Aufnahme löschen?", "Delete capture?").to_owned(),
            _ => if api::lang() == "de" { format!("{n} Aufnahmen löschen?") } else { format!("Delete {n} captures?") },
        };
        let close = move || {
            ctx.confirm.set(None);
            ctx.confirm_clear.set(false);
        };
        let ids2 = ids.clone();
        let confirm = move || {
            let cloud = has_cloud && also_cloud.get_untracked();
            let ids = if clear { ctx.data.with_untracked(|d| d.entries.iter().map(|i| i.id.clone()).collect()) } else { ids2.clone() };
            // Nächsten Eintrag auswählen (wie Capso).
            let all: Vec<String> = ctx.visible.get_untracked().into_iter().map(|i| i.id).collect();
            let next = all.iter().position(|x| ids.contains(x)).and_then(|f| all[f..].iter().find(|x| !ids.contains(x)).cloned());
            ctx.selection.set(next.iter().cloned().collect());
            ctx.anchor.set(next);
            api::fire("history", "delete", json!({ "ids": ids, "cloud": cloud }));
            close();
        };
        Some(view! {
            <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/25" on:click=move |e| { e.stop_propagation(); close() }>
                <div class="w-[300px] rounded-xl border border-black/10 bg-white p-4 text-center shadow-2xl dark:border-white/10 dark:bg-zinc-800" on:click=|e| e.stop_propagation()>
                    <div class="mx-auto mb-2 flex size-10 items-center justify-center rounded-full bg-amber-500/15 text-amber-500 [&_svg]:size-5"><Icon d=TRASH /></div>
                    <div class="text-[13px] font-semibold">{title}</div>
                    <div class="mt-1 text-[11px] text-black/55 dark:text-white/55">
                        {t("Die Aufnahme wird von diesem Mac entfernt.", "This will remove the capture from this Mac.")}
                    </div>
                    {has_cloud.then(|| view! {
                        <label class="mt-3 flex items-center justify-center gap-2 text-[12px]">
                            <input type="checkbox" class="accent-blue-500" prop:checked=move || also_cloud.get()
                                on:change=move |e| also_cloud.set(event_target_checked(&e)) />
                            {t("Auch aus der Cloud löschen", "Also delete from cloud")}
                        </label>
                    })}
                    <div class="mt-4 flex gap-2">
                        <div class="flex-1 [&>button]:w-full"><TextButton on_click=close>{t("Abbrechen", "Cancel")}</TextButton></div>
                        <div class="flex-1 [&>button]:w-full"><TextButton variant="danger" on_click=confirm.clone()>{t("Löschen", "Delete")}</TextButton></div>
                    </div>
                </div>
            </div>
        })
    }
}

#[component]
fn StatusBar() -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let n_sel = move || ctx.selection.with(HashSet::len);
    view! {
        <div class="flex h-[26px] shrink-0 items-center gap-1.5 px-4 text-[11px] text-black/40 dark:text-white/40">
            <span>{move || {
                let n = ctx.visible.with(Vec::len);
                if api::lang() == "de" { format!("{n} {}", if n == 1 { "Element" } else { "Elemente" }) } else { format!("{n} {}", if n == 1 { "item" } else { "items" }) }
            }}</span>
            <span class="size-[2.5px] rounded-full bg-current" />
            <span>{move || format_bytes(ctx.data.with(|d| d.total_size))}</span>
            <Show when=move || { n_sel() > 1 }>
                <span class="ml-3 font-medium text-blue-500">{move || if api::lang() == "de" { format!("{} ausgewählt", n_sel()) } else { format!("{} selected", n_sel()) }}</span>
                <button class="ml-1 rounded px-1.5 hover:bg-black/5 hover:text-black/80 dark:hover:bg-white/10 dark:hover:text-white/80"
                    on:click=move |_| act_many("save", ctx.ordered_selection())>{t("Sichern", "Save")}</button>
                <button class="rounded px-1.5 hover:bg-black/5 hover:text-black/80 dark:hover:bg-white/10 dark:hover:text-white/80"
                    on:click=move |_| act_many("exportPdf", ctx.ordered_selection())>"PDF"</button>
                <button class="rounded px-1.5 text-red-500 hover:bg-red-500/10"
                    on:click=move |_| ctx.ask_delete(ctx.ordered_selection())>{t("Löschen", "Delete")}</button>
            </Show>
            <div class="flex-1" />
            <span class="opacity-70">{move || {
                if !api::setting_bool("historyEnabled") && api::settings().with(|s| !s.is_null()) {
                    t("Verlauf ist deaktiviert", "History is disabled")
                } else {
                    retention_label(&api::setting_str("historyRetention"))
                }
            }}</span>
        </div>
    }
}

#[component]
fn EmptyState() -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let searching = move || !ctx.query.get().trim().is_empty() || ctx.data.with(|d| !d.entries.is_empty());
    view! {
        <div class="flex h-full flex-col items-center justify-center gap-2 text-center">
            <span class="mb-1 text-black/25 dark:text-white/25 [&_svg]:size-7"><Icon d=Signal::derive(move || if searching() { IMAGE } else { HISTORY }) /></span>
            <div class="text-[14px] font-medium text-black/55 dark:text-white/55">
                {move || if searching() { t("Keine Treffer", "No matches") } else { t("Noch keine Aufnahmen", "No captures yet") }}
            </div>
            <div class="text-[12px] text-black/35 dark:text-white/35">
                {move || if searching() {
                    t("Ändere Filter oder Suchbegriff.", "Try a different filter or search term.")
                } else if !api::setting_bool("historyEnabled") && api::settings().with(|s| !s.is_null()) {
                    t("Der Verlauf ist in den Einstellungen deaktiviert.", "History is turned off in Settings.")
                } else {
                    t("Deine Bildschirmfotos und Aufnahmen erscheinen hier", "Your screenshots and recordings will appear here")
                }}
            </div>
        </div>
    }
}
