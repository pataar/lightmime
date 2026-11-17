//! Screen capture through the XDG ScreenCast portal (works on Wayland and X11)
//! and PipeWire, using GStreamer to convert and downscale frames. Under
//! gamescope (Game Mode on the Steam Deck), which has no ScreenCast portal, it reads
//! gamescope's own PipeWire stream instead.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, Session};
use ashpd::enumflags2::BitFlags;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use crate::color::Frame;
use crate::config::{CaptureConfig, MAX_FPS, SourceKind};

/// Shared access to the most recent captured frame.
#[derive(Clone)]
pub struct FrameSource(Arc<Mutex<Option<Frame>>>);

impl FrameSource {
    /// The most recent frame. PipeWire only delivers frames when the screen
    /// changes, so this keeps returning the last one on a static screen.
    pub fn latest(&self) -> Option<Frame> {
        self.0.lock().ok()?.clone()
    }
}

/// The PipeWire node gamescope publishes its output on (the name of its stream).
const GAMESCOPE_NODE: &str = "gamescope";

/// A running screen capture. Dropping it stops the pipeline and closes the
/// portal session.
pub struct ScreenCapture {
    capture: Capture,
    /// `None` for gamescope, which is read without the portal.
    portal: Option<Portal>,
}

/// The portal session a capture came from.
struct Portal {
    session: Session<Screencast>,
    // Keeps the PipeWire connection open for the lifetime of the pipeline.
    _fd: OwnedFd,
}

/// A playing capture pipeline. Dropping it stops the pipeline.
struct Capture {
    pipeline: gst::Pipeline,
    limit: gst::Element,
    rate: gst::Element,
    latest: Arc<Mutex<Option<Frame>>>,
}

impl Capture {
    /// Build the pipeline reading from `pipewiresrc` with `target` (which
    /// stream to connect to) and start it.
    fn start(target: &str, config: &CaptureConfig, max_fps: u32) -> Result<Self> {
        let latest = Arc::new(Mutex::new(None));
        let source = format!("pipewiresrc {target} do-timestamp=true keepalive-time=1000");
        let (pipeline, limit, rate) = build_pipeline(&source, config, max_fps, latest.clone())?;
        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(e).context("starting the capture pipeline");
        }
        Ok(Self {
            pipeline,
            limit,
            rate,
            latest,
        })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl ScreenCapture {
    /// Ask the portal for a screen (or window) and start capturing it.
    ///
    /// With a `restore_token` from an earlier run the portal reuses that
    /// choice without asking. Returns the capture plus the token to store for
    /// next time. Frames above `max_fps` are dropped; see [`Self::set_max_fps`].
    ///
    /// With [`SourceKind::Gamescope`] there is no portal, picker or token:
    /// gamescope's stream is read straight from the PipeWire daemon.
    pub async fn start(
        config: &CaptureConfig,
        max_fps: u32,
        restore_token: Option<&str>,
    ) -> Result<(Self, Option<String>)> {
        Self::start_or_cancel(config, max_fps, restore_token, std::future::pending()).await
    }

    /// [`Self::start`], given up when `cancel` completes first: the portal
    /// session is closed, which takes its screen picker away with it.
    pub async fn start_or_cancel(
        config: &CaptureConfig,
        max_fps: u32,
        restore_token: Option<&str>,
        cancel: impl Future<Output = ()>,
    ) -> Result<(Self, Option<String>)> {
        gst::init().context("initialising GStreamer")?;

        let source = match config.source {
            SourceKind::Monitor => SourceType::Monitor,
            SourceKind::Window => SourceType::Window,
            SourceKind::Gamescope => return Ok((Self::start_gamescope(config, max_fps)?, None)),
        };
        let portal = Screencast::new()
            .await
            .context("connecting to the ScreenCast portal")?;
        let session = portal
            .create_session(CreateSessionOptions::default())
            .await
            .context("creating a screen-cast session")?;
        // The portal keeps a session open until it is closed, not when it is dropped.
        let result = tokio::select! {
            result = Self::start_in(&portal, &session, source, config, max_fps, restore_token) => result,
            () = cancel => Err(anyhow!("cancelled while choosing a screen")),
        };
        match result {
            Ok(cast) => Ok((
                Self {
                    capture: cast.capture,
                    portal: Some(Portal {
                        session,
                        _fd: cast.fd,
                    }),
                },
                cast.restore_token,
            )),
            Err(e) => {
                let _ = session.close().await;
                Err(e)
            }
        }
    }

    async fn start_in(
        portal: &Screencast,
        session: &Session<Screencast>,
        source: SourceType,
        config: &CaptureConfig,
        max_fps: u32,
        restore_token: Option<&str>,
    ) -> Result<Started> {
        portal
            .select_sources(
                session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(CursorMode::Hidden)
                    .set_sources(BitFlags::from(source))
                    .set_multiple(false)
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(restore_token),
            )
            .await
            .context("selecting capture sources")?;
        let streams = portal
            .start(session, None, StartCastOptions::default())
            .await
            .context("starting the screen cast")?
            .response()
            .context("screen sharing was not allowed")?;
        let stream = streams
            .streams()
            .first()
            .ok_or_else(|| anyhow!("the portal returned no streams"))?;
        let node_id = stream.pipe_wire_node_id();
        let new_token = streams.restore_token().map(str::to_owned);

        let fd = portal
            .open_pipe_wire_remote(session, OpenPipeWireRemoteOptions::default())
            .await
            .context("opening the PipeWire remote")?;

        let capture = Capture::start(
            &format!("fd={} path={node_id}", fd.as_raw_fd()),
            config,
            max_fps,
        )?;

        tracing::info!("capturing PipeWire node {node_id}");
        Ok(Started {
            capture,
            fd,
            restore_token: new_token,
        })
    }

    /// Capture gamescope's output. gamescope offers it as a PipeWire stream of
    /// its own (Steam records and streams from it), so there is nothing to ask
    /// for; inside a Flatpak this needs access to the PipeWire socket.
    fn start_gamescope(config: &CaptureConfig, max_fps: u32) -> Result<Self> {
        let capture = Capture::start(
            &format!("target-object={GAMESCOPE_NODE} autoconnect=false"),
            config,
            max_fps,
        )
        .context("connecting to gamescope's PipeWire stream (is this a gamescope session?)")?;

        tracing::info!("capturing gamescope's PipeWire stream");
        Ok(Self {
            capture,
            portal: None,
        })
    }

    /// A handle for reading frames from another thread.
    pub fn frame_source(&self) -> FrameSource {
        FrameSource(self.capture.latest.clone())
    }

    /// Change how many frames per second are captured, to follow the sync
    /// rate; frames the stream loop would skip are not worth copying.
    ///
    /// A new rate makes pipewiresrc reconnect the stream, so the caps are only touched when it changes.
    pub fn set_max_fps(&self, fps: u32) {
        let fps = fps.clamp(1, MAX_FPS);
        let Capture { rate, limit, .. } = &self.capture;
        if rate.property::<i32>("max-rate") == fps as i32 {
            return;
        }
        rate.set_property("max-rate", fps as i32);
        match framerate_caps(fps).parse::<gst::Caps>() {
            Ok(caps) => limit.set_property("caps", caps),
            Err(err) => tracing::warn!("framerate caps for {fps} fps: {err}"),
        }
    }

    /// Report a pipeline error, if one happened since the last call.
    pub fn poll_error(&self) -> Option<String> {
        let bus = self.capture.pipeline.bus()?;
        while let Some(message) = bus.pop() {
            if let gst::MessageView::Error(err) = message.view() {
                return Some(format!("{} ({:?})", err.error(), err.debug()));
            }
        }
        None
    }

    pub async fn stop(self) {
        drop(self.capture);
        if let Some(portal) = self.portal {
            let _ = portal.session.close().await;
        }
    }
}

/// What [`ScreenCapture::start_in`] sets up inside a portal session.
struct Started {
    capture: Capture,
    fd: OwnedFd,
    restore_token: Option<String>,
}

fn build_pipeline(
    source: &str,
    config: &CaptureConfig,
    max_fps: u32,
    latest: Arc<Mutex<Option<Frame>>>,
) -> Result<(gst::Pipeline, gst::Element, gst::Element)> {
    // `limit` asks the compositor not to copy frames faster than the sync rate; see `framerate_caps`.
    // videorate still drops anything above it, for compositors that ignore the limit.
    //
    // Scale down before converting, so conversion runs on a few thousand
    // pixels. Buffers are not copied out of PipeWire (`always-copy`): the
    // pipeline releases them as soon as they are scaled.
    let max_fps = max_fps.clamp(1, MAX_FPS);
    let description = format!(
        "{source} \
         ! capsfilter name=limit caps=\"{caps}\" \
         ! videorate name=rate drop-only=true max-rate={max_fps} \
         ! videoscale add-borders=false \
         ! video/x-raw,width={w},height={h},pixel-aspect-ratio=1/1 \
         ! videoconvert ! video/x-raw,format=RGB \
         ! appsink name=sink max-buffers=1 drop=true sync=false",
        caps = framerate_caps(max_fps),
        w = config.width,
        h = config.height,
    );
    let pipeline = gst::parse::launch(&description)
        .context("building the capture pipeline (is gst-plugin-pipewire installed?)")?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("capture pipeline is not a gst::Pipeline"))?;
    let sink = pipeline
        .by_name("sink")
        .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
        .ok_or_else(|| anyhow!("capture pipeline has no appsink"))?;
    let limit = pipeline
        .by_name("limit")
        .ok_or_else(|| anyhow!("capture pipeline has no framerate limit"))?;
    let rate = pipeline
        .by_name("rate")
        .ok_or_else(|| anyhow!("capture pipeline has no videorate"))?;

    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                if let Some(frame) = sample_to_frame(&sample)
                    && let Ok(mut slot) = latest.lock()
                {
                    *slot = Some(frame);
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    Ok((pipeline, limit, rate))
}

/// Caps limiting the compositor to `fps`.
///
/// `max-framerate` is a list, not a range: pipewiresrc makes the first value the default, and for a range that
/// is the minimum. The lower common rates after it keep negotiation working on screens slower than `fps`.
fn framerate_caps(fps: u32) -> String {
    let rates: Vec<String> = std::iter::once(fps)
        .chain([50, 30, 25, 24].into_iter().filter(|&rate| rate < fps))
        .map(|rate| format!("{rate}/1"))
        .collect();
    format!(
        "video/x-raw,max-framerate=(fraction){{ {} }}",
        rates.join(", ")
    )
}

/// Copy a sample into a tightly packed RGB frame (GStreamer may pad rows).
fn sample_to_frame(sample: &gst::Sample) -> Option<Frame> {
    let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let (width, height) = (info.width(), info.height());
    let stride = usize::try_from(info.stride()[0]).ok()?;
    let row_bytes = width as usize * 3;
    let mut data = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        data.extend_from_slice(map.get(start..start + row_bytes)?);
    }
    Some(Frame {
        width,
        height,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framerate_caps_prefer_the_sync_rate() {
        assert_eq!(
            framerate_caps(30),
            "video/x-raw,max-framerate=(fraction){ 30/1, 25/1, 24/1 }"
        );
        assert_eq!(
            framerate_caps(10),
            "video/x-raw,max-framerate=(fraction){ 10/1 }"
        );
    }
}
