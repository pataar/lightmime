//! The main window: sync status and controls, pairing, area choice and the
//! live settings.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use gpui::{
    AnyElement, Bounds, Context, Entity, FontWeight, Hsla, Pixels, Rgba, SharedString, Task,
    Window, div, linear_color_stop, linear_gradient, prelude::*, px,
};
use lightmime_core::color::Zone;
use lightmime_core::config::{MAX_SATURATION, MAX_SMOOTHING};
use lightmime_core::hue::{self, Bridge, ChannelColor, DiscoveredBridge};
use lightmime_core::{BridgeConfig, EngineState, Status, SyncSettings};

use crate::Shared;
use crate::autostart;
use crate::decorations::{self, Drag};
use crate::theme::{Theme, UiFont};
use crate::widgets::{ButtonKind, TextInput, button, slider, switch};

/// Slider changes are written to disk once dragging pauses this long.
const SAVE_DELAY: Duration = Duration::from_millis(600);

struct SliderSpec {
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    max: f32,
    get: fn(&SyncSettings) -> f32,
    set: fn(&mut SyncSettings, f32),
}

const SLIDERS: [SliderSpec; 3] = [
    SliderSpec {
        key: "brightness",
        label: "Brightness",
        hint: "Scales the light output.",
        max: 1.0,
        get: |s| s.brightness,
        set: |s, v| s.brightness = v,
    },
    SliderSpec {
        key: "smoothing",
        label: "Smoothing",
        hint: "Higher is calmer, lower follows the screen faster.",
        max: MAX_SMOOTHING,
        get: |s| s.smoothing,
        set: |s, v| s.smoothing = v,
    },
    SliderSpec {
        key: "saturation",
        label: "Saturation",
        hint: "100% keeps the screen's colours.",
        max: MAX_SATURATION,
        get: |s| s.saturation,
        set: |s, v| s.saturation = v,
    },
];

enum Pairing {
    Idle,
    Discovering,
    Waiting(tokio::task::AbortHandle),
}

enum Areas {
    Unloaded,
    Loading,
    Loaded(Vec<hue::EntertainmentConfiguration>),
    Failed(String),
}

pub struct MainView {
    shared: Shared,
    status: Status,
    settings: SyncSettings,
    bridge: BridgeConfig,
    host: Entity<TextInput>,
    pairing: Pairing,
    discovered: Vec<DiscoveredBridge>,
    areas: Areas,
    autostart: autostart::LoginItem,
    /// The last failed action, shown until the next one.
    notice: Option<String>,
    slider_bounds: [Rc<Cell<Bounds<Pixels>>>; 3],
    save_settings: Option<Task<()>>,
    drag: Drag,
    /// Left and right of the screen while syncing, painted behind the cards.
    screen: Option<[Hsla; 2]>,
    _watchers: Vec<Task<()>>,
}

impl MainView {
    pub fn new(shared: Shared, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let bridge = shared.bridge.lock().map(|b| b.clone()).unwrap_or_default();
        let host_text = bridge.host.clone();
        let host =
            cx.new(|cx| TextInput::new(host_text, "Bridge IP address, e.g. 192.168.1.20", cx));
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();

        let status_watcher = watch(shared.engine.subscribe(), cx, |this, s| this.status = s);
        let settings_watcher = watch(shared.engine.subscribe_settings(), cx, |this, s| {
            this.settings = s
        });
        let colors_watcher = watch(shared.engine.subscribe_colors(), cx, |this, colors| {
            this.screen = screen_edges(&colors)
        });

        let mut view = Self {
            status: shared.engine.status(),
            settings: shared.engine.settings(),
            bridge,
            host,
            pairing: Pairing::Idle,
            discovered: Vec::new(),
            areas: Areas::Unloaded,
            autostart: autostart::current(),
            notice: None,
            drag: Drag::default(),
            screen: None,
            slider_bounds: Default::default(),
            save_settings: None,
            _watchers: vec![status_watcher, settings_watcher, colors_watcher],
            shared,
        };
        if view.bridge.is_paired() {
            view.load_areas(cx);
        }
        view
    }

    /// Run `job` on the engine's tokio runtime (reqwest needs it) and hand the
    /// result back to the view.
    fn run_job<T: Send + 'static>(
        &self,
        job: impl Future<Output = Result<T>> + Send + 'static,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<T>, &mut Context<Self>) + 'static,
    ) -> tokio::task::AbortHandle {
        let handle = self.shared.runtime.spawn(job);
        let abort = handle.abort_handle();
        cx.spawn(async move |this, cx| {
            let result = match handle.await {
                Ok(result) => result,
                Err(e) if e.is_cancelled() => return,
                Err(e) => Err(e.into()),
            };
            let _ = this.update(cx, |this, cx| {
                done(this, result, cx);
                cx.notify();
            });
        })
        .detach();
        abort
    }

    fn set_bridge(&mut self, bridge: BridgeConfig) {
        if let Err(e) = self.shared.store.save_bridge(&bridge) {
            self.notice = Some(format!("Could not save the settings: {e:#}"));
        }
        if let Ok(mut shared) = self.shared.bridge.lock() {
            *shared = bridge.clone();
        }
        self.shared.engine.set_bridge(bridge.clone());
        self.bridge = bridge;
    }

    fn discover(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        self.pairing = Pairing::Discovering;
        self.run_job(hue::discover(), cx, |this, result, cx| {
            this.pairing = Pairing::Idle;
            match result {
                Ok(found) if found.is_empty() => {
                    this.notice = Some("No bridge found. Enter its IP address instead; the Hue app shows it under Settings → Bridges.".into());
                }
                Ok(found) => {
                    let first = found[0].host.clone();
                    this.host.update(cx, |input, cx| input.set_text(first, cx));
                    this.discovered = found;
                }
                Err(e) => this.notice = Some(format!("Could not search for bridges: {e:#}")),
            }
        });
    }

    fn pair(&mut self, cx: &mut Context<Self>) {
        let host = self.host.read(cx).text().trim().to_owned();
        if host.is_empty() {
            self.notice = Some("Enter the bridge's IP address first.".into());
            return;
        }
        self.notice = None;
        let device = self.shared.device_name.clone();
        let job = async move {
            let credentials = hue::pair_waiting(&host, &device).await?;
            Ok((host, credentials))
        };
        let abort = self.run_job(job, cx, |this, result, cx| {
            this.pairing = Pairing::Idle;
            match result {
                Ok((host, credentials)) => {
                    this.discovered.clear();
                    this.set_bridge(BridgeConfig {
                        host,
                        app_key: credentials.app_key,
                        client_key: credentials.client_key,
                        entertainment_config_id: String::new(),
                    });
                    this.load_areas(cx);
                }
                Err(e) => this.notice = Some(format!("Pairing failed: {e:#}")),
            }
        });
        self.pairing = Pairing::Waiting(abort);
    }

    fn cancel_pairing(&mut self) {
        if let Pairing::Waiting(abort) = std::mem::replace(&mut self.pairing, Pairing::Idle) {
            abort.abort();
        }
    }

    fn forget_bridge(&mut self, cx: &mut Context<Self>) {
        self.shared.engine.stop();
        let host = self.bridge.host.clone();
        self.set_bridge(BridgeConfig::default());
        self.host.update(cx, |input, cx| input.set_text(host, cx));
        self.areas = Areas::Unloaded;
    }

    fn load_areas(&mut self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        self.areas = Areas::Loading;
        let job = async move {
            Bridge::new(&bridge.host, &bridge.app_key)?
                .entertainment_configurations()
                .await
        };
        self.run_job(job, cx, |this, result, _| {
            this.areas = match result {
                Ok(areas) => Areas::Loaded(areas),
                Err(e) => Areas::Failed(format!("{e:#}")),
            };
        });
    }

    fn choose_area(&mut self, id: String) {
        let mut bridge = self.bridge.clone();
        bridge.entertainment_config_id = id;
        self.set_bridge(bridge);
    }

    fn set_slider(&mut self, index: usize, fraction: f32, cx: &mut Context<Self>) {
        let spec = &SLIDERS[index];
        let value = (fraction * spec.max * 100.0).round() / 100.0;
        self.shared.engine.update_settings(|s| (spec.set)(s, value));
        let store = self.shared.store.clone();
        let engine = self.shared.engine.clone();
        // Replacing the task cancels the previous wait, so a drag saves once.
        self.save_settings = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            if let Err(e) = store.save_sync(&engine.settings()) {
                tracing::warn!("could not save settings: {e:#}");
            }
        }));
    }

    fn set_autostart(&mut self, item: autostart::LoginItem, cx: &mut Context<Self>) {
        self.run_job(
            autostart::set(item),
            cx,
            move |this, result, _| match result {
                Ok(()) => this.autostart = item,
                Err(e) => this.notice = Some(format!("Could not change the login item: {e:#}")),
            },
        );
    }

    fn render_status(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (headline, colour) = match &self.status.state {
            EngineState::Idle => ("Not syncing".to_owned(), theme.muted),
            EngineState::Starting => ("Starting…".to_owned(), theme.accent),
            EngineState::Streaming => ("Syncing".to_owned(), theme.success),
            EngineState::Error(_) => ("Stopped".to_owned(), theme.danger),
        };
        let detail = match &self.status.state {
            EngineState::Streaming => {
                let mut detail = format!(
                    "{} · {} lights · {:.0} fps",
                    self.status.area, self.status.channels, self.status.fps
                );
                if let Some(latency) = self.status.latency {
                    detail += &format!(" · ~{} ms", latency.as_millis().max(1));
                }
                detail
            }
            EngineState::Starting => "Waiting for the screen and the bridge".into(),
            _ if !self.bridge.is_paired() => "Pair with a bridge to begin".into(),
            _ if self.bridge.entertainment_config_id.is_empty() => {
                "Choose an entertainment area below".into()
            }
            _ => "Ready".into(),
        };
        let can_start = self.bridge.is_paired() && !self.bridge.entertainment_config_id.is_empty();
        let syncing = self.status.state.is_active();

        card(theme)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().size(px(12.)).rounded_full().bg(colour))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(headline),
                            )
                            .child(div().text_sm().text_color(theme.muted).child(detail)),
                    )
                    .child(
                        button(
                            "sync",
                            if syncing { "Stop" } else { "Start" },
                            if syncing {
                                ButtonKind::Plain
                            } else {
                                ButtonKind::Primary
                            },
                            syncing || can_start,
                            theme,
                            cx.listener(move |this, _, _, _| {
                                if syncing {
                                    this.shared.engine.stop();
                                } else {
                                    this.shared.engine.start();
                                }
                            }),
                        )
                        .min_w(px(72.)),
                    ),
            )
            .when_some(self.status.state.error(), |this, message| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(message.to_owned()),
                )
            })
    }

    fn render_bridge(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.bridge.is_paired() {
            return section(theme, "Bridge")
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().text_sm().child(self.bridge.host.clone()))
                        .child(button(
                            "forget",
                            "Forget",
                            ButtonKind::Plain,
                            true,
                            theme,
                            cx.listener(|this, _, _, cx| this.forget_bridge(cx)),
                        )),
                )
                .into_any_element();
        }

        let waiting = matches!(self.pairing, Pairing::Waiting(_));
        let busy = !matches!(self.pairing, Pairing::Idle);
        section(theme, "Bridge")
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted)
                    .child("Find the bridge or enter its address, then press Pair."),
            )
            .child(div().flex().gap_2().child(self.host.clone()).child(button(
                "discover",
                if matches!(self.pairing, Pairing::Discovering) {
                    "Searching…"
                } else {
                    "Find"
                },
                ButtonKind::Plain,
                !busy,
                theme,
                cx.listener(|this, _, _, cx| this.discover(cx)),
            )))
            .when(self.discovered.len() > 1, |this| {
                this.child(div().flex().flex_wrap().gap_2().children(
                    self.discovered.iter().enumerate().map(|(i, found)| {
                        let host = found.host.clone();
                        button(
                            SharedString::from(format!("found-{i}")),
                            found.host.clone(),
                            ButtonKind::Plain,
                            !busy,
                            theme,
                            cx.listener(move |this, _, _, cx| {
                                let host = host.clone();
                                this.host.update(cx, |input, cx| input.set_text(host, cx));
                            }),
                        )
                    }),
                ))
            })
            .child(if waiting {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(theme.accent)
                            .child("Press the link button on the bridge…"),
                    )
                    .child(button(
                        "cancel-pair",
                        "Cancel",
                        ButtonKind::Plain,
                        true,
                        theme,
                        cx.listener(|this, _, _, cx| {
                            this.cancel_pairing();
                            cx.notify();
                        }),
                    ))
            } else {
                div().flex().justify_end().child(button(
                    "pair",
                    "Pair",
                    ButtonKind::Primary,
                    !busy,
                    theme,
                    cx.listener(|this, _, _, cx| this.pair(cx)),
                ))
            })
            .into_any_element()
    }

    fn render_areas(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .items_center()
            .child(div().flex_1().child(heading(theme, "Entertainment area")))
            .child(button(
                "refresh-areas",
                "Refresh",
                ButtonKind::Plain,
                !matches!(self.areas, Areas::Loading),
                theme,
                cx.listener(|this, _, _, cx| this.load_areas(cx)),
            ));
        let body = match &self.areas {
            Areas::Unloaded | Areas::Loading => div()
                .text_sm()
                .text_color(theme.muted)
                .child("Loading areas…")
                .into_any_element(),
            Areas::Failed(e) => div()
                .text_sm()
                .text_color(theme.danger)
                .child(format!("Could not list the areas: {e}"))
                .into_any_element(),
            Areas::Loaded(areas) if areas.is_empty() => div()
                .text_sm()
                .text_color(theme.muted)
                .child("The bridge has no entertainment areas. Create one in the Hue app under Settings → Entertainment areas.")
                .into_any_element(),
            Areas::Loaded(areas) => div()
                .flex()
                .flex_col()
                .gap_1()
                .children(areas.iter().map(|area| {
                    let selected = area.id == self.bridge.entertainment_config_id;
                    let id = area.id.clone();
                    let mut detail = format!("{} lights", area.channels.len());
                    if area.status == "active" && !(selected && self.status.state.is_active()) {
                        detail.push_str(" · in use by another app");
                    }
                    div()
                        .id(SharedString::from(format!("area-{}", area.id)))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_1()
                        .border_color(if selected { theme.accent } else { theme.border })
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.bg))
                        .on_click(cx.listener(move |this, _, _, _| this.choose_area(id.clone())))
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(area.metadata.name.clone()))
                        .child(div().text_xs().text_color(theme.muted).child(detail))
                }))
                .into_any_element(),
        };
        card(theme).child(header).child(body)
    }

    fn render_sliders(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        section(theme, "Tuning").children(SLIDERS.iter().enumerate().map(|(index, spec)| {
            let value = (spec.get)(&self.settings);
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .text_sm()
                        .child(div().flex_1().child(spec.label))
                        .child(
                            div()
                                .text_color(theme.muted)
                                .child(format!("{:.0}%", value * 100.0)),
                        ),
                )
                .child(slider(
                    spec.key,
                    value / spec.max,
                    self.slider_bounds[index].clone(),
                    theme,
                    cx.listener(move |this, fraction: &f32, _, cx| {
                        this.set_slider(index, *fraction, cx)
                    }),
                ))
                .child(div().text_xs().text_color(theme.muted).child(spec.hint))
        }))
    }

    fn render_general(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let item = self.autostart;
        let mut card = card(theme).child(setting_row(
            "Start at login",
            "Opens in the tray when you log in.",
            switch(
                "autostart",
                item.enabled,
                theme,
                cx.listener(move |this, _, _, cx| {
                    let enabled = !item.enabled;
                    let next = autostart::LoginItem {
                        enabled,
                        start_syncing: enabled && item.start_syncing,
                    };
                    this.set_autostart(next, cx)
                }),
            ),
            theme,
        ));
        if item.enabled {
            card = card.child(setting_row(
                "Start syncing at login",
                "Syncs to the chosen area right away instead of waiting for you.",
                switch(
                    "autostart-sync",
                    item.start_syncing,
                    theme,
                    cx.listener(move |this, _, _, cx| {
                        let next = autostart::LoginItem {
                            start_syncing: !item.start_syncing,
                            ..item
                        };
                        this.set_autostart(next, cx)
                    }),
                ),
                theme,
            ));
        }
        card
    }
}

fn setting_row(
    label: &'static str,
    hint: &'static str,
    control: impl IntoElement,
    theme: Theme,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_sm().child(label))
                .child(div().text_xs().text_color(theme.muted).child(hint)),
        )
        .child(control)
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window, cx);
        let paired = self.bridge.is_paired();
        let content = div()
            .id("main")
            .size_full()
            .overflow_y_scroll()
            .map(|this| match self.screen {
                Some([left, right]) => this.bg(linear_gradient(
                    90.,
                    linear_color_stop(left, 0.),
                    linear_color_stop(right, 1.),
                )),
                None => this.bg(theme.bg),
            })
            .text_color(theme.text)
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.render_status(theme, cx))
            .when_some(self.notice.clone(), |this, notice| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_1()
                        .border_color(theme.danger)
                        .text_sm()
                        .text_color(theme.danger)
                        .child(notice),
                )
            })
            .child(self.render_bridge(theme, cx))
            .when(paired, |this| this.child(self.render_areas(theme, cx)))
            .child(self.render_sliders(theme, cx))
            .child(self.render_general(theme, cx))
            .child(render_about(theme))
            .into_any_element();
        div()
            .size_full()
            .bg(theme.bg)
            .font_family(cx.global::<UiFont>().0.clone())
            .text_color(theme.text)
            .child(decorations::frame(
                window,
                theme,
                "lightmime",
                &self.drag,
                content,
            ))
    }
}

/// Average colour of the lights on the left and right half of the screen, half transparent
/// so the theme's background tones it down.
fn screen_edges(colors: &[(Zone, ChannelColor)]) -> Option<[Hsla; 2]> {
    let side = |left: bool| {
        let lights: Vec<_> = colors
            .iter()
            .filter(|(zone, _)| (zone.x < 0.5) == left)
            .collect();
        if lights.is_empty() {
            return None;
        }
        let total = lights.len() as f32 * f32::from(u16::MAX);
        let channel =
            |i: usize| lights.iter().map(|(_, c)| f32::from(c.rgb[i])).sum::<f32>() / total;
        Some(Hsla::from(Rgba {
            r: channel(0),
            g: channel(1),
            b: channel(2),
            a: 0.5,
        }))
    };
    match (side(true), side(false)) {
        (Some(left), Some(right)) => Some([left, right]),
        (Some(only), None) | (None, Some(only)) => Some([only, only]),
        (None, None) => None,
    }
}

/// Copy every value from `rx` into the view until either side goes away.
fn watch<T: Clone + 'static>(
    mut rx: tokio::sync::watch::Receiver<T>,
    cx: &mut Context<MainView>,
    apply: fn(&mut MainView, T),
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while rx.changed().await.is_ok() {
            let value = rx.borrow_and_update().clone();
            let updated = this.update(cx, |this, cx| {
                apply(this, value);
                cx.notify();
            });
            if updated.is_err() {
                break;
            }
        }
    })
}

fn card(theme: Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
}

fn heading(theme: Theme, text: &'static str) -> gpui::Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted)
        .child(text.to_uppercase())
}

/// Version, author and links to the docs and the source, below the settings.
fn render_about(theme: Theme) -> impl IntoElement {
    div()
        .flex()
        .justify_center()
        .gap_1()
        .text_xs()
        .text_color(theme.muted)
        .child(concat!(
            "lightmime ",
            env!("CARGO_PKG_VERSION"),
            " · ",
            env!("CARGO_PKG_AUTHORS"),
            " ·"
        ))
        .child(link(theme, "Docs", env!("CARGO_PKG_HOMEPAGE")))
        .child("·")
        .child(link(theme, "GitHub", env!("CARGO_PKG_REPOSITORY")))
}

fn link(theme: Theme, label: &'static str, url: &'static str) -> impl IntoElement {
    div()
        .id(label)
        .text_color(theme.accent)
        .cursor_pointer()
        .hover(|style| style.underline())
        .on_click(move |_, _, cx| cx.open_url(url))
        .child(label)
}

fn section(theme: Theme, title: &'static str) -> gpui::Div {
    card(theme).child(heading(theme, title))
}

#[cfg(test)]
impl MainView {
    /// Fill the window with a paired bridge, two areas and a running sync,
    /// without touching the network.
    pub(crate) fn show_demo(&mut self) {
        self.bridge = BridgeConfig {
            host: "192.168.1.20".into(),
            app_key: "demo".into(),
            client_key: "demo".into(),
            entertainment_config_id: "living-room".into(),
        };
        let area = |id: &str, name: &str, lights: u8| {
            let channels: Vec<_> = (0..lights)
                .map(|i| serde_json::json!({ "channel_id": i, "position": { "x": 0, "y": 0, "z": 0 } }))
                .collect();
            serde_json::from_value(serde_json::json!({
                "id": id,
                "metadata": { "name": name },
                "status": "inactive",
                "channels": channels,
            }))
            .expect("demo area")
        };
        self.areas = Areas::Loaded(vec![
            area("living-room", "Living room", 6),
            area("desk", "Desk", 2),
        ]);
        self.status = Status {
            state: EngineState::Streaming,
            fps: 30.0,
            latency: Some(std::time::Duration::from_millis(3)),
            area: "Living room".into(),
            channels: 6,
        };
        self.screen = screen_edges(&[
            tests::light(0.0, [u16::MAX, 20_000, 0]),
            tests::light(1.0, [0, 30_000, u16::MAX]),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn light(x: f32, rgb: [u16; 3]) -> (Zone, ChannelColor) {
        (
            Zone {
                channel_id: 0,
                x,
                y: 0.5,
            },
            ChannelColor { channel_id: 0, rgb },
        )
    }

    #[test]
    fn screen_edges_average_each_half() {
        assert!(screen_edges(&[]).is_none());
        let [left, right] = screen_edges(&[
            light(0.0, [u16::MAX, 0, 0]),
            light(0.2, [0, 0, 0]),
            light(0.9, [0, 0, u16::MAX]),
        ])
        .unwrap();
        assert_eq!(
            Rgba::from(left),
            Rgba {
                r: 0.5,
                g: 0.,
                b: 0.,
                a: 0.5
            }
        );
        assert_eq!(
            Rgba::from(right),
            Rgba {
                r: 0.,
                g: 0.,
                b: 1.,
                a: 0.5
            }
        );
    }
}
