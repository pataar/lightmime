//! Starting lightmime in the tray at login.
//!
//! A Flatpak asks the Background portal, which writes the autostart entry on
//! the host; a native install writes `~/.config/autostart` itself. Either way
//! the entry ends up as `~/.config/autostart/<APP_ID>.desktop`, which is how
//! the current state is read back (the Flatpak has read-only access to it).

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use ashpd::desktop::background::Background;

use crate::APP_ID;
use crate::icons::APP_ICON_NAME;

const ARGS: [&str; 2] = ["gui", "--hidden"];

fn entry_path() -> Option<PathBuf> {
    // Inside the sandbox XDG_CONFIG_HOME points into ~/.var/app, so go
    // through $HOME to reach the host's directory.
    let config = if ashpd::is_sandboxed() {
        dirs::home_dir()?.join(".config")
    } else {
        dirs::config_dir()?
    };
    Some(config.join("autostart").join(format!("{APP_ID}.desktop")))
}

pub fn is_enabled() -> bool {
    entry_path().is_some_and(|path| path.exists())
}

pub async fn set(enabled: bool) -> Result<()> {
    if ashpd::is_sandboxed() {
        let mut command = vec!["lightmime"];
        command.extend(ARGS);
        let response = Background::request()
            .reason("Start lightmime in the tray when you log in")
            .auto_start(enabled)
            .command(command)
            .dbus_activatable(false)
            .send()
            .await
            .context("asking the Background portal")?
            .response()
            .context("the desktop did not allow running in the background")?;
        if response.auto_start() != enabled {
            bail!("the desktop did not change the login item");
        }
        return Ok(());
    }

    let path = entry_path().ok_or_else(|| anyhow!("no config directory"))?;
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("removing {}", path.display()))
            }
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().context("finding the lightmime binary")?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=lightmime\nComment=Sync Hue lights with the screen\n\
         Exec=\"{}\" {}\nIcon={APP_ICON_NAME}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display(),
        ARGS.join(" ")
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, entry).with_context(|| format!("writing {}", path.display()))
}
