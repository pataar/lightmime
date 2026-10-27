//! The smallest frontend: pair if needed, pick the first entertainment area,
//! sync until Ctrl-C.
//!
//! ```sh
//! cargo run -p lightmime-core --example minimal -- <bridge-ip>                       # pairs, prints the keys
//! cargo run -p lightmime-core --example minimal -- <bridge-ip> <app-key> <client-key>
//! ```

use std::time::Duration;

use anyhow::{Context, Result};
use lightmime_core::hue::{self, Bridge};
use lightmime_core::{BridgeConfig, Config, Engine, EngineState};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let host = args
        .next()
        .context("usage: minimal <bridge-ip> [app-key client-key]")?;
    let (app_key, client_key) = match (args.next(), args.next()) {
        (Some(app_key), Some(client_key)) => (app_key, client_key),
        _ => {
            println!("Press the link button on the bridge...");
            let credentials = hue::pair_waiting(&host, "lightmime-example").await?;
            println!("Paired: {} {}", credentials.app_key, credentials.client_key);
            (credentials.app_key, credentials.client_key)
        }
    };

    let areas = Bridge::new(&host, &app_key)?
        .entertainment_configurations()
        .await?;
    let area = areas
        .first()
        .context("no entertainment areas on this bridge")?;
    println!("Streaming to {:?}", area.metadata.name);

    let config = Config {
        bridge: BridgeConfig {
            host,
            app_key,
            client_key,
            entertainment_config_id: area.id.clone(),
        },
        ..Config::default()
    };
    // No restore token path: the portal asks for a screen on every run.
    let engine = Engine::spawn(config, None);
    engine.update_settings(|s| s.brightness = 0.8);
    engine.start();

    let mut status = engine.subscribe();
    loop {
        tokio::select! {
            changed = status.changed() => {
                changed?;
                let status = status.borrow_and_update().clone();
                match status.state {
                    EngineState::Error(e) => anyhow::bail!("sync failed: {e}"),
                    state => println!("{} at {:.0} fps", state.label(), status.fps),
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    // Wait for the engine to release the area, or the lights stay in streaming mode.
    engine.stop();
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        status.wait_for(|s| !s.state.is_active()),
    )
    .await;
    Ok(())
}
