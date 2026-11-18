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

/// How far the bridge setup has got. Each step needs the one before it:
/// pair, then choose an entertainment area, then sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    /// No bridge credentials yet.
    Unpaired,
    /// Paired, but no entertainment area chosen.
    NoArea,
    /// Everything syncing needs is there.
    Ready,
}

impl BridgeConfig {
    pub fn is_paired(&self) -> bool {
        !self.host.is_empty() && !self.app_key.is_empty() && !self.client_key.is_empty()
    }

    pub fn setup(&self) -> Setup {
        if !self.is_paired() {
            Setup::Unpaired
        } else if self.entertainment_config_id.is_empty() {
            Setup::NoArea
        } else {
            Setup::Ready
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// A whole screen, chosen in the portal's screen picker.
    Monitor,
    /// A single window, chosen in the portal's screen picker.
    Window,
    /// gamescope's own PipeWire stream, without the portal: Game Mode on the
    /// Steam Deck and other gamescope sessions, which have no ScreenCast portal.
    Gamescope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    /// What to capture: what the screen-cast portal offers, or gamescope's stream.
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

/// Upper bound for [`SyncSettings::smoothing`].
pub const MAX_SMOOTHING: f32 = 0.95;
/// Upper bound for [`SyncSettings::saturation`].
pub const MAX_SATURATION: f32 = 3.0;
/// Upper bound for [`SyncSettings::black_bars`]: 4:3 on a 32:9 screen leaves 36 % on each side.
pub const MAX_BLACK_BARS: f32 = 0.4;

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
    /// Black bars up to this fraction of each edge are left out of the picture the lights
    /// sample, so letterboxed or pillarboxed video reaches the outer lights; 0 turns it off.
    pub black_bars: f32,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            fps: 30,
            brightness: 1.0,
            smoothing: 0.3,
            saturation: 1.2,
            region_size: 0.3,
            black_bars: 0.25,
        }
    }
}

impl SyncSettings {
    /// Clamp every value to its valid range.
    #[must_use = "sanitized returns the clamped settings; it does not change them in place"]
    pub fn sanitized(mut self) -> Self {
        // NaN passes through `clamp`, poisons the smoothed colours and
        // survives a round trip through the config file, so reset it.
        let defaults = Self::default();
        let finite = |v: f32, default: f32| if v.is_finite() { v } else { default };
        self.fps = self.fps.clamp(1, MAX_FPS);
        self.brightness = finite(self.brightness, defaults.brightness).clamp(0.0, 1.0);
        self.smoothing = finite(self.smoothing, defaults.smoothing).clamp(0.0, MAX_SMOOTHING);
        self.saturation = finite(self.saturation, defaults.saturation).clamp(0.0, MAX_SATURATION);
        self.region_size = finite(self.region_size, defaults.region_size).clamp(0.05, 1.0);
        self.black_bars = finite(self.black_bars, defaults.black_bars).clamp(0.0, MAX_BLACK_BARS);
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
            black_bars: f32::NAN,
            ..SyncSettings::default()
        }
        .sanitized();
        let defaults = SyncSettings::default();
        assert_eq!(settings.brightness, defaults.brightness);
        assert_eq!(settings.smoothing, defaults.smoothing);
        assert_eq!(settings.saturation, defaults.saturation);
        assert_eq!(settings.region_size, defaults.region_size);
        assert_eq!(settings.black_bars, defaults.black_bars);
    }

    #[test]
    fn capture_source_parses_gamescope() {
        let config: CaptureConfig =
            serde_json::from_str(r#"{"source": "gamescope"}"#).expect("valid config");
        assert_eq!(config.source, SourceKind::Gamescope);
    }

    #[test]
    fn setup_goes_pair_then_area() {
        let mut bridge = BridgeConfig::default();
        assert_eq!(bridge.setup(), Setup::Unpaired);
        bridge.host = "192.168.1.20".into();
        bridge.app_key = "key".into();
        bridge.client_key = "00".into();
        assert_eq!(bridge.setup(), Setup::NoArea);
        bridge.entertainment_config_id = "area".into();
        assert_eq!(bridge.setup(), Setup::Ready);
        bridge.app_key.clear();
        assert_eq!(bridge.setup(), Setup::Unpaired);
    }
}
