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
    /// Average colour of a rectangle given in normalised coordinates (0..1),
    /// as linear-light RGB in 0..1.
    fn average(&self, cx: f32, cy: f32, size: f32) -> [f32; 3] {
        let (w, h) = (self.width as f32, self.height as f32);
        let half = size / 2.0;
        let x0 = (((cx - half).max(0.0)) * w) as u32;
        let x1 = ((((cx + half).min(1.0)) * w).ceil() as u32).clamp(x0 + 1, self.width);
        let y0 = (((cy - half).max(0.0)) * h) as u32;
        let y1 = ((((cy + half).min(1.0)) * h).ceil() as u32).clamp(y0 + 1, self.height);

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
}

impl ColorMapper {
    pub fn new(zones: Vec<Zone>) -> Self {
        let smoothed = vec![[0.0; 3]; zones.len()];
        Self { zones, smoothed }
    }

    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }

    pub fn map(&mut self, frame: &Frame, settings: &SyncSettings) -> Vec<ChannelColor> {
        let alpha = 1.0 - settings.smoothing;
        self.zones
            .iter()
            .zip(self.smoothed.iter_mut())
            .map(|(zone, previous)| {
                let sampled = saturate(
                    frame.average(zone.x, zone.y, settings.region_size),
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
    rgb.map(|c| (to_u16(c / peak) as f32 * brightness).round() as u16)
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
        assert!((out[0].rgb[2] as i32 - 128 * 257).abs() < 300);
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
