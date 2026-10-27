//! The main window: sync status and controls, pairing, area choice and the
//! live settings.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use gpui::{
    AnyElement, Bounds, Context, Entity, FontWeight, Pixels, SharedString, Task, Window, div,
    prelude::*, px,
};
use lightmime_core::hue::{self, Bridge, DiscoveredBridge, PairError};
use lightmime_core::{BridgeConfig, EngineState, Status, SyncSettings};

use crate::Shared;
use crate::autostart;
use crate::theme::{Theme, UiFont};
use crate::widgets::{ButtonKind, TextInput, button, slider, switch};

/// How long to wait for the link button before giving up.
const PAIR_TIMEOUT: Duration = Duration::from_secs(60);
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
        max: 0.95,
        get: |s| s.smoothing,
        set: |s, v| s.smoothing = v,
    },
    SliderSpec {
        key: "saturation",
        label: "Saturation",
        hint: "100% keeps the screen's colours.",
        max: 3.0,
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
    autostart: bool,
    /// The last failed action, shown until the next one.
    notice: Option<String>,
    slider_bounds: [Rc<Cell<Bounds<Pixels>>>; 3],
    save_settings: Option<Task<()>>,
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

        let mut status_rx = shared.engine.subscribe();
        let status_watcher = cx.spawn(async move |this, cx| {
            while status_rx.changed().await.is_ok() {
                let status = status_rx.borrow_and_update().clone();
                if this
                    .update(cx, |this, cx| {
                        this.status = status;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut settings_rx = shared.engine.subscribe_settings();
        let settings_watcher = cx.spawn(async move |this, cx| {
            while settings_rx.changed().await.is_ok() {
                let settings = settings_rx.borrow_and_update().clone();
                if this
                    .update(cx, |this, cx| {
                        this.settings = settings;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let mut view = Self {
            status: shared.engine.status(),
            settings: shared.engine.settings(),
            bridge,
            host,
            pairing: Pairing::Idle,
            discovered: Vec::new(),
            areas: Areas::Unloaded,
            autostart: autostart::is_enabled(),
            notice: None,
            slider_bounds: Default::default(),
            save_settings: None,
            _watchers: vec![status_watcher, settings_watcher],
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
            let deadline = Instant::now() + PAIR_TIMEOUT;
            loop {
                match hue::pair(&host, &device).await {
                    Ok(credentials) => return Ok((host, credentials)),
                    Err(PairError::LinkButtonNotPressed) if Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    Err(PairError::LinkButtonNotPressed) => {
                        bail!("the link button was not pressed within a minute")
                    }
                    Err(PairError::Other(e)) => return Err(e),
                }
            }
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

    fn toggle_autostart(&mut self, cx: &mut Context<Self>) {
        let enable = !self.autostart;
        self.run_job(
            autostart::set(enable),
            cx,
            move |this, result, _| match result {
                Ok(()) => this.autostart = enable,
                Err(e) => this.notice = Some(format!("Could not change the login item: {e:#}")),
            },
        );
    }

    fn syncing(&self) -> bool {
        matches!(
            self.status.state,
            EngineState::Starting | EngineState::Streaming
        )
    }

    fn render_status(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (headline, colour) = match &self.status.state {
            EngineState::Idle => ("Not syncing".to_owned(), theme.muted),
            EngineState::Starting => ("Starting…".to_owned(), theme.accent),
            EngineState::Streaming => ("Syncing".to_owned(), theme.success),
            EngineState::Error(_) => ("Stopped".to_owned(), theme.danger),
        };
        let detail = match &self.status.state {
            EngineState::Streaming => format!(
                "{} · {} lights · {:.0} fps",
                self.status.area, self.status.channels, self.status.fps
            ),
            EngineState::Starting => "Waiting for the screen and the bridge".into(),
            _ if !self.bridge.is_paired() => "Pair with a bridge to begin".into(),
            _ if self.bridge.entertainment_config_id.is_empty() => {
                "Choose an entertainment area below".into()
            }
            _ => "Ready".into(),
        };
        let can_start = self.bridge.is_paired() && !self.bridge.entertainment_config_id.is_empty();
        let syncing = self.syncing();

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
            .when_some(
                match &self.status.state {
                    EngineState::Error(message) => Some(message.clone()),
                    _ => None,
                },
                |this, message| this.child(div().text_sm().text_color(theme.danger).child(message)),
            )
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
                    if area.status == "active" && !(selected && self.syncing()) {
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
        card(theme).child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(div().text_sm().child("Start at login"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted)
                                .child("Opens in the tray; syncing waits for you."),
                        ),
                )
                .child(switch(
                    "autostart",
                    self.autostart,
                    theme,
                    cx.listener(|this, _, _, cx| this.toggle_autostart(cx)),
                )),
        )
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window, cx);
        let paired = self.bridge.is_paired();
        div()
            .id("main")
            .font_family(cx.global::<UiFont>().0.clone())
            .size_full()
            .overflow_y_scroll()
            .bg(theme.bg)
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
    }
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
            area: "Living room".into(),
            channels: 6,
        };
    }
}
