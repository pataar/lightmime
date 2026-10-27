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
    StreamFailed(String),
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
}

struct Session {
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
                    Some(Command::StreamFailed(reason)) => self.fail(reason).await,
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
            settings: self.settings.clone(),
            status: self.status.clone(),
            stop: stop.clone(),
        };
        let commands = self.commands.clone();
        let thread = std::thread::Builder::new()
            .name("lightmime-stream".into())
            .spawn(move || {
                if let Err(e) = worker.run() {
                    let _ = commands.send(Command::StreamFailed(format!("{e:#}")));
                }
            })
            .context("spawning the streaming thread")?;

        Ok(Session {
            bridge,
            area_id: area.id,
            capture,
            stop,
            thread,
        })
    }

    async fn stop_session(&mut self) {
        let Some(Session {
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
        let black: Vec<ChannelColor> = self
            .mapper
            .zones()
            .iter()
            .map(|z| ChannelColor {
                channel_id: z.channel_id,
                rgb: [0; 3],
            })
            .collect();

        while !self.stop.load(Ordering::Relaxed) {
            let tick = Instant::now();
            let settings = self.settings.borrow().clone();
            let colors = match self.frames.latest() {
                Some(frame) => self.mapper.map(&frame, &settings),
                // Keep the session alive until the first frame arrives.
                None => black.clone(),
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
        stream.close();
        Ok(())
    }
}
