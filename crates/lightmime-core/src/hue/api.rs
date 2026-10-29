//! Hue bridge REST API: pairing and entertainment configurations.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::json;

use super::ChannelColor;
use crate::color::xy_to_rgb;

/// Error returned by [`pair`] while the bridge's link button has not been pressed.
#[derive(Debug, thiserror::Error)]
pub enum PairError {
    #[error("press the link button on the bridge")]
    LinkButtonNotPressed,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Credentials returned by a successful pairing.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub app_key: String,
    pub client_key: String,
}

/// How long [`pair_waiting`] keeps asking before giving up.
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(60);

/// [`pair`], retried until the link button is pressed or [`PAIR_TIMEOUT`]
/// passes.
pub async fn pair_waiting(host: &str, device_type: &str) -> Result<Credentials> {
    let deadline = Instant::now() + PAIR_TIMEOUT;
    loop {
        match pair(host, device_type).await {
            Ok(credentials) => return Ok(credentials),
            Err(PairError::LinkButtonNotPressed) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(PairError::LinkButtonNotPressed) => {
                bail!("the link button was not pressed within a minute")
            }
            Err(PairError::Other(e)) => return Err(e),
        }
    }
}

/// Register a new application on the bridge. Fails with
/// [`PairError::LinkButtonNotPressed`] until the link button is pressed, so
/// callers retry for a while.
pub async fn pair(host: &str, device_type: &str) -> Result<Credentials, PairError> {
    let response: serde_json::Value = http_client()?
        .post(format!("https://{host}/api"))
        .json(&json!({ "devicetype": device_type, "generateclientkey": true }))
        .send()
        .await
        .context("contacting the bridge")?
        .json()
        .await
        .context("reading the pairing response")?;

    let entry = response
        .get(0)
        .ok_or_else(|| anyhow!("unexpected pairing response: {response}"))?;
    if let Some(error) = entry.get("error") {
        // Error type 101: link button not pressed.
        if error.get("type").and_then(|t| t.as_u64()) == Some(101) {
            return Err(PairError::LinkButtonNotPressed);
        }
        return Err(anyhow!("bridge refused pairing: {error}").into());
    }
    let success = entry
        .get("success")
        .ok_or_else(|| anyhow!("unexpected pairing response: {response}"))?;
    let field = |name: &str| {
        success
            .get(name)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("pairing response is missing `{name}`"))
    };
    Ok(Credentials {
        app_key: field("username")?,
        client_key: field("clientkey")?,
    })
}

/// A bridge reported by Signify's discovery service.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveredBridge {
    pub id: String,
    #[serde(rename = "internalipaddress")]
    pub host: String,
}

/// Ask Signify's discovery service (`discovery.meethue.com`) for the bridges
/// that registered from this network's public address. The service is rate
/// limited, so call it on request rather than in a loop.
pub async fn discover() -> Result<Vec<DiscoveredBridge>> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("building HTTP client")?
        .get("https://discovery.meethue.com/")
        .send()
        .await
        .context("contacting discovery.meethue.com")?
        .error_for_status()
        .context("discovery.meethue.com refused the request")?
        .json()
        .await
        .context("reading the discovery response")
}

/// The bridge serves a certificate signed by Signify's own CA, so normal
/// verification fails; traffic stays on the local network.
fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("building HTTP client")
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Position {
    /// Left (-1) to right (1).
    pub x: f64,
    /// Back (-1) to front (1).
    pub y: f64,
    /// Bottom (-1) to top (1).
    pub z: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    pub channel_id: u8,
    #[serde(default)]
    pub position: Position,
    /// Entertainment services this channel drives: one per light, or per segment of a gradient light.
    #[serde(default)]
    pub members: Vec<ChannelMember>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelMember {
    pub service: ResourceRef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResourceRef {
    pub rid: String,
}

/// Any resource with an owning device, such as an entertainment service.
#[derive(Deserialize)]
struct Owned {
    id: String,
    owner: ResourceRef,
}

#[derive(Deserialize)]
struct Light {
    owner: ResourceRef,
    on: LightOn,
    dimming: Option<Dimming>,
    color: Option<LightColor>,
}

#[derive(Deserialize)]
struct LightOn {
    on: bool,
}

#[derive(Deserialize)]
struct Dimming {
    /// Percent, 0..100.
    brightness: f32,
}

#[derive(Deserialize)]
struct LightColor {
    xy: Xy,
}

#[derive(Deserialize)]
struct Xy {
    x: f32,
    y: f32,
}

impl Light {
    fn rgb(&self) -> [u16; 3] {
        if !self.on.on {
            return [0; 3];
        }
        let brightness = self.dimming.as_ref().map_or(1.0, |d| d.brightness / 100.0);
        // D65 white for lights that report no colour.
        let (x, y) = self
            .color
            .as_ref()
            .map_or((0.3127, 0.3290), |c| (c.xy.x, c.xy.y));
        xy_to_rgb(x, y, brightness)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntertainmentConfiguration {
    pub id: String,
    #[serde(default)]
    pub metadata: Metadata,
    /// "active" while some application is streaming.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Metadata {
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize)]
struct ClipResponse<T> {
    #[serde(default)]
    errors: Vec<ClipError>,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

#[derive(Deserialize)]
struct ClipError {
    description: String,
}

/// An authenticated connection to a paired bridge.
#[derive(Clone)]
pub struct Bridge {
    host: String,
    app_key: String,
    client: reqwest::Client,
}

impl Bridge {
    pub fn new(host: &str, app_key: &str) -> Result<Self> {
        Ok(Self {
            host: host.to_owned(),
            app_key: app_key.to_owned(),
            client: http_client()?,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// The PSK identity for DTLS. Newer bridges expect the application id from
    /// `/auth/v1`; older firmware accepts the application key.
    pub async fn psk_identity(&self) -> String {
        let id = self
            .client
            .get(format!("https://{}/auth/v1", self.host))
            .header("hue-application-key", &self.app_key)
            .send()
            .await
            .ok()
            .and_then(|r| {
                r.headers()
                    .get("hue-application-id")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            });
        id.unwrap_or_else(|| self.app_key.clone())
    }

    pub async fn entertainment_configurations(&self) -> Result<Vec<EntertainmentConfiguration>> {
        let response: ClipResponse<EntertainmentConfiguration> = self
            .client
            .get(self.resource_url(""))
            .header("hue-application-key", &self.app_key)
            .send()
            .await
            .context("listing entertainment areas")?
            .json()
            .await
            .context("parsing entertainment areas")?;
        if let Some(error) = response.errors.first() {
            bail!("bridge error: {}", error.description);
        }
        Ok(response.data)
    }

    pub async fn entertainment_configuration(
        &self,
        id: &str,
    ) -> Result<EntertainmentConfiguration> {
        self.entertainment_configurations()
            .await?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| anyhow!("entertainment area {id} not found on the bridge"))
    }

    /// Start or stop streaming mode for an entertainment area. The DTLS
    /// connection is only accepted while the area is started.
    pub async fn set_streaming(&self, id: &str, active: bool) -> Result<()> {
        let action = if active { "start" } else { "stop" };
        let response: ClipResponse<serde_json::Value> = self
            .client
            .put(self.resource_url(id))
            .header("hue-application-key", &self.app_key)
            .json(&json!({ "action": action }))
            .send()
            .await
            .with_context(|| format!("sending {action} to the entertainment area"))?
            .json()
            .await
            .context("parsing bridge response")?;
        if let Some(error) = response.errors.first() {
            bail!("bridge refused {action}: {}", error.description);
        }
        Ok(())
    }

    /// What each channel's light shows outside streaming (black when off), in the area's
    /// channel order. Every segment of a gradient light gets the light's one colour.
    pub async fn channel_colors(
        &self,
        area: &EntertainmentConfiguration,
    ) -> Result<Vec<ChannelColor>> {
        let services: Vec<Owned> = self.resources("entertainment").await?;
        let lights: Vec<Light> = self.resources("light").await?;
        Ok(area
            .channels
            .iter()
            .map(|channel| {
                let light = channel
                    .members
                    .first()
                    .and_then(|member| services.iter().find(|s| s.id == member.service.rid))
                    .and_then(|service| lights.iter().find(|l| l.owner.rid == service.owner.rid));
                ChannelColor {
                    channel_id: channel.channel_id,
                    rgb: light.map_or([0; 3], Light::rgb),
                }
            })
            .collect())
    }

    async fn resources<T: serde::de::DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        let response: ClipResponse<T> = self
            .client
            .get(format!("https://{}/clip/v2/resource/{kind}", self.host))
            .header("hue-application-key", &self.app_key)
            .send()
            .await
            .with_context(|| format!("listing {kind} resources"))?
            .json()
            .await
            .with_context(|| format!("parsing {kind} resources"))?;
        if let Some(error) = response.errors.first() {
            bail!("bridge error: {}", error.description);
        }
        Ok(response.data)
    }

    fn resource_url(&self, id: &str) -> String {
        let base = format!(
            "https://{}/clip/v2/resource/entertainment_configuration",
            self.host
        );
        if id.is_empty() {
            base
        } else {
            format!("{base}/{id}")
        }
    }
}
