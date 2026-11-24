mod brand;
mod core;
mod html;
mod server;
mod system;
mod tray;
mod types;
mod ui;

use gpui_kit::*;

fn main() {
    let Some(_instance_lock) = core::acquire_single_instance() else {
        return;
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    let core = core::Core::new(rt.handle().clone());
    {
        let core = core.clone();
        rt.spawn(async move {
            if let Err(e) = server::serve(core).await {
                eprintln!("local server failed: {e}");
            }
        });
    }

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_quit_mode(QuitMode::Explicit).run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(980.), px(760.)), cx))),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| ui::ReportApp::new(core, window, cx)))
            .expect("failed to open window");
        cx.activate(true);
    });
    drop(rt);
}
