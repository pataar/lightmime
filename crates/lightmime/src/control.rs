//! Control of a running instance over the session bus.
//!
//! `lightmime gui` and `lightmime run` own [`BUS_NAME`] and serve [`Control`]
//! on it, so `lightmime start|stop|toggle|show|status` can drive whichever is
//! running: from a compositor keybinding or a status bar module (Hyprland and
//! Waybar, for instance). Owning the name also keeps a second instance from
//! streaming to the same area: a second `lightmime gui` brings the running
//! window forward instead.
//!
//! The name is the app id, which a Flatpak may own without extra permissions.

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use lightmime_core::{Engine, Status};
use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;
use zbus::fdo::{DBusProxy, PropertiesProxy, RequestNameFlags, RequestNameReply};
use zbus::names::{BusName, InterfaceName, WellKnownName};
use zbus::{Connection, interface, proxy};

/// Same as the GUI's app id (`lightmime_gui::APP_ID`); also the Flatpak id.
pub const BUS_NAME: &str = "io.github.pataar.lightmime";
pub const OBJECT_PATH: &str = "/io/github/pataar/lightmime";
/// Versioned so the interface can change incompatibly later without
/// breaking scripts written against this one.
pub const INTERFACE: &str = "io.github.pataar.lightmime1";

/// Outcome of trying to become the running instance.
pub enum Claim {
    /// This process owns the name and serves [`Control`]; keep the
    /// connection alive for as long as it should.
    Owned(Connection),
    /// Another instance owns it.
    Taken,
    /// No session bus (a bare TTY, a misconfigured service): carry on without
    /// remote control.
    NoBus,
}

/// Serve the control interface for `engine` and try to own [`BUS_NAME`].
/// `show` opens the window; `None` when running headless.
pub async fn claim(engine: Engine, show: Option<UnboundedSender<()>>) -> Claim {
    let conn = match Connection::session().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::warn!("no session bus, `lightmime toggle` and friends will not work: {e}");
            return Claim::NoBus;
        }
    };
    let mut status = engine.subscribe();
    let control = Control {
        status: status.borrow().clone(),
        engine,
        show,
    };
    // Serve before owning the name, so no call to it finds nothing there.
    if let Err(e) = conn.object_server().at(OBJECT_PATH, control).await {
        tracing::warn!("could not serve the control interface: {e}");
        return Claim::NoBus;
    }
    match conn
        .request_name_with_flags(BUS_NAME, RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
        Ok(_) | Err(zbus::Error::NameTaken) => return Claim::Taken,
        Err(e) => {
            tracing::warn!("could not own {BUS_NAME} on the session bus: {e}");
            return Claim::NoBus;
        }
    }

    let iface = match conn
        .object_server()
        .interface::<_, Control>(OBJECT_PATH)
        .await
    {
        Ok(iface) => iface,
        Err(e) => {
            tracing::warn!("control interface missing after serving it: {e}");
            return Claim::Owned(conn);
        }
    };
    tokio::spawn(async move {
        while status.changed().await.is_ok() {
            let next = status.borrow_and_update().clone();
            let mut control = iface.get_mut().await;
            let before = std::mem::replace(&mut control.status, next);
            let after = &control.status;
            let emitter = iface.signal_emitter();
            // Fps changes every frame; only what a status bar shows is signalled.
            let mut result = Ok(());
            if before.state.label() != after.state.label() {
                result = result.and(control.state_changed(emitter).await);
            }
            if before.state.error() != after.state.error() {
                result = result.and(control.error_changed(emitter).await);
            }
            if before.area != after.area {
                result = result.and(control.area_changed(emitter).await);
            }
            if before.channels != after.channels {
                result = result.and(control.channels_changed(emitter).await);
            }
            if let Err(e) = result {
                tracing::debug!("signalling a status change: {e}");
            }
        }
    });
    Claim::Owned(conn)
}

struct Control {
    engine: Engine,
    /// Last status seen, to signal only what changed.
    status: Status,
    show: Option<UnboundedSender<()>>,
}

#[interface(name = "io.github.pataar.lightmime1")]
impl Control {
    /// Start syncing; does nothing when already syncing.
    fn start(&self) {
        self.engine.start();
    }

    /// Stop syncing and release the entertainment area.
    fn stop(&self) {
        self.engine.stop();
    }

    /// Stop when starting or syncing, start otherwise (also after an error).
    fn toggle(&self) {
        if self.engine.status().state.is_active() {
            self.engine.stop();
        } else {
            self.engine.start();
        }
    }

    /// Open the window or bring it to the front.
    fn show(&self) -> zbus::fdo::Result<()> {
        let show = self.show.as_ref().ok_or_else(|| {
            zbus::fdo::Error::NotSupported(
                "lightmime is running headless (`lightmime run`); there is no window".into(),
            )
        })?;
        show.send(())
            .map_err(|_| zbus::fdo::Error::Failed("the window is shutting down".into()))
    }

    /// `idle`, `starting`, `streaming` or `error`.
    #[zbus(property)]
    fn state(&self) -> String {
        self.status.state.label().into()
    }

    /// Why syncing stopped, while `State` is `error`; empty otherwise.
    #[zbus(property)]
    fn error(&self) -> String {
        self.status.state.error().unwrap_or_default().into()
    }

    /// Name of the entertainment area being synced.
    #[zbus(property)]
    fn area(&self) -> String {
        self.status.area.clone()
    }

    #[zbus(property)]
    fn channels(&self) -> u32 {
        self.status.channels as u32
    }

    /// Frames sent per second, read on request; not signalled.
    #[zbus(property(emits_changed_signal = "false"))]
    fn fps(&self) -> f64 {
        f64::from(self.engine.status().fps)
    }
}

#[proxy(
    interface = "io.github.pataar.lightmime1",
    default_service = "io.github.pataar.lightmime",
    default_path = "/io/github/pataar/lightmime"
)]
trait Remote {
    fn start(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
    fn toggle(&self) -> zbus::Result<()>;
    fn show(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn error(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn area(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn channels(&self) -> zbus::Result<u32>;
}

/// A method call on the running instance, for the CLI.
#[derive(Clone, Copy)]
pub enum Request {
    Start,
    Stop,
    Toggle,
    Show,
}

pub async fn send(request: Request) -> Result<()> {
    let conn = Connection::session()
        .await
        .context("connecting to the session bus")?;
    let remote = remote(&conn).await?;
    if !is_running(&conn).await? {
        bail!("lightmime is not running; start it with `lightmime gui` or `lightmime run`");
    }
    match request {
        Request::Start => remote.start().await,
        Request::Stop => remote.stop().await,
        Request::Toggle => remote.toggle().await,
        Request::Show => remote.show().await,
    }
    .map_err(|e| match e {
        zbus::Error::MethodError(_, Some(message), _) => anyhow!(message),
        e => anyhow!(e),
    })
}

async fn remote(conn: &Connection) -> Result<RemoteProxy<'static>> {
    // Uncached: `status` reads fresh values after the owner changes.
    Ok(RemoteProxy::builder(conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?)
}

async fn is_running(conn: &Connection) -> Result<bool> {
    let name = BusName::try_from(BUS_NAME)?;
    Ok(DBusProxy::new(conn).await?.name_has_owner(name).await?)
}

/// What `lightmime status` reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// `off` when no instance runs, else the engine state.
    pub state: String,
    pub error: String,
    pub area: String,
    pub channels: u32,
}

impl Snapshot {
    fn off() -> Self {
        Self {
            state: "off".into(),
            error: String::new(),
            area: String::new(),
            channels: 0,
        }
    }

    /// One line for people.
    pub fn summary(&self) -> String {
        match self.state.as_str() {
            "off" => "lightmime is not running".into(),
            "idle" => "Not syncing".into(),
            "starting" => "Starting…".into(),
            "streaming" if self.area.is_empty() => "Syncing".into(),
            "streaming" => format!("Syncing to {} ({} channels)", self.area, self.channels),
            "error" => format!("Stopped after an error: {}", self.error),
            other => other.into(),
        }
    }

    /// A Waybar custom module line (`"return-type": "json"`). `alt` and
    /// `class` carry the state, so `format-icons` and CSS can key on it.
    pub fn waybar(&self) -> String {
        #[derive(Serialize)]
        struct Waybar<'a> {
            text: &'a str,
            alt: &'a str,
            class: &'a str,
            tooltip: String,
        }
        serde_json::to_string(&Waybar {
            text: &self.state,
            alt: &self.state,
            class: &self.state,
            tooltip: self.summary(),
        })
        .expect("serializing plain strings")
    }
}

async fn snapshot(conn: &Connection, remote: &RemoteProxy<'_>) -> Snapshot {
    if !is_running(conn).await.unwrap_or(false) {
        return Snapshot::off();
    }
    let read = async {
        Ok::<_, zbus::Error>(Snapshot {
            state: remote.state().await?,
            error: remote.error().await?,
            area: remote.area().await?,
            channels: remote.channels().await?,
        })
    };
    // The instance can quit between the two calls.
    read.await.unwrap_or_else(|_| Snapshot::off())
}

/// Print the status once, or, with `follow`, again on every change until
/// killed (including while no instance runs, for a status bar).
pub async fn status(follow: bool, print: impl Fn(&Snapshot)) -> Result<()> {
    let conn = Connection::session()
        .await
        .context("connecting to the session bus")?;
    let remote = remote(&conn).await?;
    if !follow {
        print(&snapshot(&conn, &remote).await);
        return Ok(());
    }

    let name = WellKnownName::try_from(BUS_NAME)?;
    let mut owners = DBusProxy::new(&conn)
        .await?
        .receive_name_owner_changed_with_args(&[(0, BUS_NAME)])
        .await?;
    let mut changes = PropertiesProxy::builder(&conn)
        .destination(name)?
        .path(OBJECT_PATH)?
        .build()
        .await?
        .receive_properties_changed()
        .await?;
    let interface = InterfaceName::try_from(INTERFACE)?;

    let mut last = None;
    loop {
        let now = snapshot(&conn, &remote).await;
        if last.as_ref() != Some(&now) {
            print(&now);
            last = Some(now);
        }
        // Wait for the owner to change or one of our properties to.
        loop {
            tokio::select! {
                owner = owners.next() => match owner {
                    None => bail!("lost the session bus"),
                    Some(_) => break,
                },
                change = changes.next() => match change {
                    None => bail!("lost the session bus"),
                    Some(change) => {
                        if change.args().is_ok_and(|args| args.interface_name == interface) {
                            break;
                        }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waybar_line_keys_on_state() {
        let snapshot = Snapshot {
            state: "streaming".into(),
            error: String::new(),
            area: "Living \"room\"".into(),
            channels: 3,
        };
        let line: serde_json::Value = serde_json::from_str(&snapshot.waybar()).unwrap();
        assert_eq!(line["alt"], "streaming");
        assert_eq!(line["class"], "streaming");
        assert_eq!(line["tooltip"], "Syncing to Living \"room\" (3 channels)");
        assert!(!snapshot.waybar().contains('\n'));
    }

    #[test]
    fn off_when_not_running() {
        assert_eq!(Snapshot::off().summary(), "lightmime is not running");
    }
}
