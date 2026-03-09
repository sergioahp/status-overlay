use chrono::{Local, DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
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
    // Find the first '%' within a reasonable window.
    let pct_pos = after[..after.len().min(120)].find('%')?;
    let before_pct = after[..pct_pos].trim_end();
    // Walk back from the end to find where the number starts.
    let num_start = before_pct
        .rfind(|c: char| !c.is_ascii_digit() && c != '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    let pct: f64 = before_pct[num_start..].parse().ok()?;
    // If the context says "remaining", invert to get "used".
    let context = &after[..pct_pos.min(after.len())];
    if context.contains("remaining") {
        Some(100.0 - pct)
    } else {
        Some(pct)
    }
}

fn parse_usage_text(
    text: &str,
    today_messages: u64,
    today_tool_calls: u64,
) -> Option<UsageData> {
    let session_pct = extract_pct_after(text, "Current session")
        .or_else(|| extract_pct_after(text, "current session"))?;

    let weekly_pct = extract_pct_after(text, "Current week (all models)")
        .or_else(|| extract_pct_after(text, "Current week (Opus)"))
        .or_else(|| extract_pct_after(text, "Current week (Sonnet only)"))
        .or_else(|| extract_pct_after(text, "Current week (Sonnet)"))
        .or_else(|| extract_pct_after(text, "Current week"))
        .unwrap_or(0.0);

    eprintln!(
        "[claude cli] parsed session={session_pct:.0}% weekly={weekly_pct:.0}%"
    );

    Some(UsageData {
        session_pct,
        session_resets_secs: 0,
        session_resets: String::new(),
        weekly_pct,
        weekly_resets_secs: 0,
        weekly_resets: String::new(),
        extra_used_cents: 0.0,
        extra_limit_cents: 0.0,
        extra_enabled: false,
        today_messages,
        today_tool_calls,
        plan: String::new(),
        stale: false,
        fetched_at: Local::now().timestamp(),
        attempted_at: Local::now().timestamp(),
    })
}

fn fetch_cli(today_messages: u64, today_tool_calls: u64) -> Option<UsageData> {
    let claude = find_claude_binary()?;

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

    // Safety: we own these raw fds and they are valid at this point.
    cmd.stdin( unsafe { std::fs::File::from_raw_fd(slave) });
    cmd.stdout(unsafe { std::fs::File::from_raw_fd(slave_out) });
    cmd.stderr(unsafe { std::fs::File::from_raw_fd(slave_err) });

    // In the child (between fork and exec): make the slave the controlling
    // terminal so that claude's TUI works correctly.
    // Safety: setsid() and ioctl() are async-signal-safe.
    let slave_for_ctty = slave;
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            libc::ioctl(slave_for_ctty, libc::TIOCSCTTY, 0i32);
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

    let response = match ureq::get("https://api.anthropic.com/api/oauth/usage")
        .set("Authorization", &format!("Bearer {}", creds.access_token))
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

    // 3. Overage limit (best-effort).
    let overage: Option<OverageSpendLimit> =
        ureq::get(&format!(
            "https://claude.ai/api/organizations/{org_id}/overage_spend_limit"
        ))
        .set("Cookie", &cookie)
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|s| serde_json::from_str(&s).ok());

    // 4. Account info for plan name (best-effort).
    let plan: String = ureq::get("https://claude.ai/api/account")
        .set("Cookie", &cookie)
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
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

// ── Public entry point ────────────────────────────────────────────────────────

/// Try sources in order: OAuth API → web session (browser cookies / env var)
/// → CLI PTY probe.  Returns None only when all sources fail.
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
    eprintln!("[claude] web session failed, trying CLI probe…");
    if let Some(data) = fetch_cli(today_messages, today_tool_calls) {
        eprintln!("[claude] CLI probe succeeded");
        return Some(data);
    }
    None
}
