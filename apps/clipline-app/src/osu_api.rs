//! Direct osu! API integration for play-block enrichment.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::ptr;
use std::slice;

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::app::RuntimeState;
use crate::library::StorageSettings;
use crate::settings::OsuApiSettings;
use crate::util::{last_os_error, wide_null};

const OSU_TOKEN_URL: &str = "https://osu.ppy.sh/oauth/token";
const OSU_API_VERSION: &str = "20220705";
const RECENT_LIMIT: usize = 100;
const RECENT_SCORE_CEILING: usize = 500;
const OSU_RECENT_MODE: &str = "osu";
const CREDENTIAL_PREFIX: &str = "Clipline osu!";

#[derive(Debug, Deserialize)]
pub struct SaveOsuApiSettingsRequest {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    pub user: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OsuApiConnectionStatus {
    pub configured: bool,
    pub secret_present: bool,
    pub client_id: Option<String>,
    pub user: Option<String>,
    pub credential_target: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OsuApiConnectionTestResult {
    pub status: OsuApiConnectionStatus,
    pub score_count: usize,
    pub failed_count: usize,
    pub started_at_count: usize,
    pub ended_at_count: usize,
    pub pagination_ceiling_reached: bool,
}

#[derive(Debug, Clone)]
struct OsuApiConfig {
    client_id: String,
    client_secret: String,
    user: String,
}

#[derive(Debug, Clone)]
struct OsuRecentFetch {
    user_id: String,
    scores: Vec<crate::osu_enrichment::OsuProxyScore>,
    failed_count: usize,
    started_at_count: usize,
    ended_at_count: usize,
    pagination_ceiling_reached: bool,
    username: Option<String>,
}

#[tauri::command]
pub fn osu_api_status(
    state: tauri::State<'_, RuntimeState>,
) -> Result<OsuApiConnectionStatus, String> {
    if let Err(error) = reconcile_osu_credential_cleanup(&state) {
        eprintln!("reconcile pending osu! credentials: {error}");
    }
    Ok(status_from_settings(&state.settings().osu))
}

#[tauri::command]
pub fn save_osu_api_settings(
    state: tauri::State<'_, RuntimeState>,
    request: SaveOsuApiSettingsRequest,
) -> Result<OsuApiConnectionStatus, String> {
    let client_id = clean_required(&request.client_id, "osu! client id")?;
    if client_id.parse::<u64>().is_err() {
        return Err("osu! client id must be a number".into());
    }
    let user = clean_required(&request.user, "osu! user id or username")?;
    let secret = request
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let old_target = state.settings().osu.credential_target;
    let old_secret = if secret.is_none() {
        old_target
            .as_deref()
            .and_then(|target| read_secret(target).ok())
    } else {
        None
    };
    let plan = plan_osu_credential_save(
        &client_id,
        &user,
        secret,
        old_target.as_deref(),
        old_secret.as_deref(),
    )?;
    let persist = || {
        state.update_osu(|osu| {
            osu.client_id = Some(client_id.clone());
            osu.user = Some(user.clone());
            osu.credential_target = Some(plan.target.clone());
            if let Some(delete_target) = plan.delete_target.clone() {
                osu.credential_cleanup_targets.push(delete_target);
            }
            if old_target.as_deref() != Some(plan.target.as_str()) {
                osu.last_connected_username = None;
            }
        })
    };
    let settings = if let Some(secret) = plan.secret_to_write.as_deref() {
        let previous_target_secret = read_secret(&plan.target).ok();
        crate::credential_transaction::write_then_persist(
            &plan.target,
            &user,
            secret,
            previous_target_secret.as_deref(),
            write_secret,
            delete_secret_if_present,
            persist,
        )?
    } else {
        persist()?
    };
    if let Err(error) = reconcile_osu_credential_cleanup(&state) {
        eprintln!("reconcile old osu! credentials: {error}");
    }
    Ok(status_from_settings(&settings.osu))
}

#[tauri::command]
pub async fn test_osu_api_connection(
    state: tauri::State<'_, RuntimeState>,
    storage: tauri::State<'_, StorageSettings>,
) -> Result<OsuApiConnectionTestResult, String> {
    let settings = state.settings().osu;
    let config = config_from_settings(&settings)?;
    let fetch = fetch_recent_scores(&config, None).await?;
    let username = fetch.username.clone();
    let user_id = fetch.user_id.clone();
    let target = credential_target(&config.client_id, &user_id);
    let old_target = settings.credential_target.clone();
    let persist = || {
        state.update_osu(|osu| {
            osu.user = Some(user_id.clone());
            osu.credential_target = Some(target.clone());
            if let Some(old) = old_target.as_deref().filter(|old| *old != target) {
                osu.credential_cleanup_targets.push(old.to_string());
            }
            if let Some(username) = username.clone() {
                osu.last_connected_username = Some(username);
            }
        })
    };
    let next = if old_target.as_deref() != Some(target.as_str()) {
        let previous_target_secret = read_secret(&target).ok();
        crate::credential_transaction::write_then_persist(
            &target,
            &user_id,
            &config.client_secret,
            previous_target_secret.as_deref(),
            write_secret,
            delete_secret_if_present,
            persist,
        )?
    } else {
        persist()?
    };
    if let Err(error) = reconcile_osu_credential_cleanup(&state) {
        eprintln!("reconcile migrated osu! credentials: {error}");
    }
    let status = status_from_settings(&next.osu);
    let media_root = storage.media_dir();
    if let Err(e) = retry_pending_enrichment_with_settings(&next.osu, media_root).await {
        eprintln!("retry osu! enrichment after connection test: {e}");
    }
    Ok(OsuApiConnectionTestResult {
        status,
        score_count: fetch.scores.len(),
        failed_count: fetch.failed_count,
        started_at_count: fetch.started_at_count,
        ended_at_count: fetch.ended_at_count,
        pagination_ceiling_reached: fetch.pagination_ceiling_reached,
    })
}

#[tauri::command]
pub fn open_osu_api_setup_guide() -> Result<(), String> {
    let path = crate::settings::persistence::config_base().join("osu-api-setup-guide.html");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create osu! guide dir: {e}"))?;
    }
    std::fs::write(&path, osu_setup_guide_html())
        .map_err(|e| format!("write osu! setup guide: {e}"))?;
    open_path(&path, "osu! API setup guide")
}

pub async fn retry_pending_enrichment<R: Runtime>(
    app: &AppHandle<R>,
    media_root: PathBuf,
) -> Result<(), String> {
    let settings = app.state::<RuntimeState>().settings().osu;
    if retry_pending_enrichment_with_settings(&settings, media_root).await? {
        let _ = app.emit("osu-enrichment-updated", ());
    }
    Ok(())
}

async fn retry_pending_enrichment_with_settings(
    settings: &OsuApiSettings,
    media_root: PathBuf,
) -> Result<bool, String> {
    let config = match config_from_settings(settings) {
        Ok(config) => config,
        Err(e) => {
            let pending = crate::osu_enrichment::discover_pending(&media_root)?;
            if !pending.is_empty() {
                eprintln!("osu! enrichment pending: {e}");
            }
            return Ok(false);
        }
    };
    let pending = crate::osu_enrichment::discover_pending(&media_root)?;
    if pending.is_empty() {
        return Ok(false);
    }
    let earliest = pending
        .iter()
        .map(|job| job.record().recording_start_unix)
        .min();
    let fetch = fetch_recent_scores(&config, earliest).await?;
    let mut updated = false;
    for job in pending {
        match crate::osu_enrichment::apply_scores_to_pending(
            &job,
            &fetch.scores,
            fetch.pagination_ceiling_reached,
        ) {
            Ok(mapped) => {
                if !mapped.plays.is_empty() {
                    updated = true;
                }
                eprintln!(
                    "osu! enrichment complete for {}: {} play(s)",
                    job.clip_path().display(),
                    mapped.plays.len()
                );
            }
            Err(e) => {
                eprintln!(
                    "osu! enrichment failed for {}: {e}",
                    job.clip_path().display()
                );
                let _ = crate::osu_enrichment::mark_pending_failed(&job, &e);
            }
        }
    }
    Ok(updated)
}

async fn fetch_recent_scores(
    config: &OsuApiConfig,
    stop_before_unix: Option<i64>,
) -> Result<OsuRecentFetch, String> {
    let client = crate::bounded_http::control_client()?;
    let token = request_app_token(client, config).await?;
    let resolved_user = resolve_osu_user(client, &token, &config.user).await?;
    let mut offset = 0usize;
    let mut scores = Vec::new();
    let mut failed_count = 0usize;
    let mut started_at_count = 0usize;
    let mut ended_at_count = 0usize;
    let mut username = resolved_user.username.clone();
    let mut pagination_ceiling_reached = false;

    while offset < RECENT_SCORE_CEILING {
        let raw = request_recent_page(client, &token, &resolved_user.id, offset).await?;
        if raw.is_empty() {
            break;
        }
        let page_len = raw.len();
        let mut oldest_ended_at = None;
        for score in raw {
            if let Some(name) = score
                .user
                .as_ref()
                .and_then(|user| clean_optional(user.username.clone()))
            {
                username = Some(name);
            }
            if !score.passed {
                failed_count += 1;
            }
            if score.started_at.is_some() {
                started_at_count += 1;
            }
            if score.ended_at.is_some() {
                ended_at_count += 1;
            }
            match normalize_score(score) {
                Ok(score) => {
                    oldest_ended_at = Some(
                        oldest_ended_at
                            .map(|oldest: i64| oldest.min(score.ended_at_unix))
                            .unwrap_or(score.ended_at_unix),
                    );
                    scores.push(score);
                }
                Err(e) => eprintln!("skip osu! recent score: {e}"),
            }
        }
        offset += RECENT_LIMIT;
        if page_len < RECENT_LIMIT {
            break;
        }
        if let (Some(stop), Some(oldest)) = (stop_before_unix, oldest_ended_at) {
            if oldest < stop.saturating_sub(5) {
                break;
            }
        }
        if offset >= RECENT_SCORE_CEILING {
            pagination_ceiling_reached = true;
        }
    }

    Ok(OsuRecentFetch {
        user_id: resolved_user.id,
        scores,
        failed_count,
        started_at_count,
        ended_at_count,
        pagination_ceiling_reached,
        username,
    })
}

#[derive(Debug, Clone)]
struct ResolvedOsuUser {
    id: String,
    username: Option<String>,
}

async fn resolve_osu_user(
    client: &reqwest::Client,
    token: &str,
    user: &str,
) -> Result<ResolvedOsuUser, String> {
    let user = user.trim();
    if user.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(ResolvedOsuUser {
            id: user.to_string(),
            username: None,
        });
    }

    #[derive(Deserialize)]
    struct UserResponse {
        id: u64,
        #[serde(default)]
        username: Option<String>,
    }

    let mut url = reqwest::Url::parse("https://osu.ppy.sh/api/v2/users")
        .map_err(|e| format!("build osu! user lookup URL: {e}"))?;
    url.path_segments_mut()
        .map_err(|_| "build osu! user lookup URL path".to_string())?
        .push(&osu_user_lookup_segment(user))
        .push(OSU_RECENT_MODE);

    let response = client
        .get(url)
        .bearer_auth(token)
        .header("x-api-version", OSU_API_VERSION)
        .send()
        .await
        .map_err(|e| format!("resolve osu! user: {e}"))?;
    let status = response.status();
    let user: UserResponse = osu_json_response(response, status, "resolve osu! user").await?;
    Ok(ResolvedOsuUser {
        id: user.id.to_string(),
        username: user.username,
    })
}

async fn request_app_token(
    client: &reqwest::Client,
    config: &OsuApiConfig,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
    }

    let response = client
        .post(OSU_TOKEN_URL)
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("grant_type", "client_credentials"),
            ("scope", "public"),
        ])
        .send()
        .await
        .map_err(|e| format!("request osu! token: {e}"))?;
    let status = response.status();
    let token: TokenResponse = osu_json_response(response, status, "request osu! token").await?;
    Ok(token.access_token)
}

async fn request_recent_page(
    client: &reqwest::Client,
    token: &str,
    user: &str,
    offset: usize,
) -> Result<Vec<RawOsuScore>, String> {
    let mut url = reqwest::Url::parse("https://osu.ppy.sh/api/v2/users")
        .map_err(|e| format!("build osu! recent URL: {e}"))?;
    url.path_segments_mut()
        .map_err(|_| "build osu! recent URL path".to_string())?
        .push(user)
        .push("scores")
        .push("recent");
    url.query_pairs_mut()
        .append_pair("include_fails", "1")
        .append_pair("legacy_only", "0")
        .append_pair("mode", OSU_RECENT_MODE)
        .append_pair("limit", &RECENT_LIMIT.to_string())
        .append_pair("offset", &offset.to_string());

    let response = client
        .get(url)
        .bearer_auth(token)
        .header("x-api-version", OSU_API_VERSION)
        .send()
        .await
        .map_err(|e| format!("fetch osu! recent scores: {e}"))?;
    let status = response.status();
    osu_json_response(response, status, "fetch osu! recent scores").await
}

async fn osu_json_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    status: reqwest::StatusCode,
    context: &str,
) -> Result<T, String> {
    if !status.is_success() {
        let message = crate::bounded_http::response_error_message(response, status, context).await;
        return Err(format!("{context} failed with {status}: {message}"));
    }
    crate::bounded_http::response_json_limited(
        response,
        crate::bounded_http::CONTROL_JSON_MAX_BYTES,
        context,
    )
    .await
}

#[derive(Debug, Deserialize)]
struct RawOsuScore {
    id: serde_json::Value,
    #[serde(default)]
    beatmap: Option<RawOsuBeatmap>,
    #[serde(default)]
    beatmapset: Option<RawOsuBeatmapset>,
    #[serde(default)]
    mods: Vec<RawOsuMod>,
    #[serde(default)]
    rank: Option<String>,
    #[serde(default)]
    passed: bool,
    #[serde(default)]
    accuracy: Option<f64>,
    #[serde(default)]
    max_combo: Option<u32>,
    #[serde(default)]
    total_score: Option<u64>,
    #[serde(default)]
    score: Option<u64>,
    #[serde(default)]
    pp: Option<f64>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    ended_at: Option<String>,
    #[serde(default)]
    user: Option<RawOsuUser>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOsuBeatmap {
    #[serde(default)]
    id: Option<u32>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    difficulty_rating: Option<f64>,
    #[serde(default)]
    total_length: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOsuBeatmapset {
    #[serde(default)]
    id: Option<u32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    creator: Option<String>,
    #[serde(default)]
    covers: RawOsuBeatmapsetCovers,
}

#[derive(Debug, Default, Deserialize)]
struct RawOsuBeatmapsetCovers {
    #[serde(default)]
    list: Option<String>,
    #[serde(default)]
    card: Option<String>,
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    slimcover: Option<String>,
    #[serde(default, rename = "list@2x")]
    list_2x: Option<String>,
    #[serde(default, rename = "card@2x")]
    card_2x: Option<String>,
    #[serde(default, rename = "cover@2x")]
    cover_2x: Option<String>,
    #[serde(default, rename = "slimcover@2x")]
    slimcover_2x: Option<String>,
}

impl RawOsuBeatmapsetCovers {
    fn best_rail_cover(self) -> Option<String> {
        [
            self.list,
            self.card,
            self.cover,
            self.slimcover,
            self.list_2x,
            self.card_2x,
            self.cover_2x,
            self.slimcover_2x,
        ]
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct RawOsuUser {
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawOsuMod {
    Object { acronym: String },
    Text(String),
}

fn normalize_score(score: RawOsuScore) -> Result<crate::osu_enrichment::OsuProxyScore, String> {
    let id = score_id(score.id)?;
    let ended_at_unix = parse_required_time(score.ended_at.as_deref(), "ended_at")?;
    let started_at_unix = score
        .started_at
        .as_deref()
        .map(|value| parse_required_time(Some(value), "started_at"))
        .transpose()?;
    let beatmap = score.beatmap.unwrap_or_default();
    let beatmapset = score.beatmapset.unwrap_or_default();
    Ok(crate::osu_enrichment::OsuProxyScore {
        url: Some(format!("https://osu.ppy.sh/scores/osu/{id}")),
        id,
        beatmap_id: beatmap.id,
        beatmapset_id: beatmapset.id,
        cover_url: beatmapset.covers.best_rail_cover(),
        title: beatmapset.title.unwrap_or_else(|| "Unknown beatmap".into()),
        artist: beatmapset.artist.unwrap_or_else(|| "Unknown artist".into()),
        difficulty: beatmap
            .version
            .unwrap_or_else(|| "Unknown difficulty".into()),
        mapper: beatmapset.creator,
        star_rating: beatmap.difficulty_rating,
        mods: score.mods.into_iter().map(mod_acronym).collect(),
        rank: score.rank,
        passed: score.passed,
        accuracy: score.accuracy,
        max_combo: score.max_combo,
        total_score: score.total_score.or(score.score),
        pp: score.pp,
        started_at_unix,
        ended_at_unix,
        beatmap_total_length_s: beatmap.total_length,
    })
}

fn mod_acronym(value: RawOsuMod) -> String {
    match value {
        RawOsuMod::Object { acronym } => acronym,
        RawOsuMod::Text(value) => value,
    }
}

fn score_id(value: serde_json::Value) -> Result<String, String> {
    match value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .map(|value| value.to_string())
            .ok_or_else(|| "score id is not an unsigned integer".to_string()),
        serde_json::Value::String(value) if !value.trim().is_empty() => Ok(value),
        _ => Err("score id is missing".into()),
    }
}

fn parse_required_time(value: Option<&str>, field: &str) -> Result<i64, String> {
    let value = value.ok_or_else(|| format!("score {field} is missing"))?;
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp())
        .map_err(|e| format!("score {field} is invalid: {e}"))
}

fn config_from_settings(settings: &OsuApiSettings) -> Result<OsuApiConfig, String> {
    let client_id = settings
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "osu! client id is not configured".to_string())?
        .to_string();
    let user = settings
        .user
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "osu! user id or username is not configured".to_string())?
        .to_string();
    let target = settings
        .credential_target
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "osu! client secret is not stored".to_string())?;
    let client_secret = read_secret(target)?;
    Ok(OsuApiConfig {
        client_id,
        client_secret,
        user,
    })
}

fn status_from_settings(settings: &OsuApiSettings) -> OsuApiConnectionStatus {
    let secret_present = settings
        .credential_target
        .as_deref()
        .is_some_and(|target| read_secret(target).is_ok());
    let configured = settings.client_id.is_some() && settings.user.is_some() && secret_present;
    OsuApiConnectionStatus {
        configured,
        secret_present,
        client_id: settings.client_id.clone(),
        user: settings.user.clone(),
        credential_target: settings.credential_target.clone(),
        username: settings.last_connected_username.clone(),
    }
}

fn clean_required(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(trimmed.to_string())
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn credential_target(client_id: &str, user: &str) -> String {
    format!("{CREDENTIAL_PREFIX}:{client_id}:{user}")
}

#[derive(Debug, PartialEq, Eq)]
struct OsuCredentialSavePlan {
    target: String,
    secret_to_write: Option<String>,
    delete_target: Option<String>,
}

fn plan_osu_credential_save(
    client_id: &str,
    user: &str,
    new_secret: Option<&str>,
    old_target: Option<&str>,
    old_secret: Option<&str>,
) -> Result<OsuCredentialSavePlan, String> {
    let target = credential_target(client_id, user);
    if let Some(new_secret) = new_secret {
        let delete_target = old_target
            .filter(|old_target| *old_target != target)
            .map(str::to_string);
        return Ok(OsuCredentialSavePlan {
            target,
            secret_to_write: Some(new_secret.to_string()),
            delete_target,
        });
    }

    if old_target == Some(target.as_str()) {
        return Ok(OsuCredentialSavePlan {
            target,
            secret_to_write: None,
            delete_target: None,
        });
    }

    let Some(old_secret) = old_secret else {
        return Err(
            "osu! client secret is required because the saved secret is missing".to_string(),
        );
    };
    Ok(OsuCredentialSavePlan {
        target,
        secret_to_write: Some(old_secret.to_string()),
        delete_target: old_target.map(str::to_string),
    })
}

fn osu_user_lookup_segment(user: &str) -> String {
    let user = user.trim();
    if user.starts_with('@') || user.chars().all(|ch| ch.is_ascii_digit()) {
        user.to_string()
    } else {
        format!("@{user}")
    }
}

fn write_secret(target: &str, username: &str, secret: &str) -> Result<(), String> {
    let mut target_w = wide_null(OsStr::new(target));
    let mut username_w = wide_null(OsStr::new(username));
    let mut blob = secret.as_bytes().to_vec();
    let blob_len =
        u32::try_from(blob.len()).map_err(|_| "osu! client secret is too large".to_string())?;
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target_w.as_mut_ptr(),
        Comment: ptr::null_mut(),
        LastWritten: Default::default(),
        CredentialBlobSize: blob_len,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: ptr::null_mut(),
        TargetAlias: ptr::null_mut(),
        UserName: username_w.as_mut_ptr(),
    };
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(last_os_error("store osu! client secret"));
    }
    Ok(())
}

fn read_secret(target: &str) -> Result<String, String> {
    let target_w = wide_null(OsStr::new(target));
    let mut raw: *mut CREDENTIALW = ptr::null_mut();
    if unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 {
        return Err(last_os_error("read osu! client secret"));
    }
    let _free = CredentialFree(raw);
    let credential = unsafe { &*raw };
    let bytes = unsafe {
        slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
    };
    String::from_utf8(bytes.to_vec())
        .map_err(|_| "osu! client secret is not valid UTF-8".to_string())
}

fn delete_secret_if_present(target: &str) -> Result<(), String> {
    let target_w = wide_null(OsStr::new(target));
    if unsafe { CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
        return Ok(());
    }
    if unsafe { GetLastError() } == ERROR_NOT_FOUND {
        return Ok(());
    }
    Err(last_os_error("delete osu! client secret"))
}

fn reconcile_osu_credential_cleanup(state: &RuntimeState) -> Result<(), String> {
    let targets = state.settings().osu.credential_cleanup_targets;
    if targets.is_empty() {
        return Ok(());
    }
    let report = crate::credential_transaction::cleanup_targets(targets, delete_secret_if_present);
    let deleted = report.deleted;
    if !deleted.is_empty() {
        state.update_osu(|osu| {
            osu.credential_cleanup_targets
                .retain(|target| !deleted.contains(target));
        })?;
    }
    if report.failures.is_empty() {
        Ok(())
    } else {
        Err(report.failures.join(", "))
    }
}

struct CredentialFree(*mut CREDENTIALW);

impl Drop for CredentialFree {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CredFree(self.0.cast());
            }
        }
    }
}

fn open_path(path: &std::path::Path, context: &str) -> Result<(), String> {
    let operation = wide_null(OsStr::new("open"));
    let target = wide_null(path.as_os_str());
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(format!("{context} failed with shell code {result:?}"));
    }
    Ok(())
}

fn osu_setup_guide_html() -> &'static str {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Clipline osu! API setup</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, Segoe UI, sans-serif; background: #111317; color: #f5f7fb; }
    body { margin: 0; padding: 32px; line-height: 1.5; }
    main { max-width: 780px; margin: 0 auto; }
    h1 { margin: 0 0 8px; font-size: 28px; }
    h2 { margin-top: 28px; font-size: 18px; }
    a { color: #ff8ac6; }
    code { padding: 2px 5px; border-radius: 4px; background: #20242c; }
    li { margin: 8px 0; }
    .note { padding: 12px 14px; border: 1px solid #343946; border-radius: 8px; background: #181b22; }
  </style>
</head>
<body>
<main>
  <h1>Clipline osu! API setup</h1>
  <p class="note">Clipline uses an osu! OAuth app with the client credentials grant. Your client secret is stored locally in Windows Credential Manager and is never written to settings.json.</p>
  <h2>Create the osu! OAuth app</h2>
  <ol>
    <li>Open <a href="https://osu.ppy.sh/home/account/edit#oauth" target="_blank" rel="noreferrer">osu! account OAuth settings</a>.</li>
    <li>Create a new OAuth application.</li>
    <li>Name it <code>Clipline</code> or another name you recognize.</li>
    <li>For Application Callback URL, enter <code>http://127.0.0.1</code>. Clipline does not use the callback for this direct API mode, but osu! requires a value.</li>
    <li>Copy the Client ID and Client Secret into Clipline.</li>
    <li>Enter your osu! user id or username, then click <strong>Test osu! API connection</strong>.</li>
  </ol>
  <h2>What Clipline reads</h2>
  <p>Clipline requests only the public scope and fetches recent osu!standard scores, including failed submitted plays when osu! returns them.</p>
</main>
</body>
</html>
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osu_enrichment::{pending_path, OsuEnrichmentStatus, OsuPendingEnrichment};
    use clipline_test_utils::TestDir;

    #[test]
    fn non_numeric_usernames_are_resolved_before_recent_score_requests() {
        assert_eq!(osu_user_lookup_segment("Dain"), "@Dain");
        assert_eq!(osu_user_lookup_segment("@Dain"), "@Dain");
        assert_eq!(osu_user_lookup_segment("3426414"), "3426414");

        let resolved = ResolvedOsuUser {
            id: "3426414".into(),
            username: Some("Dain".into()),
        };
        assert_eq!(resolved.id, "3426414");
        assert_eq!(resolved.username.as_deref(), Some("Dain"));
    }

    #[test]
    fn normalize_score_keeps_beatmap_cover_and_star_rating() {
        let raw: RawOsuScore = serde_json::from_value(serde_json::json!({
            "id": 998877,
            "beatmap": {
                "id": 123,
                "version": "Extra",
                "total_length": 178,
                "difficulty_rating": 6.54321
            },
            "beatmapset": {
                "id": 456,
                "title": "Exit This Earth's Atomosphere",
                "artist": "Camellia",
                "creator": "Sotarks",
                "covers": {
                    "list": "https://assets.ppy.sh/beatmaps/456/covers/list.jpg",
                    "card": "https://assets.ppy.sh/beatmaps/456/covers/card.jpg"
                }
            },
            "mods": [{"acronym": "HD"}],
            "rank": "A",
            "passed": true,
            "accuracy": 0.9876,
            "ended_at": "2026-07-01T04:10:00Z"
        }))
        .expect("deserialize score");

        let score = normalize_score(raw).expect("normalize score");

        assert_eq!(
            score.cover_url.as_deref(),
            Some("https://assets.ppy.sh/beatmaps/456/covers/list.jpg")
        );
        assert_eq!(score.star_rating, Some(6.54321));
    }

    #[test]
    fn blank_secret_save_reuses_existing_secret_when_target_changes() {
        let plan = plan_osu_credential_save(
            "61835",
            "Dain",
            None,
            Some("Clipline osu!:61835:3426414"),
            Some("stored-secret"),
        )
        .expect("existing secret can be reused");

        assert_eq!(plan.target, "Clipline osu!:61835:Dain");
        assert_eq!(plan.secret_to_write.as_deref(), Some("stored-secret"));
        assert_eq!(
            plan.delete_target.as_deref(),
            Some("Clipline osu!:61835:3426414")
        );
    }

    #[test]
    fn blank_secret_save_without_existing_secret_keeps_settings_unchanged() {
        let error = plan_osu_credential_save(
            "61835",
            "Dain",
            None,
            Some("Clipline osu!:61835:3426414"),
            None,
        )
        .expect_err("missing stored secret should be actionable");

        assert!(error.contains("client secret"));
    }

    #[tokio::test]
    async fn pending_retry_without_api_credentials_reports_no_visible_update() {
        let dir = TestDir::new("clipline-osu-api", "retry-no-credentials");
        let clip = dir.path().join("session.mp4");
        std::fs::write(&clip, b"").unwrap();
        let pending = OsuPendingEnrichment {
            schema_version: 1,
            clip_path: clip.display().to_string(),
            recording_start_unix: 1_820_000_000,
            recording_end_unix: 1_820_000_120,
            clip_duration_s: 120.0,
            status: OsuEnrichmentStatus::Pending,
            attempts: 0,
            pagination_ceiling_reached: false,
            title_events: Vec::new(),
            message: None,
        };
        std::fs::write(
            pending_path(&clip),
            serde_json::to_string_pretty(&pending).unwrap(),
        )
        .unwrap();

        let changed =
            retry_pending_enrichment_with_settings(&OsuApiSettings::default(), dir.path().into())
                .await
                .unwrap();

        assert!(
            !changed,
            "missing osu! API credentials should not trigger an osu-enrichment-updated refresh loop"
        );
    }
}
