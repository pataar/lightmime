//! Philips Hue bridge: REST (CLIP v2) and Entertainment streaming.

pub mod api;
pub mod stream;

pub use api::{
    Bridge, Channel, Credentials, EntertainmentConfiguration, PairError, Position, pair,
};
pub use stream::{ChannelColor, EntertainmentStream};
