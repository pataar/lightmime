//! Renders the window off-screen for the documentation.
//!
//! `cargo test -p lightmime-gui screenshots -- --ignored` writes
//! `docs/src/images/window-{light,dark}.png`. Ignored by default: it needs a
//! GPU (or a software Vulkan driver) and the fonts installed on this machine.

use std::path::Path;
use std::sync::{Arc, Mutex};

use gpui::{AppContext, HeadlessAppContext, WindowAppearance, px, size};
use lightmime_core::{BridgeConfig, Config, Engine, SyncSettings};

use crate::Shared;
use crate::theme::{ForceAppearance, UiFont};
use crate::view::MainView;

struct NoStore;

impl crate::Store for NoStore {
    fn save_bridge(&self, _: &BridgeConfig) -> anyhow::Result<()> {
        Ok(())
    }

    fn save_sync(&self, _: &SyncSettings) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
#[ignore = "needs a GPU; run with --ignored to refresh the docs images"]
fn screenshots() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _guard = runtime.enter();
    let shared = Shared {
        engine: Engine::spawn(Config::default(), None),
        store: Arc::new(NoStore),
        runtime: runtime.handle().clone(),
        device_name: "lightmime#demo".into(),
        bridge: Arc::new(Mutex::new(BridgeConfig::default())),
    };
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src/images");
    std::fs::create_dir_all(&out).expect("creating docs/src/images");

    let mut cx = HeadlessAppContext::with_platform(
        gpui_platform::current_platform(true).text_system(),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        let font = UiFont::pick(cx);
        cx.set_global(font);
    });

    for (name, appearance) in [
        ("light", WindowAppearance::Light),
        ("dark", WindowAppearance::Dark),
    ] {
        cx.update(|cx| cx.set_global(ForceAppearance(appearance)));
        let shared = shared.clone();
        let window = cx
            .open_window(size(px(440.), px(960.)), |window, cx| {
                cx.new(|cx| {
                    let mut view = MainView::new(shared, window, cx);
                    view.show_demo();
                    view
                })
            })
            .expect("opening the window");
        cx.run_until_parked();
        let image = cx
            .capture_screenshot(window.into())
            .expect("rendering the window");
        image
            .save(out.join(format!("window-{name}.png")))
            .expect("saving the screenshot");
    }
}
