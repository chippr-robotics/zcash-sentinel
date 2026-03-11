use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

/// A shielded account monitored via a viewing key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShieldedAccount {
    pub label: String,
    pub viewing_key: String,
    pub birthday_height: u64,
    /// Last successfully synced block height.
    #[serde(default)]
    pub last_synced_height: u64,
    /// Balances per pool in zatoshis.
    #[serde(default)]
    pub balances: PoolBalances,
}

/// A transparent address monitored by scanning compact blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransparentAddress {
    pub label: String,
    pub address: String,
    /// Last successfully synced block height.
    #[serde(default)]
    pub last_synced_height: u64,
    /// Balance in zatoshis.
    #[serde(default)]
    pub balance_zatoshis: u64,
}

/// Balance breakdown by pool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PoolBalances {
    pub transparent: u64,
    pub sapling: u64,
    pub orchard: u64,
}

impl PoolBalances {
    pub fn total(&self) -> u64 {
        self.transparent + self.sapling + self.orchard
    }
}

/// Persistent storage for all watched accounts and addresses.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountStore {
    #[serde(default)]
    pub accounts: HashMap<String, ShieldedAccount>,
    #[serde(default)]
    pub addresses: HashMap<String, TransparentAddress>,
    /// File path for persistence (not serialized).
    #[serde(skip)]
    pub file_path: String,
}

impl AccountStore {
    /// Load accounts from a JSON file, or return an empty store if the file doesn't exist.
    pub fn load(path: &str) -> Result<Self> {
        let file_path = path.to_string();

        if !Path::new(path).exists() {
            info!(path = %path, "No existing accounts file, starting fresh");
            return Ok(AccountStore {
                file_path,
                ..Default::default()
            });
        }

        let data = std::fs::read_to_string(path)?;
        let mut store: AccountStore = serde_json::from_str(&data).unwrap_or_else(|e| {
            warn!(error = %e, "Failed to parse accounts file, starting fresh");
            AccountStore::default()
        });
        store.file_path = file_path;
        Ok(store)
    }

    /// Persist the current state to the JSON file.
    pub fn save(&self) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(&self.file_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let data = serde_json::to_string_pretty(self)?;
        // Write atomically via temp file
        let tmp_path = format!("{}.tmp", self.file_path);
        std::fs::write(&tmp_path, &data)?;
        std::fs::rename(&tmp_path, &self.file_path)?;
        Ok(())
    }

    /// Add a shielded account. Returns error if label already exists.
    pub fn add_account(&mut self, account: ShieldedAccount) -> Result<()> {
        if self.accounts.contains_key(&account.label) || self.addresses.contains_key(&account.label)
        {
            anyhow::bail!("Account with label '{}' already exists", account.label);
        }
        self.accounts.insert(account.label.clone(), account);
        self.save()
    }

    /// Add a transparent address. Returns error if label already exists.
    pub fn add_address(&mut self, addr: TransparentAddress) -> Result<()> {
        if self.accounts.contains_key(&addr.label) || self.addresses.contains_key(&addr.label) {
            anyhow::bail!("Account with label '{}' already exists", addr.label);
        }
        self.addresses.insert(addr.label.clone(), addr);
        self.save()
    }

    /// Remove an account or address by label.
    pub fn remove(&mut self, label: &str) -> Result<()> {
        let removed = self.accounts.remove(label).is_some() || self.addresses.remove(label).is_some();
        if !removed {
            anyhow::bail!("No account with label '{}'", label);
        }
        self.save()
    }

    /// Update balances for a shielded account.
    pub fn update_account_balances(
        &mut self,
        label: &str,
        balances: PoolBalances,
        synced_height: u64,
    ) -> Result<()> {
        if let Some(account) = self.accounts.get_mut(label) {
            account.balances = balances;
            account.last_synced_height = synced_height;
            self.save()?;
        }
        Ok(())
    }

    /// Update balance for a transparent address.
    pub fn update_address_balance(
        &mut self,
        label: &str,
        balance: u64,
        synced_height: u64,
    ) -> Result<()> {
        if let Some(addr) = self.addresses.get_mut(label) {
            addr.balance_zatoshis = balance;
            addr.last_synced_height = synced_height;
            self.save()?;
        }
        Ok(())
    }
}
