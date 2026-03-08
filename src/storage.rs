use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

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
