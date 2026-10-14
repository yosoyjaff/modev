mod adb;
mod device;
mod engine;
mod icons;
#[cfg(target_os = "macos")]
mod macos;
mod settings;
mod storage;
mod theme;
mod ui;
mod updater;

use gpui_kit::*;

actions!(modev, [Quit]);

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
        // macOS: Cmd+Q and the app menu's Quit.
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "Modev".into(),
            items: vec![MenuItem::action("Quit Modev", Quit)],
            disabled: false,
        }]);
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
            titlebar: ui::titlebar(point(px(11.), px(11.))),
            window_decorations: Some(WindowDecorations::Client),
            window_background: WindowBackgroundAppearance::Transparent,
            is_resizable: true,
            app_id: Some("modev".into()),
            ..Default::default()
        };
        cx.open_window(sim_options, move |window, cx| {
            #[cfg(target_os = "macos")]
            macos::style_window(window);
            // Closing the simulator closes modev (and the headless Chrome): with the
            // native close button macOS would only close the window and keep the app alive.
            window.on_window_should_close(cx, |_, cx| {
                cx.quit();
                true
            });
            cx.new(|cx| ui::SimulatorView::new(state.clone(), window, cx))
        })
        .expect("Could not open the simulator");
        cx.activate(true);
    });
}
