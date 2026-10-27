//! Core of lightmime: screen capture, colour extraction and Hue Entertainment
//! streaming.
//!
//! The crate is frontend-agnostic. The `lightmime` CLI, daemon and window (`lightmime-gui`)
//! all drive it through [`Engine`].

pub mod capture;
pub mod color;
pub mod config;
pub mod engine;
pub mod hue;

pub use config::{BridgeConfig, CaptureConfig, Config, SourceKind, SyncSettings};
pub use engine::{Engine, EngineState, Status};
