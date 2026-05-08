use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use chrono::{Local, Duration};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexData {
    pub plan: String,
    pub primary_pct: u32,
    pub primary_resets_secs: u64,
    pub secondary_pct: u32,
    pub secondary_resets_secs: u64,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub fetched_at: i64,
    #[serde(default)]
    pub attempted_at: i64,
}

fn has_real_usage_windows(data: &CodexData) -> bool {
    data.primary_resets_secs > 0
        || data.secondary_resets_secs > 0
        || data.primary_pct > 0
        || data.secondary_pct > 0
        || !data.plan.is_empty()
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
    // For windows ≤6h always use relative — date formats are confusing when
    // the reset is only a few hours away.
    if secs <= 6 * 3600 {
        let relative = match (h, m) {
            (0, m) => format!("{m}m"),
            (h, 0) => format!("{h}h"),
            (h, m) => format!("{h}h {m}m"),
        };
        return format!("resets in {relative}");
    }
    let now = Local::now();
    let target = now + Duration::seconds(secs as i64);
    let tomorrow = (now + Duration::days(1)).date_naive();
    if target.date_naive() == tomorrow {
        return format!("resets tomorrow {}", target.format("%-I:%M %p"));
    }
    format!("resets {}", target.format("%a %-I:%M %p"))
}

/// Returns `None` when the API call fails, auth is missing, or the response
/// does not contain usable quota windows.
pub fn fetch() -> Option<CodexData> {
    let token = match read_token() {
        Some(t) => t,
        None => return None,
    };

    let body = ureq::get("https://chatgpt.com/backend-api/wham/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .set("User-Agent", "CodexBar")
        .call()
        .ok()?
        .into_string()
        .ok()?;

    let resp: UsageResponse = serde_json::from_str(&body).ok()?;

    let primary = resp.rate_limit.as_ref().and_then(|r| r.primary_window.as_ref());
    let secondary = resp.rate_limit.as_ref().and_then(|r| r.secondary_window.as_ref());

    let data = CodexData {
        plan: resp.plan_type.unwrap_or_default(),
        primary_pct: primary.and_then(|w| w.used_percent).unwrap_or(0),
        primary_resets_secs: primary.and_then(|w| w.reset_after_seconds).unwrap_or(0),
        secondary_pct: secondary.and_then(|w| w.used_percent).unwrap_or(0),
        secondary_resets_secs: secondary.and_then(|w| w.reset_after_seconds).unwrap_or(0),
        stale: false,
        fetched_at: Local::now().timestamp(),
        attempted_at: Local::now().timestamp(),
    };

    has_real_usage_windows(&data).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_codex_home(test: impl FnOnce(PathBuf)) {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_codex_home = std::env::var_os("CODEX_HOME");
        let old_home = std::env::var_os("HOME");
        let unique = format!(
            "status-overlay-codex-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).unwrap();
        unsafe {
            std::env::set_var("CODEX_HOME", &root);
            std::env::set_var("HOME", &root);
        }
        test(root.clone());
        unsafe {
            match old_codex_home {
                Some(value) => std::env::set_var("CODEX_HOME", value),
                None => std::env::remove_var("CODEX_HOME"),
            }
            match old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fetch_returns_none_without_auth_token() {
        with_temp_codex_home(|_| {
            assert!(fetch().is_none());
        });
    }

    #[test]
    fn has_real_usage_windows_rejects_empty_logged_out_shape() {
        assert!(!has_real_usage_windows(&CodexData::default()));
        assert!(has_real_usage_windows(&CodexData {
            primary_resets_secs: 18_000,
            ..Default::default()
        }));
    }
}
