use chrono::{Local, DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageData {
    pub session_pct: f64,
    pub session_resets: String,
    pub session_resets_secs: u64,
    pub weekly_pct: f64,
    pub weekly_resets: String,
    pub weekly_resets_secs: u64,
    pub extra_used_cents: f64,
    pub extra_limit_cents: f64,
    pub today_messages: u64,
    pub today_tool_calls: u64,
    /// True when the API call failed and we are showing the last known values.
    #[serde(default)]
    pub stale: bool,
    /// Epoch seconds when this dataset was fetched successfully (0 if unknown).
    #[serde(default)]
    pub fetched_at: i64,
    /// Epoch seconds of the last attempt (success or failure).
    #[serde(default)]
    pub attempted_at: i64,
}

// ── OAuth API structs ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: OAuthCreds,
}

#[derive(Deserialize)]
struct OAuthCreds {
    #[serde(rename = "accessToken")]
    access_token: String,
}

#[derive(Deserialize)]
struct UsageWindow {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Deserialize)]
struct ExtraUsage {
    used_credits: Option<f64>,
    monthly_limit: Option<f64>,
}

#[derive(Deserialize)]
struct OAuthUsageResponse {
    five_hour: Option<UsageWindow>,
    seven_day: Option<UsageWindow>,
    extra_usage: Option<ExtraUsage>,
}

// ── Web session structs ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct OrgItem {
    uuid: String,
    capabilities: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct WebUsageResponse {
    five_hour: Option<UsageWindow>,
    seven_day: Option<UsageWindow>,
}

#[derive(Deserialize)]
struct OverageSpendLimit {
    monthly_credit_limit: Option<f64>,
    used_credits: Option<f64>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn claude_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude")
}

fn read_access_token() -> Option<String> {
    let data = fs::read_to_string(claude_dir().join(".credentials.json")).ok()?;
    let creds: Credentials = serde_json::from_str(&data).ok()?;
    Some(creds.claude_ai_oauth.access_token)
}

pub fn human_reset(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    // For windows ≤6h always use relative — "tomorrow 12:55 AM" is confusing
    // when the reset is only a few hours away.
    if secs <= 6 * 3600 {
        return format!("resets in {h}h {m}m");
    }
    let now = Local::now();
    let target = now + Duration::seconds(secs as i64);
    let tomorrow = (now + Duration::days(1)).date_naive();
    if target.date_naive() == tomorrow {
        return format!("resets tomorrow {}", target.format("%-I:%M %p"));
    }
    format!("resets {}", target.format("%a %-I:%M %p"))
}

fn secs_until(iso: &str) -> u64 {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .and_then(|dt| (dt - Utc::now()).to_std().ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_today_stats() -> (u64, u64) {
    let data = match fs::read_to_string(claude_dir().join("stats-cache.json")) {
        Ok(d) => d,
        Err(_) => return (0, 0),
    };
    let v: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return (0, 0),
    };
    let today = Local::now().format("%Y-%m-%d").to_string();
    v["dailyActivity"]
        .as_array()
        .and_then(|a| a.iter().rev().find(|e| e["date"].as_str() == Some(&today)))
        .map(|e| {
            (
                e["messageCount"].as_u64().unwrap_or(0),
                e["toolCallCount"].as_u64().unwrap_or(0),
            )
        })
        .unwrap_or((0, 0))
}

fn build_usage_data(
    five_hour: Option<&UsageWindow>,
    seven_day: Option<&UsageWindow>,
    extra_used_cents: f64,
    extra_limit_cents: f64,
    today_messages: u64,
    today_tool_calls: u64,
) -> UsageData {
    let session_resets_secs = five_hour
        .and_then(|w| w.resets_at.as_deref())
        .map(secs_until)
        .unwrap_or(0);
    let weekly_resets_secs = seven_day
        .and_then(|w| w.resets_at.as_deref())
        .map(secs_until)
        .unwrap_or(0);
    UsageData {
        session_pct: five_hour.and_then(|w| w.utilization).unwrap_or(0.0),
        session_resets_secs,
        session_resets: human_reset(session_resets_secs),
        weekly_pct: seven_day.and_then(|w| w.utilization).unwrap_or(0.0),
        weekly_resets_secs,
        weekly_resets: human_reset(weekly_resets_secs),
        extra_used_cents,
        extra_limit_cents,
        today_messages,
        today_tool_calls,
        stale: false,
        fetched_at: Local::now().timestamp(),
        attempted_at: Local::now().timestamp(),
    }
}

// ── Browser cookie reading ────────────────────────────────────────────────────

/// Query a single string value from a SQLite file, opening it immutably to
/// avoid conflicts with a running browser. Returns None on any error.
fn sqlite_query_one(db_path: &PathBuf, sql: &str) -> Option<String> {
    // Open with immutable flag: no WAL processing, no locks acquired.
    let uri = format!("file:{}?immutable=1", db_path.display());
    let conn = rusqlite::Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    conn.query_row(sql, [], |row| row.get::<_, String>(0)).ok()
}

fn find_session_key_in_firefox() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let profiles = PathBuf::from(home).join(".mozilla/firefox");
    for entry in fs::read_dir(&profiles).ok()?.flatten() {
        let db = entry.path().join("cookies.sqlite");
        if !db.exists() {
            continue;
        }
        let key = sqlite_query_one(
            &db,
            "SELECT value FROM moz_cookies \
             WHERE host LIKE '%claude.ai%' AND name = 'sessionKey' \
             ORDER BY lastAccessed DESC LIMIT 1",
        );
        if let Some(k) = key {
            if k.starts_with("sk-ant-") {
                return Some(k);
            }
        }
    }
    None
}

fn find_session_key_in_chromium() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let home = PathBuf::from(home);
    let candidates = [
        home.join(".config/google-chrome/Default/Cookies"),
        home.join(".config/google-chrome/Default/Network/Cookies"),
        home.join(".config/chromium/Default/Cookies"),
        home.join(".config/chromium/Default/Network/Cookies"),
        home.join(".config/BraveSoftware/Brave-Browser/Default/Cookies"),
    ];
    for db in &candidates {
        if !db.exists() {
            continue;
        }
        // On Linux, Chrome may store the plaintext in `value` for unencrypted
        // cookies; encrypted cookies (v10/v11 prefix) are skipped since we
        // can't decrypt them without Secret Service integration.
        let key = sqlite_query_one(
            db,
            "SELECT value FROM cookies \
             WHERE host_key LIKE '%claude.ai%' AND name = 'sessionKey' \
             ORDER BY last_access_utc DESC LIMIT 1",
        );
        if let Some(k) = key {
            if k.starts_with("sk-ant-") {
                return Some(k);
            }
        }
    }
    None
}

/// Returns a claude.ai sessionKey from (in order):
/// 1. `CLAUDE_SESSION_KEY` env var
/// 2. Firefox cookies
/// 3. Chrome / Chromium / Brave cookies (unencrypted only)
fn find_session_key() -> Option<String> {
    if let Ok(k) = std::env::var("CLAUDE_SESSION_KEY") {
        if k.starts_with("sk-ant-") {
            return Some(k);
        }
    }
    find_session_key_in_firefox().or_else(find_session_key_in_chromium)
}

// ── Fetch sources ─────────────────────────────────────────────────────────────

fn fetch_oauth(today_messages: u64, today_tool_calls: u64) -> Option<UsageData> {
    let token = read_access_token()?;

    let response = match ureq::get("https://api.anthropic.com/api/oauth/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("Accept", "application/json")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            eprintln!("[claude oauth] HTTP {code}: {body}");
            return None;
        }
        Err(e) => {
            eprintln!("[claude oauth] request error: {e}");
            return None;
        }
    };

    let body = response.into_string().ok()?;
    let resp: OAuthUsageResponse = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[claude oauth] parse error: {e}\nbody: {body}");
            return None;
        }
    };

    Some(build_usage_data(
        resp.five_hour.as_ref(),
        resp.seven_day.as_ref(),
        resp.extra_usage.as_ref().and_then(|e| e.used_credits).unwrap_or(0.0),
        resp.extra_usage.as_ref().and_then(|e| e.monthly_limit).unwrap_or(0.0),
        today_messages,
        today_tool_calls,
    ))
}

fn fetch_web(today_messages: u64, today_tool_calls: u64) -> Option<UsageData> {
    let session_key = find_session_key()?;
    let cookie = format!("sessionKey={session_key}");

    // 1. Resolve org UUID.
    let orgs_body = ureq::get("https://claude.ai/api/organizations")
        .set("Cookie", &cookie)
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let orgs: Vec<OrgItem> = serde_json::from_str(&orgs_body).ok()?;
    let org_id = orgs
        .iter()
        .find(|o| {
            o.capabilities
                .as_ref()
                .map(|c| c.iter().any(|cap| cap == "chat"))
                .unwrap_or(false)
        })
        .or_else(|| orgs.first())
        .map(|o| o.uuid.clone())?;

    // 2. Core usage.
    let usage_body = ureq::get(&format!(
        "https://claude.ai/api/organizations/{org_id}/usage"
    ))
    .set("Cookie", &cookie)
    .call()
    .ok()?
    .into_string()
    .ok()?;
    let usage: WebUsageResponse = match serde_json::from_str(&usage_body) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[claude web] parse error: {e}\nbody: {usage_body}");
            return None;
        }
    };

    // 3. Overage limit (best-effort, don't fail if unavailable).
    let overage: Option<OverageSpendLimit> =
        ureq::get(&format!(
            "https://claude.ai/api/organizations/{org_id}/overage_spend_limit"
        ))
        .set("Cookie", &cookie)
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|s| serde_json::from_str(&s).ok());

    Some(build_usage_data(
        usage.five_hour.as_ref(),
        usage.seven_day.as_ref(),
        overage.as_ref().and_then(|o| o.used_credits).unwrap_or(0.0),
        overage.as_ref().and_then(|o| o.monthly_credit_limit).unwrap_or(0.0),
        today_messages,
        today_tool_calls,
    ))
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Try sources in order: OAuth API → web session (browser cookies / env var).
/// Returns None only when all sources fail — the caller should show stale data.
pub fn fetch() -> Option<UsageData> {
    let (today_messages, today_tool_calls) = read_today_stats();

    if let Some(data) = fetch_oauth(today_messages, today_tool_calls) {
        return Some(data);
    }
    eprintln!("[claude] OAuth failed, trying web session…");
    if let Some(data) = fetch_web(today_messages, today_tool_calls) {
        eprintln!("[claude] web session succeeded");
        return Some(data);
    }
    None
}
