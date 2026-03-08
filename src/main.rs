use chrono::Local;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};

const CSS: &str = "
window {
    background: rgba(20, 20, 30, 0.85);
    border-radius: 12px;
}
#time {
    font-size: 48px;
    font-weight: bold;
    color: white;
}
#date {
    font-size: 16px;
    color: rgba(255, 255, 255, 0.7);
    margin-bottom: 8px;
}
calendar {
    background: transparent;
    color: white;
    border: none;
}
calendar header {
    color: white;
}
calendar:selected {
    background: rgba(100, 150, 255, 0.6);
    border-radius: 4px;
}
";

fn activate(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::new(app);

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);

    let anchors = [
        (Edge::Left, false),
        (Edge::Right, false),
        (Edge::Top, true),
        (Edge::Bottom, false),
    ];
    for (edge, state) in anchors {
        window.set_anchor(edge, state);
    }
    window.set_margin(Edge::Top, 16);

    let css = gtk::CssProvider::new();
    css.load_from_data(CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(&window),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    // Layout
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 4);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);
    vbox.set_halign(gtk::Align::Center);

    let time_label = gtk::Label::new(None);
    time_label.set_widget_name("time");

    let date_label = gtk::Label::new(None);
    date_label.set_widget_name("date");

    let calendar = gtk::Calendar::new();

    fn update(time_label: &gtk::Label, date_label: &gtk::Label) {
        let now = Local::now();
        time_label.set_text(&now.format("%H:%M:%S").to_string());
        date_label.set_text(&now.format("%A, %B %-d %Y").to_string());
    }

    update(&time_label, &date_label);

    // Tick every second
    let tl = time_label.clone();
    let dl = date_label.clone();
    glib::timeout_add_seconds_local(1, move || {
        update(&tl, &dl);
        glib::ControlFlow::Continue
    });

    vbox.append(&time_label);
    vbox.append(&date_label);
    vbox.append(&calendar);

    window.set_child(Some(&vbox));
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
