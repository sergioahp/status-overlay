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

fn claude_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude")
}

fn read_access_token() -> Option<String> {
    let data = fs::read_to_string(claude_dir().join(".credentials.json")).ok()?;
    let creds: Credentials = serde_json::from_str(&data).ok()?;
    Some(creds.claude_ai_oauth.access_token)
}

// "2026-03-08T17:00:00.198664+00:00" → "17:00Z"
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

/// Returns `None` when the API call fails (network error, rate limit, etc.).
/// The caller should retain and re-display the last known good value marked stale.
/// Returns `Some` with `stale: false` on success, or when there is no token configured.
pub fn fetch() -> Option<UsageData> {
    let (today_messages, today_tool_calls) = read_today_stats();

    let token = match read_access_token() {
        Some(t) => t,
        None => {
            return Some(UsageData {
                today_messages,
                today_tool_calls,
                ..Default::default()
            })
        }
    };

    let response = match ureq::get("https://api.anthropic.com/api/oauth/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("Accept", "application/json")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            eprintln!("[claude] HTTP {code}: {body}");
            return None;
        }
        Err(e) => {
            eprintln!("[claude] request error: {e}");
            return None;
        }
    };

    let body = match response.into_string() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[claude] read error: {e}");
            return None;
        }
    };

    let resp: OAuthUsageResponse = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[claude] parse error: {e}\nbody: {body}");
            return None;
        }
    };

    let session_resets_secs = resp
        .five_hour
        .as_ref()
        .and_then(|w| w.resets_at.as_deref())
        .map(secs_until)
        .unwrap_or(0);
    let weekly_resets_secs = resp
        .seven_day
        .as_ref()
        .and_then(|w| w.resets_at.as_deref())
        .map(secs_until)
        .unwrap_or(0);

    Some(UsageData {
        session_pct: resp.five_hour.as_ref().and_then(|w| w.utilization).unwrap_or(0.0),
        session_resets_secs,
        session_resets: human_reset(session_resets_secs),
        weekly_pct: resp.seven_day.as_ref().and_then(|w| w.utilization).unwrap_or(0.0),
        weekly_resets_secs,
        weekly_resets: human_reset(weekly_resets_secs),
        extra_used_cents: resp.extra_usage.as_ref().and_then(|e| e.used_credits).unwrap_or(0.0),
        extra_limit_cents: resp.extra_usage.as_ref().and_then(|e| e.monthly_limit).unwrap_or(0.0),
        today_messages,
        today_tool_calls,
        stale: false,
        fetched_at: Local::now().timestamp(),
        attempted_at: Local::now().timestamp(),
    })
}
