use chrono::{Local, DateTime, Utc, Duration, Datelike, NaiveDate, NaiveTime, TimeZone, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::FromRawFd;
use std::os::unix::process::CommandExt;

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
    /// Whether extra/overage usage is enabled for this account.
    #[serde(default)]
    pub extra_enabled: bool,
    pub today_messages: u64,
    pub today_tool_calls: u64,
    /// Plan name inferred from credentials tier or /api/account (e.g. "Pro", "Max").
    #[serde(default)]
    pub plan: String,
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
    /// Milliseconds since epoch (NOT seconds).
    #[serde(rename = "expiresAt")]
    expires_at: Option<f64>,
    scopes: Option<Vec<String>>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
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
    is_enabled: Option<bool>,
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
    is_enabled: Option<bool>,
}

#[derive(Deserialize)]
struct AccountResponse {
    memberships: Option<Vec<AccountMembership>>,
}

#[derive(Deserialize)]
struct AccountMembership {
    organization: Option<AccountOrg>,
}

#[derive(Deserialize)]
struct AccountOrg {
    rate_limit_tier: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn claude_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude")
}

fn read_credentials() -> Option<OAuthCreds> {
    let data = fs::read_to_string(claude_dir().join(".credentials.json")).ok()?;
    let creds: Credentials = serde_json::from_str(&data).ok()?;
    Some(creds.claude_ai_oauth)
}

fn normalize_plan(tier: &str) -> String {
    match tier.to_ascii_lowercase().as_str() {
        "claude_pro" | "pro" => "Pro".to_string(),
        "claude_max" | "max" => "Max".to_string(),
        "claude_team" | "team" => "Team".to_string(),
        "claude_enterprise" | "enterprise" => "Enterprise".to_string(),
        other if !other.is_empty() => {
            // Capitalize first letter.
            let mut chars = other.chars();
            match chars.next() {
                Some(f) => f.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
        _ => String::new(),
    }
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
    extra_enabled: bool,
    plan: String,
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
        extra_enabled,
        today_messages,
        today_tool_calls,
        plan,
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

// ── CLI PTY probe ─────────────────────────────────────────────────────────────

fn find_claude_binary() -> Option<PathBuf> {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let p = PathBuf::from(dir).join("claude");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// A private, empty directory for the short-lived Claude usage probe.
///
/// Claude Code inspects its working directory during startup. Keeping the
/// probe out of the user's projects prevents that background check from
/// building project context or walking unrelated files.
fn claude_probe_dir() -> Option<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    let dir = PathBuf::from(runtime_dir)
        .join("status-overlay")
        .join("claude-usage-probe");
    fs::create_dir_all(&dir).ok()?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).ok()?;
    Some(dir)
}

/// Strip ANSI/VT escape sequences from raw PTY bytes, returning plain text.
fn strip_ansi(input: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'\x1b' {
            i += 1;
            if i >= input.len() {
                break;
            }
            match input[i] {
                b'[' => {
                    // CSI sequence: ESC [ ... <final byte a-zA-Z>
                    i += 1;
                    while i < input.len() && !input[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    if i < input.len() {
                        i += 1;
                    }
                }
                b']' => {
                    // OSC sequence: ESC ] ... ST (ESC \) or BEL
                    i += 1;
                    while i < input.len() {
                        if input[i] == b'\x07' {
                            i += 1;
                            break;
                        }
                        if i + 1 < input.len() && input[i] == b'\x1b' && input[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                b'(' | b')' => {
                    // Charset designation: ESC ( X or ESC ) X
                    i += 2;
                }
                _ => {
                    // Two-byte escape (e.g. ESC = , ESC >)
                    i += 1;
                }
            }
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Write bytes to master PTY fd, ignoring errors (best-effort).
fn pty_write(master: libc::c_int, data: &[u8]) {
    unsafe {
        libc::write(master, data.as_ptr() as *const libc::c_void, data.len());
    }
}

/// Drive a PTY session: auto-respond to initial prompts, send `/usage`, collect
/// the usage panel output.  Returns None on timeout or if no usage data found.
fn pty_interact(master: libc::c_int) -> Option<String> {
    let overall_timeout = std::time::Duration::from_secs(20);
    let start = std::time::Instant::now();

    let mut raw_buf = [0u8; 4096];
    let mut raw_accum: Vec<u8> = Vec::new();

    // Prompts we auto-respond to.  Each is responded to at most once.
    // The response is sent as-is (usually "\r" or "y\r").
    const AUTO_RESPONDS: &[(&str, &[u8])] = &[
        ("Do you trust the files in this folder?", b"y\r"),
        ("Quick safety check:", b"\r"),
        ("Yes, I trust this folder", b"\r"),
        ("Ready to code here?", b"\r"),
        ("Press Enter to continue", b"\r"),
        // Command-palette items that appear after /usage is typed.
        ("Show plan", b"\r"),
        ("Show plan usage limits", b"\r"),
    ];
    let mut responded = [false; 7]; // parallel to AUTO_RESPONDS

    const STOP_CONDITIONS: &[&str] = &[
        "Current week (all models)",
        "Current week (Opus)",
        "Current week (Sonnet only)",
        "Current week (Sonnet)",
        "Current session",
        "Failed to load usage data",
    ];

    let mut first_output_at: Option<std::time::Instant> = None;
    let mut usage_sent = false;
    let mut last_enter_at = start;
    let mut stop_seen_at: Option<std::time::Instant> = None;

    loop {
        if start.elapsed() > overall_timeout {
            eprintln!("[claude cli] probe timed out after 20s");
            break;
        }

        // ── Read available bytes ──────────────────────────────────────────────
        let n = unsafe {
            libc::read(master, raw_buf.as_mut_ptr() as *mut libc::c_void, raw_buf.len())
        };

        if n > 0 {
            let new_bytes = &raw_buf[..n as usize];
            raw_accum.extend_from_slice(new_bytes);
            if first_output_at.is_none() {
                first_output_at = Some(std::time::Instant::now());
            }
        } else if n == 0 {
            // EOF — slave side closed (child exited).
            break;
        } else {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno != libc::EAGAIN && errno != libc::EWOULDBLOCK {
                // EIO (slave closed) or other hard error.
                break;
            }
            // EAGAIN / EWOULDBLOCK — no data yet; sleep and retry.
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let clean = strip_ansi(&raw_accum);

        // ── Auto-respond to prompts ───────────────────────────────────────────
        for (i, (trigger, response)) in AUTO_RESPONDS.iter().enumerate() {
            if !responded[i] && clean.contains(trigger) {
                pty_write(master, response);
                responded[i] = true;
            }
        }

        // ── Send /usage after 2s of initial output ────────────────────────────
        if !usage_sent {
            if let Some(t) = first_output_at {
                if t.elapsed() >= std::time::Duration::from_secs(2) {
                    pty_write(master, b"/usage\r");
                    usage_sent = true;
                    eprintln!("[claude cli] sent /usage");
                }
            }
        }

        // ── Periodic Enter while waiting for the usage panel (0.8s cadence) ──
        if usage_sent
            && stop_seen_at.is_none()
            && last_enter_at.elapsed() >= std::time::Duration::from_millis(800)
        {
            pty_write(master, b"\r");
            last_enter_at = std::time::Instant::now();
        }

        // ── Check stop conditions (only after /usage sent) ────────────────────
        if usage_sent && stop_seen_at.is_none() {
            for stop in STOP_CONDITIONS {
                if clean.contains(stop) {
                    stop_seen_at = Some(std::time::Instant::now());
                    eprintln!("[claude cli] stop condition: {stop}");
                    break;
                }
            }
        }

        // ── Settle for 1.5s after stop condition to collect the rest of panel ─
        if let Some(t) = stop_seen_at {
            if t.elapsed() >= std::time::Duration::from_millis(1500) {
                return Some(clean);
            }
        }
    }

    // Timed out or EOF — return what we have if it looks useful.
    let clean = strip_ansi(&raw_accum);
    if clean.contains("Current session") || clean.contains("Current week") {
        Some(clean)
    } else {
        None
    }
}

/// Extract the percentage value that appears immediately after `label` in `text`.
/// Handles both "X% used" (returned as-is) and "X% remaining" (inverted to used).
fn extract_pct_after(text: &str, label: &str) -> Option<f64> {
    let after = &text[text.find(label)? + label.len()..];
    // TUI progress bars use multibyte block characters, so a single rendered
    // row can occupy several hundred bytes even at a 160-column terminal.
    let pct_pos = after
        .char_indices()
        .take_while(|(index, _)| *index < 512)
        .find_map(|(index, character)| (character == '%').then_some(index))?;
    let before_pct = after[..pct_pos].trim_end();
    let pct: f64 = before_pct
        .rsplit(|character: char| !character.is_ascii_digit() && character != '.')
        .next()?
        .parse()
        .ok()?;
    // "remaining" appears AFTER the % sign on the same line.
    let after_pct = &after[pct_pos..];
    let line_end = after_pct.find('\n').unwrap_or(after_pct.len());
    if after_pct[..line_end].contains("remaining") {
        Some(100.0 - pct)
    } else {
        Some(pct)
    }
}

fn extract_reset_after<'a>(text: &'a str, label: &str, boundary: Option<&str>) -> Option<&'a str> {
    let lowercase = text.to_ascii_lowercase();
    let label = label.to_ascii_lowercase();
    let label_pos = lowercase.find(&label)?;
    let after = &text[label_pos + label.len()..];
    let after_lowercase = &lowercase[label_pos + label.len()..];
    let reset_pos = after_lowercase.find("resets")?;
    if boundary
        .map(str::to_ascii_lowercase)
        .and_then(|value| after_lowercase.find(&value))
        .is_some_and(|pos| pos < reset_pos)
    {
        return None;
    }
    let value = after[reset_pos + "Resets".len()..]
        .trim_start()
        .trim_start_matches(':')
        .trim_start();
    let line_end = value
        .find(|character| character == '\r' || character == '\n')
        .unwrap_or(value.len());
    let value = value[..line_end].trim();
    (!value.is_empty()).then_some(value)
}

fn latest_usage_panel(text: &str) -> &str {
    let lowercase = text.to_ascii_lowercase();
    let Some(start) = lowercase.rfind("settings:") else {
        return text;
    };
    let panel = &text[start..];
    let panel_lowercase = &lowercase[start..];
    if panel_lowercase.contains("usage")
        && (panel_lowercase.contains("current session")
            || panel_lowercase.contains("loading usage"))
    {
        panel
    } else {
        text
    }
}

fn duration_unit_secs(unit: &str) -> Option<u64> {
    match unit.trim_end_matches('.').to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(1),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(3600),
        "d" | "day" | "days" => Some(24 * 3600),
        _ => None,
    }
}

fn parse_relative_duration(value: &str) -> Option<u64> {
    let compact = value
        .to_ascii_lowercase()
        .replace("and", "")
        .replace(|character: char| character.is_whitespace() || character == ',', "");
    if !compact.is_ascii() {
        return None;
    }
    let bytes = compact.as_bytes();
    let mut total = 0u64;
    let mut parsed_any = false;
    let mut index = 0;

    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let amount_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let amount: u64 = compact[amount_start..index].parse().ok()?;
        let unit_start = index;
        while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
            index += 1;
        }
        let multiplier = duration_unit_secs(&compact[unit_start..index])?;
        total = total.checked_add(amount.checked_mul(multiplier)?)?;
        parsed_any = true;
    }

    parsed_any.then_some(total)
}

fn parse_cli_time(value: &str) -> Option<NaiveTime> {
    let compact = value
        .trim()
        .to_ascii_lowercase()
        .replace(' ', "")
        .replace('.', ":");
    let (clock, is_pm) = if let Some(clock) = compact.strip_suffix("am") {
        (clock, false)
    } else if let Some(clock) = compact.strip_suffix("pm") {
        (clock, true)
    } else {
        return NaiveTime::parse_from_str(&compact, "%H:%M").ok();
    };
    let (hour, minute) = match clock.split_once(':') {
        Some((hour, minute)) => (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?),
        None => (clock.parse::<u32>().ok()?, 0),
    };
    if !(1..=12).contains(&hour) {
        return None;
    }
    let hour = match (hour, is_pm) {
        (12, false) => 0,
        (12, true) => 12,
        (hour, true) => hour + 12,
        (hour, false) => hour,
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

fn parse_cli_month(value: &str) -> Option<u32> {
    match value.to_ascii_lowercase().as_str() {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "sept" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn parse_cli_weekday(value: &str) -> Option<Weekday> {
    match value.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tues" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thur" | "thurs" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn reset_secs(
    date: NaiveDate,
    time: NaiveTime,
    now: DateTime<Local>,
    timezone: Option<Tz>,
) -> Option<u64> {
    let target_timestamp = match timezone {
        Some(timezone) => timezone
            .from_local_datetime(&date.and_time(time))
            .single()?
            .timestamp(),
        None => Local
            .from_local_datetime(&date.and_time(time))
            .single()?
            .timestamp(),
    };
    target_timestamp
        .checked_sub(now.timestamp())
        .and_then(|seconds| u64::try_from(seconds).ok())
}

fn parse_cli_reset_secs(value: &str, now: DateTime<Local>) -> Option<u64> {
    let timezone = value
        .rfind('(')
        .zip(value.rfind(')'))
        .filter(|(start, end)| start < end)
        .and_then(|(start, end)| value[start + 1..end].trim().parse::<Tz>().ok());
    let timezone_start = value.rfind('(').unwrap_or(value.len());
    let normalized = value[..timezone_start]
        .trim()
        .trim_start_matches(':')
        .trim()
        .replace(" at ", " ")
        .replace(" At ", " ");
    let value = normalized.as_str();
    let lowercase = value.to_ascii_lowercase();
    let reference = timezone
        .map(|timezone| now.with_timezone(&timezone).naive_local())
        .unwrap_or_else(|| now.naive_local());

    if let Some(relative) = lowercase.strip_prefix("in") {
        let relative = relative.trim_start();
        if relative.starts_with(|character: char| character.is_ascii_digit()) {
            return parse_relative_duration(relative);
        }
    }

    if let Some(time_value) = lowercase.strip_prefix("tomorrow") {
        let time = parse_cli_time(time_value.trim())?;
        return reset_secs(reference.date() + Duration::days(1), time, now, timezone);
    }

    let day_start = value.find(|character: char| character.is_ascii_digit());
    if let Some(day_start) = day_start
        && parse_cli_month(value[..day_start].trim()).is_some()
    {
        let day_end = value[day_start..]
            .find(|character: char| !character.is_ascii_digit())
            .map(|offset| day_start + offset)
            .unwrap_or(value.len());
        let month = parse_cli_month(value[..day_start].trim())?;
        let day: u32 = value[day_start..day_end].parse().ok()?;
        let time = parse_cli_time(value[day_end..].trim_start_matches(',').trim())?;
        let mut year = reference.year();
        let mut date = NaiveDate::from_ymd_opt(year, month, day)?;
        if reset_secs(date, time, now, timezone).is_none() {
            year = year.checked_add(1)?;
            date = NaiveDate::from_ymd_opt(year, month, day)?;
        }
        return reset_secs(date, time, now, timezone);
    }

    let weekday_and_time = [
        "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
        "mon", "tues", "tue", "wed", "thurs", "thur", "thu", "fri", "sat", "sun",
    ]
    .iter()
    .find_map(|weekday| lowercase.strip_prefix(weekday).map(|time| (*weekday, time.trim())));
    if let Some((weekday_value, time_value)) = weekday_and_time
        && let Some(weekday) = parse_cli_weekday(weekday_value)
        && let Some(time) = parse_cli_time(time_value)
    {
        let mut days = (weekday.num_days_from_monday() as i64
            - reference.weekday().num_days_from_monday() as i64)
            .rem_euclid(7);
        if days == 0 && time <= reference.time() {
            days = 7;
        }
        return reset_secs(reference.date() + Duration::days(days), time, now, timezone);
    }

    let time = parse_cli_time(value)?;
    let mut date = reference.date();
    if time <= reference.time() {
        date += Duration::days(1);
    }
    reset_secs(date, time, now, timezone)
}

fn extract_cli_plan(text: &str) -> String {
    ["Pro", "Max", "Team", "Enterprise", "Free"]
        .iter()
        .find(|plan| {
            text.contains(&format!("Claude {plan}")) || text.contains(&format!("Claude{plan}"))
        })
        .map(|plan| (*plan).to_string())
        .unwrap_or_default()
}

fn parse_usage_text(
    text: &str,
    today_messages: u64,
    today_tool_calls: u64,
) -> Option<UsageData> {
    let usage_panel = latest_usage_panel(text);
    let session_pct = extract_pct_after(usage_panel, "Current session")
        .or_else(|| extract_pct_after(usage_panel, "current session"))?;

    let weekly_pct = extract_pct_after(usage_panel, "Current week (all models)")
        .or_else(|| extract_pct_after(usage_panel, "Current week (Opus)"))
        .or_else(|| extract_pct_after(usage_panel, "Current week (Sonnet only)"))
        .or_else(|| extract_pct_after(usage_panel, "Current week (Sonnet)"))
        .or_else(|| extract_pct_after(usage_panel, "Current week"))
        .unwrap_or(0.0);

    let now = Local::now();
    let session_reset_value = extract_reset_after(usage_panel, "Current session", Some("Current week"));
    let session_resets_secs = session_reset_value
        .and_then(|value| parse_cli_reset_secs(value, now))
        .unwrap_or(0);
    let weekly_reset_value = extract_reset_after(usage_panel, "Current week", Some("Extra usage"));
    let weekly_resets_secs = weekly_reset_value
        .and_then(|value| parse_cli_reset_secs(value, now))
        .unwrap_or(0);
    if let Some(value) = session_reset_value.filter(|_| session_resets_secs == 0) {
        eprintln!("[claude cli] could not parse session reset: {value:?}");
    }
    if let Some(value) = weekly_reset_value.filter(|_| weekly_resets_secs == 0) {
        eprintln!("[claude cli] could not parse weekly reset: {value:?}");
    }
    let plan = extract_cli_plan(text);

    eprintln!(
        "[claude cli] parsed session={session_pct:.0}% weekly={weekly_pct:.0}% session_reset={session_resets_secs}s weekly_reset={weekly_resets_secs}s plan={plan}"
    );

    Some(UsageData {
        session_pct,
        session_resets_secs,
        session_resets: human_reset(session_resets_secs),
        weekly_pct,
        weekly_resets_secs,
        weekly_resets: human_reset(weekly_resets_secs),
        extra_used_cents: 0.0,
        extra_limit_cents: 0.0,
        extra_enabled: false,
        today_messages,
        today_tool_calls,
        plan,
        stale: false,
        fetched_at: Local::now().timestamp(),
        attempted_at: Local::now().timestamp(),
    })
}

fn fetch_cli(today_messages: u64, today_tool_calls: u64) -> Option<UsageData> {
    let claude = find_claude_binary()?;
    let probe_dir = claude_probe_dir()?;

    // ── Open PTY ──────────────────────────────────────────────────────────────
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let ws = libc::winsize { ws_row: 50, ws_col: 160, ws_xpixel: 0, ws_ypixel: 0 };

    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &ws,
        )
    } != 0
    {
        eprintln!("[claude cli] openpty failed: {}", std::io::Error::last_os_error());
        return None;
    }
    unsafe { libc::fcntl(master, libc::F_SETFL, libc::O_NONBLOCK) };

    // Duplicate slave fd: Command takes ownership of each Stdio, which needs
    // a separate fd for stdin / stdout / stderr.
    let slave_out = unsafe { libc::dup(slave) };
    let slave_err = unsafe { libc::dup(slave) };
    if slave_out == -1 || slave_err == -1 {
        unsafe {
            libc::close(master);
            libc::close(slave);
            if slave_out != -1 { libc::close(slave_out); }
            if slave_err != -1 { libc::close(slave_err); }
        }
        return None;
    }

    // ── Spawn claude --allowed-tools "" ───────────────────────────────────────
    let mut cmd = std::process::Command::new(&claude);
    cmd.args(["--allowed-tools", ""]);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLUMNS", "160");
    cmd.env("LINES", "50");
    cmd.current_dir(probe_dir);

    // Safety: we own these raw fds and they are valid at this point.
    cmd.stdin( unsafe { std::fs::File::from_raw_fd(slave) });
    cmd.stdout(unsafe { std::fs::File::from_raw_fd(slave_out) });
    cmd.stderr(unsafe { std::fs::File::from_raw_fd(slave_err) });

    // In the child (between fork and exec): make the slave the controlling
    // terminal so that claude's TUI works correctly, and pin the probe to a
    // single core — its startup otherwise spikes all cores, and a background
    // usage check is not latency-sensitive. Affinity is inherited across exec
    // and by every thread claude spawns; interactive launches are unaffected.
    // Safety: setsid(), ioctl() and the sched_setaffinity syscall are
    // async-signal-safe (no allocation).
    let slave_for_ctty = slave;
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            libc::ioctl(slave_for_ctty, libc::TIOCSCTTY, 0i32);
            let mut cpus: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_SET(0, &mut cpus);
            // Best effort: on failure the probe still works, just unpinned.
            libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &cpus);
            Ok(())
        });
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[claude cli] spawn failed: {e}");
            // slave fds were moved into File objects inside cmd and are closed
            // when cmd is dropped here. Only master needs explicit close.
            unsafe { libc::close(master) };
            return None;
        }
    };
    // After spawn(): stdin/stdout/stderr Files are consumed and Rust closes the
    // parent's copies of slave, slave_out, slave_err.

    // ── Interact and parse ────────────────────────────────────────────────────
    let result = pty_interact(master)
        .and_then(|text| parse_usage_text(&text, today_messages, today_tool_calls));

    let _ = child.kill();
    let _ = child.wait();
    unsafe { libc::close(master) };

    result
}

// ── Fetch sources ─────────────────────────────────────────────────────────────

fn fetch_oauth(today_messages: u64, today_tool_calls: u64) -> Option<UsageData> {
    let creds = read_credentials()?;

    // Guard: skip call if token is expired.
    if let Some(expires_at_ms) = creds.expires_at {
        let expires_at_secs = (expires_at_ms / 1000.0) as i64;
        if Local::now().timestamp() >= expires_at_secs {
            eprintln!("[claude oauth] token expired");
            return None;
        }
    }

    // Guard: require user:profile scope.
    if let Some(ref scopes) = creds.scopes {
        if !scopes.iter().any(|s| s == "user:profile") {
            eprintln!("[claude oauth] missing user:profile scope (has: {scopes:?})");
            return None;
        }
    }

    let plan = creds
        .rate_limit_tier
        .as_deref()
        .map(normalize_plan)
        .unwrap_or_default();

    let mut response = match ureq::get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", &format!("Bearer {}", creds.access_token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .config()
        .http_status_as_error(false)
        .build()
        .call()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[claude oauth] request error: {e}");
            return None;
        }
    };

    let status = response.status();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    if !status.is_success() {
        eprintln!("[claude oauth] HTTP {}: {body}", status.as_u16());
        return None;
    }
    let resp: OAuthUsageResponse = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[claude oauth] parse error: {e}\nbody: {body}");
            return None;
        }
    };

    let extra = resp.extra_usage.as_ref();
    Some(build_usage_data(
        resp.five_hour.as_ref(),
        resp.seven_day.as_ref(),
        extra.and_then(|e| e.used_credits).unwrap_or(0.0),
        extra.and_then(|e| e.monthly_limit).unwrap_or(0.0),
        extra.and_then(|e| e.is_enabled).unwrap_or(false),
        plan,
        today_messages,
        today_tool_calls,
    ))
}

fn fetch_web(today_messages: u64, today_tool_calls: u64) -> Option<UsageData> {
    let session_key = find_session_key()?;
    let cookie = format!("sessionKey={session_key}");

    // 1. Resolve org UUID.
    let mut orgs_response = ureq::get("https://claude.ai/api/organizations")
        .header("Cookie", &cookie)
        .call()
        .ok()?;
    let orgs_body = orgs_response.body_mut().read_to_string().ok()?;
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
    let mut usage_response = ureq::get(&format!(
        "https://claude.ai/api/organizations/{org_id}/usage"
    ))
    .header("Cookie", &cookie)
    .call()
    .ok()?;
    let usage_body = usage_response.body_mut().read_to_string().ok()?;
    let usage: WebUsageResponse = match serde_json::from_str(&usage_body) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[claude web] parse error: {e}\nbody: {usage_body}");
            return None;
        }
    };

    // 3. Overage limit (best-effort).
    let overage: Option<OverageSpendLimit> =
        ureq::get(&format!(
            "https://claude.ai/api/organizations/{org_id}/overage_spend_limit"
        ))
        .header("Cookie", &cookie)
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_to_string().ok())
        .and_then(|s| serde_json::from_str(&s).ok());

    // 4. Account info for plan name (best-effort).
    let plan: String = ureq::get("https://claude.ai/api/account")
        .header("Cookie", &cookie)
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_to_string().ok())
        .and_then(|s| serde_json::from_str::<AccountResponse>(&s).ok())
        .and_then(|a| {
            a.memberships?
                .into_iter()
                .next()?
                .organization?
                .rate_limit_tier
        })
        .map(|t| normalize_plan(&t))
        .unwrap_or_default();

    let extra_enabled = overage.as_ref().and_then(|o| o.is_enabled).unwrap_or(false);
    Some(build_usage_data(
        usage.five_hour.as_ref(),
        usage.seven_day.as_ref(),
        overage.as_ref().and_then(|o| o.used_credits).unwrap_or(0.0),
        overage.as_ref().and_then(|o| o.monthly_credit_limit).unwrap_or(0.0),
        extra_enabled,
        plan,
        today_messages,
        today_tool_calls,
    ))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_plan ────────────────────────────────────────────────────────

    #[test]
    fn normalize_plan_known_tiers() {
        assert_eq!(normalize_plan("claude_pro"), "Pro");
        assert_eq!(normalize_plan("pro"), "Pro");
        assert_eq!(normalize_plan("claude_max"), "Max");
        assert_eq!(normalize_plan("max"), "Max");
        assert_eq!(normalize_plan("claude_team"), "Team");
        assert_eq!(normalize_plan("team"), "Team");
        assert_eq!(normalize_plan("claude_enterprise"), "Enterprise");
        assert_eq!(normalize_plan("enterprise"), "Enterprise");
    }

    #[test]
    fn normalize_plan_unknown_capitalizes() {
        assert_eq!(normalize_plan("business"), "Business");
        assert_eq!(normalize_plan("free"), "Free");
    }

    #[test]
    fn normalize_plan_empty() {
        assert_eq!(normalize_plan(""), "");
    }

    // ── human_reset ───────────────────────────────────────────────────────────

    #[test]
    fn human_reset_zero_is_empty() {
        assert_eq!(human_reset(0), "");
    }

    #[test]
    fn human_reset_relative_for_short_windows() {
        // ≤ 6h → relative format
        assert_eq!(human_reset(3600 + 5 * 60), "resets in 1h 5m");
        assert_eq!(human_reset(30 * 60), "resets in 30m");
        assert_eq!(human_reset(6 * 3600), "resets in 6h");
    }

    #[test]
    fn human_reset_just_over_6h_is_absolute() {
        // 6h + 1s should switch to absolute format (contains "resets" but not "resets in")
        let s = human_reset(6 * 3600 + 1);
        assert!(s.starts_with("resets "));
        assert!(!s.starts_with("resets in "));
    }

    // ── strip_ansi ────────────────────────────────────────────────────────────

    #[test]
    fn strip_ansi_plain_text_unchanged() {
        assert_eq!(strip_ansi(b"hello world"), "hello world");
    }

    #[test]
    fn strip_ansi_removes_color_codes() {
        // ESC[31m red ESC[0m
        let input = b"\x1b[31mred text\x1b[0m";
        assert_eq!(strip_ansi(input), "red text");
    }

    #[test]
    fn strip_ansi_removes_cursor_movement() {
        // ESC[2A (cursor up 2)
        let input = b"line1\x1b[2Aline2";
        assert_eq!(strip_ansi(input), "line1line2");
    }

    #[test]
    fn strip_ansi_removes_osc_with_bel() {
        // ESC ] 0 ; title BEL
        let input = b"\x1b]0;window title\x07content";
        assert_eq!(strip_ansi(input), "content");
    }

    #[test]
    fn strip_ansi_removes_osc_with_st() {
        // ESC ] 0 ; title ESC \
        let input = b"\x1b]0;window title\x1b\\content";
        assert_eq!(strip_ansi(input), "content");
    }

    #[test]
    fn strip_ansi_keeps_box_drawing_chars() {
        // Box-drawing characters are regular Unicode, not ANSI sequences.
        let input = "│ Current session 75% │".as_bytes();
        let result = strip_ansi(input);
        assert!(result.contains("Current session"));
        assert!(result.contains("75%"));
    }

    #[test]
    fn strip_ansi_complex_tui_output() {
        // Typical PTY line: color + text + reset + cursor positioning
        let input = b"\x1b[1;32mCurrent session\x1b[0m: \x1b[33m75%\x1b[0m used\x1b[K";
        let result = strip_ansi(input);
        assert_eq!(result, "Current session: 75% used");
    }

    // ── extract_pct_after ─────────────────────────────────────────────────────

    #[test]
    fn extract_pct_used_format() {
        let text = "Current session: 75% used, resets in 2h";
        assert_eq!(extract_pct_after(text, "Current session"), Some(75.0));
    }

    #[test]
    fn extract_pct_remaining_inverts() {
        let text = "Current session: 25% remaining";
        assert_eq!(extract_pct_after(text, "Current session"), Some(75.0));
    }

    #[test]
    fn extract_pct_decimal() {
        let text = "Current session: 99.5% used";
        assert_eq!(extract_pct_after(text, "Current session"), Some(99.5));
    }

    #[test]
    fn extract_pct_zero() {
        let text = "Current session: 0% used";
        assert_eq!(extract_pct_after(text, "Current session"), Some(0.0));
    }

    #[test]
    fn extract_pct_missing_label_returns_none() {
        let text = "Current session: 50% used";
        assert_eq!(extract_pct_after(text, "Current week"), None);
    }

    #[test]
    fn extract_pct_with_surrounding_box_chars() {
        // As seen in real TUI output after ANSI stripping.
        let text = "│ Current session          75% used  •  resets in 2h │";
        assert_eq!(extract_pct_after(text, "Current session"), Some(75.0));
    }

    #[test]
    fn extract_pct_handles_multibyte_text_past_search_window() {
        let text = format!("Current session 74% used {}", "█".repeat(40));
        assert_eq!(extract_pct_after(&text, "Current session"), Some(74.0));
    }

    #[test]
    fn extract_pct_handles_multibyte_progress_bar_before_value() {
        let text = format!("Current week (all models) {}74% used", "█".repeat(50));
        assert_eq!(
            extract_pct_after(&text, "Current week (all models)"),
            Some(74.0)
        );
    }

    // ── CLI metadata parsing ─────────────────────────────────────────────────

    fn cli_test_now() -> DateTime<Local> {
        Utc
            .with_ymd_and_hms(2026, 7, 20, 21, 30, 0)
            .single()
            .expect("test timestamp should exist")
            .with_timezone(&Local)
    }

    #[test]
    fn parse_cli_reset_relative_formats() {
        let now = cli_test_now();
        assert_eq!(parse_cli_reset_secs("in 2 hr 28 min", now), Some(8880));
        assert_eq!(parse_cli_reset_secs("in 2h 28m", now), Some(8880));
        assert_eq!(parse_cli_reset_secs("in2hr28min", now), Some(8880));
        assert_eq!(parse_cli_reset_secs("in 3 days", now), Some(259200));
    }

    #[test]
    fn parse_cli_reset_absolute_formats() {
        let now = cli_test_now();
        assert_eq!(
            parse_cli_reset_secs("6pm (America/Mexico_City)", now),
            Some(9000)
        );
        assert_eq!(
            parse_cli_reset_secs("Jul 23, 6am (America/Mexico_City)", now),
            Some(225000)
        );
        assert_eq!(
            parse_cli_reset_secs("Jul23,6am (America/Mexico_City)", now),
            Some(225000)
        );
        assert_eq!(
            parse_cli_reset_secs("Jul 23 6am (America/Mexico_City)", now),
            Some(225000)
        );
        assert_eq!(
            parse_cli_reset_secs("Thu 5:59 AM (America/Mexico_City)", now),
            Some(224940)
        );
        assert_eq!(
            parse_cli_reset_secs("Thu5:59AM (America/Mexico_City)", now),
            Some(224940)
        );
        assert_eq!(
            parse_cli_reset_secs("Thu at 5.59 AM (America/Mexico_City)", now),
            Some(224940)
        );
        assert_eq!(
            parse_cli_reset_secs("tomorrow 12:55 AM (America/Mexico_City)", now),
            Some(33900)
        );
        assert_eq!(
            parse_cli_reset_secs("tomorrow12:55AM (America/Mexico_City)", now),
            Some(33900)
        );
    }

    #[test]
    fn parse_cli_reset_honors_explicit_timezone() {
        let now = cli_test_now();
        assert_eq!(
            parse_cli_reset_secs("6pm (America/New_York)", now),
            Some(1800)
        );
    }

    #[test]
    fn parse_cli_reset_invalid_is_none() {
        assert_eq!(parse_cli_reset_secs("eventually", cli_test_now()), None);
        assert_eq!(parse_cli_reset_secs("", cli_test_now()), None);
    }

    #[test]
    fn parse_cli_plan_from_startup_text() {
        assert_eq!(extract_cli_plan("Sonnet 5 · Claude Pro ·"), "Pro");
        assert_eq!(extract_cli_plan("Sonnet5·ClaudePro·"), "Pro");
        assert_eq!(extract_cli_plan("Claude Enterprise"), "Enterprise");
        assert_eq!(extract_cli_plan("plan unavailable"), "");
    }

    // ── parse_usage_text ──────────────────────────────────────────────────────

    #[test]
    fn parse_usage_text_basic() {
        let text = "Current session: 75% used\nCurrent week (all models): 50% used";
        let data = parse_usage_text(text, 10, 5).expect("should parse");
        assert_eq!(data.session_pct, 75.0);
        assert_eq!(data.weekly_pct, 50.0);
        assert_eq!(data.today_messages, 10);
        assert_eq!(data.today_tool_calls, 5);
    }

    #[test]
    fn parse_usage_text_remaining_format() {
        let text = "Current session: 30% remaining\nCurrent week (all models): 40% remaining";
        let data = parse_usage_text(text, 0, 0).expect("should parse");
        assert_eq!(data.session_pct, 70.0);
        assert_eq!(data.weekly_pct, 60.0);
    }

    #[test]
    fn parse_usage_text_opus_weekly() {
        // Falls back to opus-specific label.
        let text = "Current session: 80% used\nCurrent week (Opus): 30% used";
        let data = parse_usage_text(text, 0, 0).expect("should parse");
        assert_eq!(data.session_pct, 80.0);
        assert_eq!(data.weekly_pct, 30.0);
    }

    #[test]
    fn parse_usage_text_no_session_returns_none() {
        // Without "Current session", the function must return None.
        let text = "Current week (all models): 50% used";
        assert!(parse_usage_text(text, 0, 0).is_none());
    }

    #[test]
    fn parse_usage_text_tui_box_chars() {
        // Realistic stripped TUI output with box-drawing characters.
        let text = concat!(
            "╭────────────────────────────────────────╮\n",
            "│ Current session          75% used      │\n",
            "│ Current week (all models) 50% used     │\n",
            "╰────────────────────────────────────────╯\n",
        );
        let data = parse_usage_text(text, 0, 0).expect("should parse");
        assert_eq!(data.session_pct, 75.0);
        assert_eq!(data.weekly_pct, 50.0);
    }

    #[test]
    fn parse_usage_text_includes_optional_cli_metadata() {
        let text = concat!(
            "Sonnet 5 · Claude Pro ·\r",
            "Current session 25% used\r",
            "Resets in 2 hr 28 min\r",
            "Current week (all models) 74% used\r",
            "Resets in 3 days\r",
        );
        let data = parse_usage_text(text, 10, 5).expect("core usage should parse");
        assert_eq!(data.session_resets_secs, 8880);
        assert_eq!(data.weekly_resets_secs, 259200);
        assert_eq!(data.plan, "Pro");
        assert_eq!(data.extra_used_cents, 0.0);
        assert_eq!(data.extra_limit_cents, 0.0);
        assert!(!data.extra_enabled);
    }

    #[test]
    fn parse_usage_text_keeps_core_data_when_optional_fields_are_invalid() {
        let text = concat!(
            "Current session 25% used\r",
            "Resets whenever\r",
            "Current week (all models) 74% used\r",
            "Resets eventually\r",
            "Extra usage $unknown\r",
        );
        let data = parse_usage_text(text, 10, 5).expect("core usage should parse");
        assert_eq!(data.session_pct, 25.0);
        assert_eq!(data.weekly_pct, 74.0);
        assert_eq!(data.session_resets_secs, 0);
        assert_eq!(data.weekly_resets_secs, 0);
        assert_eq!(data.extra_used_cents, 0.0);
        assert_eq!(data.extra_limit_cents, 0.0);
    }

    #[test]
    fn parse_usage_text_uses_latest_rendered_panel() {
        let text = concat!(
            "Settings: Usage\r",
            "Current session 5% used\r",
            "Resets in 1 hr\r",
            "Current week (all models) 10% used\r",
            "Resets in 1 day\r",
            "Settings: Usage\r",
            "Current session 25% used\r",
            "Resets in 2 hr\r",
            "Current week (all models) 74% used\r",
            "Resets in 3 days\r",
        );
        let data = parse_usage_text(text, 0, 0).expect("latest panel should parse");
        assert_eq!(data.session_pct, 25.0);
        assert_eq!(data.weekly_pct, 74.0);
        assert_eq!(data.session_resets_secs, 7200);
        assert_eq!(data.weekly_resets_secs, 259200);
    }

    #[test]
    fn parse_usage_text_no_weekly_defaults_to_zero() {
        // Weekly not found → defaults to 0, not an error.
        let text = "Current session: 60% used";
        let data = parse_usage_text(text, 0, 0).expect("should parse");
        assert_eq!(data.session_pct, 60.0);
        assert_eq!(data.weekly_pct, 0.0);
    }

    #[test]
    fn source_priority_stops_after_oauth_success() {
        let attempted = std::cell::RefCell::new(Vec::new());
        let result = fetch_in_order(
            || {
                attempted.borrow_mut().push("oauth");
                Some("oauth")
            },
            || {
                attempted.borrow_mut().push("cli");
                Some("cli")
            },
            || {
                attempted.borrow_mut().push("web");
                Some("web")
            },
        );
        assert_eq!(result, Some("oauth"));
        assert_eq!(*attempted.borrow(), ["oauth"]);
    }

    #[test]
    fn source_priority_falls_back_from_oauth_to_cli_then_web() {
        let attempted = std::cell::RefCell::new(Vec::new());
        let result = fetch_in_order(
            || {
                attempted.borrow_mut().push("oauth");
                None
            },
            || {
                attempted.borrow_mut().push("cli");
                None
            },
            || {
                attempted.borrow_mut().push("web");
                Some("web")
            },
        );
        assert_eq!(result, Some("web"));
        assert_eq!(*attempted.borrow(), ["oauth", "cli", "web"]);
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

fn fetch_in_order<T>(
    oauth: impl FnOnce() -> Option<T>,
    cli: impl FnOnce() -> Option<T>,
    web: impl FnOnce() -> Option<T>,
) -> Option<T> {
    if let Some(data) = oauth() {
        eprintln!("[claude] OAuth succeeded");
        return Some(data);
    }
    eprintln!("[claude] OAuth failed, trying isolated CLI probe…");
    if let Some(data) = cli() {
        eprintln!("[claude] isolated CLI probe succeeded");
        return Some(data);
    }
    eprintln!("[claude] isolated CLI probe failed, trying web session…");
    if let Some(data) = web() {
        eprintln!("[claude] web session succeeded");
        return Some(data);
    }
    None
}

/// Try sources in order: OAuth API, an isolated Claude CLI probe, then web
/// session (browser cookies / environment variable). Returns None only when
/// all sources fail.
pub fn fetch() -> Option<UsageData> {
    let (today_messages, today_tool_calls) = read_today_stats();
    fetch_in_order(
        || fetch_oauth(today_messages, today_tool_calls),
        || fetch_cli(today_messages, today_tool_calls),
        || fetch_web(today_messages, today_tool_calls),
    )
}
