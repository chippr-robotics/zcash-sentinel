use crate::store::PoolBalances;
use crate::AppState;
use crate::Config;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::{error, info, warn};
use zingolib::config::{ChainType, ZingoConfig};
use zingolib::lightclient::LightClient;
use zingolib::wallet::{LightWallet, WalletBase};

/// A single value transfer (transaction detail) serializable to JSON.
#[derive(Clone, serde::Serialize)]
pub struct TransactionDetail {
    pub txid: String,
    pub datetime: u32,
    pub status: String,
    pub blockheight: u64,
    pub kind: String,
    pub value: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_received: Option<String>,
    pub memos: Vec<String>,
}

/// Scanner manages zingolib light clients for each watched account.
pub struct Scanner {
    config: Config,
    /// Mapping from account label to an initialized LightClient.
    clients: std::collections::HashMap<String, LightClient>,
    /// Base directory for wallet data (each account gets a subdirectory).
    data_dir: PathBuf,
}

impl Scanner {
    pub fn new(config: Config) -> Self {
        let data_dir = PathBuf::from(&config.storage.accounts_file)
            .parent()
            .unwrap_or(std::path::Path::new("/var/lib/zcash-sentinel"))
            .join("wallets");
        Scanner {
            config,
            clients: std::collections::HashMap::new(),
            data_dir,
        }
    }

    /// Initialize a zingolib LightClient for a UFVK (watch-only).
    /// The client connects to lightwalletd and can sync from the given birthday height.
    pub fn init_client(
        &mut self,
        label: &str,
        viewing_key: &str,
        birthday_height: u64,
    ) -> anyhow::Result<()> {
        if self.clients.contains_key(label) {
            return Ok(());
        }

        info!(
            label = %label,
            birthday = birthday_height,
            "Initializing light client for account"
        );

        let server_uri: http::Uri = self.config.lightwalletd.endpoint.parse()?;
        let wallet_dir = self.data_dir.join(label);
        std::fs::create_dir_all(&wallet_dir)?;

        let zingo_config = ZingoConfig::build(ChainType::Mainnet)
            .set_lightwalletd_uri(server_uri)
            .set_wallet_dir(wallet_dir)
            .create();

        let birthday = zcash_protocol::consensus::BlockHeight::from_u32(birthday_height as u32);

        // Create a watch-only wallet from the UFVK
        let wallet = LightWallet::new(
            zingo_config.chain,
            WalletBase::Ufvk(viewing_key.to_string()),
            birthday,
            zingo_config.wallet_settings.clone(),
        )?;

        let client = LightClient::create_from_wallet(wallet, zingo_config, true)?;

        self.clients.insert(label.to_string(), client);
        info!(label = %label, "Light client initialized");
        Ok(())
    }

    /// Remove a light client for an account.
    pub fn remove_client(&mut self, label: &str) {
        self.clients.remove(label);
    }

    /// Sync a specific account and return updated balances.
    pub async fn sync_account(
        &mut self,
        label: &str,
    ) -> anyhow::Result<(PoolBalances, u64)> {
        let client = self
            .clients
            .get_mut(label)
            .ok_or_else(|| anyhow::anyhow!("No client for account '{}'", label))?;

        // Perform sync with lightwalletd and wait for completion
        let sync_result = client.sync_and_await().await?;
        let synced_height: u64 = sync_result.sync_end_height.into();

        // Get balance breakdown for the default account (AccountId::ZERO)
        let balance = client
            .account_balance(zip32::AccountId::ZERO)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get balance: {:?}", e))?;

        let pool_balances = PoolBalances {
            transparent: balance
                .total_transparent_balance
                .map_or(0, |z| z.into_u64()),
            sapling: balance
                .total_sapling_balance
                .map_or(0, |z| z.into_u64()),
            orchard: balance
                .total_orchard_balance
                .map_or(0, |z| z.into_u64()),
        };

        Ok((pool_balances, synced_height))
    }

    /// Get value transfers (transactions with memos) for an account.
    /// Returns them sorted newest-first.
    pub async fn get_transactions(
        &self,
        label: &str,
    ) -> anyhow::Result<Vec<TransactionDetail>> {
        let client = self
            .clients
            .get(label)
            .ok_or_else(|| anyhow::anyhow!("No client for account '{}'", label))?;

        let transfers = client
            .value_transfers(true)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get value transfers: {:?}", e))?;

        let details: Vec<TransactionDetail> = transfers
            .iter()
            .map(|vt| TransactionDetail {
                txid: vt.txid.to_string(),
                datetime: vt.datetime,
                status: vt.status.to_string(),
                blockheight: u64::from(vt.blockheight),
                kind: vt.kind.to_string(),
                value: vt.value,
                fee: vt.transaction_fee,
                recipient_address: vt.recipient_address.clone(),
                pool_received: vt.pool_received.clone(),
                memos: vt.memos.clone(),
            })
            .collect();

        Ok(details)
    }

    /// Get the current chain tip height from lightwalletd via the do_info JSON output.
    pub async fn get_chain_height(&self) -> anyhow::Result<u64> {
        if let Some(client) = self.clients.values().next() {
            // do_info returns a JSON string containing chain height info
            let info_str = client.do_info().await;
            // Parse the JSON to extract the block height
            if let Ok(info) = serde_json::from_str::<serde_json::Value>(&info_str) {
                if let Some(height) = info.get("block_height").and_then(|v| v.as_u64()) {
                    return Ok(height);
                }
            }
            Ok(0)
        } else {
            Ok(0)
        }
    }
}

/// Main scanner loop: periodically syncs all watched accounts and updates metrics.
pub async fn run_scanner_loop(state: Arc<AppState>) {
    let interval = Duration::from_secs(state.config.scanner.poll_interval_secs);
    let mut ticker = time::interval(interval);

    // On first tick, initialize clients for all persisted accounts
    {
        let store = state.store.read().await;
        let mut scanner = state.scanner.write().await;
        for (label, account) in &store.accounts {
            if let Err(e) =
                scanner.init_client(label, &account.viewing_key, account.birthday_height)
            {
                error!(label = %label, error = %e, "Failed to init client for persisted account");
            }
        }
    }

    loop {
        ticker.tick().await;
        info!("Starting sync cycle");

        let sync_start = std::time::Instant::now();

        // Get the list of accounts to sync
        let account_labels: Vec<String> = {
            let store = state.store.read().await;
            store.accounts.keys().cloned().collect()
        };

        // Sync each account (requires &mut self, so we hold write lock)
        for label in &account_labels {
            let result = {
                let mut scanner = state.scanner.write().await;
                scanner.sync_account(label).await
            };

            match result {
                Ok((balances, synced_height)) => {
                    info!(
                        label = %label,
                        transparent = balances.transparent,
                        sapling = balances.sapling,
                        orchard = balances.orchard,
                        height = synced_height,
                        "Account synced"
                    );

                    // Update metrics
                    state
                        .metrics
                        .update_account_balance(label, &balances, synced_height);

                    // Persist to store
                    let mut store = state.store.write().await;
                    if let Err(e) =
                        store.update_account_balances(label, balances, synced_height)
                    {
                        error!(label = %label, error = %e, "Failed to persist balances");
                    }
                }
                Err(e) => {
                    warn!(label = %label, error = %e, "Failed to sync account");
                }
            }
        }

        // Update chain height metric
        {
            let scanner = state.scanner.read().await;
            match scanner.get_chain_height().await {
                Ok(height) if height > 0 => {
                    state.metrics.chain_height.set(height as f64);

                    // Update sync lag for each account
                    let store = state.store.read().await;
                    for (label, account) in &store.accounts {
                        let lag = height.saturating_sub(account.last_synced_height);
                        state
                            .metrics
                            .sync_lag_blocks
                            .with_label_values(&[label])
                            .set(lag as f64);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(error = %e, "Failed to get chain height");
                }
            }
        }

        let sync_duration = sync_start.elapsed();
        state
            .metrics
            .last_sync_duration_seconds
            .set(sync_duration.as_secs_f64());

        info!(
            duration_secs = sync_duration.as_secs_f64(),
            accounts = account_labels.len(),
            "Sync cycle complete"
        );
    }
}
