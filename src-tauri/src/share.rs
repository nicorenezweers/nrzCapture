//! Cloud-Freigabe über eigenen Speicher (Cloudflare R2, Amazon S3/S3-kompatibel, Aliyun OSS, Tencent COS).
//! Port von Capso `ShareKit` (ShareConfig, S3CompatibleHTTP, ShareSigning, *Destination, IDGenerator,
//! UploadAttemptGate, ShareCoordinator). Signaturen (AWS SigV4, OSS HMAC-SHA1, COS q-sign) selbst implementiert.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, WebviewWindow};

use crate::core::{self, t};

// ───────────────────────────── Fehler ─────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareError {
    NotConfigured,
    InvalidCredentials,
    InvalidUrlPrefix(String),
    Network(String),
    QuotaExceeded,
    /// Hochladen ok, aber die öffentliche URL ist nicht erreichbar.
    PublicAccessUnreachable,
    Unknown(String),
}

impl ShareError {
    /// Zweisprachige Meldung für Toasts/UI.
    pub fn message(&self) -> String {
        match self {
            Self::NotConfigured => t("Cloud-Freigabe ist nicht eingerichtet.", "Cloud sharing is not configured.").into(),
            Self::InvalidCredentials => t("Cloud-Zugangsdaten sind ungültig.", "Cloud credentials are invalid.").into(),
            Self::InvalidUrlPrefix(r) => format!("{} {r}", t("Ungültiges URL-Präfix:", "Invalid URL prefix:")),
            Self::Network(u) => format!("{} {u}", t("Netzwerkfehler:", "Network error:")),
            Self::QuotaExceeded => t("Cloud-Kontingent überschritten.", "Cloud quota exceeded.").into(),
            Self::PublicAccessUnreachable => {
                t("Hochladen ok, aber die öffentliche URL ist nicht erreichbar.", "Upload OK but public URL unreachable.").into()
            }
            Self::Unknown(d) => format!("{} {d}", t("Hochladen fehlgeschlagen:", "Upload failed:")),
        }
    }
}

// ───────────────────────────── Anbieter & Konfiguration ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    R2,
    S3,
    TencentCos,
    AliyunOss,
}

impl Provider {
    pub const ALL: [Provider; 4] = [Provider::R2, Provider::S3, Provider::TencentCos, Provider::AliyunOss];

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == s)
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::R2 => "r2",
            Self::S3 => "s3",
            Self::TencentCos => "tencentCOS",
            Self::AliyunOss => "aliyunOSS",
        }
    }
    pub fn display_name(self) -> &'static str {
        match self {
            Self::R2 => "Cloudflare R2",
            Self::S3 => "Amazon S3",
            Self::TencentCos => "Tencent COS",
            Self::AliyunOss => "Aliyun OSS",
        }
    }
    fn dashboard_url(self) -> &'static str {
        match self {
            Self::R2 => "https://dash.cloudflare.com/?to=/:account/r2/overview",
            Self::S3 => "https://s3.console.aws.amazon.com/s3/home",
            Self::TencentCos => "https://console.cloud.tencent.com/cos",
            Self::AliyunOss => "https://oss.console.aliyun.com/",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareConfig {
    pub provider: Provider,
    /// Normalisiert: ohne abschließenden Schrägstrich.
    pub url_prefix: String,
    pub bucket: String,
    pub account_id: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub path_prefix: Option<String>,
    pub access_key: String,
    pub secret_key: String,
}

fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

impl ShareConfig {
    /// Aus den Einstellungen (`cloudShare*`) – prüft Pflichtfelder und URL-Präfix.
    pub fn from_settings(s: &crate::settings::Settings) -> Result<Self, ShareError> {
        let provider = non_empty(&s.cloud_share_provider).and_then(|p| Provider::parse(&p)).ok_or(ShareError::NotConfigured)?;
        let cfg = Self {
            provider,
            url_prefix: normalize_prefix(non_empty(&s.cloud_share_url_prefix).unwrap_or_default().as_str()),
            bucket: non_empty(&s.cloud_share_bucket).unwrap_or_default(),
            account_id: non_empty(&s.cloud_share_account_id).unwrap_or_default(),
            region: non_empty(&s.cloud_share_region).unwrap_or_default(),
            endpoint: non_empty(&s.cloud_share_endpoint),
            path_prefix: non_empty(&s.cloud_share_path_prefix),
            // Zugangsdaten liegen im Schlüsselbund; Werte in den Einstellungen (Assistent-Test / Altbestand) haben Vorrang.
            access_key: non_empty(&s.cloud_share_access_key).or_else(|| keychain_get(provider, "accessKey")).unwrap_or_default(),
            secret_key: non_empty(&s.cloud_share_secret_key).or_else(|| keychain_get(provider, "secretKey")).unwrap_or_default(),
        };
        cfg.validate_for_upload()?;
        Ok(cfg)
    }

    pub fn validate_for_upload(&self) -> Result<(), ShareError> {
        validate_prefix(&self.url_prefix)?;
        let required = match self.provider {
            Provider::R2 => &self.account_id,
            _ => &self.region,
        };
        if self.bucket.is_empty() || required.is_empty() || self.access_key.is_empty() || self.secret_key.is_empty() {
            return Err(ShareError::NotConfigured);
        }
        Ok(())
    }

    /// Objektschlüssel inkl. Pfad-Präfix.
    pub fn object_key(&self, key: &str) -> String {
        normalize_path_prefix(self.path_prefix.as_deref()) + key
    }

    /// Öffentliche URL = URL-Präfix + kodierter Objektschlüssel.
    pub fn public_url(&self, object_key: &str) -> String {
        format!("{}/{}", self.url_prefix, encode_object_key(object_key))
    }

    /// Objektschlüssel aus einer öffentlichen URL (für „auch aus der Cloud löschen“).
    pub fn object_key_from_url(&self, url: &str) -> Option<String> {
        let rest = url.strip_prefix(&self.url_prefix)?.strip_prefix('/')?;
        (!rest.is_empty()).then(|| percent_decode(rest))
    }
}

pub fn normalize_prefix(prefix: &str) -> String {
    prefix.trim_end_matches('/').to_owned()
}

pub fn validate_prefix(prefix: &str) -> Result<(), ShareError> {
    let Some(rest) = prefix.strip_prefix("https://") else {
        return Err(ShareError::InvalidUrlPrefix(t("Muss mit https:// beginnen", "Must start with https://").into()));
    };
    if rest.contains('?') || rest.contains('#') {
        return Err(ShareError::InvalidUrlPrefix(
            t("Darf keine Query oder Fragment enthalten", "Must not contain query string or fragment").into(),
        ));
    }
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || host.contains(' ') {
        return Err(ShareError::InvalidUrlPrefix(t("Muss einen gültigen Hostnamen enthalten", "Must contain a valid hostname").into()));
    }
    Ok(())
}

/// `"/shots/"` → `"shots/"`, leer → `""`.
pub fn normalize_path_prefix(raw: Option<&str>) -> String {
    let v = raw.unwrap_or("").trim().trim_matches('/');
    if v.is_empty() {
        String::new()
    } else {
        format!("{v}/")
    }
}

/// Prozent-Kodierung wie Capso (`alphanumerics` + `-._~` bleiben).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Jedes Pfadsegment einzeln kodieren, `/` bleibt.
pub fn encode_object_key(key: &str) -> String {
    key.split('/').map(percent_encode).collect::<Vec<_>>().join("/")
}

// ───────────────────────────── ID-Generator ─────────────────────────────

const ID_CHARSET: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";

/// 7 Zeichen Base32 (wie Capso `IDGenerator.shortID`).
pub fn short_id() -> String {
    let mut buf = [0u8; 7];
    if getrandom::getrandom(&mut buf).is_err() {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (n >> (i * 5)) as u8;
        }
    }
    buf.iter().map(|b| ID_CHARSET[(b & 31) as usize] as char).collect()
}

// ───────────────────────────── Upload-Sperre ─────────────────────────────

/// Verhindert überlappende Uploads derselben Datei (Capso `UploadAttemptGate`, je Schlüssel).
#[derive(Default)]
pub struct UploadAttemptGate {
    in_flight: HashSet<String>,
}

impl UploadAttemptGate {
    pub fn begin(&mut self, key: &str) -> bool {
        self.in_flight.insert(key.to_owned())
    }
    pub fn finish(&mut self, key: &str) {
        self.in_flight.remove(key);
    }
    pub fn contains(&self, key: &str) -> bool {
        self.in_flight.contains(key)
    }
}

static GATE: Mutex<Option<UploadAttemptGate>> = Mutex::new(None);

fn gate<R>(f: impl FnOnce(&mut UploadAttemptGate) -> R) -> R {
    f(GATE.lock().unwrap().get_or_insert_with(UploadAttemptGate::default))
}

fn gate_key(path: &Path) -> String {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().into_owned()
}

/// Läuft gerade ein Upload für diese Datei?
pub fn is_uploading(path: &Path) -> bool {
    gate(|g| g.contains(&gate_key(path)))
}

// ───────────────────────────── Krypto-Helfer ─────────────────────────────

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC");
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

pub fn hmac_sha1(key: &[u8], msg: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    let mut m = Hmac::<sha1::Sha1>::new_from_slice(key).expect("HMAC");
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn sha1_hex(data: &[u8]) -> String {
    use sha1::Digest as _;
    hex::encode(sha1::Sha1::digest(data))
}

pub fn base64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// SHA-256 einer Datei in 1-MB-Blöcken (große Aufnahmen nicht komplett in den Speicher laden).
pub fn sha256_file(path: &Path) -> std::io::Result<(String, u64)> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        h.update(&buf[..n]);
    }
    Ok((hex::encode(h.finalize()), size))
}

pub const EMPTY_PAYLOAD_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

// ───────────────────────────── AWS SigV4 ─────────────────────────────

pub fn sigv4_signing_key(secret: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// Allgemeiner SigV4-`Authorization`-Header. `headers` = (kleingeschriebener Name, Wert), beliebige Reihenfolge.
#[allow(clippy::too_many_arguments)]
pub fn sigv4_authorization(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    headers: &[(&str, &str)],
    payload_hash: &str,
    amz_date: &str,
    region: &str,
    service: &str,
    access_key: &str,
    secret_key: &str,
) -> String {
    let mut hs: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_lowercase(), v.trim().to_owned())).collect();
    hs.sort();
    let canonical_headers: String = hs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = hs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical_request = [method, canonical_uri, canonical_query, &canonical_headers, &signed_headers, payload_hash].join("\n");
    let date_stamp = &amz_date[..8];
    let scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = ["AWS4-HMAC-SHA256", amz_date, &scope, &sha256_hex(canonical_request.as_bytes())].join("\n");
    let signature = hex::encode(hmac_sha256(&sigv4_signing_key(secret_key, date_stamp, region, service), string_to_sign.as_bytes()));
    format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}")
}

/// Ziel einer S3-Anfrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `http` oder `https` – bei eigenen Endpunkten übernommen (MinIO im LAN).
    pub scheme: String,
    /// Host plus optional Port (ohne Pfad).
    pub host: String,
    pub canonical_uri: String,
}

impl Target {
    pub fn url(&self) -> String {
        format!("{}://{}{}", self.scheme, self.host, self.canonical_uri)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEndpoint {
    pub scheme: String,
    pub host: String,
    /// Leer oder `"/" + kodierte Segmente`, ohne abschließenden Schrägstrich.
    pub path_prefix: String,
}

pub fn parse_endpoint(raw: &str) -> Option<ParsedEndpoint> {
    let v = raw.trim();
    if v.is_empty() {
        return None;
    }
    let lower = v.to_lowercase();
    let (scheme, rest) = if lower.starts_with("https://") {
        ("https", &v[8..])
    } else if lower.starts_with("http://") {
        ("http", &v[7..])
    } else {
        ("https", v)
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return None;
    }
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if host.is_empty() || host.contains(' ') {
        return None;
    }
    let enc = encode_object_key(path);
    Some(ParsedEndpoint { scheme: scheme.into(), host: host.into(), path_prefix: if enc.is_empty() { String::new() } else { format!("/{enc}") } })
}

/// Amazon S3 virtual-hosted, oder path-style bei eigenem Endpunkt.
pub fn amazon_s3_target(bucket: &str, object_key: &str, region: &str, endpoint: Option<&str>) -> Target {
    let key = encode_object_key(object_key);
    if let Some(p) = endpoint.and_then(parse_endpoint) {
        return Target { scheme: p.scheme, host: p.host, canonical_uri: format!("{}/{bucket}/{key}", p.path_prefix) };
    }
    Target { scheme: "https".into(), host: format!("{bucket}.s3.{region}.amazonaws.com"), canonical_uri: format!("/{key}") }
}

pub fn normalize_account_id(raw: &str) -> String {
    let mut v = raw.trim();
    let lower = v.to_lowercase();
    if lower.starts_with("https://") {
        v = &v[8..];
    } else if lower.starts_with("http://") {
        v = &v[7..];
    }
    let v = v.trim_end_matches('/');
    v.find(".r2.cloudflarestorage.com").map(|i| &v[..i]).unwrap_or(v).to_owned()
}

/// Cloudflare R2: path-style auf `<accountId>.r2.cloudflarestorage.com`.
pub fn r2_target(account_id: &str, bucket: &str, object_key: &str) -> Target {
    Target {
        scheme: "https".into(),
        host: format!("{}.r2.cloudflarestorage.com", normalize_account_id(account_id)),
        canonical_uri: format!("/{bucket}/{}", encode_object_key(object_key)),
    }
}

/// Fertig signierte HTTP-Anfrage (ohne Body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl SignedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn amz_timestamps(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y%m%dT%H%M%SZ").to_string()
}

/// S3-SigV4-Anfrage wie Capso: echte Payload-SHA-256, signierte Header
/// `content-type;host;x-amz-content-sha256;x-amz-date` (ohne Content-Type bei DELETE).
#[allow(clippy::too_many_arguments)]
pub fn s3_signed_request(
    method: &str,
    target: &Target,
    region: &str,
    access_key: &str,
    secret_key: &str,
    content_type: Option<&str>,
    payload_hash: &str,
    content_length: Option<u64>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SignedRequest, ShareError> {
    if target.host.is_empty() || target.host.contains(' ') {
        return Err(ShareError::Unknown("Invalid S3 endpoint URL".into()));
    }
    let amz_date = amz_timestamps(now);
    let mut signed: Vec<(&str, &str)> = vec![("host", &target.host), ("x-amz-content-sha256", payload_hash), ("x-amz-date", &amz_date)];
    if let Some(ct) = content_type {
        signed.push(("content-type", ct));
    }
    let auth = sigv4_authorization(method, &target.canonical_uri, "", &signed, payload_hash, &amz_date, region, "s3", access_key, secret_key);
    let mut headers = vec![
        ("Host".to_owned(), target.host.clone()),
        ("X-Amz-Date".to_owned(), amz_date.clone()),
        ("X-Amz-Content-Sha256".to_owned(), payload_hash.to_owned()),
        ("Authorization".to_owned(), auth),
    ];
    if let Some(ct) = content_type {
        headers.push(("Content-Type".into(), ct.into()));
    }
    if let Some(len) = content_length {
        headers.push(("Content-Length".into(), len.to_string()));
    }
    Ok(SignedRequest { method: method.into(), url: target.url(), headers })
}

/// Aliyun-OSS-Endpunkt: `oss-cn-hangzhou` → `oss-cn-hangzhou.aliyuncs.com`.
pub fn oss_endpoint(raw: &str) -> String {
    let mut v = raw.trim();
    v = v.strip_prefix("https://").unwrap_or(v);
    v = v.strip_prefix("http://").unwrap_or(v);
    let v = v.trim_end_matches('/');
    if v.contains('.') {
        v.to_owned()
    } else {
        format!("{v}.aliyuncs.com")
    }
}

/// Aliyun OSS (Signatur V1, HMAC-SHA1). PUT setzt `x-oss-object-acl: public-read`.
pub fn oss_signed_request(cfg: &ShareConfig, method: &str, object_key: &str, content_type: Option<&str>, date: &str) -> SignedRequest {
    let host = format!("{}.{}", cfg.bucket, oss_endpoint(&cfg.region));
    let resource = format!("/{}/{object_key}", cfg.bucket);
    let acl = if method == "PUT" { "x-oss-object-acl:public-read\n" } else { "" };
    let sts = format!("{method}\n\n{}\n{date}\n{acl}{resource}", content_type.unwrap_or(""));
    let sig = base64(&hmac_sha1(cfg.secret_key.as_bytes(), sts.as_bytes()));
    let mut headers = vec![
        ("Host".to_owned(), host.clone()),
        ("Date".to_owned(), date.to_owned()),
        ("Authorization".to_owned(), format!("OSS {}:{sig}", cfg.access_key)),
    ];
    if let Some(ct) = content_type {
        headers.push(("Content-Type".into(), ct.into()));
        headers.push(("x-oss-object-acl".into(), "public-read".into()));
    }
    SignedRequest { method: method.into(), url: format!("https://{host}/{}", encode_object_key(object_key)), headers }
}

/// Tencent COS (q-sign-algorithm=sha1), Schlüssel 1 h gültig.
pub fn cos_signed_request(cfg: &ShareConfig, method: &str, object_key: &str, content_type: Option<&str>, now_secs: i64) -> SignedRequest {
    let host = format!("{}.cos.{}.myqcloud.com", cfg.bucket, cfg.region);
    let enc = encode_object_key(object_key);
    let key_time = format!("{now_secs};{}", now_secs + 3600);
    let sign_key = hex::encode(hmac_sha1(cfg.secret_key.as_bytes(), key_time.as_bytes()));
    let http_string = format!("{}\n/{enc}\n\n\n", method.to_lowercase());
    let sts = format!("sha1\n{key_time}\n{}\n", sha1_hex(http_string.as_bytes()));
    let sig = hex::encode(hmac_sha1(sign_key.as_bytes(), sts.as_bytes()));
    let auth = format!(
        "q-sign-algorithm=sha1&q-ak={}&q-sign-time={key_time}&q-key-time={key_time}&q-header-list=&q-url-param-list=&q-signature={sig}",
        cfg.access_key
    );
    let mut headers = vec![("Host".to_owned(), host.clone()), ("Authorization".to_owned(), auth)];
    if let Some(ct) = content_type {
        headers.push(("Content-Type".into(), ct.into()));
    }
    SignedRequest { method: method.into(), url: format!("https://{host}/{enc}"), headers }
}

/// Signierte Anfrage für den eingestellten Anbieter. `payload` = (SHA-256-Hex, Länge) bei PUT.
pub fn signed_request(
    cfg: &ShareConfig,
    method: &str,
    object_key: &str,
    content_type: Option<&str>,
    payload: Option<(&str, u64)>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SignedRequest, ShareError> {
    let (hash, len) = payload.map(|(h, l)| (h, Some(l))).unwrap_or((EMPTY_PAYLOAD_HASH, None));
    match cfg.provider {
        Provider::R2 => {
            let tg = r2_target(&cfg.account_id, &cfg.bucket, object_key);
            s3_signed_request(method, &tg, "auto", &cfg.access_key, &cfg.secret_key, content_type, hash, len, now)
        }
        Provider::S3 => {
            let tg = amazon_s3_target(&cfg.bucket, object_key, &cfg.region, cfg.endpoint.as_deref());
            s3_signed_request(method, &tg, &cfg.region, &cfg.access_key, &cfg.secret_key, content_type, hash, len, now)
        }
        Provider::AliyunOss => {
            Ok(oss_signed_request(cfg, method, object_key, content_type, &now.format("%a, %d %b %Y %H:%M:%S GMT").to_string()))
        }
        Provider::TencentCos => Ok(cos_signed_request(cfg, method, object_key, content_type, now.timestamp())),
    }
}

fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let s = xml.find(&format!("<{tag}>"))? + tag.len() + 2;
    let e = xml.find(&format!("</{tag}>"))?;
    (s <= e).then(|| xml[s..e].trim().to_owned()).filter(|v| !v.is_empty())
}

/// HTTP-Fehler → `ShareError` (wie Capso `mapResponseError`).
pub fn map_response_error(status: u16, body: &str) -> ShareError {
    if let Some(code) = xml_tag(body, "Code") {
        return match code.as_str() {
            "InvalidAccessKeyId" | "SignatureDoesNotMatch" | "AccessDenied" | "InvalidToken" => ShareError::InvalidCredentials,
            "QuotaExceeded" | "EntityTooLarge" => ShareError::QuotaExceeded,
            "NoSuchBucket" => ShareError::Unknown("Bucket not found — verify the bucket name in Cloud Share settings".into()),
            _ => ShareError::Unknown(format!("{code}: {}", xml_tag(body, "Message").unwrap_or_else(|| body.to_owned()))),
        };
    }
    match status {
        401 | 403 => ShareError::InvalidCredentials,
        413 | 507 => ShareError::QuotaExceeded,
        _ if body.is_empty() => ShareError::Unknown(format!("HTTP {status}")),
        _ => ShareError::Unknown(format!("HTTP {status}: {body}")),
    }
}

// ───────────────────────────── HTTP ─────────────────────────────

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(120)).timeout_write(Duration::from_secs(600)).build()
}

fn send(req: &SignedRequest, body: Option<(Box<dyn Read + Send>, u64)>) -> Result<(), ShareError> {
    let mut r = agent().request(&req.method, &req.url);
    for (k, v) in &req.headers {
        // Host setzt ureq selbst (inkl. Port), Content-Length unten.
        if !k.eq_ignore_ascii_case("host") && !k.eq_ignore_ascii_case("content-length") {
            r = r.set(k, v);
        }
    }
    let res = match body {
        Some((reader, len)) => r.set("Content-Length", &len.to_string()).send(reader),
        None => r.call(),
    };
    match res {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, resp)) => Err(map_response_error(code, &resp.into_string().unwrap_or_default())),
        Err(ureq::Error::Transport(t)) => Err(ShareError::Network(t.to_string())),
    }
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "tif" | "tiff" => "image/tiff",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Datei unter `key` hochladen, liefert die öffentliche URL.
pub fn upload_file(cfg: &ShareConfig, path: &Path, key: &str) -> Result<String, ShareError> {
    let object_key = cfg.object_key(key);
    let (hash, size) = sha256_file(path).map_err(|e| ShareError::Unknown(format!("Couldn't read file for upload: {e}")))?;
    let req = signed_request(cfg, "PUT", &object_key, Some(content_type(path)), Some((&hash, size)), chrono::Utc::now())?;
    let file = std::fs::File::open(path).map_err(|e| ShareError::Unknown(e.to_string()))?;
    send(&req, Some((Box::new(file), size)))?;
    Ok(cfg.public_url(&object_key))
}

/// Objekt löschen (Schlüssel inkl. Pfad-Präfix).
pub fn delete_object(cfg: &ShareConfig, object_key: &str) -> Result<(), ShareError> {
    let req = signed_request(cfg, "DELETE", object_key, None, None, chrono::Utc::now())?;
    send(&req, None)
}

/// Rundlauf-Test: kleine Datei hochladen, über die öffentliche URL lesen, löschen. Liefert Sekunden.
pub fn validate_config(cfg: &ShareConfig) -> Result<f64, ShareError> {
    let start = Instant::now();
    let key = format!("_nrzcapture_test_{}.txt", short_id());
    let tmp = core::temp_dir().join(&key);
    std::fs::write(&tmp, "ok").map_err(|e| ShareError::Unknown(e.to_string()))?;
    let res = upload_file(cfg, &tmp, &key);
    let _ = std::fs::remove_file(&tmp);
    let url = res?;
    let object_key = cfg.object_key(&key);
    let fetched = agent().get(&url).set("Cache-Control", "no-cache").call().ok().and_then(|r| (r.status() == 200).then(|| r.into_string().ok()).flatten());
    if fetched.as_deref() != Some("ok") {
        let _ = delete_object(cfg, &object_key);
        return Err(ShareError::PublicAccessUnreachable);
    }
    delete_object(cfg, &object_key)?;
    Ok(start.elapsed().as_secs_f64())
}

// ───────────────────────────── App-Integration ─────────────────────────────

/// Schlüsselbund-Dienst je Anbieter (Capso `com.awesomemacapps.capso.share.<provider>`), Konten `accessKey` / `secretKey`.
pub fn keychain_service(p: Provider) -> String {
    format!("com.nrz.capture.share.{}", p.id())
}

fn keychain_get(p: Provider, account: &str) -> Option<String> {
    core::keychain_get(&keychain_service(p), account)
}

/// Zugangsdaten im Schlüsselbund speichern. Rückgabe: Erfolg.
pub fn store_credentials(p: Provider, access: &str, secret: &str) -> bool {
    let svc = keychain_service(p);
    core::keychain_set(&svc, "accessKey", access) && core::keychain_set(&svc, "secretKey", secret)
}

/// Einmalige Migration: Klartext-Zugangsdaten aus `settings.json` in den Schlüsselbund verschieben.
pub fn migrate_keys(app: &AppHandle) {
    let s = crate::settings::get();
    let (ak, sk) = (non_empty(&s.cloud_share_access_key), non_empty(&s.cloud_share_secret_key));
    if ak.is_none() && sk.is_none() {
        return;
    }
    let Some(p) = non_empty(&s.cloud_share_provider).and_then(|p| Provider::parse(&p)) else { return };
    let ok = store_credentials(p, ak.as_deref().unwrap_or(""), sk.as_deref().unwrap_or(""));
    core::diag("CloudShare", &format!("migrated credentials to keychain ok={ok}"));
    if ok {
        crate::settings::update(app, |s| {
            s.cloud_share_access_key = None;
            s.cloud_share_secret_key = None;
        });
    }
}

/// Assistent speichern: Zugangsdaten zuerst in den Schlüsselbund (bei Fehler abbrechen, Capso `CloudShareWizardView`),
/// übrige Felder in die Einstellungen.
fn save_config(app: &AppHandle, values: &Value) -> Result<(), String> {
    let p = values["cloudShareProvider"].as_str().and_then(Provider::parse).ok_or(t("Anbieter fehlt", "Provider missing"))?;
    let get = |k: &str| values[k].as_str().map(str::trim).unwrap_or("").to_owned();
    if !store_credentials(p, &get("cloudShareAccessKey"), &get("cloudShareSecretKey")) {
        return Err(t("Zugangsdaten konnten nicht im Schlüsselbund gespeichert werden", "Could not save credentials to Keychain").into());
    }
    let mut v = serde_json::to_value(crate::settings::get()).map_err(|e| e.to_string())?;
    if let Some(o) = values.as_object() {
        for (k, val) in o {
            if k != "cloudShareAccessKey" && k != "cloudShareSecretKey" {
                v[k] = val.clone();
            }
        }
    }
    v["cloudShareAccessKey"] = Value::Null;
    v["cloudShareSecretKey"] = Value::Null;
    let s: crate::settings::Settings = serde_json::from_value(v).map_err(|e| e.to_string())?;
    crate::settings::update(app, |cur| *cur = s);
    Ok(())
}

fn config() -> Result<ShareConfig, ShareError> {
    ShareConfig::from_settings(&crate::settings::get())
}

fn not_configured(app: &AppHandle) {
    core::toast(app, t("Cloud-Freigabe ist nicht eingerichtet", "Cloud sharing is not set up"));
    crate::prefs::open(app, Some("cloud"));
}

fn emit_state(app: &AppHandle, path: &Path, state: &str, url: Option<&str>, error: Option<&str>) {
    let _ = app.emit("share-state", json!({ "path": path, "state": state, "url": url, "error": error }));
}

/// Kern des Uploads (blockierend). `on_start(url)` wird aufgerufen, sobald die URL feststeht.
fn do_upload(app: &AppHandle, path: &Path, cfg: &ShareConfig, on_start: impl FnOnce(&str)) -> Result<String, ShareError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let id = short_id();
    let key = if ext.is_empty() { id } else { format!("{id}.{ext}") };
    let pre_url = cfg.public_url(&cfg.object_key(&key));
    crate::history::ensure_entry(app, path);
    emit_state(app, path, "uploading", Some(&pre_url), None);
    on_start(&pre_url);
    match upload_file(cfg, path, &key) {
        Ok(url) => {
            crate::history::attach_cloud_url(app, path, &url);
            emit_state(app, path, "done", Some(&url), None);
            Ok(url)
        }
        Err(e) => {
            core::diag("CloudShare", &format!("upload of {} failed: {}", path.display(), e.message()));
            emit_state(app, path, "failed", None, Some(&e.message()));
            Err(e)
        }
    }
}

/// Hochladen (blockierend), liefert öffentliche URL.
pub fn upload(app: &AppHandle, path: &Path) -> Result<String, String> {
    let cfg = config().map_err(|e| e.message())?;
    let k = gate_key(path);
    if !gate(|g| g.begin(&k)) {
        return Err(t("Diese Datei wird bereits hochgeladen.", "This file is already being uploaded.").into());
    }
    let r = do_upload(app, path, &cfg, |_| {});
    gate(|g| g.finish(&k));
    r.map_err(|e| e.message())
}

/// Im Hintergrund hochladen, Link kopieren (sofort, falls `cloudShareCopyLinkWhenUploadStarts`), Hinweis zeigen.
pub fn upload_and_copy(app: &AppHandle, path: &Path) {
    let cfg = match config() {
        Ok(c) => c,
        Err(ShareError::NotConfigured) => return not_configured(app),
        Err(e) => {
            core::toast(app, &e.message());
            return;
        }
    };
    if !path.is_file() {
        core::toast(app, t("Datei nicht gefunden", "File not found"));
        return;
    }
    let k = gate_key(path);
    if !gate(|g| g.begin(&k)) {
        core::toast(app, t("Wird bereits hochgeladen …", "Already uploading…"));
        return;
    }
    let (app, path) = (app.clone(), path.to_path_buf());
    std::thread::spawn(move || {
        let early = crate::settings::get().cloud_share_copy_link_on_start;
        let res = do_upload(&app, &path, &cfg, |url| {
            if early {
                core::clipboard::copy_text(url);
                core::toast(&app, t("Wird hochgeladen – Link kopiert", "Uploading – link copied"));
            } else {
                core::toast(&app, t("Wird hochgeladen …", "Uploading…"));
            }
        });
        match res {
            Ok(url) => {
                if !early {
                    core::clipboard::copy_text(&url);
                }
                core::toast(&app, t("Hochgeladen – Link kopiert", "Uploaded – link copied"));
            }
            Err(e) => core::toast(&app, &e.message()),
        }
        gate(|g| g.finish(&k));
    });
}

/// Cloud-Kopie zu einer öffentlichen URL löschen (blockierend).
pub fn delete_cloud_copy(url: &str) -> Result<(), String> {
    let cfg = config().map_err(|e| e.message())?;
    let key = cfg.object_key_from_url(url).ok_or_else(|| t("URL gehört nicht zum eingestellten Speicher.", "URL does not belong to the configured storage.").to_string())?;
    delete_object(&cfg, &key).map_err(|e| e.message())
}

pub fn is_configured() -> bool {
    config().is_ok()
}

/// Konfiguration aus Einstellungen + optionalen Überschreibungen (`{cloudShareProvider: …}`) – für den Einrichtungsassistenten.
fn config_with(values: &Value) -> Result<ShareConfig, ShareError> {
    let mut v = serde_json::to_value(crate::settings::get()).map_err(|e| ShareError::Unknown(e.to_string()))?;
    if let Some(o) = values.as_object() {
        for (k, val) in o {
            v[k] = val.clone();
        }
    }
    let s: crate::settings::Settings = serde_json::from_value(v).map_err(|e| ShareError::Unknown(e.to_string()))?;
    ShareConfig::from_settings(&s)
}

/// Feldbeschreibungen für den Einrichtungsassistenten im Einstellungsfenster.
fn providers_json() -> Value {
    let f = |key: &str, label: &str, placeholder: &str, required: bool, secret: bool| {
        json!({ "key": key, "label": label, "placeholder": placeholder, "required": required, "secret": secret })
    };
    let list: Vec<Value> = Provider::ALL
        .into_iter()
        .map(|p| {
            let mut fields = vec![];
            match p {
                Provider::R2 => fields.push(f("cloudShareAccountId", t("Konto-ID", "Account ID"), t("32-stellige Hex-ID", "32-character hex"), true, false)),
                Provider::S3 => {
                    fields.push(f("cloudShareRegion", t("Region", "Region"), "us-east-1", true, false));
                    fields.push(f("cloudShareEndpoint", t("Endpunkt", "Endpoint"), t("Optional für S3-kompatible Dienste", "Optional for S3-compatible services"), false, false));
                }
                Provider::TencentCos => fields.push(f("cloudShareRegion", t("Region", "Region"), "ap-guangzhou", true, false)),
                Provider::AliyunOss => fields.push(f("cloudShareRegion", t("Endpunkt / Region", "Endpoint / Region"), "oss-cn-hangzhou", true, false)),
            }
            fields.push(f("cloudShareBucket", "Bucket", if p == Provider::TencentCos { "bucket-1250000000" } else { "my-bucket" }, true, false));
            let (ak, sk) = match p {
                Provider::TencentCos => ("SecretId", "SecretKey"),
                Provider::AliyunOss => ("AccessKey ID", "AccessKey Secret"),
                _ => ("Access Key ID", "Secret Access Key"),
            };
            fields.push(f("cloudShareAccessKey", ak, "", true, false));
            fields.push(f("cloudShareSecretKey", sk, "", true, true));
            fields.push(f("cloudSharePathPrefix", t("Pfad-Präfix", "Path Prefix"), t("Optional, z. B. screenshots/", "Optional, e.g. screenshots/"), false, false));
            fields.push(f("cloudShareUrlPrefix", t("Öffentliches URL-Präfix", "Public URL Prefix"), "https://share.example.com", true, false));
            json!({ "id": p.id(), "name": p.display_name(), "dashboardUrl": p.dashboard_url(), "fields": fields })
        })
        .collect();
    Value::Array(list)
}

pub fn dispatch(app: &AppHandle, _win: &WebviewWindow, cmd: &str, a: Value) -> Result<Value, String> {
    let path = || a["path"].as_str().map(PathBuf::from).ok_or_else(|| "path fehlt".to_string());
    match cmd {
        "providers" => Ok(providers_json()),
        "isConfigured" => Ok(json!(is_configured())),
        "testConnection" => {
            let cfg = config_with(&a["values"]).map_err(|e| e.message())?;
            let secs = validate_config(&cfg).map_err(|e| e.message())?;
            Ok(json!({ "ok": true, "seconds": secs }))
        }
        // Zugangsdaten für den Assistenten (aus dem Schlüsselbund).
        "credentials" => {
            let p = a["provider"].as_str().and_then(Provider::parse).or_else(|| crate::settings::get().cloud_share_provider.as_deref().and_then(Provider::parse));
            Ok(match p {
                Some(p) => json!({ "accessKey": keychain_get(p, "accessKey").unwrap_or_default(), "secretKey": keychain_get(p, "secretKey").unwrap_or_default() }),
                None => json!({ "accessKey": "", "secretKey": "" }),
            })
        }
        "saveConfig" => save_config(app, &a["values"]).map(|_| Value::Null),
        "reset" => {
            if let Some(p) = crate::settings::get().cloud_share_provider.as_deref().and_then(Provider::parse) {
                let svc = keychain_service(p);
                core::keychain_delete(&svc, "accessKey");
                core::keychain_delete(&svc, "secretKey");
            }
            crate::settings::update(app, |s| {
                s.cloud_share_provider = None;
                s.cloud_share_url_prefix = None;
                s.cloud_share_account_id = None;
                s.cloud_share_bucket = None;
                s.cloud_share_region = None;
                s.cloud_share_endpoint = None;
                s.cloud_share_path_prefix = None;
                s.cloud_share_access_key = None;
                s.cloud_share_secret_key = None;
                s.cloud_share_auto_upload = false;
            });
            Ok(Value::Null)
        }
        "upload" => upload(app, &path()?).map(Value::String),
        "uploadAndCopy" => {
            upload_and_copy(app, &path()?);
            Ok(Value::Null)
        }
        "isUploading" => Ok(json!(is_uploading(&path()?))),
        "deleteCloudCopy" => delete_cloud_copy(a["url"].as_str().unwrap_or("")).map(|_| Value::Null),
        _ => Err(format!("share: unbekannter Befehl {cmd}")),
    }
}

// ───────────────────────────── Tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 7, 30, 12, 0, 0).unwrap()
    }

    fn cfg(provider: Provider, fields: &[(&str, &str)]) -> ShareConfig {
        let get = |k: &str| fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.to_string());
        ShareConfig {
            provider,
            url_prefix: normalize_prefix("https://cdn.example.com"),
            bucket: get("bucket").unwrap_or_else(|| "capso".into()),
            account_id: get("accountID").unwrap_or_default(),
            region: get("region").unwrap_or_default(),
            endpoint: get("endpoint"),
            path_prefix: get("pathPrefix"),
            access_key: get("ak").unwrap_or_else(|| "id".into()),
            secret_key: get("sk").unwrap_or_else(|| "secret".into()),
        }
    }

    // ── AWS-Testvektoren ──

    #[test]
    fn sigv4_signing_key_aws_docs_vector() {
        // AWS-Doku „Examples of how to derive a signing key“.
        let k = sigv4_signing_key("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20120215", "us-east-1", "iam");
        assert_eq!(hex::encode(k), "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d");
    }

    #[test]
    fn sigv4_get_vanilla_test_suite() {
        // aws-sig-v4-test-suite: get-vanilla
        let auth = sigv4_authorization(
            "GET",
            "/",
            "",
            &[("Host", "example.amazonaws.com"), ("X-Amz-Date", "20150830T123600Z")],
            EMPTY_PAYLOAD_HASH,
            "20150830T123600Z",
            "us-east-1",
            "service",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn sigv4_s3_get_object_docs_example() {
        // Amazon-S3-Doku „Signature Calculations for the Authorization Header“ – GET Object.
        let auth = sigv4_authorization(
            "GET",
            "/test.txt",
            "",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", EMPTY_PAYLOAD_HASH),
                ("x-amz-date", "20130524T000000Z"),
            ],
            EMPTY_PAYLOAD_HASH,
            "20130524T000000Z",
            "us-east-1",
            "s3",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        );
        assert!(auth.ends_with("Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"), "{auth}");
        assert!(auth.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
    }

    // ── Adressierung (Capso S3CompatibleHTTPTests) ──

    #[test]
    fn amazon_virtual_hosted() {
        let t = amazon_s3_target("capso", "clips/demo.mp4", "us-west-2", None);
        assert_eq!(t.scheme, "https");
        assert_eq!(t.host, "capso.s3.us-west-2.amazonaws.com");
        assert_eq!(t.canonical_uri, "/clips/demo.mp4");
        assert_eq!(t.url(), "https://capso.s3.us-west-2.amazonaws.com/clips/demo.mp4");
        assert_eq!(amazon_s3_target("capso", "a.png", "eu-central-1", Some("   ")).host, "capso.s3.eu-central-1.amazonaws.com");
    }

    #[test]
    fn custom_endpoints_path_style() {
        let t = amazon_s3_target("capso", "clips/demo one.mp4", "us-east-1", Some("https://minio.example.com/"));
        assert_eq!(t.url(), "https://minio.example.com/capso/clips/demo%20one.mp4");
        let t = amazon_s3_target("shots", "x.png", "us-east-1", Some("http://127.0.0.1:9000"));
        assert_eq!((t.scheme.as_str(), t.host.as_str(), t.canonical_uri.as_str()), ("http", "127.0.0.1:9000", "/shots/x.png"));
        let r = s3_signed_request("PUT", &t, "us-east-1", "AKID", "SECRET", Some("image/png"), &sha256_hex(b"x"), Some(1), fixed()).unwrap();
        assert_eq!(r.url, "http://127.0.0.1:9000/shots/x.png");
        assert_eq!(r.header("Host"), Some("127.0.0.1:9000"));
        let t = amazon_s3_target("capso", "a.png", "us-east-1", Some("https://minio.example.com/base"));
        assert_eq!((t.host.as_str(), t.canonical_uri.as_str()), ("minio.example.com", "/base/capso/a.png"));
        assert_eq!(parse_endpoint("minio.lan:9000"), Some(ParsedEndpoint { scheme: "https".into(), host: "minio.lan:9000".into(), path_prefix: String::new() }));
    }

    #[test]
    fn r2_target_and_account_normalization() {
        let t = r2_target("abc123", "capso", "a/b.png");
        assert_eq!((t.host.as_str(), t.canonical_uri.as_str()), ("abc123.r2.cloudflarestorage.com", "/capso/a/b.png"));
        assert_eq!(r2_target("https://abc123.r2.cloudflarestorage.com", "capso", "x.png").host, "abc123.r2.cloudflarestorage.com");
        assert_eq!(normalize_account_id("https://abc123.r2.cloudflarestorage.com/"), "abc123");
        assert_eq!(normalize_account_id("abc123"), "abc123");
    }

    // ── SigV4 PUT/DELETE ──

    #[test]
    fn signed_put_shape() {
        let payload = b"hello-capso";
        let t = Target { scheme: "https".into(), host: "capso.s3.us-east-1.amazonaws.com".into(), canonical_uri: "/demo.mp4".into() };
        let r = s3_signed_request(
            "PUT",
            &t,
            "us-east-1",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            Some("video/mp4"),
            &sha256_hex(payload),
            Some(payload.len() as u64),
            fixed(),
        )
        .unwrap();
        assert_eq!(r.method, "PUT");
        assert_eq!(r.header("X-Amz-Content-Sha256"), Some(sha256_hex(payload).as_str()));
        assert_eq!(r.header("Content-Type"), Some("video/mp4"));
        assert_eq!(r.header("Content-Length"), Some("11"));
        assert_eq!(r.header("X-Amz-Date"), Some("20260730T120000Z"));
        assert_eq!(r.url, "https://capso.s3.us-east-1.amazonaws.com/demo.mp4");
        let auth = r.header("Authorization").unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20260730/us-east-1/s3/aws4_request, "));
        assert!(auth.contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, "));
    }

    /// Zweite, unabhängige Implementierung (wie Capso `independentAuthorization`).
    fn independent(method: &str, host: &str, uri: &str, region: &str, ak: &str, sk: &str, ct: Option<&str>, ph: &str) -> String {
        let amz = "20260730T120000Z";
        let (ch, sh) = match ct {
            Some(ct) => (format!("content-type:{ct}\nhost:{host}\nx-amz-content-sha256:{ph}\nx-amz-date:{amz}\n"), "content-type;host;x-amz-content-sha256;x-amz-date"),
            None => (format!("host:{host}\nx-amz-content-sha256:{ph}\nx-amz-date:{amz}\n"), "host;x-amz-content-sha256;x-amz-date"),
        };
        let cr = format!("{method}\n{uri}\n\n{ch}\n{sh}\n{ph}");
        let scope = format!("20260730/{region}/s3/aws4_request");
        let sts = format!("AWS4-HMAC-SHA256\n{amz}\n{scope}\n{}", sha256_hex(cr.as_bytes()));
        let mut k = hmac_sha256(format!("AWS4{sk}").as_bytes(), b"20260730");
        for m in [region.as_bytes(), b"s3", b"aws4_request"] {
            k = hmac_sha256(&k, m);
        }
        format!("AWS4-HMAC-SHA256 Credential={ak}/{scope}, SignedHeaders={sh}, Signature={}", hex::encode(hmac_sha256(&k, sts.as_bytes())))
    }

    #[test]
    fn signed_put_matches_independent_signer() {
        let p = b"wire-compatible";
        let t = Target { scheme: "https".into(), host: "bucket.s3.us-west-2.amazonaws.com".into(), canonical_uri: "/clips/demo.mp4".into() };
        let sk = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        let r = s3_signed_request("PUT", &t, "us-west-2", "AKIDEXAMPLE", sk, Some("video/mp4"), &sha256_hex(p), Some(p.len() as u64), fixed()).unwrap();
        assert_eq!(r.header("Authorization").unwrap(), independent("PUT", &t.host, &t.canonical_uri, "us-west-2", "AKIDEXAMPLE", sk, Some("video/mp4"), &sha256_hex(p)));
    }

    #[test]
    fn signed_delete_shape_and_independent() {
        let t = r2_target("abc123", "capso", "old.png");
        let r = s3_signed_request("DELETE", &t, "auto", "AKID", "SECRET", None, EMPTY_PAYLOAD_HASH, None, fixed()).unwrap();
        assert_eq!(r.header("Content-Type"), None);
        assert_eq!(r.header("Content-Length"), None);
        assert_eq!(r.header("X-Amz-Content-Sha256"), Some(EMPTY_PAYLOAD_HASH));
        let auth = r.header("Authorization").unwrap();
        assert!(auth.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date, "));
        assert!(auth.contains("/20260730/auto/s3/aws4_request"));
        assert_eq!(auth, independent("DELETE", &t.host, &t.canonical_uri, "auto", "AKID", "SECRET", None, EMPTY_PAYLOAD_HASH));
    }

    #[test]
    fn empty_payload_hash_and_invalid_host() {
        assert_eq!(sha256_hex(b""), EMPTY_PAYLOAD_HASH);
        let t = Target { scheme: "https".into(), host: "host with spaces".into(), canonical_uri: "/k".into() };
        assert_eq!(
            s3_signed_request("PUT", &t, "us-east-1", "A", "S", Some("text/plain"), EMPTY_PAYLOAD_HASH, Some(0), fixed()),
            Err(ShareError::Unknown("Invalid S3 endpoint URL".into()))
        );
    }

    #[test]
    fn sha256_file_matches_in_memory_across_chunks() {
        let p = std::env::temp_dir().join(format!("nrz-sha-{}.bin", short_id()));
        let data: Vec<u8> = (0..1_500_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let (h, n) = sha256_file(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(n, data.len() as u64);
        assert_eq!(h, sha256_hex(&data));
    }

    // ── Fehlerzuordnung ──

    #[test]
    fn maps_response_errors() {
        for c in ["SignatureDoesNotMatch", "InvalidAccessKeyId", "AccessDenied", "InvalidToken"] {
            assert_eq!(map_response_error(403, &format!("<Error><Code>{c}</Code><Message>nope</Message></Error>")), ShareError::InvalidCredentials);
        }
        for c in ["QuotaExceeded", "EntityTooLarge"] {
            assert_eq!(map_response_error(400, &format!("<Error><Code>{c}</Code></Error>")), ShareError::QuotaExceeded);
        }
        assert_eq!(
            map_response_error(404, "<Error><Code>NoSuchBucket</Code><Message>gone</Message></Error>"),
            ShareError::Unknown("Bucket not found — verify the bucket name in Cloud Share settings".into())
        );
        assert_eq!(map_response_error(503, "<Error><Code>SlowDown</Code><Message>wait</Message></Error>"), ShareError::Unknown("SlowDown: wait".into()));
        assert_eq!(map_response_error(401, ""), ShareError::InvalidCredentials);
        assert_eq!(map_response_error(507, ""), ShareError::QuotaExceeded);
        assert_eq!(map_response_error(500, ""), ShareError::Unknown("HTTP 500".into()));
        assert_eq!(map_response_error(500, "boom"), ShareError::Unknown("HTTP 500: boom".into()));
    }

    // ── Aliyun OSS / Tencent COS ──

    #[test]
    fn oss_signature_docs_vector() {
        // RFC 2202 (HMAC-SHA1, Testfall 1) + OSS-StringToSign, Erwartung unabhängig mit Python `hmac` berechnet.
        assert_eq!(hex::encode(hmac_sha1(&[0x0b; 20], b"Hi There")), "b617318655057264e28bc0b6fb378c8ef146be00");
        let sts = "PUT\nODBGOERFMDMzQTczRUY3NUE3NzA5QzdFNUYzMDQxNEM=\ntext/html\nThu, 17 Nov 2005 18:49:58 GMT\nx-oss-magic:abracadabra\nx-oss-meta-author:foo@example.com\n/oss-example/nelson";
        assert_eq!(base64(&hmac_sha1(b"OtxrzxIsfpFjA7SwPzILwy8Bw21TLhquhboDYROV", sts.as_bytes())), "fV5fq7DPwNbrrig7nvUSZIVyruI=");
    }

    #[test]
    fn oss_request_shape() {
        let c = cfg(Provider::AliyunOss, &[("region", "oss-cn-hangzhou"), ("pathPrefix", "shots")]);
        let r = signed_request(&c, "PUT", &c.object_key("a b.png"), Some("image/png"), Some(("x", 1)), fixed()).unwrap();
        assert_eq!(r.url, "https://capso.oss-cn-hangzhou.aliyuncs.com/shots/a%20b.png");
        assert_eq!(r.header("Date"), Some("Thu, 30 Jul 2026 12:00:00 GMT"));
        assert_eq!(r.header("x-oss-object-acl"), Some("public-read"));
        let sts = "PUT\n\nimage/png\nThu, 30 Jul 2026 12:00:00 GMT\nx-oss-object-acl:public-read\n/capso/shots/a b.png";
        let expected = format!("OSS id:{}", base64(&hmac_sha1(b"secret", sts.as_bytes())));
        assert_eq!(r.header("Authorization"), Some(expected.as_str()));
        assert_eq!(oss_endpoint("https://oss-eu-central-1.aliyuncs.com/"), "oss-eu-central-1.aliyuncs.com");
        let d = signed_request(&c, "DELETE", "k.png", None, None, fixed()).unwrap();
        assert_eq!(d.header("x-oss-object-acl"), None);
    }

    #[test]
    fn cos_request_shape() {
        let c = cfg(Provider::TencentCos, &[("region", "ap-guangzhou"), ("bucket", "capso-1250000000")]);
        let now = fixed().timestamp();
        let r = signed_request(&c, "PUT", "x.png", Some("image/png"), Some(("x", 1)), fixed()).unwrap();
        assert_eq!(r.url, "https://capso-1250000000.cos.ap-guangzhou.myqcloud.com/x.png");
        let kt = format!("{now};{}", now + 3600);
        let sign_key = hex::encode(hmac_sha1(b"secret", kt.as_bytes()));
        let sts = format!("sha1\n{kt}\n{}\n", sha1_hex(b"put\n/x.png\n\n\n"));
        let sig = hex::encode(hmac_sha1(sign_key.as_bytes(), sts.as_bytes()));
        let auth = r.header("Authorization").unwrap();
        assert_eq!(
            auth,
            format!("q-sign-algorithm=sha1&q-ak=id&q-sign-time={kt}&q-key-time={kt}&q-header-list=&q-url-param-list=&q-signature={sig}")
        );
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    // ── ShareConfig / IDGenerator / Gate ──

    #[test]
    fn config_prefixes_and_urls() {
        assert_eq!(normalize_prefix("https://x.com/"), "https://x.com");
        assert_eq!(normalize_prefix("https://x.com///"), "https://x.com");
        assert!(validate_prefix("https://share.example.com").is_ok());
        for bad in ["http://x.com", "https://", "https:///", "https://x.com?foo=bar", "https://x.com#frag"] {
            assert!(matches!(validate_prefix(bad), Err(ShareError::InvalidUrlPrefix(_))), "{bad}");
        }
        let c = cfg(Provider::S3, &[("region", "us-east-1"), ("pathPrefix", "/screenshots/")]);
        assert_eq!(c.object_key("capture one.png"), "screenshots/capture one.png");
        assert_eq!(c.public_url(&c.object_key("capture one.png")), "https://cdn.example.com/screenshots/capture%20one.png");
        assert_eq!(c.object_key_from_url("https://cdn.example.com/screenshots/capture%20one.png").as_deref(), Some("screenshots/capture one.png"));
        assert_eq!(c.object_key_from_url("https://other.com/x.png"), None);
        assert_eq!(cfg(Provider::S3, &[]).public_url("a/b c.png"), "https://cdn.example.com/a/b%20c.png");
        assert_eq!(normalize_path_prefix(None), "");
        assert_eq!(normalize_path_prefix(Some(" / ")), "");
    }

    #[test]
    fn config_validation_required_fields() {
        assert!(cfg(Provider::R2, &[("accountID", "abc123")]).validate_for_upload().is_ok());
        assert_eq!(cfg(Provider::R2, &[]).validate_for_upload(), Err(ShareError::NotConfigured));
        for p in [Provider::S3, Provider::TencentCos, Provider::AliyunOss] {
            assert_eq!(cfg(p, &[]).validate_for_upload(), Err(ShareError::NotConfigured));
            assert!(cfg(p, &[("region", "r")]).validate_for_upload().is_ok());
        }
        assert_eq!(cfg(Provider::S3, &[("region", "r"), ("ak", "")]).validate_for_upload(), Err(ShareError::NotConfigured));
        assert_eq!(Provider::ALL.map(|p| p.id()), ["r2", "s3", "tencentCOS", "aliyunOSS"]);
        assert_eq!(Provider::parse("aliyunOSS"), Some(Provider::AliyunOss));
        assert_eq!(Provider::S3.display_name(), "Amazon S3");
    }

    #[test]
    fn short_ids() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let id = short_id();
            assert_eq!(id.len(), 7);
            assert!(id.bytes().all(|b| ID_CHARSET.contains(&b)));
            seen.insert(id);
        }
        assert_eq!(seen.len(), 1000);
    }

    #[test]
    fn upload_gate() {
        let mut g = UploadAttemptGate::default();
        assert!(g.begin("a"));
        assert!(!g.begin("a"));
        assert!(g.begin("b"));
        g.finish("a");
        assert!(g.begin("a"));
    }

    #[test]
    fn percent_round_trip() {
        assert_eq!(percent_encode("ä b/c"), "%C3%A4%20b%2Fc");
        assert_eq!(percent_decode("%C3%A4%20b"), "ä b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(content_type(Path::new("a.MP4")), "video/mp4");
    }
}
