//! Philips Hue bridge: REST (CLIP v2) and Entertainment streaming.

pub mod api;
pub mod stream;

pub use api::{
    Bridge, Channel, Credentials, DiscoveredBridge, EntertainmentConfiguration, PairError,
    Position, discover, pair,
};
pub use stream::{ChannelColor, EntertainmentStream};
