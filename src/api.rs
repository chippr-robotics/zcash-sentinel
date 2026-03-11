use crate::store::{PoolBalances, ShieldedAccount, TransparentAddress};
use crate::AppState;
use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

// --- Request/Response types ---

#[derive(Deserialize)]
pub struct AddAccountRequest {
    pub label: String,
    pub viewing_key: String,
    #[serde(default)]
    pub birthday_height: Option<u64>,
}

#[derive(Deserialize)]
pub struct AddAddressRequest {
    pub label: String,
    pub address: String,
}

#[derive(Serialize)]
pub struct AccountResponse {
    pub label: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub balances: BalanceResponse,
    pub last_synced_height: u64,
}

#[derive(Serialize)]
pub struct BalanceResponse {
    pub transparent_zatoshis: u64,
    pub sapling_zatoshis: u64,
    pub orchard_zatoshis: u64,
    pub total_zatoshis: u64,
    pub total_zec: f64,
}

impl From<&PoolBalances> for BalanceResponse {
    fn from(b: &PoolBalances) -> Self {
        BalanceResponse {
            transparent_zatoshis: b.transparent,
            sapling_zatoshis: b.sapling,
            orchard_zatoshis: b.orchard,
            total_zatoshis: b.total(),
            total_zec: b.total() as f64 / 100_000_000.0,
        }
    }
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub watched_accounts: usize,
    pub watched_addresses: usize,
    pub lightwalletd_endpoint: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// --- Handlers ---

/// GET /api/health
async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let store = state.store.read().await;
    Json(HealthResponse {
        status: "ok".to_string(),
        watched_accounts: store.accounts.len(),
        watched_addresses: store.addresses.len(),
        lightwalletd_endpoint: state.config.lightwalletd.endpoint.clone(),
    })
}

/// GET /api/accounts - list all monitored accounts with balances.
async fn list_accounts_handler(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<AccountResponse>> {
    let store = state.store.read().await;
    let mut result = Vec::new();

    for (_, account) in &store.accounts {
        result.push(AccountResponse {
            label: account.label.clone(),
            account_type: "shielded".to_string(),
            balances: BalanceResponse::from(&account.balances),
            last_synced_height: account.last_synced_height,
        });
    }

    for (_, addr) in &store.addresses {
        let balances = PoolBalances {
            transparent: addr.balance_zatoshis,
            sapling: 0,
            orchard: 0,
        };
        result.push(AccountResponse {
            label: addr.label.clone(),
            account_type: "transparent".to_string(),
            balances: BalanceResponse::from(&balances),
            last_synced_height: addr.last_synced_height,
        });
    }

    Json(result)
}

/// GET /api/accounts/:label - get details for a specific account.
async fn get_account_handler(
    State(state): State<Arc<AppState>>,
    Path(label): Path<String>,
) -> impl IntoResponse {
    let store = state.store.read().await;

    if let Some(account) = store.accounts.get(&label) {
        return (
            StatusCode::OK,
            Json(serde_json::to_value(AccountResponse {
                label: account.label.clone(),
                account_type: "shielded".to_string(),
                balances: BalanceResponse::from(&account.balances),
                last_synced_height: account.last_synced_height,
            })
            .unwrap()),
        );
    }

    if let Some(addr) = store.addresses.get(&label) {
        let balances = PoolBalances {
            transparent: addr.balance_zatoshis,
            sapling: 0,
            orchard: 0,
        };
        return (
            StatusCode::OK,
            Json(serde_json::to_value(AccountResponse {
                label: addr.label.clone(),
                account_type: "transparent".to_string(),
                balances: BalanceResponse::from(&balances),
                last_synced_height: addr.last_synced_height,
            })
            .unwrap()),
        );
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::to_value(ErrorResponse {
            error: format!("Account '{}' not found", label),
        })
        .unwrap()),
    )
}

/// POST /api/accounts - add a new shielded account.
async fn add_account_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddAccountRequest>,
) -> impl IntoResponse {
    let birthday = req
        .birthday_height
        .unwrap_or(state.config.scanner.default_birthday_height);

    info!(label = %req.label, birthday = birthday, "Adding shielded account");

    // Initialize the scanner client first
    {
        let mut scanner = state.scanner.write().await;
        if let Err(e) = scanner.init_client(&req.label, &req.viewing_key, birthday) {
            error!(label = %req.label, error = %e, "Failed to init scanner client");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::to_value(ErrorResponse {
                    error: format!("Failed to initialize scanner: {}", e),
                })
                .unwrap()),
            );
        }
    }

    // Add to persistent store
    let account = ShieldedAccount {
        label: req.label.clone(),
        viewing_key: req.viewing_key,
        birthday_height: birthday,
        last_synced_height: 0,
        balances: Default::default(),
    };

    {
        let mut store = state.store.write().await;
        if let Err(e) = store.add_account(account) {
            // Roll back scanner client
            let mut scanner = state.scanner.write().await;
            scanner.remove_client(&req.label);
            return (
                StatusCode::CONFLICT,
                Json(serde_json::to_value(ErrorResponse {
                    error: e.to_string(),
                })
                .unwrap()),
            );
        }
        state
            .metrics
            .watched_accounts_total
            .set((store.accounts.len() + store.addresses.len()) as f64);
    }

    info!(label = %req.label, "Shielded account added successfully");
    (
        StatusCode::CREATED,
        Json(serde_json::to_value(serde_json::json!({
            "status": "created",
            "label": req.label,
            "birthday_height": birthday,
        }))
        .unwrap()),
    )
}

/// POST /api/addresses - add a transparent address.
async fn add_address_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddAddressRequest>,
) -> impl IntoResponse {
    info!(label = %req.label, address = %req.address, "Adding transparent address");

    let addr = TransparentAddress {
        label: req.label.clone(),
        address: req.address,
        last_synced_height: 0,
        balance_zatoshis: 0,
    };

    let mut store = state.store.write().await;
    if let Err(e) = store.add_address(addr) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::to_value(ErrorResponse {
                error: e.to_string(),
            })
            .unwrap()),
        );
    }

    state
        .metrics
        .watched_accounts_total
        .set((store.accounts.len() + store.addresses.len()) as f64);

    info!(label = %req.label, "Transparent address added successfully");
    (
        StatusCode::CREATED,
        Json(serde_json::to_value(serde_json::json!({
            "status": "created",
            "label": req.label,
        }))
        .unwrap()),
    )
}

/// GET /api/accounts/:label/transactions - get transactions and memos for an account.
async fn get_transactions_handler(
    State(state): State<Arc<AppState>>,
    Path(label): Path<String>,
) -> impl IntoResponse {
    // Verify the account exists
    {
        let store = state.store.read().await;
        if !store.accounts.contains_key(&label) && !store.addresses.contains_key(&label) {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::to_value(ErrorResponse {
                    error: format!("Account '{}' not found", label),
                })
                .unwrap()),
            );
        }
    }

    let scanner = state.scanner.read().await;
    match scanner.get_transactions(&label).await {
        Ok(transactions) => (
            StatusCode::OK,
            Json(serde_json::to_value(serde_json::json!({
                "label": label,
                "count": transactions.len(),
                "transactions": transactions,
            }))
            .unwrap()),
        ),
        Err(e) => {
            error!(label = %label, error = %e, "Failed to get transactions");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::to_value(ErrorResponse {
                    error: format!("Failed to retrieve transactions: {}", e),
                })
                .unwrap()),
            )
        }
    }
}

/// DELETE /api/accounts/:label - remove an account or address.
async fn delete_account_handler(
    State(state): State<Arc<AppState>>,
    Path(label): Path<String>,
) -> impl IntoResponse {
    info!(label = %label, "Removing account");

    // Remove scanner client
    {
        let mut scanner = state.scanner.write().await;
        scanner.remove_client(&label);
    }

    // Remove metrics
    state.metrics.remove_account_metrics(&label);

    // Remove from store
    let mut store = state.store.write().await;
    if let Err(e) = store.remove(&label) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::to_value(ErrorResponse {
                error: e.to_string(),
            })
            .unwrap()),
        );
    }

    state
        .metrics
        .watched_accounts_total
        .set((store.accounts.len() + store.addresses.len()) as f64);

    (
        StatusCode::OK,
        Json(serde_json::to_value(serde_json::json!({
            "status": "deleted",
            "label": label,
        }))
        .unwrap()),
    )
}

/// Start the management API HTTP server.
pub async fn serve_api(bind_addr: &str, state: Arc<AppState>) -> Result<()> {
    let app = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/accounts", get(list_accounts_handler))
        .route("/api/accounts", post(add_account_handler))
        .route("/api/accounts/{label}", get(get_account_handler))
        .route("/api/accounts/{label}", delete(delete_account_handler))
        .route(
            "/api/accounts/{label}/transactions",
            get(get_transactions_handler),
        )
        .route("/api/addresses", post(add_address_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!(addr = %bind_addr, "Management API server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
