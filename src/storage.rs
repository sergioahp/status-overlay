use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fs, path::PathBuf};

const HISTORY_DB_NAME: &str = "history.sqlite3";
const LEGACY_IMPORT_META_KEY: &str = "legacy_json_history_imported_v1";

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

fn history_db_path() -> PathBuf {
    state_dir().join(HISTORY_DB_NAME)
}

fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn open_history_db() -> Option<Connection> {
    ensure_dir().ok()?;
    let conn = Connection::open(history_db_path()).ok()?;
    init_history_db(&conn).ok()?;
    migrate_legacy_json_history(&conn);
    Some(conn)
}

fn init_history_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = 5000;

        CREATE TABLE IF NOT EXISTS app_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS claude_usage_samples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            recorded_at INTEGER NOT NULL,
            fetched_at INTEGER NOT NULL,
            attempted_at INTEGER NOT NULL,
            session_pct REAL NOT NULL,
            session_resets TEXT NOT NULL,
            session_resets_secs INTEGER NOT NULL,
            weekly_pct REAL NOT NULL,
            weekly_resets TEXT NOT NULL,
            weekly_resets_secs INTEGER NOT NULL,
            extra_used_cents REAL NOT NULL,
            extra_limit_cents REAL NOT NULL,
            extra_enabled INTEGER NOT NULL,
            today_messages INTEGER NOT NULL,
            today_tool_calls INTEGER NOT NULL,
            plan TEXT NOT NULL,
            stale INTEGER NOT NULL,
            snapshot_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_claude_usage_samples_fetched_at
        ON claude_usage_samples (fetched_at DESC, id DESC);

        CREATE TABLE IF NOT EXISTS codex_usage_samples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            recorded_at INTEGER NOT NULL,
            fetched_at INTEGER NOT NULL,
            attempted_at INTEGER NOT NULL,
            plan TEXT NOT NULL,
            primary_pct INTEGER NOT NULL,
            primary_resets_secs INTEGER NOT NULL,
            secondary_pct INTEGER NOT NULL,
            secondary_resets_secs INTEGER NOT NULL,
            stale INTEGER NOT NULL,
            snapshot_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_codex_usage_samples_fetched_at
        ON codex_usage_samples (fetched_at DESC, id DESC);
        ",
    )
}

pub fn save_usage(data: &crate::usage::UsageData) {
    let _ = save_json("usage.json", data);
}

pub fn load_usage() -> Option<crate::usage::UsageData> {
    load_latest_usage_from_db().or_else(|| load_json("usage.json"))
}

pub fn save_codex(data: &crate::codex::CodexData) {
    let _ = save_json("codex.json", data);
}

pub fn load_codex() -> Option<crate::codex::CodexData> {
    load_latest_codex_from_db().or_else(|| load_json("codex.json"))
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

fn insert_usage_row(conn: &Connection, data: &crate::usage::UsageData) -> rusqlite::Result<()> {
    let snapshot_json = serde_json::to_string(data).unwrap_or_default();
    conn.execute(
        "
        INSERT INTO claude_usage_samples (
            recorded_at,
            fetched_at,
            attempted_at,
            session_pct,
            session_resets,
            session_resets_secs,
            weekly_pct,
            weekly_resets,
            weekly_resets_secs,
            extra_used_cents,
            extra_limit_cents,
            extra_enabled,
            today_messages,
            today_tool_calls,
            plan,
            stale,
            snapshot_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        ",
        params![
            Utc::now().timestamp(),
            data.fetched_at,
            data.attempted_at,
            data.session_pct,
            &data.session_resets,
            data.session_resets_secs as i64,
            data.weekly_pct,
            &data.weekly_resets,
            data.weekly_resets_secs as i64,
            data.extra_used_cents,
            data.extra_limit_cents,
            bool_to_i64(data.extra_enabled),
            data.today_messages as i64,
            data.today_tool_calls as i64,
            &data.plan,
            bool_to_i64(data.stale),
            snapshot_json,
        ],
    )?;
    Ok(())
}

fn insert_codex_row(conn: &Connection, data: &crate::codex::CodexData) -> rusqlite::Result<()> {
    let snapshot_json = serde_json::to_string(data).unwrap_or_default();
    conn.execute(
        "
        INSERT INTO codex_usage_samples (
            recorded_at,
            fetched_at,
            attempted_at,
            plan,
            primary_pct,
            primary_resets_secs,
            secondary_pct,
            secondary_resets_secs,
            stale,
            snapshot_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ",
        params![
            Utc::now().timestamp(),
            data.fetched_at,
            data.attempted_at,
            &data.plan,
            data.primary_pct as i64,
            data.primary_resets_secs as i64,
            data.secondary_pct as i64,
            data.secondary_resets_secs as i64,
            bool_to_i64(data.stale),
            snapshot_json,
        ],
    )?;
    Ok(())
}

fn load_latest_usage_from_db() -> Option<crate::usage::UsageData> {
    let conn = open_history_db()?;
    let snapshot_json: String = conn
        .query_row(
            "
            SELECT snapshot_json
            FROM claude_usage_samples
            ORDER BY fetched_at DESC, id DESC
            LIMIT 1
            ",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()??;
    serde_json::from_str(&snapshot_json).ok()
}

fn load_latest_codex_from_db() -> Option<crate::codex::CodexData> {
    let conn = open_history_db()?;
    let snapshot_json: String = conn
        .query_row(
            "
            SELECT snapshot_json
            FROM codex_usage_samples
            ORDER BY fetched_at DESC, id DESC
            LIMIT 1
            ",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()??;
    serde_json::from_str(&snapshot_json).ok()
}

fn migrate_legacy_json_history(conn: &Connection) {
    let already_imported = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            [LEGACY_IMPORT_META_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some();
    if already_imported {
        return;
    }

    for sample in load_json::<Vec<UsageSample>>("usage_history.json").unwrap_or_default() {
        let data = crate::usage::UsageData {
            session_pct: sample.session_pct,
            weekly_pct: sample.weekly_pct,
            fetched_at: sample.ts,
            attempted_at: sample.ts,
            ..Default::default()
        };
        let _ = insert_usage_row(conn, &data);
    }

    for sample in load_json::<Vec<CodexSample>>("codex_history.json").unwrap_or_default() {
        let data = crate::codex::CodexData {
            primary_pct: sample.primary_pct,
            secondary_pct: sample.secondary_pct,
            fetched_at: sample.ts,
            attempted_at: sample.ts,
            ..Default::default()
        };
        let _ = insert_codex_row(conn, &data);
    }

    let _ = conn.execute(
        "INSERT OR REPLACE INTO app_meta (key, value) VALUES (?1, ?2)",
        params![LEGACY_IMPORT_META_KEY, Utc::now().timestamp().to_string()],
    );
}

pub fn append_usage_sample(data: &crate::usage::UsageData) {
    let Some(conn) = open_history_db() else {
        return;
    };
    let _ = insert_usage_row(&conn, data);
}

pub fn append_codex_sample(data: &crate::codex::CodexData) {
    let Some(conn) = open_history_db() else {
        return;
    };
    let _ = insert_codex_row(&conn, data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_state_home(test: impl FnOnce(PathBuf)) {
        let _guard = ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("XDG_STATE_HOME");
        let unique = format!(
            "status-overlay-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).unwrap();
        unsafe {
            std::env::set_var("XDG_STATE_HOME", &root);
        }
        test(root.clone());
        unsafe {
            match old {
                Some(value) => std::env::set_var("XDG_STATE_HOME", value),
                None => std::env::remove_var("XDG_STATE_HOME"),
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn append_usage_sample_persists_and_loads_from_sqlite() {
        with_temp_state_home(|_| {
            let sample = crate::usage::UsageData {
                session_pct: 42.5,
                weekly_pct: 87.0,
                today_messages: 123,
                today_tool_calls: 45,
                fetched_at: 1_777_000_000,
                attempted_at: 1_777_000_000,
                plan: "Pro".to_string(),
                ..Default::default()
            };

            append_usage_sample(&sample);

            let loaded = load_usage().expect("usage sample should load from sqlite");
            assert_eq!(loaded.session_pct, 42.5);
            assert_eq!(loaded.weekly_pct, 87.0);
            assert_eq!(loaded.today_messages, 123);
            assert!(history_db_path().exists());
        });
    }

    #[test]
    fn legacy_json_history_is_imported_once() {
        with_temp_state_home(|_| {
            let dir = ensure_dir().unwrap();
            let legacy = vec![
                CodexSample { ts: 10, primary_pct: 11, secondary_pct: 12 },
                CodexSample { ts: 20, primary_pct: 21, secondary_pct: 22 },
            ];
            fs::write(
                dir.join("codex_history.json"),
                serde_json::to_string(&legacy).unwrap(),
            )
            .unwrap();

            append_codex_sample(&crate::codex::CodexData {
                primary_pct: 33,
                secondary_pct: 44,
                fetched_at: 30,
                attempted_at: 30,
                ..Default::default()
            });

            let conn = Connection::open(history_db_path()).unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM codex_usage_samples", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 3);

            append_codex_sample(&crate::codex::CodexData {
                primary_pct: 55,
                secondary_pct: 66,
                fetched_at: 40,
                attempted_at: 40,
                ..Default::default()
            });

            let count_after: i64 = conn
                .query_row("SELECT COUNT(*) FROM codex_usage_samples", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count_after, 4);
        });
    }
}
