//! The daemon's config file: the core config plus daemon-only sections.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lightmime_core::Config;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    #[serde(flatten)]
    pub core: Config,
    pub mqtt: MqttConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MqttConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Home Assistant's discovery prefix.
    pub discovery_prefix: String,
    pub base_topic: String,
    /// Identifies this machine in topics and in Home Assistant. Defaults to
    /// the hostname.
    pub node_id: String,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 1883,
            username: String::new(),
            password: String::new(),
            discovery_prefix: "homeassistant".into(),
            base_topic: "lightmime".into(),
            node_id: String::new(),
        }
    }
}

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lightmime/config.toml")
}

/// Where the screen-cast portal restore token is kept between runs.
pub fn restore_token_path() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lightmime/restore_token")
}

impl AppConfig {
    /// Load the config, or defaults when the file does not exist yet.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Write the config with owner-only permissions; it holds bridge and MQTT
    /// credentials.
    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serialising config")?;
        let tmp = path.with_extension("toml.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }
}
