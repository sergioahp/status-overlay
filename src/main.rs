mod usage;

use chrono::Local;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};

const CSS: &str = "
window {
    background: rgba(218, 119, 86, 0.65);
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
#section-label {
    font-size: 11px;
    font-weight: bold;
    color: rgba(255, 255, 255, 0.6);
    letter-spacing: 1px;
    margin-top: 8px;
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

    // Background thread fetches every 60s
    std::thread::spawn(move || loop {
        let _ = sender.send(usage::fetch());
        std::thread::sleep(std::time::Duration::from_secs(60));
    });

    // GTK timer drains the channel
    let recv = receiver.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        if let Ok(data) = recv.lock().unwrap().try_recv() {
            update_usage(&data);
        }
        glib::ControlFlow::Continue
    });

    // Calendar
    let calendar = gtk::Calendar::new();

    vbox.append(&time_label);
    vbox.append(&date_label);
    vbox.append(&usage_box);
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

    window.present();
}

fn main() {
    let app = gtk::Application::new(
        Some("dev.status-overlay"),
        gio::ApplicationFlags::default(),
    );
    app.connect_activate(activate);
    app.run();
}
