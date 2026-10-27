//! Home Assistant integration over MQTT discovery: a sync switch, sliders for
//! the live settings, and status sensors.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use lightmime_core::{Engine, EngineState, Status, SyncSettings};
use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, QoS};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::settings::{AppConfig, MqttConfig};

/// The fps sensor changes every second; publishing that often only fills the
/// recorder.
const FPS_INTERVAL: Duration = Duration::from_secs(10);

/// A slider exposed to Home Assistant, shown as a percentage.
struct Slider {
    key: &'static str,
    name: &'static str,
    icon: &'static str,
    max: f32,
    get: fn(&SyncSettings) -> f32,
    set: fn(&mut SyncSettings, f32),
}

const SLIDERS: &[Slider] = &[
    Slider {
        key: "brightness",
        name: "Brightness",
        icon: "mdi:brightness-6",
        max: 100.0,
        get: |s| s.brightness,
        set: |s, v| s.brightness = v,
    },
    Slider {
        key: "smoothing",
        name: "Smoothing",
        icon: "mdi:blur",
        max: 95.0,
        get: |s| s.smoothing,
        set: |s, v| s.smoothing = v,
    },
    Slider {
        key: "saturation",
        name: "Saturation",
        icon: "mdi:palette",
        max: 300.0,
        get: |s| s.saturation,
        set: |s, v| s.saturation = v,
    },
];

pub struct MqttBridge {
    config: MqttConfig,
    node: String,
    engine: Engine,
    /// Settings changed from Home Assistant are written back here.
    config_path: PathBuf,
}

impl MqttBridge {
    pub fn new(config: MqttConfig, engine: Engine, config_path: PathBuf) -> Self {
        let node = if config.node_id.is_empty() {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "lightmime".into())
        } else {
            config.node_id.clone()
        };
        let node = node
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        Self {
            config,
            node,
            engine,
            config_path,
        }
    }

    fn topic(&self, suffix: &str) -> String {
        format!("{}/{}/{suffix}", self.config.base_topic, self.node)
    }

    /// Run until `shutdown` flips to true, then mark the device offline.
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<()> {
        let mut options = MqttOptions::new(
            format!("lightmime-{}", self.node),
            &self.config.host,
            self.config.port,
        );
        options.set_keep_alive(Duration::from_secs(30));
        if !self.config.username.is_empty() {
            options.set_credentials(&self.config.username, &self.config.password);
        }
        options.set_last_will(LastWill::new(
            self.topic("availability"),
            "offline",
            QoS::AtLeastOnce,
            true,
        ));
        let (client, mut events) = AsyncClient::new(options, 32);

        let mut status_rx = self.engine.subscribe();
        let mut last_fps_publish = Instant::now() - FPS_INTERVAL;
        tracing::info!(
            "connecting to MQTT broker {}:{}",
            self.config.host,
            self.config.port
        );

        loop {
            tokio::select! {
                event = events.poll() => match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        tracing::info!("connected to MQTT");
                        self.announce(&client).await?;
                    }
                    Ok(Event::Incoming(Incoming::Publish(message))) => {
                        let payload = String::from_utf8_lossy(&message.payload).trim().to_owned();
                        self.handle_command(&client, &message.topic, &payload).await?;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("MQTT connection error: {e}; retrying in 5 s");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                },
                () = async { let _ = shutdown.wait_for(|stop| *stop).await; } => {
                    let status = self.engine.status();
                    self.publish_status(&client, &status, true).await?;
                    client
                        .publish(self.topic("availability"), QoS::AtLeastOnce, true, "offline")
                        .await?;
                    client.disconnect().await?;
                    // Drive the event loop until the disconnect has gone out.
                    let flush = async { while events.poll().await.is_ok() {} };
                    let _ = tokio::time::timeout(Duration::from_secs(2), flush).await;
                    break;
                }
                changed = status_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let status = status_rx.borrow_and_update().clone();
                    let publish_fps = last_fps_publish.elapsed() >= FPS_INTERVAL
                        || status.state != EngineState::Streaming;
                    self.publish_status(&client, &status, publish_fps).await?;
                    if publish_fps {
                        last_fps_publish = Instant::now();
                    }
                }
            }
        }
        Ok(())
    }

    /// Publish discovery configs, subscribe to commands and push current state.
    /// Runs on every (re)connect so Home Assistant restarts are handled too.
    async fn announce(&self, client: &AsyncClient) -> Result<()> {
        let device = json!({
            "identifiers": [format!("lightmime_{}", self.node)],
            "name": format!("lightmime ({})", self.node),
            "manufacturer": "lightmime",
            "model": "Hue Entertainment screen sync",
            "sw_version": env!("CARGO_PKG_VERSION"),
        });
        let availability = self.topic("availability");

        let mut entities: Vec<(&str, &str, Value)> = vec![
            (
                "switch",
                "sync",
                json!({
                    "name": "Sync",
                    "icon": "mdi:television-ambient-light",
                    "state_topic": self.topic("sync/state"),
                    "command_topic": self.topic("sync/set"),
                }),
            ),
            (
                "sensor",
                "status",
                json!({
                    "name": "Status",
                    "icon": "mdi:information-outline",
                    "state_topic": self.topic("status/state"),
                    "json_attributes_topic": self.topic("status/attributes"),
                    "device_class": "enum",
                    "options": ["idle", "starting", "streaming", "error"],
                }),
            ),
            (
                "sensor",
                "fps",
                json!({
                    "name": "Frame rate",
                    "icon": "mdi:speedometer",
                    "state_topic": self.topic("fps/state"),
                    "unit_of_measurement": "fps",
                    "state_class": "measurement",
                    "entity_category": "diagnostic",
                }),
            ),
        ];
        for slider in SLIDERS {
            entities.push((
                "number",
                slider.key,
                json!({
                    "name": slider.name,
                    "icon": slider.icon,
                    "state_topic": self.topic(&format!("{}/state", slider.key)),
                    "command_topic": self.topic(&format!("{}/set", slider.key)),
                    "min": 0,
                    "max": slider.max,
                    "step": 1,
                    "unit_of_measurement": "%",
                    "mode": "slider",
                    "entity_category": "config",
                }),
            ));
        }

        for (component, key, mut config) in entities {
            let object = config.as_object_mut().expect("entity config is an object");
            object.insert(
                "unique_id".into(),
                json!(format!("lightmime_{}_{key}", self.node)),
            );
            object.insert(
                "object_id".into(),
                json!(format!("lightmime_{}_{key}", self.node)),
            );
            object.insert("availability_topic".into(), json!(availability));
            object.insert("device".into(), device.clone());
            let topic = format!(
                "{}/{component}/lightmime_{}/{key}/config",
                self.config.discovery_prefix, self.node
            );
            client
                .publish(topic, QoS::AtLeastOnce, true, config.to_string())
                .await?;
        }

        client
            .subscribe(self.topic("+/set"), QoS::AtLeastOnce)
            .await?;
        // Re-announce when Home Assistant restarts.
        client
            .subscribe(
                format!("{}/status", self.config.discovery_prefix),
                QoS::AtLeastOnce,
            )
            .await?;
        client
            .publish(availability, QoS::AtLeastOnce, true, "online")
            .await?;
        self.publish_settings(client).await?;
        self.publish_status(client, &self.engine.status(), true)
            .await
    }

    async fn handle_command(&self, client: &AsyncClient, topic: &str, payload: &str) -> Result<()> {
        if topic == format!("{}/status", self.config.discovery_prefix) {
            if payload == "online" {
                self.announce(client).await?;
            }
            return Ok(());
        }
        let Some(key) = topic
            .strip_prefix(&self.topic(""))
            .and_then(|rest| rest.strip_suffix("/set"))
        else {
            return Ok(());
        };

        if key == "sync" {
            match payload {
                "ON" => self.engine.start(),
                "OFF" => self.engine.stop(),
                other => tracing::warn!("ignoring sync command {other:?}"),
            }
            return Ok(());
        }
        let Some(slider) = SLIDERS.iter().find(|s| s.key == key) else {
            return Ok(());
        };
        let Ok(percent) = payload.parse::<f32>() else {
            tracing::warn!("ignoring non-numeric {key} value {payload:?}");
            return Ok(());
        };
        self.engine
            .update_settings(|s| (slider.set)(s, percent / 100.0));
        self.persist_settings();
        self.publish_settings(client).await
    }

    async fn publish_settings(&self, client: &AsyncClient) -> Result<()> {
        let settings = self.engine.settings();
        for slider in SLIDERS {
            let value = ((slider.get)(&settings) * 100.0).round();
            client
                .publish(
                    self.topic(&format!("{}/state", slider.key)),
                    QoS::AtLeastOnce,
                    true,
                    value.to_string(),
                )
                .await?;
        }
        Ok(())
    }

    async fn publish_status(
        &self,
        client: &AsyncClient,
        status: &Status,
        with_fps: bool,
    ) -> Result<()> {
        let syncing = matches!(status.state, EngineState::Starting | EngineState::Streaming);
        client
            .publish(
                self.topic("sync/state"),
                QoS::AtLeastOnce,
                true,
                if syncing { "ON" } else { "OFF" },
            )
            .await?;
        client
            .publish(
                self.topic("status/state"),
                QoS::AtLeastOnce,
                true,
                status.state.label(),
            )
            .await?;
        let error = match &status.state {
            EngineState::Error(message) => Some(message.as_str()),
            _ => None,
        };
        let attributes =
            json!({ "area": status.area, "channels": status.channels, "error": error });
        client
            .publish(
                self.topic("status/attributes"),
                QoS::AtLeastOnce,
                true,
                attributes.to_string(),
            )
            .await?;
        if with_fps {
            client
                .publish(
                    self.topic("fps/state"),
                    QoS::AtMostOnce,
                    false,
                    status.fps.to_string(),
                )
                .await?;
        }
        Ok(())
    }

    /// Keep slider changes across restarts.
    fn persist_settings(&self) {
        let result = AppConfig::load(&self.config_path).and_then(|mut config| {
            config.core.sync = self.engine.settings();
            config.save(&self.config_path)
        });
        if let Err(e) = result {
            tracing::warn!("could not save settings: {e:#}");
        }
    }
}
