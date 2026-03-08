mod codex;
mod ipc;
mod notify;
mod usage;

use chrono::Local;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};

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

    vbox.append(&section_lbl);
    vbox.append(&session_lbl);
    vbox.append(&session_bar);
    vbox.append(&weekly_lbl);
    vbox.append(&weekly_bar);
    vbox.append(&extra_lbl);
    vbox.append(&today_lbl);

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

    vbox.append(&section_lbl);
    vbox.append(&primary_lbl);
    vbox.append(&primary_bar);
    vbox.append(&secondary_lbl);
    vbox.append(&secondary_bar);

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
    };

    (vbox, update)
}

fn activate(app: &gtk::Application) {
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

    // Channel: background thread → GTK main loop
    let (sender, receiver) = std::sync::mpsc::channel::<usage::UsageData>();
    let receiver = std::sync::Arc::new(std::sync::Mutex::new(receiver));

    // --- Claude usage thread ---
    std::thread::spawn(move || {
        let mut prev_session: u32 = 0;
        let mut prev_weekly: u32 = 0;
        let mut last_data: Option<usage::UsageData> = None;
        loop {
            match usage::fetch() {
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
                    let _ = sender.send(data);
                }
                None => {
                    if let Some(ref cached) = last_data {
                        let mut stale = cached.clone();
                        stale.stale = true;
                        let _ = sender.send(stale);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(300));
        }
    });

    let recv = receiver.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        if let Ok(data) = recv.lock().unwrap().try_recv() {
            update_usage(&data);
        }
        glib::ControlFlow::Continue
    });

    // --- Codex usage section ---
    let (codex_box, update_codex) = build_codex_section();
    let (codex_tx, codex_rx) = std::sync::mpsc::channel::<codex::CodexData>();
    let codex_rx = std::sync::Arc::new(std::sync::Mutex::new(codex_rx));

    std::thread::spawn(move || {
        let mut prev_primary: u32 = 0;
        let mut prev_secondary: u32 = 0;
        let mut last_data: Option<codex::CodexData> = None;
        loop {
            match codex::fetch() {
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
                    let _ = codex_tx.send(data);
                }
                None => {
                    if let Some(ref cached) = last_data {
                        let mut stale = cached.clone();
                        stale.stale = true;
                        let _ = codex_tx.send(stale);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });

    let codex_recv = codex_rx.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        if let Ok(data) = codex_recv.lock().unwrap().try_recv() {
            update_codex(&data);
        }
        glib::ControlFlow::Continue
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
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if let Ok(cmd) = ipc_recv.lock().unwrap().try_recv() {
            match cmd {
                ipc::Command::Show   => win.present(),
                ipc::Command::Hide   => win.hide(),
                ipc::Command::Toggle => {
                    if win.is_visible() { win.hide(); } else { win.present(); }
                }
                ipc::Command::Quit   => win.close(),
            }
        }
        glib::ControlFlow::Continue
    });

    window.present();
}

fn main() {
    // Client mode: status-overlay <show|hide|toggle|quit>
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match ipc::send(&args[1]) {
            Ok(resp) => println!("{resp}"),
            Err(e)   => eprintln!("error: {e} (is the daemon running?)"),
        }
        return;
    }

    // Daemon mode
    let app = gtk::Application::new(
        Some("dev.status-overlay"),
        gio::ApplicationFlags::default(),
    );
    app.connect_activate(activate);
    app.run();
}
