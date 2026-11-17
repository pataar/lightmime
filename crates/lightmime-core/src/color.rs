//! Turning captured frames into per-channel colours.

use std::sync::LazyLock;

use crate::config::SyncSettings;
use crate::hue::{Channel, ChannelColor};

/// A downscaled frame, tightly packed RGB8.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Frame {
    /// Average colour of a rectangle given in coordinates normalised (0..1) to the picture
    /// inside `bars`, as linear-light RGB in 0..1.
    fn average(&self, cx: f32, cy: f32, size: f32, bars: Bars) -> [f32; 3] {
        let (left, top) = (bars.x as f32, bars.y as f32);
        let w = self.width.saturating_sub(2 * bars.x).max(1) as f32;
        let h = self.height.saturating_sub(2 * bars.y).max(1) as f32;
        let half = size / 2.0;
        let x0 = (left + (cx - half).max(0.0) * w) as u32;
        let x1 = ((left + (cx + half).min(1.0) * w).ceil() as u32).clamp(x0 + 1, self.width);
        let y0 = (top + (cy - half).max(0.0) * h) as u32;
        let y1 = ((top + (cy + half).min(1.0) * h).ceil() as u32).clamp(y0 + 1, self.height);

        let mut sum = [0.0f32; 3];
        let mut count = 0u32;
        for y in y0..y1 {
            let row = (y * self.width) as usize * 3;
            for x in x0..x1 {
                let i = row + x as usize * 3;
                let Some(px) = self.data.get(i..i + 3) else {
                    continue;
                };
                for c in 0..3 {
                    sum[c] += srgb_to_linear(px[c]);
                }
                count += 1;
            }
        }
        if count == 0 {
            return [0.0; 3];
        }
        sum.map(|s| s / count as f32)
    }

    fn is_dark(&self, x: u32, y: u32) -> bool {
        let i = (y * self.width + x) as usize * 3;
        self.data
            .get(i..i + 3)
            .is_none_or(|px| px.iter().all(|&c| c <= DARK))
    }

    /// Dark lines from each end of an axis, as `(start, end)`; `None` when either end is dark
    /// beyond `limit` lines, as in a dark scene, where the bars can't be told from the picture.
    fn dark_edges(&self, limit: u32, vertical: bool) -> Option<(u32, u32)> {
        let (len, across) = if vertical {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        let dark_line = |i: u32| {
            (0..across).all(|j| {
                if vertical {
                    self.is_dark(j, i)
                } else {
                    self.is_dark(i, j)
                }
            })
        };
        let count = |lines: &mut dyn Iterator<Item = u32>| {
            let n = lines
                .take(limit as usize + 1)
                .take_while(|&i| dark_line(i))
                .count() as u32;
            (n <= limit).then_some(n)
        };
        Some((count(&mut (0..len))?, count(&mut (0..len).rev())?))
    }
}

/// Brightest value (sRGB, 0..255) a pixel of a black bar may have. Video black is 0, or 16 when a
/// player shows limited-range video as full range.
const DARK: u8 = 16;

/// How long new bars must stay before the lights follow them, in seconds. A dark edge in the
/// picture comes and goes; bars of a film stay.
const BARS_HOLD_SECS: f32 = 2.0;

/// Black bars around the picture, in pixels cut from each side: `x` from the left and from the
/// right, `y` from the top and from the bottom.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Bars {
    x: u32,
    y: u32,
}

/// Follows the black bars on one axis. Bars of letterboxed and pillarboxed video are the same on
/// both sides, so it cuts as much as the thinner one, and only once that stayed for a while.
#[derive(Debug, Default)]
struct BarTracker {
    current: u32,
    pending: u32,
    seen: u32,
}

impl BarTracker {
    fn update(&mut self, edges: Option<(u32, u32)>, limit: u32, hold: u32) -> u32 {
        self.current = self.current.min(limit);
        let Some((a, b)) = edges else {
            return self.current;
        };
        let candidate = a.min(b);
        if a.max(b) < self.current {
            // The picture reaches into both bars: the film ended or went full screen. Subtitles
            // in one bar don't count.
            self.current = candidate;
            self.seen = 0;
        } else if candidate == self.current {
            self.seen = 0;
        } else {
            if candidate != self.pending {
                self.pending = candidate;
                self.seen = 0;
            }
            self.seen += 1;
            if self.seen >= hold {
                self.current = candidate;
                self.seen = 0;
            }
        }
        self.current
    }
}

/// Where on the screen a channel samples, derived from its position in the
/// entertainment area: x runs left to right, z bottom to top.
#[derive(Debug, Clone, Copy)]
pub struct Zone {
    pub channel_id: u8,
    pub x: f32,
    pub y: f32,
}

impl Zone {
    #[expect(
        clippy::manual_midpoint,
        reason = "maps -1..1 to 0..1, like `y` below; `midpoint` would hide that"
    )]
    pub fn from_channel(channel: &Channel) -> Self {
        Self {
            channel_id: channel.channel_id,
            x: ((channel.position.x as f32 + 1.0) / 2.0).clamp(0.0, 1.0),
            y: ((1.0 - channel.position.z as f32) / 2.0).clamp(0.0, 1.0),
        }
    }
}

/// Stateful colour pipeline: sampling, saturation, smoothing and brightness.
pub struct ColorMapper {
    zones: Vec<Zone>,
    smoothed: Vec<[f32; 3]>,
    bars_x: BarTracker,
    bars_y: BarTracker,
    /// Frame size the bars were measured on; they start over when it changes.
    frame_size: (u32, u32),
}

impl ColorMapper {
    pub fn new(zones: Vec<Zone>) -> Self {
        let smoothed = vec![[0.0; 3]; zones.len()];
        Self {
            zones,
            smoothed,
            bars_x: BarTracker::default(),
            bars_y: BarTracker::default(),
            frame_size: (0, 0),
        }
    }

    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }

    pub fn map(&mut self, frame: &Frame, settings: &SyncSettings) -> Vec<ChannelColor> {
        let alpha = 1.0 - settings.smoothing;
        let bars = self.bars(frame, settings);
        self.zones
            .iter()
            .zip(self.smoothed.iter_mut())
            .map(|(zone, previous)| {
                let sampled = saturate(
                    frame.average(zone.x, zone.y, settings.region_size, bars),
                    settings.saturation,
                );
                for c in 0..3 {
                    previous[c] += alpha * (sampled[c] - previous[c]);
                }
                ChannelColor {
                    channel_id: zone.channel_id,
                    rgb: previous.map(|v| to_u16(v * settings.brightness)),
                }
            })
            .collect()
    }

    fn bars(&mut self, frame: &Frame, settings: &SyncSettings) -> Bars {
        if self.frame_size != (frame.width, frame.height) {
            self.frame_size = (frame.width, frame.height);
            self.bars_x = BarTracker::default();
            self.bars_y = BarTracker::default();
        }
        let hold = (BARS_HOLD_SECS * settings.fps as f32) as u32;
        let limit = |len: u32| (settings.black_bars * len as f32) as u32;
        let (limit_x, limit_y) = (limit(frame.width), limit(frame.height));
        Bars {
            x: self
                .bars_x
                .update(frame.dark_edges(limit_x, false), limit_x, hold),
            y: self
                .bars_y
                .update(frame.dark_edges(limit_y, true), limit_y, hold),
        }
    }
}

/// Scale the distance from grey; works on linear RGB.
fn saturate(rgb: [f32; 3], factor: f32) -> [f32; 3] {
    let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    rgb.map(|c| (luma + (c - luma) * factor).clamp(0.0, 1.0))
}

/// Every sampled pixel goes through this each tick, so the `powf` is done
/// once per possible value instead.
fn srgb_to_linear(v: u8) -> f32 {
    static TABLE: LazyLock<[f32; 256]> = LazyLock::new(|| {
        std::array::from_fn(|i| {
            let v = i as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
    });
    TABLE[v as usize]
}

/// A Hue light's CIE xy colour and brightness (0..1) as stream RGB, using Signify's wide-gamut
/// matrix.
///
/// ponytail: brightness scales the gamma-encoded value, assuming Hue's dimming curve is roughly
/// perceptual; if lights jump when fading in or out of sync, this is the knob to tune.
pub fn xy_to_rgb(x: f32, y: f32, brightness: f32) -> [u16; 3] {
    let y = y.max(1e-4);
    let (cie_x, cie_z) = (x / y, (1.0 - x - y) / y);
    let rgb = [
        1.656492 * cie_x - 0.354851 - 0.255038 * cie_z,
        -0.707196 * cie_x + 1.655397 + 0.036152 * cie_z,
        0.051713 * cie_x - 0.121364 + 1.01153 * cie_z,
    ]
    .map(|c| c.max(0.0));
    let peak = rgb.iter().copied().fold(f32::EPSILON, f32::max);
    let brightness = brightness.clamp(0.0, 1.0);
    rgb.map(|c| (f32::from(to_u16(c / peak)) * brightness).round() as u16)
}

/// The bridge expects gamma-encoded values, like what the screen shows.
fn to_u16(linear: f32) -> u16 {
    let v = linear.clamp(0.0, 1.0);
    let encoded = if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 65535.0).round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(rgb: [u8; 3]) -> Frame {
        Frame {
            width: 4,
            height: 2,
            data: rgb.repeat(8),
        }
    }

    #[test]
    fn white_point_is_neutral_and_dims() {
        let [r, g, b] = xy_to_rgb(0.3127, 0.3290, 1.0);
        let peak = r.max(g).max(b);
        assert_eq!(peak, 65535);
        assert!(r.min(g).min(b) > 60000, "{r} {g} {b}");
        assert_eq!(xy_to_rgb(0.3127, 0.3290, 0.5).iter().max(), Some(&32768));
        assert_eq!(xy_to_rgb(0.3127, 0.3290, 0.0), [0; 3]);
    }

    #[test]
    fn red_xy_is_red() {
        let [r, g, b] = xy_to_rgb(0.68, 0.31, 1.0);
        assert_eq!(r, 65535);
        assert!(g < 20000 && b < 20000, "{g} {b}");
    }

    #[test]
    fn solid_frame_maps_to_its_colour() {
        let settings = SyncSettings {
            smoothing: 0.0,
            saturation: 1.0,
            brightness: 1.0,
            ..Default::default()
        };
        let mut mapper = ColorMapper::new(vec![Zone {
            channel_id: 3,
            x: 0.5,
            y: 0.5,
        }]);
        let out = mapper.map(&solid([255, 0, 128]), &settings);
        assert_eq!(out[0].channel_id, 3);
        assert_eq!(out[0].rgb[0], 65535);
        assert_eq!(out[0].rgb[1], 0);
        assert!((i32::from(out[0].rgb[2]) - 128 * 257).abs() < 300);
    }

    #[test]
    fn left_and_right_zones_sample_their_half() {
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255]);
        }
        let frame = Frame {
            width: 4,
            height: 2,
            data,
        };
        let settings = SyncSettings {
            smoothing: 0.0,
            saturation: 1.0,
            region_size: 0.4,
            ..Default::default()
        };
        let mut mapper = ColorMapper::new(vec![
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
        ]);
        let out = mapper.map(&frame, &settings);
        assert_eq!(out[0].rgb, [65535, 0, 0]);
        assert_eq!(out[1].rgb, [0, 0, 65535]);
    }

    #[test]
    fn smoothing_converges() {
        let settings = SyncSettings {
            smoothing: 0.5,
            saturation: 1.0,
            ..Default::default()
        };
        let mut mapper = ColorMapper::new(vec![Zone {
            channel_id: 0,
            x: 0.5,
            y: 0.5,
        }]);
        let first = mapper.map(&solid([255, 255, 255]), &settings)[0].rgb[0];
        let mut last = first;
        for _ in 0..20 {
            last = mapper.map(&solid([255, 255, 255]), &settings)[0].rgb[0];
        }
        assert!(first < last);
        assert_eq!(last, 65535);
    }

    /// A 20 × 10 frame: `bars` black columns on each side, then the picture in `fill`.
    fn pillarboxed(bars: u32, fill: [u8; 3]) -> Frame {
        let (width, height) = (20, 10);
        let data = (0..height)
            .flat_map(|_| {
                (0..width).map(move |x| {
                    if x < bars || x >= width - bars {
                        [0; 3]
                    } else {
                        fill
                    }
                })
            })
            .flatten()
            .collect();
        Frame {
            width,
            height,
            data,
        }
    }

    fn left_edge() -> ColorMapper {
        ColorMapper::new(vec![Zone {
            channel_id: 0,
            x: 0.0,
            y: 0.5,
        }])
    }

    fn bar_settings() -> SyncSettings {
        SyncSettings {
            fps: 5,
            smoothing: 0.0,
            saturation: 1.0,
            region_size: 0.1,
            ..Default::default()
        }
    }

    /// Frames the default hold takes at `bar_settings`' frame rate.
    const HOLD: usize = 10;

    #[test]
    fn side_bars_are_cut_once_they_stay() {
        let settings = bar_settings();
        let mut mapper = left_edge();
        let frame = pillarboxed(3, [255, 0, 0]);
        for _ in 1..HOLD {
            assert_eq!(mapper.map(&frame, &settings)[0].rgb, [0; 3]);
        }
        assert_eq!(mapper.map(&frame, &settings)[0].rgb, [65535, 0, 0]);
    }

    #[test]
    fn bars_beyond_the_limit_stay() {
        let settings = SyncSettings {
            black_bars: 0.1,
            ..bar_settings()
        };
        let mut mapper = left_edge();
        for _ in 0..HOLD * 2 {
            assert_eq!(
                mapper.map(&pillarboxed(3, [255, 0, 0]), &settings)[0].rgb,
                [0; 3]
            );
        }
    }

    #[test]
    fn bars_hold_through_dark_scenes_and_one_sided_content() {
        let settings = bar_settings();
        let mut mapper = left_edge();
        for _ in 0..HOLD {
            mapper.map(&pillarboxed(3, [255, 0, 0]), &settings);
        }
        assert_eq!(mapper.bars_x.current, 3);
        // A black frame says nothing about the bars.
        mapper.map(&pillarboxed(0, [0; 3]), &settings);
        let mut subtitle = pillarboxed(3, [255, 0, 0]);
        subtitle.data[..3].copy_from_slice(&[255; 3]);
        mapper.map(&subtitle, &settings);
        assert_eq!(mapper.bars_x.current, 3);
        // Picture on both sides: the bars are gone at once.
        mapper.map(&pillarboxed(0, [0, 0, 255]), &settings);
        assert_eq!(mapper.bars_x.current, 0);
    }

    #[test]
    fn channel_position_maps_to_screen() {
        let channel = Channel {
            channel_id: 1,
            position: crate::hue::Position {
                x: -1.0,
                y: 0.0,
                z: 1.0,
            },
            members: Vec::new(),
        };
        let zone = Zone::from_channel(&channel);
        assert_eq!((zone.x, zone.y), (0.0, 0.0));
    }
}
