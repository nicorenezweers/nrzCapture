//! Beautify-Panel (Port von Capso `BeautifyPanel`): Hintergrund, Abstand, Ecken, Schatten, Seitenverhältnis.
//! Jede Änderung wird live angezeigt und als Einstellung `beautifyStyle` gespeichert (Format: siehe `model::Beautify`).
use super::canvas::Ed;
use super::model::*;
use crate::api::{self, t};
use crate::ui::Toggle;
use leptos::prelude::*;

fn save(ed: Ed) {
    api::set_setting("beautifyStyle", serde_json::to_value(ed.beautify.get_untracked()).unwrap_or_default());
}

fn hex4(c: [f64; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8)
}

fn parse_hex4(s: &str) -> Option<[f64; 4]> {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).ok()?;
    Some([((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0, 1.0])
}

#[component]
fn SliderRow(ed: Ed, label: (&'static str, &'static str), max: f64, get: fn(&Beautify) -> f64, set: fn(&mut Beautify, f64)) -> impl IntoView {
    view! {
        <label class="flex items-center gap-2 text-[12px]">
            <span class="text-zinc-500">{move || t(label.0, label.1)}</span>
            <input type="range" min="0" max=max step="1" class="w-24 accent-blue-500"
                prop:value=move || ed.beautify.with(get).to_string()
                on:input=move |e| { if let Ok(v) = event_target_value(&e).parse::<f64>() { ed.beautify.update(|b| set(b, v)) } }
                on:change=move |_| save(ed) />
            <span class="w-6 text-right tabular-nums text-zinc-500">{move || (ed.beautify.with(get) as i64).to_string()}</span>
        </label>
    }
}

#[component]
pub fn BeautifyPanel(ed: Ed) -> impl IntoView {
    let enabled = Signal::derive(move || ed.beautify.with(|b| b.enabled));
    let style = move || ed.beautify.with(|b| b.background_style.clone());
    let seg = move |id: &'static str, de: &'static str, en: &'static str| {
        view! {
            <button class=move || format!("h-6 rounded px-2 text-[12px] {}", if style() == id { "bg-white dark:bg-white/20 shadow-sm font-medium" } else { "text-zinc-500" })
                on:click=move |_| { ed.beautify.update(|b| b.background_style = id.into()); save(ed); }>
                {move || t(de, en)}
            </button>
        }
    };
    let colors = move || BEAUTIFY_COLORS
        .iter()
        .map(|&c| {
            let sel = move || ed.beautify.with(|b| b.background_color == c);
            view! {
                <button class=move || format!("size-[18px] rounded-full border border-black/15 {}", if sel() { "ring-2 ring-blue-500 ring-offset-1" } else { "" })
                    style:background=css_rgba4(c)
                    on:click=move |_| { ed.beautify.update(|b| b.background_color = c); save(ed); }></button>
            }
        })
        .collect_view();
    let gradients = move || GRADIENTS
        .iter()
        .map(|&g| {
            let sel = move || ed.beautify.with(|b| b.gradient == g);
            view! {
                <button class=move || format!("size-[18px] rounded-full border border-black/15 {}", if sel() { "ring-2 ring-blue-500 ring-offset-1" } else { "" })
                    style:background=format!("linear-gradient(135deg, {}, {})", css_rgba4(g[0]), css_rgba4(g[1]))
                    on:click=move |_| { ed.beautify.update(|b| b.gradient = g); save(ed); }></button>
            }
        })
        .collect_view();
    view! {
        <div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-black/10 dark:border-white/10 bg-white/60 dark:bg-zinc-800/60 px-3 py-2">
            <label class="flex items-center gap-2 text-[12px]">
                <Toggle checked=enabled on_change=move |v| ed.beautify.update(|b| b.enabled = v) />
                {move || t("Hintergrund aktivieren", "Enable background")}
            </label>
            <Show when=move || enabled.get()>
                <div class="flex items-center gap-0.5 rounded-md bg-black/5 dark:bg-white/10 p-0.5">
                    {seg("solid", "Farbe", "Solid")}
                    {seg("gradient", "Verlauf", "Gradient")}
                    {seg("liquidGlass", "Liquid Glass", "Liquid Glass")}
                    {seg("none", "Ohne", "None")}
                </div>
                <Show when=move || style() == "solid">
                    <div class="flex items-center gap-1.5">
                        {colors()}
                        <input type="color" class="size-6 cursor-pointer rounded border-0 bg-transparent p-0" title=move || t("Eigene Farbe", "Custom Color")
                            prop:value=move || ed.beautify.with(|b| hex4(b.background_color))
                            on:input=move |e| { if let Some(c) = parse_hex4(&event_target_value(&e)) { ed.beautify.update(|b| b.background_color = c) } }
                            on:change=move |_| save(ed) />
                    </div>
                </Show>
                <Show when=move || style() == "gradient">
                    <div class="flex items-center gap-1.5">{gradients()}</div>
                </Show>
                <SliderRow ed=ed label=("Abstand", "Padding") max=80.0 get=|b| b.padding set=|b, v| b.padding = v />
                <SliderRow ed=ed label=("Ecken", "Corners") max=24.0 get=|b| b.corner_radius set=|b, v| b.corner_radius = v />
                <label class="flex items-center gap-2 text-[12px]">
                    <input type="checkbox" class="accent-blue-500" prop:checked=move || ed.beautify.with(|b| b.shadow_enabled)
                        on:change=move |e| { let v = event_target_checked(&e); ed.beautify.update(|b| b.shadow_enabled = v); save(ed); } />
                    {move || t("Schatten", "Shadow")}
                    <input type="range" min="0" max="40" step="1" class="w-20 accent-blue-500"
                        disabled=move || !ed.beautify.with(|b| b.shadow_enabled)
                        prop:value=move || ed.beautify.with(|b| b.shadow_radius).to_string()
                        on:input=move |e| { if let Ok(v) = event_target_value(&e).parse::<f64>() { ed.beautify.update(|b| b.shadow_radius = v) } }
                        on:change=move |_| save(ed) />
                </label>
                <label class="flex items-center gap-2 text-[12px]">
                    <span class="text-zinc-500">{move || t("Seitenverhältnis", "Aspect Ratio")}</span>
                    <select class="h-6 rounded-md border border-black/10 dark:border-white/15 bg-white/80 dark:bg-white/10 px-1 text-[12px]"
                        on:change=move |e| { let v = event_target_value(&e); ed.beautify.update(|b| b.aspect = v); save(ed); }>
                        {BEAUTIFY_ASPECTS.iter().map(|&a| view! {
                            <option value=a selected=move || ed.beautify.with(|b| b.aspect == a)>
                                {move || if a == "auto" { t("Automatisch", "Auto").to_string() } else { a.replace(':', " : ") }}
                            </option>
                        }).collect_view()}
                    </select>
                </label>
            </Show>
        </div>
    }
}
