use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};

fn activate(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::new(app);

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);

    // Float at top-center; no exclusive zone so it doesn't push other windows
    let anchors = [
        (Edge::Left, false),
        (Edge::Right, false),
        (Edge::Top, true),
        (Edge::Bottom, false),
    ];
    for (edge, state) in anchors {
        window.set_anchor(edge, state);
    }
    window.set_margin(Edge::Top, 8);

    // Transparent background via CSS
    let css = gtk::CssProvider::new();
    css.load_from_data("window { background: transparent; } label { color: white; }");
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(&window),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    let label = gtk::Label::new(Some("status-overlay"));
    window.set_child(Some(&label));
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
