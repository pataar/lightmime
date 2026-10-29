//! The sync engine: owns the capture and the bridge session, and runs the
//! streaming loop. Frontends talk to it through an [`Engine`] handle.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::sync::{mpsc, watch};

use crate::capture::ScreenCapture;
use crate::color::{ColorMapper, Zone};
use crate::config::{BridgeConfig, Config, SyncSettings};
use crate::hue::{Bridge, ChannelColor, EntertainmentStream};

#[derive(Debug, Clone, Default, PartialEq)]
pub enum EngineState {
    #[default]
    Idle,
    Starting,
    Streaming,
    Error(String),
}

impl EngineState {
    pub fn label(&self) -> &str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Streaming => "streaming",
            Self::Error(_) => "error",
        }
    }

    /// Starting or streaming: what frontends show as "sync on".
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Starting | Self::Streaming)
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Error(message) => Some(message),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub state: EngineState,
    /// Frames actually sent per second.
    pub fps: f32,
    /// Name of the entertainment area being streamed to.
    pub area: String,
    pub channels: usize,
}

enum Command {
    Start,
    Stop,
    SetBridge(BridgeConfig),
    StreamFailed { session_id: u64, reason: String },
}

/// Handle to a running engine. Cheap to clone; the engine stops when the last
/// handle is dropped.
#[derive(Clone)]
pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
    status: watch::Receiver<Status>,
    settings: Arc<watch::Sender<SyncSettings>>,
}

impl Engine {
    /// Spawn the engine on the current tokio runtime. The portal restore token
    /// is read from and written to `restore_token_path`, so screen selection
    /// is only asked once.
    pub fn spawn(config: Config, restore_token_path: Option<PathBuf>) -> Self {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (status_tx, status) = watch::channel(Status::default());
        let settings = Arc::new(watch::Sender::new(config.sync.clone().sanitized()));
        let actor = Actor {
            config,
            restore_token_path,
            settings: settings.subscribe(),
            status: Arc::new(status_tx),
            commands: commands.clone(),
            session: None,
            sessions_opened: 0,
        };
        tokio::spawn(actor.run(command_rx));
        Self {
            commands,
            status,
            settings,
        }
    }

    pub fn start(&self) {
        let _ = self.commands.send(Command::Start);
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    /// Watch status changes (state, fps, area).
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    /// Use another bridge or entertainment area, for example after pairing
    /// from a frontend. A running session restarts with it.
    pub fn set_bridge(&self, bridge: BridgeConfig) {
        let _ = self.commands.send(Command::SetBridge(bridge));
    }

    pub fn settings(&self) -> SyncSettings {
        self.settings.borrow().clone()
    }

    /// Watch settings changes, including those made by another frontend.
    pub fn subscribe_settings(&self) -> watch::Receiver<SyncSettings> {
        self.settings.subscribe()
    }

    /// Change settings; takes effect on the next frame.
    pub fn update_settings(&self, update: impl FnOnce(&mut SyncSettings)) {
        self.settings.send_if_modified(|settings| {
            let mut next = settings.clone();
            update(&mut next);
            let next = next.sanitized();
            let changed = next != *settings;
            *settings = next;
            changed
        });
    }
}

struct Actor {
    config: Config,
    restore_token_path: Option<PathBuf>,
    settings: watch::Receiver<SyncSettings>,
    status: Arc<watch::Sender<Status>>,
    commands: mpsc::UnboundedSender<Command>,
    session: Option<Session>,
    sessions_opened: u64,
}

struct Session {
    id: u64,
    bridge: Bridge,
    area_id: String,
    capture: ScreenCapture,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Actor {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        let mut health = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Start) => self.start().await,
                    Some(Command::Stop) => {
                        self.stop_session().await;
                        self.set_state(EngineState::Idle);
                    }
                    Some(Command::SetBridge(bridge)) => {
                        let restart = self.session.is_some();
                        self.config.bridge = bridge;
                        if restart {
                            self.stop_session().await;
                            self.start().await;
                        }
                    }
                    // A thread can fail after its session was stopped or replaced; that must not end the current one.
                    Some(Command::StreamFailed { session_id, reason }) => {
                        if self.session.as_ref().is_some_and(|s| s.id == session_id) {
                            self.fail(reason).await;
                        }
                    }
                    None => break,
                },
                Ok(()) = self.settings.changed() => {
                    let fps = self.settings.borrow_and_update().fps;
                    if let Some(session) = &self.session {
                        session.capture.set_max_fps(fps);
                    }
                }
                _ = health.tick() => {
                    let error = self.session.as_ref().and_then(|s| s.capture.poll_error());
                    if let Some(error) = error {
                        self.fail(format!("screen capture failed: {error}")).await;
                    }
                }
            }
        }
        self.stop_session().await;
    }

    async fn start(&mut self) {
        if self.session.is_some() {
            return;
        }
        self.set_state(EngineState::Starting);
        match self.open_session().await {
            Ok(session) => self.session = Some(session),
            Err(e) => {
                tracing::error!("could not start syncing: {e:#}");
                self.set_state(EngineState::Error(format!("{e:#}")));
            }
        }
    }

    async fn fail(&mut self, reason: String) {
        tracing::error!("{reason}");
        self.stop_session().await;
        self.set_state(EngineState::Error(reason));
    }

    async fn open_session(&mut self) -> Result<Session> {
        let bridge_config = &self.config.bridge;
        if !bridge_config.is_paired() {
            bail!("not paired with a Hue bridge yet");
        }
        if bridge_config.entertainment_config_id.is_empty() {
            bail!("no entertainment area configured");
        }
        let bridge = Bridge::new(&bridge_config.host, &bridge_config.app_key)?;
        let area = bridge
            .entertainment_configuration(&bridge_config.entertainment_config_id)
            .await?;
        if area.channels.is_empty() {
            bail!("entertainment area '{}' has no lights", area.metadata.name);
        }

        let token = self
            .restore_token_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty());
        let fps = self.settings.borrow_and_update().fps;
        let (capture, new_token) =
            ScreenCapture::start(&self.config.capture, fps, token.as_deref()).await?;
        if let (Some(path), Some(token)) = (&self.restore_token_path, new_token) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(path, token) {
                tracing::warn!("could not store the screen-cast restore token: {e}");
            }
        }

        // Read before starting the area, while the lights still show their own state.
        let room = bridge.channel_colors(&area).await.unwrap_or_else(|e| {
            tracing::warn!("could not read the lights' state, fading from black: {e:#}");
            area.channels
                .iter()
                .map(|c| ChannelColor {
                    channel_id: c.channel_id,
                    rgb: [0; 3],
                })
                .collect()
        });

        bridge
            .set_streaming(&area.id, true)
            .await
            .context("starting the entertainment area")?;
        let identity = bridge.psk_identity().await;

        let zones: Vec<Zone> = area.channels.iter().map(Zone::from_channel).collect();
        self.status.send_modify(|s| {
            s.area = area.metadata.name.clone();
            s.channels = zones.len();
        });

        let stop = Arc::new(AtomicBool::new(false));
        let worker = StreamWorker {
            host: bridge_config.host.clone(),
            identity,
            client_key: bridge_config.client_key.clone(),
            area_id: area.id.clone(),
            frames: capture.frame_source(),
            mapper: ColorMapper::new(zones),
            room,
            settings: self.settings.clone(),
            status: self.status.clone(),
            stop: stop.clone(),
        };
        self.sessions_opened += 1;
        let session_id = self.sessions_opened;
        let commands = self.commands.clone();
        let thread = std::thread::Builder::new()
            .name("lightmime-stream".into())
            .spawn(move || {
                if let Err(e) = worker.run() {
                    let _ = commands.send(Command::StreamFailed {
                        session_id,
                        reason: format!("{e:#}"),
                    });
                }
            })
            .context("spawning the streaming thread")?;

        Ok(Session {
            id: session_id,
            bridge,
            area_id: area.id,
            capture,
            stop,
            thread,
        })
    }

    async fn stop_session(&mut self) {
        let Some(Session {
            id: _,
            bridge,
            area_id,
            capture,
            stop,
            thread,
        }) = self.session.take()
        else {
            return;
        };
        stop.store(true, Ordering::Relaxed);
        let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        if let Err(e) = bridge.set_streaming(&area_id, false).await {
            tracing::warn!("could not stop the entertainment area: {e:#}");
        }
        capture.stop().await;
    }

    fn set_state(&self, state: EngineState) {
        self.status.send_modify(|s| {
            if state != EngineState::Streaming {
                s.fps = 0.0;
            }
            s.state = state;
        });
    }
}

/// Runs on its own thread: DTLS is blocking and the loop is time-critical.
struct StreamWorker {
    host: String,
    identity: String,
    client_key: String,
    area_id: String,
    frames: crate::capture::FrameSource,
    mapper: ColorMapper,
    /// What the lights showed before streaming; sync fades in from it and back to it on stop.
    room: Vec<ChannelColor>,
    settings: watch::Receiver<SyncSettings>,
    status: Arc<watch::Sender<Status>>,
    stop: Arc<AtomicBool>,
}

impl StreamWorker {
    fn run(mut self) -> Result<()> {
        let mut stream = EntertainmentStream::connect(
            &self.host,
            &self.identity,
            &self.client_key,
            &self.area_id,
        )?;
        tracing::info!("streaming to the bridge");
        self.status
            .send_modify(|s| s.state = EngineState::Streaming);

        let mut sent = 0u32;
        let mut window = Instant::now();
        let mut first_frame: Option<Instant> = None;
        let mut colors = self.room.clone();

        while !self.stop.load(Ordering::Relaxed) {
            let tick = Instant::now();
            let settings = self.settings.borrow().clone();
            colors = match self.frames.latest() {
                Some(frame) => {
                    let live = self.mapper.map(&frame, &settings);
                    let shown = first_frame.get_or_insert_with(Instant::now).elapsed();
                    wipe(&self.room, &live, self.mapper.zones(), shown)
                }
                // Keep the session alive until the first frame arrives.
                None => self.room.clone(),
            };
            stream.send(&colors)?;
            sent += 1;

            let elapsed = window.elapsed();
            if elapsed >= Duration::from_secs(1) {
                let fps = sent as f32 / elapsed.as_secs_f32();
                self.status
                    .send_modify(|s| s.fps = (fps * 10.0).round() / 10.0);
                sent = 0;
                window = Instant::now();
            }

            let frame_time = Duration::from_secs_f64(1.0 / settings.fps.max(1) as f64);
            if let Some(remaining) = frame_time.checked_sub(tick.elapsed()) {
                std::thread::sleep(remaining);
            }
        }

        let stopping = Instant::now();
        while stopping.elapsed() < FADE {
            stream.send(&wipe(
                &colors,
                &self.room,
                self.mapper.zones(),
                stopping.elapsed(),
            ))?;
            std::thread::sleep(Duration::from_millis(20));
        }
        stream.close();
        Ok(())
    }
}

/// How long the wipe between the lights' own state and sync takes to cross the area.
const FADE: Duration = Duration::from_millis(1200);

/// Crossfades `from` into `to` left to right; each light crosses over the last 40% of [`FADE`].
fn wipe(
    from: &[ChannelColor],
    to: &[ChannelColor],
    zones: &[Zone],
    elapsed: Duration,
) -> Vec<ChannelColor> {
    let progress = elapsed.as_secs_f32() / FADE.as_secs_f32();
    from.iter()
        .zip(to)
        .zip(zones)
        .map(|((from, to), zone)| {
            let level = ((progress - zone.x * 0.6) / 0.4).clamp(0.0, 1.0);
            ChannelColor {
                channel_id: to.channel_id,
                rgb: std::array::from_fn(|i| {
                    let (a, b) = (from.rgb[i] as f32, to.rgb[i] as f32);
                    (a + (b - a) * level).round() as u16
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(channel_id: u8, value: u16) -> ChannelColor {
        ChannelColor {
            channel_id,
            rgb: [value; 3],
        }
    }

    #[test]
    fn wipe_crosses_over_from_the_left() {
        let zones = [
            Zone {
                channel_id: 0,
                x: 0.0,
                y: 0.5,
            },
            Zone {
                channel_id: 1,
                x: 1.0,
                y: 0.5,
            },
        ];
        let from = [solid(0, 1000), solid(1, 1000)];
        let to = [solid(0, 65535), solid(1, 65535)];
        let levels = |elapsed: Duration| {
            wipe(&from, &to, &zones, elapsed)
                .iter()
                .map(|c| c.rgb[0])
                .collect::<Vec<_>>()
        };
        assert_eq!(levels(Duration::ZERO), [1000, 1000]);
        let halfway = levels(FADE / 2);
        assert!(halfway[0] > halfway[1], "left should lead: {halfway:?}");
        assert_eq!(levels(FADE), [65535, 65535]);
    }
}
