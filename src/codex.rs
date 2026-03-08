use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Default)]
pub struct CodexData {
    pub plan: String,
    pub primary_pct: u32,
    pub primary_resets_secs: u64,
    pub secondary_pct: u32,
    pub secondary_resets_secs: u64,
}

#[derive(Deserialize)]
struct AuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<AuthTokens>,
}

#[derive(Deserialize)]
struct AuthTokens {
    access_token: Option<String>,
}

#[derive(Deserialize)]
struct UsageResponse {
    plan_type: Option<String>,
    rate_limit: Option<RateLimit>,
}

#[derive(Deserialize)]
struct RateLimit {
    primary_window: Option<Window>,
    secondary_window: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: Option<u32>,
    reset_after_seconds: Option<u64>,
}

fn codex_dir() -> PathBuf {
    std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".codex")
        })
}

fn read_token() -> Option<String> {
    let data = fs::read_to_string(codex_dir().join("auth.json")).ok()?;
    let auth: AuthFile = serde_json::from_str(&data).ok()?;
    auth.tokens
        .as_ref()
        .and_then(|t| t.access_token.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| auth.openai_api_key.filter(|s| !s.is_empty()))
}

pub fn fmt_resets(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("resets in {h}h {m}m")
    } else {
        format!("resets in {m}m")
    }
}

pub fn fetch() -> CodexData {
    let token = match read_token() {
        Some(t) => t,
        None => return CodexData::default(),
    };

    let resp: UsageResponse = match ureq::get("https://chatgpt.com/backend-api/wham/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .set("User-Agent", "CodexBar")
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(r) => r,
        None => return CodexData::default(),
    };

    let primary = resp.rate_limit.as_ref().and_then(|r| r.primary_window.as_ref());
    let secondary = resp.rate_limit.as_ref().and_then(|r| r.secondary_window.as_ref());

    CodexData {
        plan: resp.plan_type.unwrap_or_default(),
        primary_pct: primary.and_then(|w| w.used_percent).unwrap_or(0),
        primary_resets_secs: primary.and_then(|w| w.reset_after_seconds).unwrap_or(0),
        secondary_pct: secondary.and_then(|w| w.used_percent).unwrap_or(0),
        secondary_resets_secs: secondary.and_then(|w| w.reset_after_seconds).unwrap_or(0),
    }
}
