mod codex;
mod ipc;
mod notify;
mod usage;
mod storage;

use chrono::Local;
use gtk::prelude::*;
use gtk::gdk;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::time::{Duration, Instant};

use plotters::prelude::*;

const CSS: &str = "
window {
    background: rgba(247, 118, 142, 0.45);
    border-radius: 16px;
}
#time {
    font-size: 48px;
    font-weight: bold;
    color: white;
}
#date {
    font-size: 16px;
    color: rgba(255, 255, 255, 0.85);
    margin-bottom: 12px;
}
#usage-section {
    background: rgba(218, 119, 86, 0.7);
    border-radius: 10px;
    padding: 8px;
    margin-top: 8px;
}
#section-label {
    font-size: 11px;
    font-weight: bold;
    color: rgba(255, 255, 255, 0.6);
    letter-spacing: 1px;
    margin-bottom: 2px;
}
#usage-row {
    font-size: 13px;
    color: white;
    margin-bottom: 2px;
}
#today-label {
    font-size: 13px;
    color: rgba(255, 255, 255, 0.9);
    margin-top: 4px;
}
progressbar trough {
    background: rgba(255, 255, 255, 0.2);
    border-radius: 4px;
    min-height: 8px;
}
progressbar progress {
    background: rgba(255, 255, 255, 0.75);
    border-radius: 4px;
}
#codex-section {
    background: rgba(16, 163, 127, 0.7);
    border-radius: 10px;
    padding: 8px;
    margin-top: 8px;
}
calendar {
    background: transparent;
    color: white;
    border: none;
    margin-top: 12px;
}
calendar header {
    color: white;
}
calendar:selected {
    background: rgba(255, 255, 255, 0.3);
    border-radius: 4px;
}
";

fn build_usage_section() -> (gtk::Box, impl Fn(&usage::UsageData)) {
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

    let plot = gtk::Picture::new();
    plot.set_hexpand(true);
    plot.set_size_request(320, 90);

    vbox.append(&section_lbl);
    vbox.append(&session_lbl);
    vbox.append(&session_bar);
    vbox.append(&weekly_lbl);
    vbox.append(&weekly_bar);
    vbox.append(&extra_lbl);
    vbox.append(&today_lbl);
    vbox.append(&updated_lbl);
    vbox.append(&plot);

    let update = move |d: &usage::UsageData| {
        section_lbl.set_text(if d.stale { "CLAUDE USAGE (stale)" } else { "CLAUDE USAGE" });

        session_lbl.set_text(&format!("5h session  {:.0}%  resets {}", d.session_pct, d.session_resets));
        session_bar.set_fraction((d.session_pct / 100.0).clamp(0.0, 1.0));

        weekly_lbl.set_text(&format!("7d weekly   {:.0}%  resets {}", d.weekly_pct, d.weekly_resets));
        weekly_bar.set_fraction((d.weekly_pct / 100.0).clamp(0.0, 1.0));

        if d.extra_limit_cents > 0.0 {
            extra_lbl.set_text(&format!(
                "Extra  ${:.2} / ${:.2}",
                d.extra_used_cents / 100.0,
                d.extra_limit_cents / 100.0,
            ));
        }

        today_lbl.set_text(&format!(
            "Today  {} msgs  ·  {} tool calls",
            d.today_messages, d.today_tool_calls
        ));

        if !d.stale {
            updated_lbl.set_text(&format!("Updated {}", Local::now().format("%H:%M:%S")));
        }

        if let Some(tex) = draw_usage_plot(&storage::load_usage_history()) {
            plot.set_paintable(Some(&tex));
        }
    };

    (vbox, update)
}

fn build_codex_section() -> (gtk::Box, impl Fn(&codex::CodexData)) {
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

    let codex_updated_lbl = gtk::Label::new(None);
    codex_updated_lbl.set_widget_name("today-label");
    codex_updated_lbl.set_halign(gtk::Align::Start);

    let plot = gtk::Picture::new();
    plot.set_hexpand(true);
    plot.set_size_request(320, 90);

    vbox.append(&section_lbl);
    vbox.append(&primary_lbl);
    vbox.append(&primary_bar);
    vbox.append(&secondary_lbl);
    vbox.append(&secondary_bar);
    vbox.append(&codex_updated_lbl);
    vbox.append(&plot);

    let update = move |d: &codex::CodexData| {
        section_lbl.set_text(if d.stale { "CODEX USAGE (stale)" } else { "CODEX USAGE" });

        let plan = if d.plan.is_empty() { String::new() } else { format!("  ({})", d.plan) };
        primary_lbl.set_text(&format!(
            "5h session{plan}  {}%  {}",
            d.primary_pct,
            codex::fmt_resets(d.primary_resets_secs)
        ));
        primary_bar.set_fraction((d.primary_pct as f64 / 100.0).clamp(0.0, 1.0));

        secondary_lbl.set_text(&format!(
            "7d weekly   {}%  {}",
            d.secondary_pct,
            codex::fmt_resets(d.secondary_resets_secs)
        ));
        secondary_bar.set_fraction((d.secondary_pct as f64 / 100.0).clamp(0.0, 1.0));

        if !d.stale {
            codex_updated_lbl.set_text(&format!("Updated {}", Local::now().format("%H:%M:%S")));
        }

        if let Some(tex) = draw_codex_plot(&storage::load_codex_history()) {
            plot.set_paintable(Some(&tex));
        }
    };

    (vbox, update)
}

fn svg_to_texture(svg: &str) -> Option<gdk::Texture> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(svg.as_bytes()).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}

fn draw_usage_plot(samples: &[storage::UsageSample]) -> Option<gdk::Texture> {
    if samples.len() < 2 {
        return None;
    }
    let w = 320;
    let h = 90;
    let mut svg = String::new();
    {
        let root = plotters_svg::SVGBackend::with_string(&mut svg, (w, h)).into_drawing_area();
        root.fill(&WHITE).ok()?;
        let max_y = samples
            .iter()
            .map(|s| s.session_pct.max(s.weekly_pct))
            .fold(0.0_f64, f64::max)
            .max(100.0);
        let (min_x, max_x) = (
            samples.first()?.ts,
            samples.last()?.ts.max(samples.first()?.ts + 1),
        );
        let mut chart = ChartBuilder::on(&root)
            .margin(5)
            .set_left_and_bottom_label_area_size(24)
            .build_cartesian_2d(min_x..max_x, 0.0..max_y)
            .ok()?;
        chart
            .configure_mesh()
            .disable_x_mesh()
            .y_desc("% used")
            .x_labels(3)
            .y_labels(5)
            .label_style(("Inter", 10))
            .draw()
            .ok()?;
        chart
            .draw_series(LineSeries::new(
                samples.iter().map(|s| (s.ts, s.session_pct)),
                &RED.mix(0.8),
            ))
            .ok()?;
        chart
            .draw_series(LineSeries::new(
                samples.iter().map(|s| (s.ts, s.weekly_pct)),
                &BLUE.mix(0.8),
            ))
            .ok()?;
        root.present().ok()?;
    }
    svg_to_texture(&svg)
}

fn draw_codex_plot(samples: &[storage::CodexSample]) -> Option<gdk::Texture> {
    if samples.len() < 2 {
        return None;
    }
    let w = 320;
    let h = 90;
    let mut svg = String::new();
    {
        let root = plotters_svg::SVGBackend::with_string(&mut svg, (w, h)).into_drawing_area();
        root.fill(&WHITE).ok()?;
        let max_y = samples
            .iter()
            .map(|s| s.primary_pct.max(s.secondary_pct) as f64)
            .fold(0.0_f64, f64::max)
            .max(100.0);
        let (min_x, max_x) = (
            samples.first()?.ts,
            samples.last()?.ts.max(samples.first()?.ts + 1),
        );
        let mut chart = ChartBuilder::on(&root)
            .margin(5)
            .set_left_and_bottom_label_area_size(24)
            .build_cartesian_2d(min_x..max_x, 0.0..max_y)
            .ok()?;
        chart
            .configure_mesh()
            .disable_x_mesh()
            .y_desc("% used")
            .x_labels(3)
            .y_labels(5)
            .label_style(("Inter", 10))
            .draw()
            .ok()?;
        chart
            .draw_series(LineSeries::new(
                samples.iter().map(|s| (s.ts, s.primary_pct as f64)),
                &GREEN.mix(0.8),
            ))
            .ok()?;
        chart
            .draw_series(LineSeries::new(
                samples.iter().map(|s| (s.ts, s.secondary_pct as f64)),
                &BLACK.mix(0.7),
            ))
            .ok()?;
        root.present().ok()?;
    }
    svg_to_texture(&svg)
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
    css.load_from_data(CSS);
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

    fn tick(time_label: &gtk::Label, date_label: &gtk::Label) {
        let now = Local::now();
        time_label.set_text(&now.format("%H:%M:%S").to_string());
        date_label.set_text(&now.format("%A, %B %-d %Y").to_string());
    }
    tick(&time_label, &date_label);
    let tl = time_label.clone();
    let dl = date_label.clone();
    glib::timeout_add_seconds_local(1, move || {
        tick(&tl, &dl);
        glib::ControlFlow::Continue
    });

    // Usage section
    let (usage_box, update_usage) = build_usage_section();

    let claude_refresh = std::sync::Arc::new(tokio::sync::Notify::new());

    if let Some(mut cached) = storage::load_usage() {
        cached.stale = true;
        update_usage(&cached);
        storage::append_usage_sample(&cached);
    }

    // --- Claude usage task ---
    let (claude_tx, claude_rx) = async_channel::unbounded::<usage::UsageData>();
    let claude_notify = claude_refresh.clone();
    rt.spawn(async move {
        let mut prev_session: u32 = 0;
        let mut prev_weekly: u32 = 0;
        let mut last_data: Option<usage::UsageData> = None;
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
                    prev_session = data.session_pct.round() as u32;
                    prev_weekly  = data.weekly_pct.round() as u32;
                    last_data = Some(data.clone());
                    storage::save_usage(&data);
                    storage::append_usage_sample(&data);
                    let _ = claude_tx.send(data).await;
                    last_fetch = Instant::now();
                }
                None => {
                    if let Some(ref cached) = last_data {
                        let mut stale = cached.clone();
                        stale.stale = true;
                        let _ = claude_tx.send(stale).await;
                    }
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
    let (codex_box, update_codex) = build_codex_section();
    let (codex_tx, codex_rx) = async_channel::unbounded::<codex::CodexData>();

    let codex_refresh = std::sync::Arc::new(tokio::sync::Notify::new());

    if let Some(mut cached) = storage::load_codex() {
        cached.stale = true;
        update_codex(&cached);
        storage::append_codex_sample(&cached);
    }

    let codex_notify = codex_refresh.clone();
    rt.spawn(async move {
        let mut prev_primary: u32 = 0;
        let mut prev_secondary: u32 = 0;
        let mut last_data: Option<codex::CodexData> = None;
        let poll_every = Duration::from_secs(60);
        loop {
            let result = tokio::task::spawn_blocking(codex::fetch).await.unwrap_or_default();
            match result {
                Some(data) => {
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
                    prev_primary   = data.primary_pct;
                    prev_secondary = data.secondary_pct;
                    last_data = Some(data.clone());
                    storage::save_codex(&data);
                    storage::append_codex_sample(&data);
                    let _ = codex_tx.send(data).await;
                }
                None => {
                    if let Some(ref cached) = last_data {
                        let mut stale = cached.clone();
                        stale.stale = true;
                        let _ = codex_tx.send(stale).await;
                    }
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
                    claude_wakeup.notify_one();
                    codex_wakeup.notify_one();
                }
                ipc::Command::Hide   => win.hide(),
                ipc::Command::Toggle => {
                    if win.is_visible() {
                        win.hide();
                    } else {
                        win.present();
                        claude_wakeup.notify_one();
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

    let claude_on_show = claude_refresh.clone();
    let codex_on_show = codex_refresh.clone();
    window.connect_show(move |_| {
        claude_on_show.notify_one();
        codex_on_show.notify_one();
    });

    window.present();
}

fn main() {
    // Client mode: status-overlay <show|hide|toggle|refresh|quit>
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match ipc::send(&args[1]) {
            Ok(resp) => println!("{resp}"),
            Err(e)   => eprintln!("error: {e} (is the daemon running?)"),
        }
        return;
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
