//! Colours for the window, following the desktop's light or dark preference.
//! Taken from the brand palette in `assets/brand/README.md`.

use gpui::{App, Global, Hsla, SharedString, Window, WindowAppearance, rgb};

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Hsla,
    pub surface: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
}

/// Overrides the desktop's light or dark preference (used for screenshots).
pub struct ForceAppearance(pub WindowAppearance);

impl Global for ForceAppearance {}

impl Theme {
    pub fn of(window: &Window, cx: &App) -> Self {
        let appearance = cx
            .try_global::<ForceAppearance>()
            .map_or_else(|| window.appearance(), |force| force.0);
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        }
    }

    fn light() -> Self {
        Self {
            bg: rgb(0xf4f2ff).into(),
            surface: rgb(0xffffff).into(),
            border: rgb(0xdedbf2).into(),
            text: rgb(0x14152a).into(),
            muted: rgb(0x5d5f7d).into(),
            accent: rgb(0x7c5cff).into(),
            accent_text: rgb(0xffffff).into(),
            danger: rgb(0xd0365f).into(),
            success: rgb(0x0f9fbf).into(),
        }
    }

    fn dark() -> Self {
        Self {
            bg: rgb(0x0c0d18).into(),
            surface: rgb(0x1c1d33).into(),
            border: rgb(0x2c2d4a).into(),
            text: rgb(0xf4f2ff).into(),
            muted: rgb(0xa3a4c4).into(),
            accent: rgb(0x8f74ff).into(),
            accent_text: rgb(0xffffff).into(),
            danger: rgb(0xf0479b).into(),
            success: rgb(0x22c7e8).into(),
        }
    }
}

/// The font the window uses. gpui maps its default font to IBM Plex Sans on
/// Linux, which few systems have, so pick an installed UI font instead.
pub struct UiFont(pub SharedString);

impl Global for UiFont {}

impl UiFont {
    pub fn pick(cx: &App) -> Self {
        const PREFERRED: [&str; 9] = [
            "Adwaita Sans",
            "Cantarell",
            "Noto Sans",
            "Inter",
            "Ubuntu",
            "IBM Plex Sans",
            "DejaVu Sans",
            "Liberation Sans",
            "Roboto",
        ];
        let installed = cx.text_system().all_font_names();
        let family = PREFERRED
            .iter()
            .find(|name| installed.iter().any(|font| font == *name))
            .map(|name| SharedString::from(*name))
            .unwrap_or_else(|| ".SystemUIFont".into());
        Self(family)
    }
}
