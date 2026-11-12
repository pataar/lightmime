//! The sync engine: owns the capture and the bridge session, and runs the
//! streaming loop. Frontends talk to it through an [`Engine`] handle.
//!
//! The engine is an actor. Its `Phase` is the state machine; the
//! [`EngineState`] it publishes is derived from it, and only the actor writes
//! that. Opening a session (bridge REST calls, the portal's screen picker) runs
//! as a separate task, so `stop` and `set_bridge` are heard while it runs.

use std::mem::DropGuard;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::sync::{mpsc, oneshot, watch};

use crate::capture::ScreenCapture;
use crate::color::{ColorMapper, Zone};
use crate::config::{BridgeConfig, Config, Setup, SyncSettings};
use crate::hue::{Bridge, ChannelColor, EntertainmentStream};

/// What frontends show. Every change is made by the engine:
///
/// | From                    | On                          | To          |
/// |-------------------------|-----------------------------|-------------|
/// | `Idle`, `Error`         | start                       | `Starting`  |
/// | `Starting`              | area started, DTLS connects | `Streaming` |
/// | `Starting`              | opening fails               | `Error`     |
/// | `Starting`, `Streaming` | stop                        | `Idle`      |
/// | `Streaming`             | stream or capture fails     | `Error`     |
/// | `Error`                 | stop                        | `Idle`      |
/// | `Starting`, `Streaming` | `set_bridge`                | `Starting`  |
///
/// A stop while `Starting` cancels the opening (closing the portal's screen
/// picker); the state stays `Starting` until whatever was opened is released.
/// The labels are a public API (MQTT, D-Bus, Waybar); see [`Self::LABELS`].
#[derive(Debug, Clone, Default, PartialEq)]
pub enum EngineState {
    #[default]
    Idle,
    Starting,
    Streaming,
    Error(String),
}

impl EngineState {
    /// Every [`Self::label`], in the order of the table above.
    pub const LABELS: [&str; 4] = ["idle", "starting", "streaming", "error"];

    pub fn label(&self) -> &'static str {
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
    /// Round trip to the bridge, measured once when the stream connects.
    pub latency: Option<Duration>,
    /// Name of the entertainment area being streamed to.
    pub area: String,
    pub channels: usize,
}

enum Command {
    Start,
    Stop,
    Toggle,
    SetBridge(BridgeConfig),
    /// Answered once nothing is opening; queued behind earlier commands.
    Settled(oneshot::Sender<()>),
    /// The opening task finished.
    Opened {
        attempt: u64,
        result: Result<Box<Opened>>,
    },
    /// The stream thread's DTLS handshake succeeded.
    Connected {
        session_id: u64,
        latency: Duration,
    },
    StreamFailed {
        session_id: u64,
        reason: String,
    },
}

/// Handle to a running engine. Cheap to clone; the engine stops when the last
/// handle is dropped.
#[derive(Clone)]
pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
    status: watch::Receiver<Status>,
    settings: Arc<watch::Sender<SyncSettings>>,
    colors: watch::Receiver<Vec<(Zone, ChannelColor)>>,
}

impl Engine {
    /// Spawn the engine on the current tokio runtime. The portal restore token
    /// is read from and written to `restore_token_path`, so screen selection
    /// is only asked once.
    pub fn spawn(config: Config, restore_token_path: Option<PathBuf>) -> Self {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (status_tx, status) = watch::channel(Status::default());
        let settings = Arc::new(watch::Sender::new(config.sync.clone().sanitized()));
        let (colors_tx, colors) = watch::channel(Vec::new());
        let actor = Actor {
            config,
            restore_token_path,
            settings: settings.subscribe(),
            status: Arc::new(status_tx),
            colors: Arc::new(colors_tx),
            // Weak, so the channel closes, and the actor stops, with the last handle.
            commands: commands.downgrade(),
            phase: Phase::Idle,
            counter: 0,
            settled: Vec::new(),
        };
        tokio::spawn(actor.run(command_rx));
        Self {
            commands,
            status,
            settings,
            colors,
        }
    }

    /// Start syncing; does nothing while starting or streaming.
    pub fn start(&self) {
        let _ = self.commands.send(Command::Start);
    }

    /// Stop syncing (or starting) and release the entertainment area. Also
    /// clears an error.
    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }

    /// Stop when starting or streaming, start otherwise. Decided by the
    /// engine when it gets there, so two quick toggles always cancel out.
    pub fn toggle(&self) {
        let _ = self.commands.send(Command::Toggle);
    }

    /// [`Self::stop`], then wait until the area on the bridge is released.
    /// Call it before exiting, or the lights stay locked in streaming mode.
    pub async fn stop_and_wait(&self) {
        let (done, settled) = oneshot::channel();
        self.stop();
        if self.commands.send(Command::Settled(done)).is_ok() {
            let _ = settled.await;
        }
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

    /// Watch the colours sent to the lights, with where on the screen each one comes from.
    /// Empty while not streaming.
    pub fn subscribe_colors(&self) -> watch::Receiver<Vec<(Zone, ChannelColor)>> {
        self.colors.clone()
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

/// The actor's state machine. [`EngineState`] is what it shows of it.
enum Phase {
    /// Nothing open. Published as `Idle` or `Error`.
    Idle,
    /// An opening task runs. Published as `Starting`.
    Starting {
        attempt: u64,
        /// Set to true to cancel the opening; never set back.
        cancel: watch::Sender<bool>,
        then: Then,
    },
    /// The area is started and the stream thread runs. Published as
    /// `Starting` until the thread connects, then `Streaming`.
    Active(Session),
}

/// What to do once the opening task finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    /// Start streaming.
    Run,
    /// Release what was opened and go idle.
    Stop,
    /// Release what was opened and open again, with the current bridge.
    Restart,
}

/// A request that changes what happens after an opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
    Start,
    Stop,
    SetBridge,
}

impl Then {
    /// Fold in a request that arrives while starting. Anything but `Run`
    /// cancels the opening, which cannot be taken back, so a start after a
    /// stop has to open again.
    #[expect(
        clippy::match_same_arms,
        reason = "one arm per case reads as the transition table"
    )]
    fn on(self, request: Request) -> Self {
        match (self, request) {
            (Self::Run, Request::Start) => Self::Run,
            (_, Request::Start) => Self::Restart,
            (_, Request::Stop) => Self::Stop,
            (Self::Stop, Request::SetBridge) => Self::Stop,
            (_, Request::SetBridge) => Self::Restart,
        }
    }
}

struct Actor {
    config: Config,
    restore_token_path: Option<PathBuf>,
    settings: watch::Receiver<SyncSettings>,
    status: Arc<watch::Sender<Status>>,
    colors: Arc<watch::Sender<Vec<(Zone, ChannelColor)>>>,
    commands: mpsc::WeakUnboundedSender<Command>,
    phase: Phase,
    /// Numbers openings and sessions, so late messages about an old one are ignored.
    counter: u64,
    /// [`Engine::stop_and_wait`] calls waiting for an opening to finish.
    settled: Vec<oneshot::Sender<()>>,
}

/// What is held on the bridge and the screen while a session is open.
struct Lease {
    bridge: Bridge,
    area_id: String,
    capture: ScreenCapture,
}

impl Lease {
    /// Stop the area, or the lights stay locked in streaming mode, and the capture.
    async fn release(self) {
        if let Err(e) = self.bridge.set_streaming(&self.area_id, false).await {
            tracing::warn!("could not stop the entertainment area: {e:#}");
        }
        self.capture.stop().await;
    }
}

/// A finished opening: the area is started, the stream thread not yet.
struct Opened {
    lease: Lease,
    worker: StreamWorker,
}

struct Session {
    id: u64,
    lease: Lease,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Session {
    /// Stop the stream thread (it wipes back to the room first), then release.
    async fn close(self) {
        self.stop.store(true, Ordering::Relaxed);
        let thread = self.thread;
        let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        self.lease.release().await;
    }
}

impl Actor {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        let mut health = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.handle(command).await,
                    None => break,
                },
                Ok(()) = self.settings.changed() => {
                    let fps = self.settings.borrow_and_update().fps;
                    if let Phase::Active(session) = &self.phase {
                        session.lease.capture.set_max_fps(fps);
                    }
                }
                _ = health.tick() => {
                    let error = match &self.phase {
                        Phase::Active(session) => session.lease.capture.poll_error(),
                        _ => None,
                    };
                    if let Some(error) = error {
                        self.fail(format!("screen capture failed: {error}")).await;
                    }
                }
            }
        }
        // The last handle is gone; release whatever is still open.
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Active(session) => session.close().await,
            Phase::Starting { cancel, .. } => {
                // The opening task sees the channel gone and releases what it opened.
                cancel.send_replace(true);
            }
            Phase::Idle => {}
        }
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Start => self.request(Request::Start).await,
            Command::Stop => self.request(Request::Stop).await,
            Command::Toggle => {
                let request = match self.phase {
                    Phase::Idle => Request::Start,
                    Phase::Starting { .. } | Phase::Active(_) => Request::Stop,
                };
                self.request(request).await;
            }
            Command::SetBridge(bridge) => {
                self.config.bridge = bridge;
                self.request(Request::SetBridge).await;
            }
            Command::Settled(done) => match self.phase {
                Phase::Starting { .. } => self.settled.push(done),
                Phase::Idle | Phase::Active(_) => {
                    let _ = done.send(());
                }
            },
            Command::Opened { attempt, result } => self.opened(attempt, result).await,
            Command::Connected {
                session_id,
                latency,
            } => {
                if self.is_session(session_id) {
                    self.status.send_modify(|s| {
                        s.state = EngineState::Streaming;
                        s.latency = Some(latency);
                    });
                }
            }
            // A thread can fail after its session was stopped or replaced; that must not end the current one.
            Command::StreamFailed { session_id, reason } => {
                if self.is_session(session_id) {
                    self.fail(reason).await;
                }
            }
        }
    }

    fn is_session(&self, id: u64) -> bool {
        matches!(&self.phase, Phase::Active(session) if session.id == id)
    }

    async fn request(&mut self, request: Request) {
        match &mut self.phase {
            Phase::Idle => match request {
                Request::Start => self.begin(),
                // Clears an error.
                Request::Stop => self.set_state(EngineState::Idle),
                Request::SetBridge => {}
            },
            Phase::Starting { cancel, then, .. } => {
                *then = then.on(request);
                if *then != Then::Run {
                    cancel.send_replace(true);
                }
            }
            Phase::Active(_) => match request {
                Request::Start => {}
                Request::Stop => {
                    self.close().await;
                    self.set_state(EngineState::Idle);
                }
                Request::SetBridge => {
                    self.close().await;
                    self.begin();
                }
            },
        }
    }

    /// Idle → Starting: open a session on its own task.
    fn begin(&mut self) {
        self.counter += 1;
        let attempt = self.counter;
        let (cancel, cancelled) = watch::channel(false);
        let opening = Opening {
            config: self.config.clone(),
            restore_token_path: self.restore_token_path.clone(),
            settings: self.settings.clone(),
            status: self.status.clone(),
            colors: self.colors.clone(),
            cancelled,
        };
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = opening.open().await.map(Box::new);
            let unsent = match commands.upgrade() {
                Some(commands) => commands
                    .send(Command::Opened { attempt, result })
                    .err()
                    .map(|e| e.0),
                None => Some(Command::Opened { attempt, result }),
            };
            if let Some(Command::Opened {
                result: Ok(opened), ..
            }) = unsent
            {
                // The engine is gone; nobody else will release this.
                opened.lease.release().await;
            }
        });
        self.phase = Phase::Starting {
            attempt,
            cancel,
            then: Then::Run,
        };
        self.set_state(EngineState::Starting);
    }

    /// Starting → Active, Idle, Error, or Starting again.
    async fn opened(&mut self, attempt: u64, result: Result<Box<Opened>>) {
        let then = match &self.phase {
            Phase::Starting {
                attempt: current,
                then,
                ..
            } if *current == attempt => *then,
            // Cannot happen: only one opening runs at a time. Release it anyway.
            _ => {
                if let Ok(opened) = result {
                    opened.lease.release().await;
                }
                return;
            }
        };
        self.phase = Phase::Idle;
        match (then, result) {
            (Then::Run, Ok(opened)) => self.launch(*opened),
            (Then::Run, Err(e)) => {
                tracing::error!("could not start syncing: {e:#}");
                self.set_state(EngineState::Error(format!("{e:#}")));
            }
            (Then::Stop | Then::Restart, result) => {
                match result {
                    Ok(opened) => opened.lease.release().await,
                    Err(e) => tracing::debug!("opening given up: {e:#}"),
                }
                if then == Then::Restart {
                    self.begin();
                } else {
                    self.set_state(EngineState::Idle);
                }
            }
        }
        if !matches!(self.phase, Phase::Starting { .. }) {
            for done in self.settled.drain(..) {
                let _ = done.send(());
            }
        }
    }

    /// Start the stream thread on an opened session.
    fn launch(&mut self, opened: Opened) {
        let Opened { lease, worker } = opened;
        // The rate may have changed while the portal asked for a screen.
        lease.capture.set_max_fps(self.settings.borrow().fps);
        self.counter += 1;
        let session_id = self.counter;
        let stop = worker.stop.clone();
        let commands = self.commands.clone();
        let spawned = std::thread::Builder::new()
            .name("lightmime-stream".into())
            .spawn(move || worker.run(session_id, &commands));
        match spawned {
            Ok(thread) => {
                self.phase = Phase::Active(Session {
                    id: session_id,
                    lease,
                    stop,
                    thread,
                });
            }
            Err(e) => {
                let reason = format!("spawning the streaming thread: {e}");
                tracing::error!("{reason}");
                // Release on the runtime; the actor must not wait on the bridge here.
                tokio::spawn(lease.release());
                self.set_state(EngineState::Error(reason));
            }
        }
    }

    async fn fail(&mut self, reason: String) {
        tracing::error!("{reason}");
        self.close().await;
        self.set_state(EngineState::Error(reason));
    }

    /// Active → Idle, without publishing a state.
    async fn close(&mut self) {
        if let Phase::Active(session) = std::mem::replace(&mut self.phase, Phase::Idle) {
            session.close().await;
        }
    }

    fn set_state(&self, state: EngineState) {
        self.status.send_modify(|s| {
            if state != EngineState::Streaming {
                s.fps = 0.0;
                s.latency = None;
            }
            s.state = state;
        });
    }
}

/// Everything opening a session needs, so it can run apart from the actor.
struct Opening {
    config: Config,
    restore_token_path: Option<PathBuf>,
    settings: watch::Receiver<SyncSettings>,
    status: Arc<watch::Sender<Status>>,
    colors: Arc<watch::Sender<Vec<(Zone, ChannelColor)>>>,
    cancelled: watch::Receiver<bool>,
}

impl Opening {
    async fn open(self) -> Result<Opened> {
        let bridge_config = self.config.bridge.clone();
        match bridge_config.setup() {
            Setup::Unpaired => bail!("not paired with a Hue bridge yet"),
            Setup::NoArea => bail!("no entertainment area configured"),
            Setup::Ready => {}
        }
        let bridge = Bridge::new(&bridge_config.host, &bridge_config.app_key)?;
        let area = self
            .unless_cancelled(
                bridge.entertainment_configuration(&bridge_config.entertainment_config_id),
            )
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
        let fps = self.settings.borrow().fps;
        let mut cancelled = self.cancelled.clone();
        let (capture, new_token) = ScreenCapture::start_or_cancel(
            &self.config.capture,
            fps,
            token.as_deref(),
            async move {
                let _ = cancelled.wait_for(|c| *c).await;
            },
        )
        .await?;
        if let (Some(path), Some(token)) = (&self.restore_token_path, new_token) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(path, token) {
                tracing::warn!("could not store the screen-cast restore token: {e}");
            }
        }

        // Read before starting the area, while the lights still show their own state.
        let room = match self.unless_cancelled(bridge.channel_colors(&area)).await {
            Ok(room) => room,
            Err(e) if self.is_cancelled() => {
                capture.stop().await;
                return Err(e);
            }
            Err(e) => {
                tracing::warn!("could not read the lights' state, fading from black: {e:#}");
                area.channels
                    .iter()
                    .map(|c| ChannelColor {
                        channel_id: c.channel_id,
                        rgb: [0; 3],
                    })
                    .collect()
            }
        };

        // Not cancelled from here: the area must be stopped once started, which
        // releasing the returned lease does.
        if let Err(e) = bridge.set_streaming(&area.id, true).await {
            capture.stop().await;
            return Err(e).context("starting the entertainment area");
        }
        let identity = bridge.psk_identity().await;

        let zones: Vec<Zone> = area.channels.iter().map(Zone::from_channel).collect();
        self.status.send_modify(|s| {
            s.area.clone_from(&area.metadata.name);
            s.channels = zones.len();
        });

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
            colors: self.colors.clone(),
            stop: Arc::new(AtomicBool::new(false)),
        };
        Ok(Opened {
            lease: Lease {
                bridge,
                area_id: area.id,
                capture,
            },
            worker,
        })
    }

    fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }

    /// Run `step` unless the opening is cancelled first.
    async fn unless_cancelled<T>(&self, step: impl Future<Output = Result<T>>) -> Result<T> {
        let mut cancelled = self.cancelled.clone();
        tokio::select! {
            result = step => result,
            _ = cancelled.wait_for(|c| *c) => bail!("cancelled"),
        }
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
    colors: Arc<watch::Sender<Vec<(Zone, ChannelColor)>>>,
    stop: Arc<AtomicBool>,
}

impl StreamWorker {
    /// Stream until stopped; tells the actor when it connects and when it
    /// fails, a panic included.
    fn run(mut self, session_id: u64, commands: &mpsc::WeakUnboundedSender<Command>) {
        let send = |command| {
            if let Some(commands) = commands.upgrade() {
                let _ = commands.send(command);
            }
        };
        let colors = Arc::clone(&self.colors);
        // Reports the outcome when the thread ends. A panic unwinds past the
        // assignment below and leaves the failure in place, so the actor still
        // releases the area instead of showing a dead session as streaming.
        let mut outcome = DropGuard::new(
            Err("the streaming thread panicked".to_owned()),
            |outcome: Result<(), String>| {
                colors.send_replace(Vec::new());
                if let Err(reason) = outcome {
                    send(Command::StreamFailed { session_id, reason });
                }
            },
        );
        *outcome = self
            .stream(|latency| {
                send(Command::Connected {
                    session_id,
                    latency,
                });
            })
            .map_err(|e| format!("{e:#}"));
    }

    fn stream(&mut self, connected: impl FnOnce(Duration)) -> Result<()> {
        let mut stream = EntertainmentStream::connect(
            &self.host,
            &self.identity,
            &self.client_key,
            &self.area_id,
        )?;
        tracing::info!("streaming to the bridge");
        connected(stream.round_trip());

        let mut sent = 0u32;
        let mut window = Instant::now();
        let mut fade = Fade::Holding;
        let mut colors = self.room.clone();

        while !self.stop.load(Ordering::Relaxed) {
            let tick = Instant::now();
            let settings = self.settings.borrow().clone();
            let frame = self.frames.latest();
            fade = fade.next(frame.is_some());
            colors = match (fade, frame) {
                (Fade::Holding, _) | (_, None) => self.room.clone(),
                (Fade::In(since), Some(frame)) => {
                    let live = self.mapper.map(&frame, &settings);
                    wipe(&self.room, &live, self.mapper.zones(), since.elapsed())
                }
                (Fade::Live, Some(frame)) => self.mapper.map(&frame, &settings),
            };
            stream.send(&colors)?;
            self.publish(&colors);
            sent += 1;

            let elapsed = window.elapsed();
            if elapsed >= Duration::from_secs(1) {
                let fps = sent as f32 / elapsed.as_secs_f32();
                self.status
                    .send_modify(|s| s.fps = (fps * 10.0).round() / 10.0);
                sent = 0;
                window = Instant::now();
            }

            let frame_time = Duration::from_secs_f64(1.0 / f64::from(settings.fps.max(1)));
            if let Some(remaining) = frame_time.checked_sub(tick.elapsed()) {
                std::thread::sleep(remaining);
            }
        }

        // Wipe back to what the lights showed before, from wherever the fade got to.
        let stopping = Instant::now();
        while stopping.elapsed() < FADE {
            let fading = wipe(&colors, &self.room, self.mapper.zones(), stopping.elapsed());
            stream.send(&fading)?;
            self.publish(&fading);
            std::thread::sleep(Duration::from_millis(20));
        }
        stream.close();
        Ok(())
    }

    fn publish(&self, colors: &[ChannelColor]) {
        let zones = self.mapper.zones().iter().copied();
        self.colors
            .send_replace(zones.zip(colors.iter().copied()).collect());
    }
}

/// Where the stream is between the lights' own state and the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fade {
    /// No frame yet: the lights' own state keeps the bridge's session alive.
    Holding,
    /// Wiping from the lights' own state into sync since the first frame.
    In(Instant),
    /// Following the screen.
    Live,
}

impl Fade {
    fn next(self, has_frame: bool) -> Self {
        match self {
            Self::Holding if has_frame => Self::In(Instant::now()),
            Self::In(since) if since.elapsed() >= FADE => Self::Live,
            other => other,
        }
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
                    let (a, b) = (f32::from(from.rgb[i]), f32::from(to.rgb[i]));
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

    #[test]
    fn requests_while_starting() {
        use Request::*;
        assert_eq!(Then::Run.on(Start), Then::Run);
        assert_eq!(Then::Run.on(Stop), Then::Stop);
        assert_eq!(Then::Run.on(SetBridge), Then::Restart);
        // A stop has cancelled the opening; starting again must open anew.
        assert_eq!(Then::Stop.on(Start), Then::Restart);
        assert_eq!(Then::Stop.on(SetBridge), Then::Stop);
        assert_eq!(Then::Restart.on(Start), Then::Restart);
        assert_eq!(Then::Restart.on(Stop), Then::Stop);
        assert_eq!(Then::Restart.on(SetBridge), Then::Restart);
    }

    #[test]
    fn fade_holds_until_the_first_frame() {
        assert_eq!(Fade::Holding.next(false), Fade::Holding);
        let Fade::In(since) = Fade::Holding.next(true) else {
            panic!("the first frame starts the wipe");
        };
        assert_eq!(Fade::In(since).next(true), Fade::In(since));
        let long_ago = Instant::now().checked_sub(FADE).unwrap();
        assert_eq!(Fade::In(long_ago).next(true), Fade::Live);
        assert_eq!(Fade::Live.next(true), Fade::Live);
    }

    #[test]
    fn labels_are_complete() {
        let states = [
            EngineState::Idle,
            EngineState::Starting,
            EngineState::Streaming,
            EngineState::Error(String::new()),
        ];
        assert_eq!(states.map(|s| s.label()), EngineState::LABELS);
    }

    async fn settled_state(engine: &Engine) -> EngineState {
        let (done, settled) = oneshot::channel();
        engine.commands.send(Command::Settled(done)).unwrap();
        settled.await.unwrap();
        engine.status().state
    }

    #[tokio::test]
    async fn start_without_a_bridge_fails_and_stop_clears_it() {
        let engine = Engine::spawn(Config::default(), None);
        engine.start();
        let state = settled_state(&engine).await;
        assert_eq!(state.error(), Some("not paired with a Hue bridge yet"));

        // After an error, toggle starts again.
        engine.toggle();
        assert!(settled_state(&engine).await.error().is_some());

        engine.stop_and_wait().await;
        assert_eq!(engine.status().state, EngineState::Idle);
    }

    #[tokio::test]
    async fn stop_while_starting_ends_idle_not_in_error() {
        let engine = Engine::spawn(Config::default(), None);
        engine.start();
        engine.stop();
        assert_eq!(settled_state(&engine).await, EngineState::Idle);
    }
}
