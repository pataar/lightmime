//! The lightmime window and tray icon.
//!
//! Embeds a core [`Engine`] the same way the daemon does. The binary decides
//! where settings are stored (see [`Store`]), so this crate knows nothing
//! about the config file or MQTT.

mod autostart;
mod decorations;
mod icons;
mod theme;
mod tray;
mod view;
mod widgets;

#[cfg(test)]
mod screenshots;

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowHandle, px, size};
use lightmime_core::{BridgeConfig, Engine, SyncSettings};

use crate::view::MainView;

pub use icons::APP_ID;

/// Persists what the window changes.
pub trait Store: Send + Sync + 'static {
    fn save_bridge(&self, bridge: &BridgeConfig) -> Result<()>;
    fn save_sync(&self, sync: &SyncSettings) -> Result<()>;
}

pub struct Options {
    pub engine: Engine,
    pub bridge: BridgeConfig,
    pub store: Arc<dyn Store>,
    /// The runtime the engine runs on; network calls from the window go there.
    pub runtime: tokio::runtime::Handle,
    /// How this machine introduces itself to the bridge when pairing.
    pub device_name: String,
    /// Start in the tray without opening the window. Ignored when there is no
    /// tray to reopen it from.
    pub hidden: bool,
    /// Quits the app when it completes, as the tray's Quit does; for SIGTERM.
    pub quit_signal: Pin<Box<dyn Future<Output = ()> + Send>>,
}

/// State shared by the window and the app-level handlers.
#[derive(Clone)]
pub(crate) struct Shared {
    pub engine: Engine,
    pub store: Arc<dyn Store>,
    pub runtime: tokio::runtime::Handle,
    pub device_name: String,
    /// The current bridge, kept here so a reopened window starts from it.
    pub bridge: Arc<Mutex<BridgeConfig>>,
}

struct Ui {
    shared: Shared,
    window: Option<WindowHandle<MainView>>,
    has_tray: bool,
}

impl gpui::Global for Ui {}

/// Run the window and tray icon until the user quits. Blocks the calling
/// thread; stopping the engine afterwards is up to the caller.
pub fn run(options: Options) {
    let Options {
        engine,
        bridge,
        store,
        runtime,
        device_name,
        hidden,
        quit_signal,
    } = options;
    let shared = Shared {
        engine: engine.clone(),
        store,
        runtime: runtime.clone(),
        device_name,
        bridge: Arc::new(Mutex::new(bridge)),
    };

    let (actions_tx, mut actions) = tokio::sync::mpsc::unbounded_channel();
    let quit = actions_tx.clone();
    runtime.spawn(async move {
        quit_signal.await;
        let _ = quit.send(tray::Action::Quit);
    });
    let tray = match runtime.block_on(tray::spawn(engine, actions_tx)) {
        Ok(tray) => Some(tray),
        Err(e) => {
            tracing::warn!("no tray icon, closing the window quits: {e:#}");
            None
        }
    };
    let has_tray = tray.is_some();

    gpui_platform::application().run(move |cx: &mut App| {
        let font = theme::UiFont::pick(cx);
        cx.set_global(font);
        cx.set_global(Ui {
            shared,
            window: None,
            has_tray,
        });
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() && !cx.global::<Ui>().has_tray {
                cx.quit();
            }
        })
        .detach();

        if !hidden || !has_tray {
            show_window(cx);
        }

        cx.spawn(async move |cx| {
            while let Some(action) = actions.recv().await {
                cx.update(|cx| match action {
                    tray::Action::Show => show_window(cx),
                    tray::Action::Quit => cx.quit(),
                });
            }
        })
        .detach();
    });

    if let Some(tray) = tray {
        runtime.block_on(tray.shutdown());
    }
}

/// Bring the window to the front, opening it if it was closed.
fn show_window(cx: &mut App) {
    let existing = cx.global::<Ui>().window;
    if let Some(handle) = existing
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }

    let shared = cx.global::<Ui>().shared.clone();
    let options = gpui::WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(440.), px(760.)),
            cx,
        ))),
        window_min_size: Some(size(px(360.), px(420.))),
        titlebar: Some(TitlebarOptions {
            title: Some("lightmime".into()),
            ..Default::default()
        }),
        ..icons::window_options()
    };
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| MainView::new(shared, window, cx))
    }) {
        Ok(handle) => cx.global_mut::<Ui>().window = Some(handle),
        Err(e) => tracing::error!("could not open the window: {e:#}"),
    }
}
