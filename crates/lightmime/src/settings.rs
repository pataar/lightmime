//! The daemon's config file: the core config plus daemon-only sections.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

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
    /// Connect over TLS (usually port 8883).
    pub tls: bool,
    /// PEM file with the CA that signed the broker's certificate. Empty means
    /// the system's trusted CAs.
    pub ca_file: String,
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
            tls: false,
            ca_file: String::new(),
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
    /// credentials. Only values that differ from the file on disk are
    /// written, so the user's comments, layout and unknown keys survive.
    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let current = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let text = self
            .merge_into(&current)
            .with_context(|| format!("updating {}", path.display()))?;
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

    /// `current` (the file's text) with this config's values written into it.
    fn merge_into(&self, current: &str) -> Result<String> {
        let mut doc: toml_edit::DocumentMut = current.parse().context("parsing")?;
        let on_disk: Self = toml::from_str(current).context("parsing")?;
        // Compared as toml's own output: going through `toml::Value` would
        // widen f32 to f64 and write 0.8 as 0.800000011920929.
        let serialise = |config: &Self| -> Result<toml_edit::DocumentMut> {
            let text = toml::to_string(config).context("serialising config")?;
            Ok(text.parse()?)
        };
        let old = serialise(&on_disk)?;
        let new = serialise(self)?;
        merge(doc.as_table_mut(), old.as_table(), new.as_table())?;
        Ok(doc.to_string())
    }

    /// Load, change and save the config under a process-wide lock, so the
    /// window and MQTT, which share a process, don't overwrite each other's
    /// changes or the temporary file.
    pub fn update(path: &Path, change: impl FnOnce(&mut Self)) -> Result<()> {
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let mut config = Self::load(path)?;
        change(&mut config);
        config.save(path)
    }
}

/// Writes the keys that changed from `old` to `new` into `doc`. Keys whose
/// value did not change are left alone, as are their comments, and a changed
/// value keeps the comments around it.
fn merge(
    doc: &mut dyn toml_edit::TableLike,
    old: &dyn toml_edit::TableLike,
    new: &dyn toml_edit::TableLike,
) -> Result<()> {
    let empty = toml_edit::Table::new();
    for (key, item) in new.iter() {
        let before = old.get(key);
        if let Some(table) = item.as_table_like() {
            let before = before
                .and_then(toml_edit::Item::as_table_like)
                .unwrap_or(&empty);
            let created = !doc.contains_key(key);
            let sub = doc
                .entry(key)
                .or_insert_with(toml_edit::table)
                .as_table_like_mut()
                .ok_or_else(|| anyhow::anyhow!("`{key}` is not a table"))?;
            merge(sub, before, table)?;
            // Sections that still hold only defaults stay out of the file.
            if created && sub.is_empty() {
                doc.remove(key);
            }
            continue;
        }
        let Some(value) = item.as_value() else {
            continue;
        };
        if before.and_then(toml_edit::Item::as_value).map(bare) == Some(bare(value)) {
            continue;
        }
        match doc.get_mut(key) {
            // Replacing the value in place keeps the key and the comment above it.
            Some(toml_edit::Item::Value(existing)) => {
                let decor = existing.decor().clone();
                *existing = value.clone();
                *existing.decor_mut() = decor;
            }
            _ => {
                let mut value = value.clone();
                value.decor_mut().clear();
                doc.insert(key, toml_edit::Item::Value(value));
            }
        }
    }
    let removed: Vec<_> = old
        .iter()
        .map(|(key, _)| key.to_owned())
        .filter(|key| !new.contains_key(key))
        .collect();
    for key in removed {
        doc.remove(&key);
    }
    Ok(())
}

/// A value as TOML text without the whitespace and comments around it.
fn bare(value: &toml_edit::Value) -> String {
    let mut value = value.clone();
    value.decor_mut().clear();
    value.to_string()
}

/// Saves what the window changes into the config file, keeping the sections
/// it does not know about.
#[cfg(feature = "gui")]
pub struct FileStore(pub PathBuf);

#[cfg(feature = "gui")]
impl lightmime_gui::Store for FileStore {
    fn save_bridge(&self, bridge: &lightmime_core::BridgeConfig) -> Result<()> {
        AppConfig::update(&self.0, |config| config.core.bridge = bridge.clone())
    }

    fn save_sync(&self, sync: &lightmime_core::SyncSettings) -> Result<()> {
        AppConfig::update(&self.0, |config| config.core.sync = sync.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"# My Hue setup
[bridge]
host = "192.168.1.2" # living room
app_key = "key"

[sync]
# dimmer at night
brightness = 0.5
fps = 30

[future]
unknown = true
"#;

    #[test]
    fn save_keeps_comments_and_unknown_keys() {
        let mut config: AppConfig = toml::from_str(FILE).unwrap();
        config.core.sync.brightness = 0.8;
        config.core.bridge.entertainment_config_id = "area".into();
        let text = config.merge_into(FILE).unwrap();
        assert_eq!(
            text,
            r#"# My Hue setup
[bridge]
host = "192.168.1.2" # living room
app_key = "key"
entertainment_config_id = "area"

[sync]
# dimmer at night
brightness = 0.8
fps = 30

[future]
unknown = true
"#
        );
        let reloaded: AppConfig = toml::from_str(&text).unwrap();
        assert_eq!(reloaded.core.sync.brightness, 0.8);
    }

    #[test]
    fn save_without_changes_leaves_the_file_alone() {
        let config: AppConfig = toml::from_str(FILE).unwrap();
        assert_eq!(config.merge_into(FILE).unwrap(), FILE);
    }

    #[test]
    fn save_keeps_the_example_config_intact() {
        let example = include_str!("../../../contrib/config.example.toml");
        let mut config: AppConfig = toml::from_str(example).unwrap();
        assert_eq!(config.merge_into(example).unwrap(), example);
        config.core.sync.smoothing = 0.1;
        let text = config.merge_into(example).unwrap();
        assert_eq!(text.lines().count(), example.lines().count());
        let reloaded: AppConfig = toml::from_str(&text).unwrap();
        assert_eq!(reloaded.core.sync.smoothing, 0.1);
    }

    #[test]
    fn save_to_a_new_file_writes_only_what_differs_from_defaults() {
        let mut config = AppConfig::default();
        config.core.bridge.host = "hue".into();
        assert_eq!(config.merge_into("").unwrap(), "[bridge]\nhost = \"hue\"\n");
    }
}
