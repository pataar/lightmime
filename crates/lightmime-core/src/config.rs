//! Configuration types shared by every frontend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bridge: BridgeConfig,
    pub capture: CaptureConfig,
    pub sync: SyncSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BridgeConfig {
    /// IP address or hostname of the Hue bridge.
    pub host: String,
    /// Application key ("username") returned when pairing.
    pub app_key: String,
    /// 32-character hex client key returned when pairing; the DTLS PSK.
    pub client_key: String,
    /// Entertainment area to stream to.
    pub entertainment_config_id: String,
}

impl BridgeConfig {
    pub fn is_paired(&self) -> bool {
        !self.host.is_empty() && !self.app_key.is_empty() && !self.client_key.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Monitor,
    Window,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    /// What the screen-cast portal offers to share.
    pub source: SourceKind,
    /// Frames are downscaled to this size before colour extraction.
    pub width: u32,
    pub height: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            source: SourceKind::Monitor,
            width: 64,
            height: 36,
        }
    }
}

/// Upper bound for [`SyncSettings::fps`]; capture is capped at this rate too.
pub const MAX_FPS: u32 = 60;

/// Settings that can change while streaming.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncSettings {
    /// Frames per second sent to the bridge (the bridge handles ~25-50).
    pub fps: u32,
    /// Output brightness multiplier, 0.0-1.0.
    pub brightness: f32,
    /// Exponential smoothing, 0.0 (none) to 0.95 (very slow).
    pub smoothing: f32,
    /// Saturation multiplier; 1.0 keeps the captured colours.
    pub saturation: f32,
    /// Size of the sampled screen region per light, as a fraction of the frame.
    pub region_size: f32,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            fps: 30,
            brightness: 1.0,
            smoothing: 0.3,
            saturation: 1.2,
            region_size: 0.3,
        }
    }
}

impl SyncSettings {
    /// Clamp every value to its valid range.
    pub fn sanitized(mut self) -> Self {
        // NaN passes through `clamp`, poisons the smoothed colours and
        // survives a round trip through the config file, so reset it.
        let defaults = Self::default();
        let finite = |v: f32, default: f32| if v.is_finite() { v } else { default };
        self.fps = self.fps.clamp(1, MAX_FPS);
        self.brightness = finite(self.brightness, defaults.brightness).clamp(0.0, 1.0);
        self.smoothing = finite(self.smoothing, defaults.smoothing).clamp(0.0, 0.95);
        self.saturation = finite(self.saturation, defaults.saturation).clamp(0.0, 3.0);
        self.region_size = finite(self.region_size, defaults.region_size).clamp(0.05, 1.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitized_replaces_non_finite_values() {
        let settings = SyncSettings {
            brightness: f32::NAN,
            smoothing: f32::NAN,
            saturation: f32::INFINITY,
            region_size: f32::NEG_INFINITY,
            ..SyncSettings::default()
        }
        .sanitized();
        let defaults = SyncSettings::default();
        assert_eq!(settings.brightness, defaults.brightness);
        assert_eq!(settings.smoothing, defaults.smoothing);
        assert_eq!(settings.saturation, defaults.saturation);
        assert_eq!(settings.region_size, defaults.region_size);
    }
}
