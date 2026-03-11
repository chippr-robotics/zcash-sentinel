mod api;
mod metrics;
mod scanner;
mod store;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "zcash-watchman", about = "Zcash balance monitoring service")]
struct Cli {
    /// Path to configuration file
    #[arg(long, short, default_value = "/etc/zcash-watchman/config.toml")]
    config: PathBuf,
}

#[derive(serde::Deserialize, Clone)]
pub struct Config {
    pub lightwalletd: LightwalletdConfig,
    pub server: ServerConfig,
    pub scanner: ScannerConfig,
    pub storage: StorageConfig,
}

#[derive(serde::Deserialize, Clone)]
pub struct LightwalletdConfig {
    pub endpoint: String,
}

#[derive(serde::Deserialize, Clone)]
pub struct ServerConfig {
    pub metrics_bind: String,
    pub api_bind: String,
}

#[derive(serde::Deserialize, Clone)]
pub struct ScannerConfig {
    pub poll_interval_secs: u64,
    pub default_birthday_height: u64,
}

#[derive(serde::Deserialize, Clone)]
pub struct StorageConfig {
    pub accounts_file: String,
}

/// Shared application state accessible by all components.
pub struct AppState {
    pub config: Config,
    pub store: RwLock<store::AccountStore>,
    pub scanner: RwLock<scanner::Scanner>,
    pub metrics: metrics::WatchmanMetrics,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let cli = Cli::parse();

    // Load configuration
    let config_str = std::fs::read_to_string(&cli.config)
        .map_err(|e| anyhow::anyhow!("Failed to read config file {:?}: {}", cli.config, e))?;
    let config: Config = toml::from_str(&config_str)?;

    info!(
        endpoint = %config.lightwalletd.endpoint,
        poll_interval = config.scanner.poll_interval_secs,
        "Starting zcash-watchman"
    );

    // Load persisted accounts
    let store = store::AccountStore::load(&config.storage.accounts_file)?;
    let account_count = store.accounts.len() + store.addresses.len();
    info!(accounts = account_count, "Loaded persisted accounts");

    // Initialize metrics
    let watchman_metrics = metrics::WatchmanMetrics::new()?;
    watchman_metrics.watched_accounts_total.set(account_count as f64);

    // Initialize scanner
    let scanner = scanner::Scanner::new(config.clone());

    // Build shared state
    let state = Arc::new(AppState {
        config: config.clone(),
        store: RwLock::new(store),
        scanner: RwLock::new(scanner),
        metrics: watchman_metrics,
    });

    // Spawn the metrics HTTP server
    let metrics_state = Arc::clone(&state);
    let metrics_bind = config.server.metrics_bind.clone();
    let metrics_handle = tokio::spawn(async move {
        if let Err(e) = metrics::serve_metrics(&metrics_bind, metrics_state).await {
            error!(error = %e, "Metrics server failed");
        }
    });

    // Spawn the management API server
    let api_state = Arc::clone(&state);
    let api_bind = config.server.api_bind.clone();
    let api_handle = tokio::spawn(async move {
        if let Err(e) = api::serve_api(&api_bind, api_state).await {
            error!(error = %e, "API server failed");
        }
    });

    // Spawn the scanner loop
    let scanner_state = Arc::clone(&state);
    let scanner_handle = tokio::spawn(async move {
        scanner::run_scanner_loop(scanner_state).await;
    });

    info!(
        metrics = %config.server.metrics_bind,
        api = %config.server.api_bind,
        "zcash-watchman is running"
    );

    // Wait for any task to finish (they should all run indefinitely)
    tokio::select! {
        r = metrics_handle => {
            error!(?r, "Metrics server exited unexpectedly");
        }
        r = api_handle => {
            error!(?r, "API server exited unexpectedly");
        }
        r = scanner_handle => {
            error!(?r, "Scanner loop exited unexpectedly");
        }
    }

    Ok(())
}
