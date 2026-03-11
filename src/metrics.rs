use crate::store::PoolBalances;
use crate::AppState;
use anyhow::Result;
use axum::{extract::State, response::IntoResponse, routing::get, Router};
use prometheus::{Encoder, GaugeVec, Gauge, Opts, Registry, TextEncoder};
use std::sync::Arc;

/// All Prometheus metrics for zcash-sentinel.
pub struct SentinelMetrics {
    pub registry: Registry,
    /// Balance per account per pool in zatoshis.
    pub balance_zatoshis: GaugeVec,
    /// Total balance per account in ZEC.
    pub total_balance_zec: GaugeVec,
    /// Last synced block height per account.
    pub sync_height: GaugeVec,
    /// Chain tip height from lightwalletd.
    pub chain_height: Gauge,
    /// Blocks behind chain tip per account.
    pub sync_lag_blocks: GaugeVec,
    /// Duration of last sync cycle in seconds.
    pub last_sync_duration_seconds: Gauge,
    /// Number of monitored accounts.
    pub watched_accounts_total: Gauge,
}

impl SentinelMetrics {
    pub fn new() -> Result<Self> {
        let registry = Registry::new();

        let balance_zatoshis = GaugeVec::new(
            Opts::new(
                "zcash_balance_zatoshis",
                "Balance per account per pool in zatoshis",
            ),
            &["account", "pool"],
        )?;
        registry.register(Box::new(balance_zatoshis.clone()))?;

        let total_balance_zec = GaugeVec::new(
            Opts::new(
                "zcash_total_balance_zec",
                "Total balance per account in ZEC",
            ),
            &["account"],
        )?;
        registry.register(Box::new(total_balance_zec.clone()))?;

        let sync_height = GaugeVec::new(
            Opts::new(
                "zcash_sync_height",
                "Last synced block height per account",
            ),
            &["account"],
        )?;
        registry.register(Box::new(sync_height.clone()))?;

        let chain_height = Gauge::new(
            "zcash_chain_height",
            "Chain tip height from lightwalletd",
        )?;
        registry.register(Box::new(chain_height.clone()))?;

        let sync_lag_blocks = GaugeVec::new(
            Opts::new(
                "zcash_sync_lag_blocks",
                "Number of blocks behind chain tip per account",
            ),
            &["account"],
        )?;
        registry.register(Box::new(sync_lag_blocks.clone()))?;

        let last_sync_duration_seconds = Gauge::new(
            "zcash_last_sync_duration_seconds",
            "Duration of the last sync cycle in seconds",
        )?;
        registry.register(Box::new(last_sync_duration_seconds.clone()))?;

        let watched_accounts_total = Gauge::new(
            "zcash_watched_accounts_total",
            "Total number of monitored accounts",
        )?;
        registry.register(Box::new(watched_accounts_total.clone()))?;

        Ok(SentinelMetrics {
            registry,
            balance_zatoshis,
            total_balance_zec,
            sync_height,
            chain_height,
            sync_lag_blocks,
            last_sync_duration_seconds,
            watched_accounts_total,
        })
    }

    /// Update all metrics for a given account after a successful sync.
    pub fn update_account_balance(
        &self,
        label: &str,
        balances: &PoolBalances,
        synced_height: u64,
    ) {
        // Per-pool balances in zatoshis
        self.balance_zatoshis
            .with_label_values(&[label, "transparent"])
            .set(balances.transparent as f64);
        self.balance_zatoshis
            .with_label_values(&[label, "sapling"])
            .set(balances.sapling as f64);
        self.balance_zatoshis
            .with_label_values(&[label, "orchard"])
            .set(balances.orchard as f64);

        // Total balance in ZEC (1 ZEC = 100_000_000 zatoshis)
        let total_zec = balances.total() as f64 / 100_000_000.0;
        self.total_balance_zec
            .with_label_values(&[label])
            .set(total_zec);

        // Sync height
        self.sync_height
            .with_label_values(&[label])
            .set(synced_height as f64);
    }

    /// Remove all metrics for an account (when it's deleted).
    pub fn remove_account_metrics(&self, label: &str) {
        let _ = self.balance_zatoshis.remove_label_values(&[label, "transparent"]);
        let _ = self.balance_zatoshis.remove_label_values(&[label, "sapling"]);
        let _ = self.balance_zatoshis.remove_label_values(&[label, "orchard"]);
        let _ = self.total_balance_zec.remove_label_values(&[label]);
        let _ = self.sync_height.remove_label_values(&[label]);
        let _ = self.sync_lag_blocks.remove_label_values(&[label]);
    }
}

/// Handler for GET /metrics - returns Prometheus text format.
async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = state.metrics.registry.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        buffer,
    )
}

/// Start the Prometheus metrics HTTP server.
pub async fn serve_metrics(bind_addr: &str, state: Arc<AppState>) -> Result<()> {
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!(addr = %bind_addr, "Metrics server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
