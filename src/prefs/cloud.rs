//! Tab „Cloud-Freigabe“ + geführter Einrichtungs-Assistent (Capso `CloudShareSettingsView` / `CloudShareWizardView`).
//! Backend-Befehle `share.testConnection` (Ok = Erfolg, Err = Fehlertext) und `share.reset` liefert das HISTORY-Modul.
use leptos::{prelude::*, task::spawn_local};
use serde_json::{json, Value};

use super::widgets::*;
use crate::{api, api::t, ui};

const PROVIDERS: &[&str] = &["r2", "s3", "tencentCOS", "aliyunOSS"];

fn provider_name(p: &str) -> &'static str {
    match p {
        "r2" => "Cloudflare R2",
        "s3" => "Amazon S3",
        "tencentCOS" => "Tencent COS",
        "aliyunOSS" => "Aliyun OSS",
        _ => "–",
    }
}

fn provider_blurb(p: &str) -> &'static str {
    match p {
        "r2" => t("Kostenloses Kontingent, kein Egress-Entgelt – empfohlen.", "Generous free tier, zero egress fees — recommended."),
        "s3" => t("AWS S3 oder S3-kompatible Dienste (MinIO, Backblaze B2, Wasabi …).", "AWS S3 or S3-compatible services (MinIO, Backblaze B2, Wasabi…)."),
        "tencentCOS" => t("Tencent Cloud Object Storage.", "Tencent Cloud Object Storage."),
        _ => t("Alibaba Cloud Object Storage Service.", "Alibaba Cloud Object Storage Service."),
    }
}

fn dashboard_url(p: &str) -> &'static str {
    match p {
        "r2" => "https://dash.cloudflare.com/?to=/:account/r2/overview",
        "s3" => "https://s3.console.aws.amazon.com/s3/home",
        "tencentCOS" => "https://console.cloud.tencent.com/cos",
        _ => "https://oss.console.aliyun.com/",
    }
}

fn region_label(p: &str) -> &'static str {
    match p {
        "r2" => t("Konto-ID", "Account ID"),
        "aliyunOSS" => t("Endpunkt / Region", "Endpoint / Region"),
        _ => t("Region", "Region"),
    }
}

fn region_placeholder(p: &str) -> &'static str {
    match p {
        "r2" => t("32 Hex-Zeichen", "32-character hex"),
        "s3" => "us-east-1",
        "tencentCOS" => "ap-guangzhou",
        _ => "oss-cn-hangzhou",
    }
}

fn access_label(p: &str) -> &'static str {
    match p {
        "tencentCOS" => "SecretId",
        "aliyunOSS" => "AccessKey ID",
        _ => "Access Key ID",
    }
}

fn secret_label(p: &str) -> &'static str {
    match p {
        "aliyunOSS" => "AccessKey Secret",
        _ => "Secret Access Key",
    }
}

fn opt_str(key: &str) -> String {
    api::setting(key).as_str().unwrap_or("").to_owned()
}

/// Wie Capso `isCloudShareConfigured` (Zugangsdaten liegen im Schlüsselbund).
fn configured() -> bool {
    let ok = |k: &str| !opt_str(k).trim().is_empty();
    let id_ok = match opt_str("cloudShareProvider").as_str() {
        "r2" => ok("cloudShareAccountId"),
        "s3" | "tencentCOS" | "aliyunOSS" => ok("cloudShareRegion"),
        _ => false,
    };
    id_ok && ok("cloudShareUrlPrefix") && ok("cloudShareBucket")
}

/// URL-Präfix prüfen (wie Capso: öffentliches HTTPS).
pub fn url_prefix_error(s: &str) -> Option<&'static str> {
    let s = s.trim();
    if s.is_empty() {
        return Some(t("Öffentliches URL-Präfix fehlt.", "Public URL prefix is missing."));
    }
    if !s.starts_with("https://") {
        return Some(t("Muss mit https:// beginnen.", "Must start with https://."));
    }
    if s.len() <= "https://".len() || s.contains(' ') {
        return Some(t("Ungültige URL.", "Invalid URL."));
    }
    None
}

async fn test_connection() -> Result<String, String> {
    match api::rpc("share", "testConnection", Value::Null).await {
        Ok(v) => Ok(v.as_str().unwrap_or("").to_owned()),
        Err(e) if e.contains("unbekannter Befehl") => Err(t("Verbindungstest ist im Freigabe-Modul noch nicht verfügbar.", "Connection test is not available in the share module yet.").to_owned()),
        Err(e) => Err(e),
    }
}

#[component]
pub fn CloudTab() -> impl IntoView {
    let wizard = RwSignal::new(false);
    let test_msg = RwSignal::new(None::<(bool, String)>);
    let confirm_reset = RwSignal::new(false);
    let test = move || {
        test_msg.set(Some((true, t("Teste …", "Testing…").to_owned())));
        spawn_local(async move {
            test_msg.set(Some(match test_connection().await {
                Ok(_) => (true, t("Bucket erreichbar", "Bucket is reachable").to_owned()),
                Err(e) => (false, e),
            }));
        });
    };
    let reset = move || {
        confirm_reset.set(false);
        spawn_local(async move {
            if api::rpc("share", "reset", Value::Null).await.is_err() {
                // Fallback ohne Backend-Befehl: Einstellungen leeren.
                let keys = ["cloudShareProvider", "cloudShareUrlPrefix", "cloudShareAccountId", "cloudShareBucket", "cloudShareRegion", "cloudShareEndpoint", "cloudSharePathPrefix", "cloudShareAccessKey", "cloudShareSecretKey"];
                let values: serde_json::Map<String, Value> = keys.iter().map(|k| (k.to_string(), Value::Null)).collect();
                let _ = api::rpc("settings", "patch", json!({"values": values})).await;
            }
            test_msg.set(None);
        });
    };
    view! {
        <Show when=configured fallback=move || view! {
            <div class="mb-5 rounded-2xl border border-black/5 bg-white/70 p-6 text-center shadow-sm dark:border-white/10 dark:bg-white/5">
                <div class="mx-auto mb-3 flex size-12 items-center justify-center rounded-full bg-blue-500/10 text-blue-500 [&_svg]:size-6"><ui::Icon d=ui::CLOUD_UPLOAD/></div>
                <div class="mb-1 text-[15px] font-semibold">{t("Teile Aufnahmen über deinen eigenen Speicher", "Share captures through your own storage")}</div>
                <p class="mx-auto mb-4 max-w-[420px] text-[12px] text-black/60 dark:text-white/60">
                    {t("Lade Screenshots und Aufnahmen in deinen R2-, S3-, COS- oder OSS-Bucket und kopiere sofort einen Link. Die Einrichtung dauert etwa fünf Minuten.",
                       "Upload screenshots and recordings to your R2, S3, COS or OSS bucket and copy a link instantly. Setup takes about five minutes.")}
                </p>
                <ui::TextButton variant="primary" on_click=move || wizard.set(true)>{t("Einrichten …", "Set Up…")}</ui::TextButton>
            </div>
        }>
            <ui::Card title=t("Konfiguration", "Configuration")>
                <ui::Row label=t("Anbieter", "Provider") hint=""><span class="text-[12px] text-black/60 dark:text-white/60">{move || provider_name(&opt_str("cloudShareProvider"))}</span></ui::Row>
                <ui::Row label=t("Öffentliches URL-Präfix", "Public URL Prefix") hint=""><span class="max-w-[260px] truncate text-[12px] text-black/60 dark:text-white/60">{move || opt_str("cloudShareUrlPrefix")}</span></ui::Row>
                <ui::Row label=t("Bucket", "Bucket") hint=""><span class="text-[12px] text-black/60 dark:text-white/60">{move || opt_str("cloudShareBucket")}</span></ui::Row>
                <ui::Row label=t("Region / Endpunkt", "Region / Endpoint") hint="">
                    <span class="max-w-[260px] truncate text-[12px] text-black/60 dark:text-white/60">{move || {
                        let p = opt_str("cloudShareProvider");
                        if p == "r2" { opt_str("cloudShareAccountId") } else { [opt_str("cloudShareRegion"), opt_str("cloudShareEndpoint")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ") }
                    }}</span>
                </ui::Row>
                {text_row(t("Pfad-Präfix", "Path Prefix"), t("Optionaler Unterordner im Bucket", "Optional folder inside the bucket"), "cloudSharePathPrefix", "screenshots/", false, true)}
            </ui::Card>
            <ui::Card title=t("Verhalten", "Behavior")>
                {toggle_row(t("Neue Aufnahmen automatisch hochladen", "Automatically Upload New Captures"), t("Neue Screenshots und Aufnahmen hochladen und den Link kopieren", "Upload new screenshots and recordings, then copy the share link"), "cloudShareAutoUpload")}
                {toggle_row(t("Link beim Start des Uploads kopieren", "Copy Link When Upload Starts"), t("Link sofort einfügbar; er funktioniert, sobald der Upload fertig ist", "Paste the link right away; it works once the upload finishes"), "cloudShareCopyLinkOnStart")}
            </ui::Card>
            <ui::Card title=t("Verwalten", "Manage")>
                <ui::Row label=t("Anbieter ändern", "Change Provider") hint=t("Speicheranbieter oder Zugangsdaten ändern", "Update storage provider or credentials")>
                    <ui::TextButton on_click=move || wizard.set(true)>{t("Ändern …", "Change…")}</ui::TextButton>
                </ui::Row>
                <ui::Row label=t("Verbindung testen", "Test Connection") hint=t("Prüfen, ob der Bucket erreichbar ist", "Verify the bucket is reachable")>
                    {move || test_msg.get().map(|(ok, m)| view! { <span class=if ok { "max-w-[240px] truncate text-[11px] text-green-600" } else { "max-w-[240px] truncate text-[11px] text-red-500" } title=m.clone()>{m.clone()}</span> })}
                    <ui::TextButton on_click=test>{t("Testen", "Test")}</ui::TextButton>
                </ui::Row>
                <ui::Row label=t("Konfiguration zurücksetzen", "Reset Configuration") hint=t("Alle Zugangsdaten der Cloud-Freigabe entfernen", "Remove all Cloud Share credentials")>
                    <ui::TextButton variant="danger" on_click=move || confirm_reset.set(true)>{t("Zurücksetzen", "Reset")}</ui::TextButton>
                </ui::Row>
            </ui::Card>
        </Show>

        <Sheet open=confirm_reset>
            <h2 class="mb-2 text-[15px] font-semibold">{t("Cloud-Freigabe zurücksetzen?", "Reset Cloud Share?")}</h2>
            <p class="mb-4 text-[12px] text-black/70 dark:text-white/70">{t("Alle gespeicherten Zugangsdaten werden entfernt. Bereits geteilte Links bleiben gültig.", "All stored credentials will be removed. Links you already shared keep working.")}</p>
            <div class="flex justify-end gap-2">
                <ui::TextButton on_click=move || confirm_reset.set(false)>{t("Abbrechen", "Cancel")}</ui::TextButton>
                <ui::TextButton variant="danger" on_click=reset>{t("Zurücksetzen", "Reset")}</ui::TextButton>
            </div>
        </Sheet>

        <Sheet open=wizard wide=true>
            {move || wizard.get().then(|| view! { <Wizard open=wizard/> })}
        </Sheet>
    }
}

/// Assistent: 1 Einführung · 2 Anbieter · 3 Schlüssel besorgen · 4 Zugangsdaten · 5 URL-Präfix & Test · Fertig.
#[component]
fn Wizard(open: RwSignal<bool>) -> impl IntoView {
    let step = RwSignal::new(0usize);
    let init = |k: &str| RwSignal::new(opt_str(k));
    let provider = RwSignal::new(Some(opt_str("cloudShareProvider")).filter(|p| PROVIDERS.contains(&p.as_str())).unwrap_or_else(|| "r2".into()));
    let (account, region, endpoint, bucket, access, secret, prefix, path) = (
        init("cloudShareAccountId"),
        init("cloudShareRegion"),
        init("cloudShareEndpoint"),
        init("cloudShareBucket"),
        RwSignal::new(String::new()),
        RwSignal::new(String::new()),
        init("cloudShareUrlPrefix"),
        init("cloudSharePathPrefix"),
    );
    // Zugangsdaten aus dem Schlüsselbund laden, sobald der Assistent öffnet.
    Effect::new(move |_| {
        if open.get() {
            let p = provider.get_untracked();
            spawn_local(async move {
                if let Ok(v) = api::rpc("share", "credentials", json!({"provider": p})).await {
                    access.set(v["accessKey"].as_str().unwrap_or("").to_owned());
                    secret.set(v["secretKey"].as_str().unwrap_or("").to_owned());
                }
            });
        }
    });
    // Test-Zustand: None = nicht getestet, Some(None) = läuft, Some(Some(Ok/Err)).
    let test = RwSignal::new(None::<Option<Result<String, String>>>);

    let creds_valid = move || {
        let p = provider.get();
        let id_ok = if p == "r2" { !account.get().trim().is_empty() } else { !region.get().trim().is_empty() };
        id_ok && !bucket.get().trim().is_empty() && !access.get().trim().is_empty() && !secret.get().trim().is_empty()
    };
    let opt = |s: String| if s.trim().is_empty() { Value::Null } else { json!(s.trim()) };
    let persist = move || {
        let values = json!({
            "cloudShareProvider": provider.get_untracked(),
            "cloudShareAccountId": opt(account.get_untracked()),
            "cloudShareRegion": opt(region.get_untracked()),
            "cloudShareEndpoint": opt(endpoint.get_untracked()),
            "cloudShareBucket": opt(bucket.get_untracked()),
            "cloudShareAccessKey": opt(access.get_untracked()),
            "cloudShareSecretKey": opt(secret.get_untracked()),
            "cloudShareUrlPrefix": opt(prefix.get_untracked().trim_end_matches('/').to_owned()),
            "cloudSharePathPrefix": opt(path.get_untracked()),
        });
        // Zugangsdaten → Schlüsselbund, übrige Felder → Einstellungen.
        async move { api::rpc("share", "saveConfig", json!({"values": values})).await }
    };
    let run_test = move || {
        test.set(Some(None));
        spawn_local(async move {
            if let Err(e) = persist().await {
                return test.set(Some(Some(Err(e))));
            }
            test.set(Some(Some(test_connection().await)));
        });
    };
    let save = move || {
        spawn_local(async move {
            let _ = persist().await;
            step.set(5);
        })
    };
    let can_next = move || match step.get() {
        3 => creds_valid(),
        4 => url_prefix_error(&prefix.get()).is_none(),
        _ => true,
    };

    let field = move |label: &'static str, sig: RwSignal<String>, ph: &'static str, password: bool| {
        view! {
            <label class="mb-3 block">
                <div class="mb-1 text-[12px] font-medium text-black/70 dark:text-white/70">{label}</div>
                <input type=if password { "password" } else { "text" } placeholder=ph
                    class="h-8 w-full rounded-md border border-black/10 bg-white/80 px-2 text-[13px] outline-none focus:ring-2 focus:ring-blue-500/40 dark:border-white/10 dark:bg-white/10"
                    prop:value=move || sig.get() on:input=move |e| { sig.set(event_target_value(&e)); test.set(None); }/>
            </label>
        }
    };
    let header = |label: String, title: String, sub: String| {
        view! {
            <div class="mb-4">
                <div class="text-[11px] font-semibold uppercase tracking-wide text-blue-500">{label}</div>
                <div class="text-[17px] font-bold">{title}</div>
                <p class="text-[12px] text-black/60 dark:text-white/60">{sub}</p>
            </div>
        }
    };

    let body = move || match step.get() {
        0 => view! {
            {header(t("Schritt 1 – Einführung", "Step 1 — Introduction").into(), t("Cloud-Freigabe einrichten", "Set up Cloud Share").into(),
                t("nrzCapture lädt Aufnahmen in deinen eigenen Speicher und kopiert einen öffentlichen Link.", "nrzCapture uploads captures to your own storage and copies a public link.").into())}
            {[(t("Gehört dir.", "Yours forever."), t("Dateien liegen in deinem Konto – keine fremden Server.", "Files live in your own account — no third-party servers.")),
              (t("Kostenlos-Kontingent-freundlich.", "Free tier friendly."), t("R2 bietet großzügigen freien Speicher ohne Egress-Kosten.", "R2 offers generous free storage with no egress fees.")),
              (t("Fünf Minuten Einrichtung.", "Five minutes to set up."), t("Dieser Assistent führt dich durch jeden Schritt.", "This assistant walks you through every step."))]
                .into_iter().map(|(a, b)| view! {
                    <div class="mb-2 flex gap-3 rounded-xl border border-black/5 bg-white/70 p-3 dark:border-white/10 dark:bg-white/5">
                        <div class="text-blue-500 [&_svg]:size-5"><ui::Icon d=ui::CHECK/></div>
                        <div><div class="text-[13px] font-semibold">{a}</div><div class="text-[12px] text-black/60 dark:text-white/60">{b}</div></div>
                    </div>
                }).collect_view()}
        }.into_any(),
        1 => view! {
            {header(t("Schritt 2 – Anbieter", "Step 2 — Provider").into(), t("Speicheranbieter wählen", "Choose a storage provider").into(),
                t("Wähle, wohin nrzCapture hochlädt. Dateien bleiben in deinem eigenen Konto.", "Choose where nrzCapture uploads screenshots. Files stay in your own storage account.").into())}
            <div class="grid grid-cols-2 gap-2">
                {PROVIDERS.iter().map(|p| {
                    let p: &'static str = p;
                    view! {
                        <button class=move || format!("rounded-xl border p-3 text-left transition-colors {}",
                                if provider.get() == p { "border-blue-500 bg-blue-500/10" } else { "border-black/10 bg-white/70 hover:bg-white dark:border-white/10 dark:bg-white/5 dark:hover:bg-white/10" })
                            on:click=move |_| { provider.set(p.to_owned()); test.set(None); }>
                            <div class="text-[13px] font-semibold">{provider_name(p)}</div>
                            <div class="text-[11px] text-black/55 dark:text-white/55">{provider_blurb(p)}</div>
                        </button>
                    }
                }).collect_view()}
            </div>
        }.into_any(),
        2 => {
            let p = provider.get();
            view! {
                {header(t("Schritt 3 – Anbieter", "Step 3 — Provider").into(), format!("{} {}", provider_name(&p), t("– Zugangsdaten vorbereiten", "— prepare credentials")),
                    t("Öffne das Dashboard deines Anbieters und lass dieses Fenster offen. Die Werte fügst du im nächsten Schritt ein.", "Open your provider dashboard and keep this window nearby. You'll paste the values on the next screen.").into())}
                <ol class="mb-4 space-y-2">
                    {[(t("Bucket anlegen oder wählen", "Create or choose a bucket"), t("Ein Bucket, in den nrzCapture Objekte schreiben darf.", "A bucket that nrzCapture can write objects into.")),
                      (t("Hochgeladene Dateien öffentlich lesbar machen", "Make uploaded files publicly readable"), t("Öffentliche Bucket-URL oder eigene Domain/CDN verbinden – nrzCapture braucht ein öffentliches HTTPS-Präfix für den Link.", "Use a public bucket URL or connect a custom domain/CDN. nrzCapture needs a public HTTPS prefix so it can copy a share link.")),
                      (t("Zugangsschlüssel erstellen", "Create access credentials"), t("Schlüssel mit Lese-/Schreibrecht für Objekte, möglichst auf diesen Bucket beschränkt.", "Create a key with object read/write permission scoped to this bucket when your provider supports scoped keys.")),
                      (t("Benötigte Werte kopieren", "Copy the required values"), t("Bucket, Region/Endpunkt bzw. Konto-ID, Access Key und Secret bereithalten.", "Keep the bucket, region/endpoint or account ID, access key, and secret ready."))]
                        .into_iter().enumerate().map(|(i, (a, b))| view! {
                            <li class="flex gap-3">
                                <span class="flex size-5 shrink-0 items-center justify-center rounded-full bg-blue-500 text-[11px] font-bold text-white">{i + 1}</span>
                                <div><div class="text-[13px] font-semibold">{a}</div><div class="text-[12px] text-black/60 dark:text-white/60">{b}</div></div>
                            </li>
                        }).collect_view()}
                </ol>
                <ui::TextButton on_click=move || api::fire("core", "open", json!({"target": dashboard_url(&provider.get_untracked())}))>
                    {t("Anbieter-Dashboard öffnen", "Open Provider Dashboard")}<ui::Icon d=ui::EXTERNAL/>
                </ui::TextButton>
            }.into_any()
        }
        3 => {
            let p = provider.get();
            let is_r2 = p == "r2";
            view! {
                {header(t("Schritt 4 – Zugangsdaten", "Step 4 — Credentials").into(), format!("{} {}", provider_name(&p), t("verbinden", "credentials")),
                    t("Die Schlüssel werden im macOS-Schlüsselbund dieses Macs gespeichert und nur an deinen Bucket gesendet.", "These are stored in macOS Keychain on this Mac and never sent anywhere except your bucket.").into())}
                {if is_r2 { field(region_label(&p), account, region_placeholder(&p), false).into_any() } else { field(region_label(&p), region, region_placeholder(&p), false).into_any() }}
                {field(t("Bucket", "Bucket"), bucket, "my-screenshots", false)}
                {field(access_label(&p), access, "", false)}
                {field(secret_label(&p), secret, "", true)}
                {(p == "s3").then(|| field(t("Endpunkt (optional für S3-kompatible Dienste)", "Endpoint (optional for S3-compatible services)"), endpoint, "https://s3.example.com", false))}
            }.into_any()
        }
        4 => view! {
            {header(t("Schritt 5 – Öffentliche URL", "Step 5 — Public URL").into(), t("Öffentliches URL-Präfix", "Public URL prefix").into(),
                t("Unter dieser Adresse sind hochgeladene Dateien erreichbar, z. B. https://pub-….r2.dev oder deine eigene Domain.", "Uploaded files are reachable under this address, e.g. https://pub-….r2.dev or your own domain.").into())}
            {field(t("Öffentliches URL-Präfix", "Public URL Prefix"), prefix, "https://cdn.example.com", false)}
            {field(t("Pfad-Präfix (optional)", "Path Prefix (optional)"), path, "screenshots/", false)}
            {move || url_prefix_error(&prefix.get()).filter(|_| !prefix.get().is_empty()).map(|e| view! { <div class="mb-2 text-[11px] text-red-500">{e}</div> })}
            <div class="flex items-center gap-2">
                <ui::TextButton disabled=Signal::derive(move || url_prefix_error(&prefix.get()).is_some() || matches!(test.get(), Some(None))) on_click=run_test>
                    {t("Verbindung testen", "Test Connection")}
                </ui::TextButton>
                {move || match test.get() {
                    Some(None) => view! { <span class="text-[12px] text-black/50">{t("Teste – lädt eine kleine Datei hoch …", "Testing — uploading a small file…")}</span> }.into_any(),
                    Some(Some(Ok(_))) => view! { <span class="text-[12px] text-green-600">{t("✓ Verbindung erfolgreich", "✓ Connection successful")}</span> }.into_any(),
                    Some(Some(Err(e))) => view! { <span class="text-[12px] text-red-500">{e}</span> }.into_any(),
                    None => ().into_any(),
                }}
            </div>
        }.into_any(),
        _ => view! {
            <div class="py-6 text-center">
                <div class="mx-auto mb-3 flex size-12 items-center justify-center rounded-full bg-green-500/15 text-green-600 [&_svg]:size-6"><ui::Icon d=ui::CHECK/></div>
                <div class="text-[17px] font-bold">{t("Alles bereit!", "You're all set!")}</div>
                <p class="mx-auto mt-1 max-w-[380px] text-[12px] text-black/60 dark:text-white/60">
                    {t("Nutze „Bereich aufnehmen & teilen“ oder den Hochladen-Knopf in Quick Access – der Link landet in der Zwischenablage.",
                       "Use “Capture Area & Share” or the upload button in Quick Access — the link goes straight to your clipboard.")}
                </p>
            </div>
        }.into_any(),
    };

    view! {
        <div class="mb-4 flex items-center justify-between">
            <div class="flex items-center gap-1.5">
                {(0..6).map(|i| view! { <span class=move || format!("h-1.5 rounded-full transition-all {}", if step.get() == i { "w-5 bg-blue-500" } else if step.get() > i { "w-1.5 bg-blue-500/60" } else { "w-1.5 bg-black/15 dark:bg-white/20" })></span> }).collect_view()}
                <span class="ml-2 text-[11px] text-black/45 dark:text-white/45">{move || format!("{} {} 6", step.get() + 1, t("von", "of"))}</span>
            </div>
            <ui::TextButton variant="ghost" on_click=move || open.set(false)><ui::Icon d=ui::X/></ui::TextButton>
        </div>
        <div class="min-h-[300px]">{body}</div>
        <div class="mt-4 flex items-center justify-between border-t border-black/5 pt-3 dark:border-white/10">
            <div>
                <Show when=move || { let s = step.get(); s > 0 && s < 5 }>
                    <ui::TextButton variant="ghost" on_click=move || step.update(|s| *s -= 1)>{t("Zurück", "Back")}</ui::TextButton>
                </Show>
            </div>
            {move || match step.get() {
                5 => view! { <ui::TextButton variant="primary" on_click=move || open.set(false)>{t("Fertig", "Done")}</ui::TextButton> }.into_any(),
                // Speichern erst nach erfolgreichem Test – oder wenn das Backend den Test noch nicht kennt.
                4 => view! {
                    <ui::TextButton variant="primary"
                        disabled=Signal::derive(move || !can_next() || !matches!(test.get(), Some(Some(Ok(_)))) && !matches!(test.get(), Some(Some(Err(ref e))) if e.contains("noch nicht") || e.contains("not available")))
                        on_click=save>{t("Speichern", "Save")}</ui::TextButton>
                }.into_any(),
                _ => view! { <ui::TextButton variant="primary" disabled=Signal::derive(move || !can_next()) on_click=move || step.update(|s| *s += 1)>{t("Weiter", "Next")}</ui::TextButton> }.into_any(),
            }}
        </div>
    }
}
