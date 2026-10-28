//! Screen capture through the XDG ScreenCast portal (works on Wayland and X11)
//! and PipeWire, using GStreamer to convert and downscale frames.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use ashpd::desktop::PersistMode;
use ashpd::desktop::Session;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
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

/// A running screen capture. Dropping it stops the pipeline and closes the
/// portal session.
pub struct ScreenCapture {
    pipeline: gst::Pipeline,
    limit: gst::Element,
    rate: gst::Element,
    latest: Arc<Mutex<Option<Frame>>>,
    session: Session<Screencast>,
    // Keeps the PipeWire connection open for the lifetime of the pipeline.
    _pipewire_fd: OwnedFd,
}

impl ScreenCapture {
    /// Ask the portal for a screen (or window) and start capturing it.
    ///
    /// With a `restore_token` from an earlier run the portal reuses that
    /// choice without asking. Returns the capture plus the token to store for
    /// next time. Frames above `max_fps` are dropped; see [`Self::set_max_fps`].
    pub async fn start(
        config: &CaptureConfig,
        max_fps: u32,
        restore_token: Option<&str>,
    ) -> Result<(Self, Option<String>)> {
        gst::init().context("initialising GStreamer")?;

        let portal = Screencast::new()
            .await
            .context("connecting to the ScreenCast portal")?;
        let session = portal
            .create_session(Default::default())
            .await
            .context("creating a screen-cast session")?;
        let source = match config.source {
            SourceKind::Monitor => SourceType::Monitor,
            SourceKind::Window => SourceType::Window,
        };
        portal
            .select_sources(
                &session,
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
            .start(&session, None, Default::default())
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
            .open_pipe_wire_remote(&session, Default::default())
            .await
            .context("opening the PipeWire remote")?;

        let latest = Arc::new(Mutex::new(None));
        let source = format!(
            "pipewiresrc fd={} path={node_id} do-timestamp=true keepalive-time=1000",
            fd.as_raw_fd()
        );
        let (pipeline, limit, rate) = build_pipeline(&source, config, max_fps, latest.clone())?;
        pipeline
            .set_state(gst::State::Playing)
            .context("starting the capture pipeline")?;

        tracing::info!("capturing PipeWire node {node_id}");
        Ok((
            Self {
                pipeline,
                limit,
                rate,
                latest,
                session,
                _pipewire_fd: fd,
            },
            new_token,
        ))
    }

    /// A handle for reading frames from another thread.
    pub fn frame_source(&self) -> FrameSource {
        FrameSource(self.latest.clone())
    }

    /// Change how many frames per second are captured, to follow the sync
    /// rate; frames the stream loop would skip are not worth copying.
    ///
    /// A new rate makes pipewiresrc reconnect the stream, so the caps are only touched when it changes.
    pub fn set_max_fps(&self, fps: u32) {
        let fps = fps.clamp(1, MAX_FPS);
        if self.rate.property::<i32>("max-rate") == fps as i32 {
            return;
        }
        self.rate.set_property("max-rate", fps as i32);
        match framerate_caps(fps).parse::<gst::Caps>() {
            Ok(caps) => self.limit.set_property("caps", caps),
            Err(err) => tracing::warn!("framerate caps for {fps} fps: {err}"),
        }
    }

    /// Report a pipeline error, if one happened since the last call.
    pub fn poll_error(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        while let Some(message) = bus.pop() {
            if let gst::MessageView::Error(err) = message.view() {
                return Some(format!("{} ({:?})", err.error(), err.debug()));
            }
        }
        None
    }

    pub async fn stop(self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        let _ = self.session.close().await;
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
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
