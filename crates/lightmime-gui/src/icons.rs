//! App identity and icons shared by the window and the tray.
//!
//! Desktops resolve the window icon through the app id: on Wayland the
//! compositor looks up `<APP_ID>.desktop` and uses its `Icon=`, on X11 the
//! app id becomes `WM_CLASS`, matched against `StartupWMClass=`.

use gpui::WindowOptions;

/// Wayland app id / X11 class; must match the `.desktop` file name in `contrib/`.
pub const APP_ID: &str = "io.github.pataar.lightmime";

/// Full-colour app icon in the `hicolor` theme.
pub const APP_ICON_NAME: &str = "lightmime";

/// Monochrome tray icon; the `-symbolic` suffix lets themes recolour it to
/// match the panel.
pub const TRAY_ICON_NAME: &str = "lightmime-symbolic";

/// Window options carrying the app id, so the desktop shows the lightmime icon.
pub fn window_options() -> WindowOptions {
    WindowOptions {
        app_id: Some(APP_ID.into()),
        ..Default::default()
    }
}

/// Extra icon theme directory for the tray (`ksni::Tray::icon_theme_path`).
///
/// Debug builds point at the repository's `assets/icons`, so `cargo run`
/// shows the icon without installing it. Release builds rely on the icons
/// being installed into the system or user `hicolor` theme.
pub fn tray_icon_theme_path() -> String {
    if cfg!(debug_assertions) {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons").into()
    } else {
        String::new()
    }
}
