//! The tray icon (StatusNotifierItem): sync on/off, open the window, quit.

use anyhow::{Context, Result};
use ksni::menu::StandardItem;
use ksni::{Handle, MenuItem, ToolTip, TrayMethods};
use lightmime_core::{Engine, EngineState, Status};
use tokio::sync::mpsc::UnboundedSender;

use crate::APP_ID;
use crate::icons::{TRAY_ICON_NAME, tray_icon_theme_path};

/// What the tray asks of the app; sync on/off goes to the engine directly.
pub enum Action {
    Show,
    Quit,
}

pub struct Tray {
    engine: Engine,
    status: Status,
    actions: UnboundedSender<Action>,
}

/// Register the tray icon and keep it in step with the engine. Fails when the
/// desktop has no StatusNotifierItem host (GNOME without an AppIndicator
/// extension, for one).
pub async fn spawn(engine: Engine, actions: UnboundedSender<Action>) -> Result<Handle<Tray>> {
    let mut status = engine.subscribe();
    let tray = Tray {
        status: status.borrow().clone(),
        engine,
        actions,
    };
    // In a sandbox the well-known name cannot be owned; the unique name works.
    let handle = tray
        .disable_dbus_name(ashpd::is_sandboxed())
        .spawn()
        .await
        .context("registering the tray icon")?;

    let updates = handle.clone();
    tokio::spawn(async move {
        while status.changed().await.is_ok() {
            let next = status.borrow_and_update().clone();
            if updates.update(|tray| tray.status = next).await.is_none() {
                break;
            }
        }
    });
    Ok(handle)
}

impl Tray {
    fn syncing(&self) -> bool {
        matches!(
            self.status.state,
            EngineState::Starting | EngineState::Streaming
        )
    }

    fn summary(&self) -> String {
        match &self.status.state {
            EngineState::Idle => "Not syncing".into(),
            EngineState::Starting => "Starting…".into(),
            EngineState::Streaming if self.status.area.is_empty() => "Syncing".into(),
            EngineState::Streaming => format!("Syncing to {}", self.status.area),
            EngineState::Error(_) => "Stopped after an error".into(),
        }
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        "lightmime".into()
    }

    fn icon_name(&self) -> String {
        // A Flatpak may only export icons named after its app id.
        if ashpd::is_sandboxed() {
            format!("{APP_ID}-symbolic")
        } else {
            TRAY_ICON_NAME.into()
        }
    }

    fn icon_theme_path(&self) -> String {
        tray_icon_theme_path()
    }

    fn tool_tip(&self) -> ToolTip {
        let description = match &self.status.state {
            EngineState::Error(message) => message.clone(),
            _ => String::new(),
        };
        ToolTip {
            title: format!("lightmime: {}", self.summary()),
            description,
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.actions.send(Action::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let syncing = self.syncing();
        vec![
            StandardItem {
                label: self.summary(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: if syncing {
                    "Stop syncing"
                } else {
                    "Start syncing"
                }
                .into(),
                activate: Box::new(move |tray: &mut Self| {
                    if syncing {
                        tray.engine.stop();
                    } else {
                        tray.engine.start();
                    }
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open lightmime".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.actions.send(Action::Show);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.actions.send(Action::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
