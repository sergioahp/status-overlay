use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{fs, path::PathBuf};

fn state_dir() -> PathBuf {
    std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("status-overlay")
}

fn ensure_dir() -> std::io::Result<PathBuf> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn save_json<T: Serialize>(name: &str, value: &T) -> std::io::Result<()> {
    let dir = ensure_dir()?;
    let path = dir.join(name);
    let data = serde_json::to_string_pretty(value)?;
    fs::write(path, data)
}

fn load_json<T: DeserializeOwned>(name: &str) -> Option<T> {
    let path = state_dir().join(name);
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save_usage(data: &crate::usage::UsageData) {
    let _ = save_json("usage.json", data);
}

pub fn load_usage() -> Option<crate::usage::UsageData> {
    load_json("usage.json")
}

pub fn save_codex(data: &crate::codex::CodexData) {
    let _ = save_json("codex.json", data);
}

pub fn load_codex() -> Option<crate::codex::CodexData> {
    load_json("codex.json")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageSample {
    pub ts: i64,
    pub session_pct: f64,
    pub weekly_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexSample {
    pub ts: i64,
    pub primary_pct: u32,
    pub secondary_pct: u32,
}

fn retain_recent<T>(samples: &mut Vec<T>, retention_secs: i64, sample_ts: impl Fn(&T) -> i64) {
    let cutoff = Utc::now().timestamp() - retention_secs;
    let keep_from = samples
        .iter()
        .position(|sample| sample_ts(sample) >= cutoff)
        .unwrap_or(samples.len());
    if keep_from > 0 {
        samples.drain(0..keep_from);
    }
}

const CLAUDE_HISTORY_RETENTION_SECS: i64 = 14 * 24 * 60 * 60;
const CODEX_HISTORY_RETENTION_SECS: i64 = 14 * 24 * 60 * 60;

pub fn append_usage_sample(data: &crate::usage::UsageData) {
    let mut hist: Vec<UsageSample> = load_json("usage_history.json").unwrap_or_default();
    hist.push(UsageSample {
        ts: data.fetched_at.max(Utc::now().timestamp()),
        session_pct: data.session_pct,
        weekly_pct: data.weekly_pct,
    });
    retain_recent(&mut hist, CLAUDE_HISTORY_RETENTION_SECS, |sample| sample.ts);
    let _ = save_json("usage_history.json", &hist);
}

pub fn append_codex_sample(data: &crate::codex::CodexData) {
    let mut hist: Vec<CodexSample> = load_json("codex_history.json").unwrap_or_default();
    hist.push(CodexSample {
        ts: data.fetched_at.max(Utc::now().timestamp()),
        primary_pct: data.primary_pct,
        secondary_pct: data.secondary_pct,
    });
    retain_recent(&mut hist, CODEX_HISTORY_RETENTION_SECS, |sample| sample.ts);
    let _ = save_json("codex_history.json", &hist);
}
