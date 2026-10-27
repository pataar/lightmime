//! Hue bridge REST API: pairing and entertainment configurations.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::json;

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
