mod adb;
mod device;
mod engine;
mod icons;
mod settings;
mod storage;
mod theme;
mod ui;
mod updater;

use gpui_kit::*;

fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://localhost:8100".to_string());

    application().with_assets(icons::Assets).run(move |cx| {
        init(cx);
        let theme = settings::Settings::load().theme;
        theme::set_light(theme.is_light(cx), cx);
        theme::install(cx);
        ui::init(cx);
        let display_height = cx
            .primary_display()
            .map(|d| f32::from(d.bounds().size.height))
            .unwrap_or(1000.0);

        // Shared state. The inspector (Console/Network) is not opened at startup:
        // it is opened on demand from the simulator.
        let state = cx.new(|cx| ui::Modev::new(url, display_height, cx));

        // Simulator: a single window (one dock icon) with the bar and the web
        // as fixed panels. Without gpui-kit's Root, which paints an opaque background.
        let sim_size = state.read(cx).simulator_window_size();
        let sim_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, sim_size, cx))),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            window_background: WindowBackgroundAppearance::Transparent,
            is_resizable: true,
            app_id: Some("modev".into()),
            ..Default::default()
        };
        cx.open_window(sim_options, move |window, cx| {
            cx.new(|cx| ui::SimulatorView::new(state.clone(), window, cx))
        })
        .expect("Could not open the simulator");
        cx.activate(true);
    });
}
