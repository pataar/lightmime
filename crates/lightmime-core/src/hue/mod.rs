//! Philips Hue bridge: REST (CLIP v2) and Entertainment streaming.

pub mod api;
pub mod stream;

pub use api::{
    Bridge, Channel, Credentials, DiscoveredBridge, EntertainmentConfiguration, PAIR_TIMEOUT,
    PairError, Position, discover, pair, pair_waiting,
};
pub use stream::{ChannelColor, EntertainmentStream};
