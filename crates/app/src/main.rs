//! `neuro-convert-app`: the desktop app. All conversion work goes through `nc_convert::Job`;
//! this crate only presents it (see `.tasks/10-01-2026/02-nc-app/`).

mod actions;
mod app;
mod domain;
mod services;
mod settings;
mod state;
mod store;
mod viewmodels;
mod views;
mod widgets;

use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::{px, size, App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions};
use nc_convert::Registry;

fn main() {
    // Resolve a relative path against the launch directory now, before the platform starts
    let start_path = std::env::args_os().nth(1).map(std::path::PathBuf::from).map(|p| std::fs::canonicalize(&p).unwrap_or(p));
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx: &mut App| {
        gpui_kit::init(cx);
        actions::bind_keys(cx);
        cx.on_action(|_: &actions::Quit, cx| cx.quit());
        // One main window: closing it ends the app (Linux / Windows convention)
        cx.on_window_closed(|cx, _| cx.quit()).detach();

        let settings_path = settings::Settings::path();
        let settings = settings::Settings::load(settings_path.as_deref());
        let store = cx.new(|_| store::Store::new(domain::Workspace::new(Arc::new(Registry::builtin()), settings, settings_path)));

        // The app draws its own title bar (gpui-kit `TitleBar`): on client-decorated sessions
        // (GNOME Wayland) it supplies minimize / maximize / close; where the compositor decorates
        // (e.g. KDE) it leaves the buttons to the compositor.
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions { title: Some("neuro-convert".into()), ..TitleBar::title_bar_options() }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1400.), px(880.)), cx))),
            window_min_size: Some(size(px(1000.), px(620.))),
            app_id: Some("org.neuro-kitchen.neuro-convert".into()),
            ..TitleBar::window_options()
        };
        // NC_APP_LOG=1: every status message also goes to stderr (bug reports, scripted runs)
        if std::env::var_os("NC_APP_LOG").is_some() {
            let mut last = None;
            cx.subscribe(&store, move |store, event: &domain::AppEvent, cx| {
                if *event != domain::AppEvent::Status {
                    return;
                }
                let s = &store.read(cx).ws.state;
                let line = (s.phase_label(), s.message.clone());
                if last.as_ref() != Some(&line) {
                    eprintln!("[neuro-convert-app] {}: {}", line.0, line.1.as_deref().unwrap_or(""));
                    last = Some(line);
                }
            })
            .detach();
        }

        let s = store.clone();
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| app::NcApp::new(s, window, cx))).expect("failed to open the main window");
        cx.activate(true);

        // `neuro-convert-app <recording>` (e.g. "open with" from a file manager) opens it at start
        if let Some(path) = start_path {
            store.update(cx, |s, cx| s.open(path, None, cx));
        }
    });
}
