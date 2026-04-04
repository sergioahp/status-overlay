mod clock;
mod codex;
mod ipc;
mod notify;
mod usage;
mod storage;

use chrono::{DateTime, Local, TimeZone};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::time::{Duration, Instant};
use std::fs;

/// Format a unix timestamp as a human-readable "X ago" string relative to `now`.
fn fmt_ago(ts: i64, now: DateTime<Local>) -> String {
    if ts == 0 {
        return "--".to_string();
    }
    let dt = match Local.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(dt) => dt,
        _ => return "--".to_string(),
    };
    let secs = (now - dt).num_seconds().max(0);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        let m = secs / 60;
        format!("{m} minute{} ago", if m == 1 { "" } else { "s" })
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m}m ago")
    }
}

fn format_fetch_label(fetched_at: i64, attempted_at: i64, stale: bool, now: DateTime<Local>) -> String {
    if stale {
        format!("Updated {} · last attempt {}", fmt_ago(fetched_at, now), fmt_ago(attempted_at, now))
    } else {
        format!("Updated {}", fmt_ago(fetched_at, now))
    }
}

const EMBEDDED_CSS: &str = include_str!("style.css");

fn load_css() -> String {
    if let Ok(path) = std::env::var("STATUS_OVERLAY_CSS") {
        if let Ok(data) = fs::read_to_string(&path) {
            return data;
        }
    }
    EMBEDDED_CSS.to_string()
}

fn build_usage_section() -> (gtk::Box, impl Fn(&usage::UsageData), impl Fn(DateTime<Local>)) {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
    vbox.set_halign(gtk::Align::Fill);
    vbox.set_widget_name("usage-section");

    let section_lbl = gtk::Label::new(Some("CLAUDE USAGE"));
    section_lbl.set_widget_name("section-label");
    section_lbl.set_halign(gtk::Align::Start);

    // Session row
    let session_lbl = gtk::Label::new(None);
    session_lbl.set_widget_name("usage-row");
    session_lbl.set_halign(gtk::Align::Start);
    let session_bar = gtk::ProgressBar::new();
    session_bar.set_hexpand(true);

    // Weekly row
    let weekly_lbl = gtk::Label::new(None);
    weekly_lbl.set_widget_name("usage-row");
    weekly_lbl.set_halign(gtk::Align::Start);
    let weekly_bar = gtk::ProgressBar::new();
    weekly_bar.set_hexpand(true);

    // Extra / today
    let extra_lbl = gtk::Label::new(None);
    extra_lbl.set_widget_name("usage-row");
    extra_lbl.set_halign(gtk::Align::Start);

    let today_lbl = gtk::Label::new(None);
    today_lbl.set_widget_name("today-label");
    today_lbl.set_halign(gtk::Align::Start);

    let updated_lbl = gtk::Label::new(None);
    updated_lbl.set_widget_name("today-label");
    updated_lbl.set_halign(gtk::Align::Start);

    vbox.append(&section_lbl);
    vbox.append(&session_lbl);
    vbox.append(&session_bar);
    vbox.append(&weekly_lbl);
    vbox.append(&weekly_bar);
    vbox.append(&extra_lbl);
    vbox.append(&today_lbl);
    vbox.append(&updated_lbl);

    // Shared last-data for the tick closure.
    let last_data: std::rc::Rc<std::cell::RefCell<Option<usage::UsageData>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let last_data_t = last_data.clone();

    // Label clones for the tick closure.
    let session_lbl_t = session_lbl.clone();
    let weekly_lbl_t = weekly_lbl.clone();
    let updated_lbl_t = updated_lbl.clone();

    let update = move |d: &usage::UsageData| {
        let now = Local::now();
        *last_data.borrow_mut() = Some(d.clone());

        let plan_part = if d.plan.is_empty() { String::new() } else { format!("  ({})", d.plan) };
        section_lbl.set_text(&format!(
            "CLAUDE USAGE{}{}",
            plan_part,
            if d.stale { " (stale)" } else { "" },
        ));
        session_bar.set_fraction((d.session_pct / 100.0).clamp(0.0, 1.0));
        weekly_bar.set_fraction((d.weekly_pct / 100.0).clamp(0.0, 1.0));

        if d.extra_enabled && d.extra_limit_cents > 0.0 {
            extra_lbl.set_text(&format!(
                "Extra  ${:.2} / ${:.2}",
                d.extra_used_cents / 100.0,
                d.extra_limit_cents / 100.0,
            ));
            extra_lbl.set_visible(true);
        } else {
            extra_lbl.set_visible(false);
        }
        today_lbl.set_text(&format!(
            "Today  {} msgs  ·  {} tool calls",
            d.today_messages, d.today_tool_calls
        ));

        // Also update the time-sensitive labels immediately on new data.
        let elapsed = (now.timestamp() - d.fetched_at).max(0) as u64;
        session_lbl.set_text(&format!(
            "5h session  {:.0}%  {}",
            d.session_pct,
            usage::human_reset(d.session_resets_secs.saturating_sub(elapsed)),
        ));
        weekly_lbl.set_text(&format!(
            "7d weekly   {:.0}%  {}",
            d.weekly_pct,
            usage::human_reset(d.weekly_resets_secs.saturating_sub(elapsed)),
        ));
        updated_lbl.set_text(&format_fetch_label(d.fetched_at, d.attempted_at, d.stale, now));
    };

    // Called every second by the Clock dispatcher to refresh time-relative text.
    let tick = move |now: DateTime<Local>| {
        let guard = last_data_t.borrow();
        let Some(ref d) = *guard else { return };
        let elapsed = (now.timestamp() - d.fetched_at).max(0) as u64;
        session_lbl_t.set_text(&format!(
            "5h session  {:.0}%  {}",
            d.session_pct,
            usage::human_reset(d.session_resets_secs.saturating_sub(elapsed)),
        ));
        weekly_lbl_t.set_text(&format!(
            "7d weekly   {:.0}%  {}",
            d.weekly_pct,
            usage::human_reset(d.weekly_resets_secs.saturating_sub(elapsed)),
        ));
        updated_lbl_t.set_text(&format_fetch_label(d.fetched_at, d.attempted_at, d.stale, now));
    };

    (vbox, update, tick)
}

fn build_codex_section() -> (gtk::Box, impl Fn(&codex::CodexData), impl Fn(DateTime<Local>)) {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
    vbox.set_halign(gtk::Align::Fill);
    vbox.set_widget_name("codex-section");

    let section_lbl = gtk::Label::new(Some("CODEX USAGE"));
    section_lbl.set_widget_name("section-label");
    section_lbl.set_halign(gtk::Align::Start);

    let primary_lbl = gtk::Label::new(None);
    primary_lbl.set_widget_name("usage-row");
    primary_lbl.set_halign(gtk::Align::Start);
    let primary_bar = gtk::ProgressBar::new();
    primary_bar.set_hexpand(true);

    let secondary_lbl = gtk::Label::new(None);
    secondary_lbl.set_widget_name("usage-row");
    secondary_lbl.set_halign(gtk::Align::Start);
    let secondary_bar = gtk::ProgressBar::new();
    secondary_bar.set_hexpand(true);

    let updated_lbl = gtk::Label::new(None);
    updated_lbl.set_widget_name("today-label");
    updated_lbl.set_halign(gtk::Align::Start);

    vbox.append(&section_lbl);
    vbox.append(&primary_lbl);
    vbox.append(&primary_bar);
    vbox.append(&secondary_lbl);
    vbox.append(&secondary_bar);
    vbox.append(&updated_lbl);

    let last_data: std::rc::Rc<std::cell::RefCell<Option<codex::CodexData>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let last_data_t = last_data.clone();

    let primary_lbl_t = primary_lbl.clone();
    let secondary_lbl_t = secondary_lbl.clone();
    let updated_lbl_t = updated_lbl.clone();

    let update = move |d: &codex::CodexData| {
        let now = Local::now();
        *last_data.borrow_mut() = Some(d.clone());

        section_lbl.set_text(if d.stale { "CODEX USAGE (stale)" } else { "CODEX USAGE" });
        primary_bar.set_fraction((d.primary_pct as f64 / 100.0).clamp(0.0, 1.0));
        secondary_bar.set_fraction((d.secondary_pct as f64 / 100.0).clamp(0.0, 1.0));

        let elapsed = (now.timestamp() - d.fetched_at).max(0) as u64;
        let plan = if d.plan.is_empty() { String::new() } else { format!("  ({})", d.plan) };
        primary_lbl.set_text(&format!(
            "5h session{plan}  {}%  {}",
            d.primary_pct,
            codex::fmt_resets(d.primary_resets_secs.saturating_sub(elapsed)),
        ));
        secondary_lbl.set_text(&format!(
            "7d weekly   {}%  {}",
            d.secondary_pct,
            codex::fmt_resets(d.secondary_resets_secs.saturating_sub(elapsed)),
        ));
        updated_lbl.set_text(&format_fetch_label(d.fetched_at, d.attempted_at, d.stale, now));
    };

    let tick = move |now: DateTime<Local>| {
        let guard = last_data_t.borrow();
        let Some(ref d) = *guard else { return };
        let elapsed = (now.timestamp() - d.fetched_at).max(0) as u64;
        let plan = if d.plan.is_empty() { String::new() } else { format!("  ({})", d.plan) };
        primary_lbl_t.set_text(&format!(
            "5h session{plan}  {}%  {}",
            d.primary_pct,
            codex::fmt_resets(d.primary_resets_secs.saturating_sub(elapsed)),
        ));
        secondary_lbl_t.set_text(&format!(
            "7d weekly   {}%  {}",
            d.secondary_pct,
            codex::fmt_resets(d.secondary_resets_secs.saturating_sub(elapsed)),
        ));
        updated_lbl_t.set_text(&format_fetch_label(d.fetched_at, d.attempted_at, d.stale, now));
    };

    (vbox, update, tick)
}

fn activate(app: &gtk::Application, rt: tokio::runtime::Handle) {
    let window = gtk::ApplicationWindow::new(app);

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("status-overlay"));

    let anchors = [
        (Edge::Left, true),
        (Edge::Right, true),
        (Edge::Top, false),
        (Edge::Bottom, false),
    ];
    for (edge, state) in anchors {
        window.set_anchor(edge, state);
    }

    let monitor_width = gtk::prelude::WidgetExt::display(&window)
        .monitors()
        .into_iter()
        .next()
        .and_then(|obj| obj.ok())
        .and_then(|obj: glib::Object| obj.downcast::<gtk::gdk::Monitor>().ok())
        .map(|m| m.geometry().width())
        .unwrap_or(1920);
    let side_margin = monitor_width / 10;
    window.set_margin(Edge::Left, side_margin);
    window.set_margin(Edge::Right, side_margin);

    let css = gtk::CssProvider::new();
    css.load_from_data(&load_css());
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(&window),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 4);
    vbox.set_margin_top(20);
    vbox.set_margin_bottom(20);
    vbox.set_margin_start(28);
    vbox.set_margin_end(28);
    vbox.set_halign(gtk::Align::Fill);

    // Clock
    let time_label = gtk::Label::new(None);
    time_label.set_widget_name("time");

    let date_label = gtk::Label::new(None);
    date_label.set_widget_name("date");

    // Usage section
    let (usage_box, update_usage, tick_usage) = build_usage_section();

    let cached_usage = storage::load_usage();
    if let Some(ref cached) = cached_usage {
        let mut display = cached.clone();
        display.stale = true;
        update_usage(&display);
    }

    let claude_refresh = std::sync::Arc::new(tokio::sync::Notify::new());

    // --- Claude usage task ---
    let (claude_tx, claude_rx) = async_channel::unbounded::<usage::UsageData>();
    let claude_notify = claude_refresh.clone();
    rt.spawn(async move {
        let mut prev_session: u32 = 0;
        let mut prev_weekly: u32 = 0;
        let mut prev_session_reset_secs: u64 = 0;
        let mut prev_weekly_reset_secs: u64 = 0;
        // Seed with stored data so a failed first fetch shows last known values,
        // not zeros.
        let mut last_data: Option<usage::UsageData> = cached_usage;
        let mut claude_recover_notice_sent = false;
        let mut claude_pre_reset_notice_sent = false; // 5h window
        let mut claude_weekly_pre_2h_sent = false;
        let mut claude_weekly_pre_1h_sent = false;
        let poll_every = Duration::from_secs(300);
        let min_gap   = Duration::from_secs(30);
        let mut last_fetch = Instant::now() - poll_every; // ensure first fetch is immediate
        loop {
            // Enforce 30s min gap between Claude fetches
            loop {
                let elapsed = Instant::now().saturating_duration_since(last_fetch);
                if elapsed >= min_gap {
                    break;
                }
                let remaining = min_gap - elapsed;
                tokio::select! {
                    _ = tokio::time::sleep(remaining) => {},
                    _ = claude_notify.notified() => {},
                }
            }

            let result = tokio::task::spawn_blocking(usage::fetch).await.unwrap_or_default();
            match result {
                Some(data) => {
                    let mut data = data;
                    data.attempted_at = Local::now().timestamp();
                    data.fetched_at = data.attempted_at;
                    if let Some(t) = notify::transition(prev_session, data.session_pct.round() as u32) {
                        match t {
                            notify::Transition::Low      => notify::send("Claude session low", &format!("{}% of 5h session used", data.session_pct)),
                            notify::Transition::Depleted => notify::send("Claude session depleted", "5h session quota reached"),
                            notify::Transition::Restored => notify::send("Claude session restored", "5h session quota available again"),
                        }
                    }
                    if let Some(t) = notify::transition(prev_weekly, data.weekly_pct.round() as u32) {
                        match t {
                            notify::Transition::Low      => notify::send("Claude weekly low", &format!("{}% of 7d quota used", data.weekly_pct)),
                            notify::Transition::Depleted => notify::send("Claude weekly depleted", "7-day quota reached"),
                            notify::Transition::Restored => notify::send("Claude weekly restored", "Weekly quota available again"),
                        }
                    }
                    // Weekly pre-reset reminders (2h and 1h) if >=10% remaining
                    if data.weekly_resets_secs > prev_weekly_reset_secs + 60 {
                        claude_weekly_pre_1h_sent = false;
                        claude_weekly_pre_2h_sent = false;
                    }
                    let weekly_remaining = 100.0 - data.weekly_pct;
                    if data.weekly_resets_secs > 0
                        && data.weekly_resets_secs <= 7200
                        && weekly_remaining >= 10.0
                        && !claude_weekly_pre_2h_sent
                        && data.weekly_pct < 100.0
                    {
                        notify::send(
                            "Claude weekly resets in ~2h",
                            &format!("{weekly_remaining:.0}% of weekly remains; use it before reset"),
                        );
                        claude_weekly_pre_2h_sent = true;
                    }
                    if data.weekly_resets_secs > 0
                        && data.weekly_resets_secs <= 3600
                        && weekly_remaining >= 10.0
                        && !claude_weekly_pre_1h_sent
                        && data.weekly_pct < 100.0
                    {
                        notify::send(
                            "Claude weekly resets in ~1h",
                            &format!("{weekly_remaining:.0}% of weekly remains; use it before reset"),
                        );
                        claude_weekly_pre_1h_sent = true;
                    }
                    // Pre-reset reminder when >30% remains and reset is within 1h
                    if data.session_resets_secs > prev_session_reset_secs + 60 {
                        claude_pre_reset_notice_sent = false; // reset after a window refresh
                    }
                    if data.session_resets_secs > 0
                        && data.session_resets_secs <= 3600
                        && (100.0 - data.session_pct) >= 30.0
                        && !claude_pre_reset_notice_sent
                        && data.session_pct < 100.0
                    {
                        notify::send(
                            "Claude 5h resets in ~1h",
                            &format!("{:.0}% still unused; grab it now", 100.0 - data.session_pct),
                        );
                        claude_pre_reset_notice_sent = true;
                    }
                    let depleted = data.session_pct >= 100.0 && data.weekly_pct >= 100.0;
                    let avail_secs = data.session_resets_secs.max(data.weekly_resets_secs);
                    if depleted {
                        if avail_secs > 0 && avail_secs <= 3600 && !claude_recover_notice_sent {
                            notify::send("Claude back soon", "Quota should reopen in ~1 hour");
                            claude_recover_notice_sent = true;
                        }
                    } else {
                        claude_recover_notice_sent = false;
                    }
                    prev_session_reset_secs = data.session_resets_secs;
                    prev_weekly_reset_secs = data.weekly_resets_secs;
                    prev_session = data.session_pct.round() as u32;
                    prev_weekly  = data.weekly_pct.round() as u32;
                    last_data = Some(data.clone());
                    storage::save_usage(&data);
                    storage::append_usage_sample(&data);
                    let _ = claude_tx.send(data).await;
                    last_fetch = Instant::now();
                }
                None => {
                    let mut stale = last_data.clone().unwrap_or_default();
                    stale.attempted_at = Local::now().timestamp();
                    stale.stale = true;
                    let _ = claude_tx.send(stale).await;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(poll_every) => {},
                _ = claude_notify.notified() => {},
            }
        }
    });

    glib::spawn_future_local(async move {
        while let Ok(data) = claude_rx.recv().await {
            update_usage(&data);
        }
    });

    // --- Codex usage section ---
    let (codex_box, update_codex, tick_codex) = build_codex_section();

    let cached_codex = storage::load_codex();
    if let Some(ref cached) = cached_codex {
        let mut display = cached.clone();
        display.stale = true;
        update_codex(&display);
    }
    let (codex_tx, codex_rx) = async_channel::unbounded::<codex::CodexData>();

    let codex_refresh = std::sync::Arc::new(tokio::sync::Notify::new());

    let codex_notify = codex_refresh.clone();
    rt.spawn(async move {
        let mut prev_primary: u32 = 0;
        let mut prev_secondary: u32 = 0;
        let mut prev_primary_reset_secs: u64 = 0;
        let mut prev_secondary_reset_secs: u64 = 0;
        // Seed with stored data so a failed first fetch shows last known values.
        let mut last_data: Option<codex::CodexData> = cached_codex;
        let mut codex_recover_notice_sent = false;
        let mut codex_pre_reset_2h_sent = false;
        let mut codex_pre_reset_1h_sent = false;
        let mut codex_primary_pre_reset_sent = false;
        let poll_every = Duration::from_secs(60);
        loop {
            let result = tokio::task::spawn_blocking(codex::fetch).await.unwrap_or_default();
            match result {
                Some(data) => {
                    let mut data = data;
                    data.attempted_at = Local::now().timestamp();
                    data.fetched_at = data.attempted_at;
                    if let Some(t) = notify::transition(prev_primary, data.primary_pct) {
                        match t {
                            notify::Transition::Low      => notify::send("Codex session low", &format!("{}% of session used", data.primary_pct)),
                            notify::Transition::Depleted => notify::send("Codex session depleted", "Session quota reached"),
                            notify::Transition::Restored => notify::send("Codex session restored", "Session quota available again"),
                        }
                    }
                    if let Some(t) = notify::transition(prev_secondary, data.secondary_pct) {
                        match t {
                            notify::Transition::Low      => notify::send("Codex weekly low", &format!("{}% of weekly quota used", data.secondary_pct)),
                            notify::Transition::Depleted => notify::send("Codex weekly depleted", "Weekly quota reached"),
                            notify::Transition::Restored => notify::send("Codex weekly restored", "Weekly quota available again"),
                        }
                    }
                    // Primary (5h) pre-reset: >30% remains, within 1h
                    if data.primary_resets_secs > prev_primary_reset_secs + 60 {
                        codex_primary_pre_reset_sent = false;
                    }
                    let primary_remaining = 100u32.saturating_sub(data.primary_pct);
                    if data.primary_resets_secs > 0
                        && data.primary_resets_secs <= 3600
                        && primary_remaining >= 30
                        && !codex_primary_pre_reset_sent
                        && data.primary_pct < 100
                    {
                        notify::send(
                            "Codex 5h resets in ~1h",
                            &format!("{primary_remaining}% still unused; grab it now"),
                        );
                        codex_primary_pre_reset_sent = true;
                    }
                    // Pre-reset reminders for secondary window: 2h and 1h if >=10% remains
                    if data.secondary_resets_secs > prev_secondary_reset_secs + 60 {
                        codex_pre_reset_1h_sent = false;
                        codex_pre_reset_2h_sent = false;
                    }
                    let remaining_secondary = 100u32.saturating_sub(data.secondary_pct);
                    if data.secondary_resets_secs > 0
                        && data.secondary_resets_secs <= 7200
                        && remaining_secondary >= 10
                        && !codex_pre_reset_2h_sent
                        && data.secondary_pct < 100
                    {
                        notify::send(
                            "Codex weekly resets in ~2h",
                            &format!("{remaining_secondary}% still available; use it before reset"),
                        );
                        codex_pre_reset_2h_sent = true;
                    }
                    if data.secondary_resets_secs > 0
                        && data.secondary_resets_secs <= 3600
                        && remaining_secondary >= 10
                        && !codex_pre_reset_1h_sent
                        && data.secondary_pct < 100
                    {
                        notify::send(
                            "Codex weekly resets in ~1h",
                            &format!("{remaining_secondary}% still available; use it before reset"),
                        );
                        codex_pre_reset_1h_sent = true;
                    }

                    let depleted = data.primary_pct >= 100 && data.secondary_pct >= 100;
                    let avail_secs = data.primary_resets_secs.max(data.secondary_resets_secs);
                    if depleted {
                        if avail_secs > 0 && avail_secs <= 3600 && !codex_recover_notice_sent {
                            notify::send("Codex back soon", "Quota should reopen in ~1 hour");
                            codex_recover_notice_sent = true;
                        }
                    } else {
                        codex_recover_notice_sent = false;
                    }
                    prev_primary_reset_secs = data.primary_resets_secs;
                    prev_secondary_reset_secs = data.secondary_resets_secs;
                    prev_primary   = data.primary_pct;
                    prev_secondary = data.secondary_pct;
                    last_data = Some(data.clone());
                    storage::save_codex(&data);
                    storage::append_codex_sample(&data);
                    let _ = codex_tx.send(data).await;
                }
                None => {
                    let mut stale = last_data.clone().unwrap_or_default();
                    stale.attempted_at = Local::now().timestamp();
                    stale.stale = true;
                    let _ = codex_tx.send(stale).await;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(poll_every) => {},
                _ = codex_notify.notified() => {},
            }
        }
    });

    glib::spawn_future_local(async move {
        while let Ok(data) = codex_rx.recv().await {
            update_codex(&data);
        }
    });

    // Central clock dispatcher — drives the clock display and all time-relative
    // labels at second boundaries, accurate to within ~50ms.
    {
        let tl = time_label.clone();
        let dl = date_label.clone();
        clock::Clock::new()
            .on_second(move |now| {
                tl.set_text(&now.format("%H:%M:%S").to_string());
                dl.set_text(&now.format("%A, %B %-d %Y").to_string());
                tick_usage(now);
                tick_codex(now);
            })
            .start();
    }

    // Calendar
    let calendar = gtk::Calendar::new();

    vbox.append(&time_label);
    vbox.append(&date_label);
    vbox.append(&usage_box);
    vbox.append(&codex_box);
    vbox.append(&calendar);

    window.set_child(Some(&vbox));

    let key_controller = gtk::EventControllerKey::new();
    let win = window.clone();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::q {
            win.close();
        }
        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);

    // IPC channel: socket thread → GTK main loop
    let (ipc_tx, ipc_rx) = std::sync::mpsc::channel::<ipc::Command>();
    let ipc_rx = std::sync::Arc::new(std::sync::Mutex::new(ipc_rx));

    std::thread::spawn(move || ipc::listen(ipc_tx));

    let win = window.clone();
    let ipc_recv = ipc_rx.clone();
    let claude_wakeup = claude_refresh.clone();
    let codex_wakeup = codex_refresh.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok(cmd) = ipc_recv.lock().unwrap().try_recv() {
            match cmd {
                ipc::Command::Show   => {
                    win.present();
                    codex_wakeup.notify_one();
                }
                ipc::Command::Hide   => win.hide(),
                ipc::Command::Toggle => {
                    if win.is_visible() {
                        win.hide();
                    } else {
                        win.present();
                        codex_wakeup.notify_one();
                    }
                }
                ipc::Command::Refresh => {
                    claude_wakeup.notify_one();
                    codex_wakeup.notify_one();
                }
                ipc::Command::Quit   => win.close(),
            }
        }
        glib::ControlFlow::Continue
    });

    let codex_on_show = codex_refresh.clone();
    window.connect_show(move |_| {
        codex_on_show.notify_one();
    });

    window.present();
}

fn main() {
    // Client mode: status-overlay <show|hide|toggle|refresh|quit|--help|-h>
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                println!("Usage: status-overlay <command>\nCommands: show | hide | toggle | refresh | quit");
                return;
            }
            other => {
                match ipc::send(other) {
                    Ok(resp) => println!("{resp}"),
                    Err(e)   => eprintln!("error: {e} (is the daemon running?)"),
                }
                return;
            }
        }
    }

    // Daemon mode
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let app = gtk::Application::new(
        Some("dev.status-overlay"),
        gio::ApplicationFlags::default(),
    );
    let handle = rt.handle().clone();
    app.connect_activate(move |app| activate(app, handle.clone()));
    app.run();
}
