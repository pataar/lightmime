mod mqtt;
mod settings;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use lightmime_core::hue::{self, Bridge, ChannelColor, EntertainmentStream, PairError};
use lightmime_core::{Engine, EngineState};

use crate::settings::AppConfig;

/// Sync Philips Hue lights with your screen.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Config file [default: ~/.config/lightmime/config.toml]
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Pair with a Hue bridge (press its link button when asked).
    Pair {
        /// Bridge IP address or hostname.
        #[arg(long)]
        host: String,
    },
    /// Copy bridge credentials and the entertainment area from Lumux.
    ImportLumux,
    /// List the bridge's entertainment areas.
    Areas,
    /// Choose the entertainment area to stream to (id or name).
    UseArea { area: String },
    /// Stream a colour sweep for a few seconds, without screen capture, to
    /// check the bridge connection.
    Test {
        #[arg(long, default_value_t = 8)]
        seconds: u64,
    },
    /// Run headless: sync until stopped, controlled over MQTT (Home
    /// Assistant) when that is enabled.
    Run {
        /// Start syncing immediately instead of waiting for a command.
        #[arg(long)]
        start: bool,
    },
    /// Open the window and tray icon. Runs the MQTT bridge too when enabled.
    #[cfg(feature = "gui")]
    Gui {
        /// Start in the tray without opening the window.
        #[arg(long)]
        hidden: bool,
        /// Start syncing immediately.
        #[arg(long)]
        start: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(settings::default_config_path);
    let mut config = AppConfig::load(&config_path)?;

    match cli.command {
        Command::Pair { host } => pair(&mut config, &config_path, host).await,
        Command::ImportLumux => import_lumux(&mut config, &config_path),
        Command::Areas => areas(&config).await,
        Command::UseArea { area } => use_area(&mut config, &config_path, &area).await,
        Command::Test { seconds } => test(&config, seconds).await,
        Command::Run { start } => run(config, config_path, start, Frontend::Headless).await,
        #[cfg(feature = "gui")]
        Command::Gui { hidden, start } => {
            run(config, config_path, start, Frontend::Gui { hidden }).await
        }
    }
}

/// How this machine introduces itself to the bridge when pairing.
fn device_name() -> String {
    format!(
        "lightmime#{}",
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_default()
    )
}

async fn pair(config: &mut AppConfig, path: &Path, host: String) -> Result<()> {
    let device = device_name();
    println!("Press the link button on the Hue bridge (waiting 60 s)...");
    let deadline = Instant::now() + Duration::from_secs(60);
    let credentials = loop {
        match hue::pair(&host, &device).await {
            Ok(credentials) => break credentials,
            Err(PairError::LinkButtonNotPressed) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(PairError::LinkButtonNotPressed) => {
                bail!("the link button was not pressed in time")
            }
            Err(PairError::Other(e)) => return Err(e),
        }
    };
    config.core.bridge.host = host;
    config.core.bridge.app_key = credentials.app_key;
    config.core.bridge.client_key = credentials.client_key;
    config.save(path)?;
    println!("Paired. Saved to {}", path.display());
    println!("Next: `lightmime areas` and `lightmime use-area <name>`.");
    Ok(())
}

fn import_lumux(config: &mut AppConfig, path: &Path) -> Result<()> {
    let lumux = dirs::home_dir()
        .ok_or_else(|| anyhow!("no home directory"))?
        .join(".var/app/io.github.enginkirmaci.lumux/config/lumux/settings.json");
    let text =
        std::fs::read_to_string(&lumux).with_context(|| format!("reading {}", lumux.display()))?;
    let settings: serde_json::Value = serde_json::from_str(&text)?;
    let field = |name: &str| {
        settings["hue"][name]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("Lumux settings have no hue.{name}"))
    };
    let bridge = &mut config.core.bridge;
    bridge.host = field("bridge_ip")?;
    bridge.app_key = field("app_key")?;
    bridge.client_key = field("client_key")?;
    bridge.entertainment_config_id = field("entertainment_config_id").unwrap_or_default();
    config.save(path)?;
    println!(
        "Imported bridge {} from Lumux into {}",
        config.core.bridge.host,
        path.display()
    );
    Ok(())
}

fn bridge(config: &AppConfig) -> Result<Bridge> {
    let bridge = &config.core.bridge;
    if !bridge.is_paired() {
        bail!(
            "not paired yet: run `lightmime pair --host <bridge-ip>` or `lightmime import-lumux`"
        );
    }
    Bridge::new(&bridge.host, &bridge.app_key)
}

async fn areas(config: &AppConfig) -> Result<()> {
    let selected = &config.core.bridge.entertainment_config_id;
    for area in bridge(config)?.entertainment_configurations().await? {
        let marker = if &area.id == selected { "*" } else { " " };
        println!(
            "{marker} {}  {:<24} {} channels, {}",
            area.id,
            area.metadata.name,
            area.channels.len(),
            area.status
        );
    }
    Ok(())
}

async fn use_area(config: &mut AppConfig, path: &Path, wanted: &str) -> Result<()> {
    let areas = bridge(config)?.entertainment_configurations().await?;
    let area = areas
        .iter()
        .find(|a| a.id == wanted || a.metadata.name.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| anyhow!("no entertainment area called {wanted:?}; see `lightmime areas`"))?;
    config.core.bridge.entertainment_config_id = area.id.clone();
    config.save(path)?;
    println!("Using '{}' ({})", area.metadata.name, area.id);
    Ok(())
}

async fn test(config: &AppConfig, seconds: u64) -> Result<()> {
    let bridge = bridge(config)?;
    let area_id = config.core.bridge.entertainment_config_id.clone();
    if area_id.is_empty() {
        bail!("no entertainment area selected; run `lightmime use-area <name>`");
    }
    let area = bridge.entertainment_configuration(&area_id).await?;
    println!(
        "Starting '{}' ({} channels)",
        area.metadata.name,
        area.channels.len()
    );
    bridge.set_streaming(&area_id, true).await?;
    let identity = bridge.psk_identity().await;

    let host = config.core.bridge.host.clone();
    let client_key = config.core.bridge.client_key.clone();
    let channels: Vec<u8> = area.channels.iter().map(|c| c.channel_id).collect();
    let stream_area = area_id.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut stream = EntertainmentStream::connect(&host, &identity, &client_key, &stream_area)?;
        println!("DTLS connected, sweeping colours for {seconds} s");
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(seconds) {
            let t = started.elapsed().as_secs_f32();
            let colors: Vec<ChannelColor> = channels
                .iter()
                .enumerate()
                .map(|(i, &channel_id)| ChannelColor {
                    channel_id,
                    rgb: hue_to_rgb(t * 0.25 + i as f32 / channels.len().max(1) as f32),
                })
                .collect();
            stream.send(&colors)?;
            std::thread::sleep(Duration::from_millis(33));
        }
        stream.close();
        Ok(())
    })
    .await?;

    bridge.set_streaming(&area_id, false).await?;
    result?;
    println!("Done; the bridge connection works.");
    Ok(())
}

fn hue_to_rgb(h: f32) -> [u16; 3] {
    let h = h.fract() * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r, g, b].map(|v: f32| (v * 65535.0) as u16)
}

enum Frontend {
    /// Run until SIGINT or SIGTERM.
    Headless,
    /// Run until the window's tray menu (or the window, without a tray) quits.
    #[cfg(feature = "gui")]
    Gui { hidden: bool },
}

async fn run(
    config: AppConfig,
    config_path: PathBuf,
    start: bool,
    frontend: Frontend,
) -> Result<()> {
    let engine = Engine::spawn(config.core.clone(), Some(settings::restore_token_path()));

    let mut status = engine.subscribe();
    tokio::spawn(async move {
        let mut last = status.borrow().state.clone();
        while status.changed().await.is_ok() {
            let state = status.borrow_and_update().state.clone();
            if state != last {
                match &state {
                    EngineState::Error(e) => tracing::error!("sync error: {e}"),
                    other => tracing::info!("sync {}", other.label()),
                }
                last = state;
            }
        }
    });

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mqtt_task = if config.mqtt.enabled {
        let bridge =
            mqtt::MqttBridge::new(config.mqtt.clone(), engine.clone(), config_path.clone());
        Some(tokio::spawn(async move {
            if let Err(e) = bridge.run(shutdown_rx).await {
                tracing::error!("MQTT bridge stopped: {e:#}");
            }
        }))
    } else {
        if !start && matches!(frontend, Frontend::Headless) {
            tracing::warn!(
                "MQTT is disabled and --start was not given; nothing will start syncing"
            );
        }
        None
    };

    if start {
        engine.start();
    }

    match frontend {
        Frontend::Headless => shutdown_signal().await,
        #[cfg(feature = "gui")]
        Frontend::Gui { hidden } => {
            let options = lightmime_gui::Options {
                engine: engine.clone(),
                bridge: config.core.bridge.clone(),
                store: std::sync::Arc::new(settings::FileStore(config_path)),
                runtime: tokio::runtime::Handle::current(),
                device_name: device_name(),
                hidden,
                quit_signal: Box::pin(shutdown_signal()),
            };
            // gpui owns this thread until the app quits.
            tokio::task::block_in_place(|| lightmime_gui::run(options));
        }
    }
    tracing::info!("shutting down");
    engine.stop();
    // Give the engine time to stop the entertainment area on the bridge.
    let mut status = engine.subscribe();
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        status.wait_for(|s| !matches!(s.state, EngineState::Starting | EngineState::Streaming)),
    )
    .await;
    let _ = shutdown_tx.send(true);
    if let Some(task) = mqtt_task {
        let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
    }
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("installing SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}
