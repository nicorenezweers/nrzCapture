//! Schwebende Steuerleiste (Capso `RecordingControlsView`: Stopp, Zeit, Pause/Fortsetzen, Neustart, Löschen)
//! und Warnhinweis (Capso `RecordingWarningPanel`).
use super::*;
use crate::{api, ui::*};
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

#[component]
fn Round(#[prop(into)] title: Signal<String>, #[prop(into)] icon: Signal<&'static str>, cmd: &'static str) -> impl IntoView {
    view! {
        <button title=move || title.get() class="flex size-7 items-center justify-center rounded-full bg-white/5 text-white/85 hover:bg-white/15 [&_svg]:size-3.5"
            on:click=move |_| api::fire("record", cmd, Value::Null)>
            <Icon d=icon />
        </button>
    }
}

#[component]
pub fn Controls() -> impl IntoView {
    let elapsed = RwSignal::new(0.0);
    let paused = RwSignal::new(false);
    let apply = move |v: Value| {
        elapsed.set(v["elapsed"].as_f64().unwrap_or(0.0));
        paused.set(v["paused"].as_bool().unwrap_or(false));
    };
    spawn_local(async move {
        if let Ok(v) = api::rpc("record", "state", Value::Null).await {
            apply(v);
        }
    });
    api::on("rec-state", apply);
    view! {
        <div class="flex h-screen w-screen items-center justify-center">
            <div class="flex items-center gap-2.5 rounded-full border border-white/10 bg-black/80 px-3.5 py-2.5 shadow-[0_4px_24px_rgba(0,0,0,0.35)] backdrop-blur-xl">
                <button title=api::t("Aufnahme beenden", "Stop recording")
                    class="flex size-7 items-center justify-center rounded-full bg-red-500 text-white hover:bg-red-600 [&_svg]:size-3 [&_svg]:fill-current"
                    on:click=move |_| api::fire("record", "stop", Value::Null)>
                    <Icon d=STOP />
                </button>
                <span class=move || format!("min-w-[52px] font-mono text-[14px] font-medium tabular-nums {}", if paused.get() { "text-white/50" } else { "text-white" })>
                    {move || format_time(elapsed.get())}
                </span>
                <div class="h-[18px] w-px bg-white/20"></div>
                <Round title=Signal::derive(move || if paused.get() { api::t("Fortsetzen", "Resume").to_owned() } else { api::t("Pause", "Pause").to_owned() })
                    icon=Signal::derive(move || if paused.get() { PLAY } else { PAUSE }) cmd="togglePause" />
                <Round title=api::t("Neu starten", "Restart recording") icon=RESTART cmd="restart" />
                <Round title=api::t("Aufnahme löschen", "Delete recording") icon=TRASH cmd="delete" />
            </div>
        </div>
    }
}

#[component]
pub fn Warning() -> impl IntoView {
    let data = RwSignal::new(Value::Null);
    spawn_local(async move { data.set(api::window_data().await) });
    api::on("window-data", move |v| data.set(v));
    let act = move |action: &'static str| {
        let kind = data.with_untracked(|d| d["kind"].as_str().unwrap_or("").to_owned());
        api::fire("record", "warningAction", json!({ "action": action, "kind": kind }));
    };
    let silent = move || data.with(|d| d["kind"] == "micSilent");
    view! {
        <div class="flex h-screen w-screen items-end justify-center p-1">
            <div class="w-full rounded-xl border border-white/10 bg-zinc-900/90 p-3 text-white shadow-xl backdrop-blur-xl">
                <div class="flex gap-2.5">
                    <span class="mt-0.5 text-amber-400 [&_svg]:size-4"><Icon d=ALERT /></span>
                    <p class="text-[12px] leading-snug">{move || data.with(|d| d["message"].as_str().unwrap_or("").to_owned())}</p>
                </div>
                <div class="mt-2.5 flex justify-end gap-1.5">
                    <Show when=silent>
                        <TextButton variant="ghost" on_click=move || act("dontWarn")>{api::t("Nicht mehr warnen", "Don't Warn Again")}</TextButton>
                    </Show>
                    <TextButton variant="ghost" on_click=move || act("ignore")>{api::t("Ignorieren", "Ignore")}</TextButton>
                    <TextButton variant="danger" on_click=move || act("stop")>{api::t("Aufnahme beenden", "Stop Recording")}</TextButton>
                </div>
            </div>
        </div>
    }
}
