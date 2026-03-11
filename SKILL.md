---
name: zcash-sentinel
description: Monitor Zcash balances, view transactions and memos, add/remove shielded accounts and transparent addresses, check sync status and Prometheus metrics via the zcash-sentinel API.
allowed-tools: Bash(curl:*), Bash(docker:*)
---

# Zcash Sentinel — Balance Monitoring API

The zcash-sentinel service (Sigil Sentinel) monitors Zcash balances by syncing with a lightwalletd backend using zingolib. It supports shielded accounts (via Unified Full Viewing Keys), transparent addresses, transaction history with decrypted memos, and Prometheus metrics.

**Base URL:** `http://172.17.0.1:9101`
**Metrics URL:** `http://172.17.0.1:9100/metrics`

All commands use `curl` against the management and metrics endpoints.

## Quick Reference

```bash
# Health check — sync status, account counts, lightwalletd connection
curl -s http://172.17.0.1:9101/api/health | jq .

# List all monitored accounts with current balances
curl -s http://172.17.0.1:9101/api/accounts | jq .

# Get a specific account by label
curl -s http://172.17.0.1:9101/api/accounts/my-wallet | jq .

# View transactions and memos for an account
curl -s http://172.17.0.1:9101/api/accounts/my-wallet/transactions | jq .

# Add a shielded account (Unified Full Viewing Key)
curl -s -X POST http://172.17.0.1:9101/api/accounts \
  -H 'Content-Type: application/json' \
  -d '{"label": "my-wallet", "viewing_key": "uview1...", "birthday_height": 2000000}' | jq .

# Add a transparent address
curl -s -X POST http://172.17.0.1:9101/api/addresses \
  -H 'Content-Type: application/json' \
  -d '{"label": "t-addr-donations", "address": "t1..."}' | jq .

# Remove an account or address by label
curl -s -X DELETE http://172.17.0.1:9101/api/accounts/my-wallet | jq .

# Check Prometheus metrics (balances, sync height, chain tip)
curl -s http://172.17.0.1:9100/metrics | grep zcash_

# Check container status and recent logs
docker logs --tail 20 zcash-sentinel
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/health` | Service health, watched count, lightwalletd endpoint |
| GET | `/api/accounts` | List all accounts and addresses with balances |
| GET | `/api/accounts/{label}` | Get single account by label |
| GET | `/api/accounts/{label}/transactions` | Transaction history with memos |
| POST | `/api/accounts` | Add shielded account (UFVK) |
| POST | `/api/addresses` | Add transparent address |
| DELETE | `/api/accounts/{label}` | Remove account or address |

## Response Formats

### Account balance

Balances are returned in both zatoshis (integer) and ZEC (float), broken down by pool:

```json
{
  "label": "my-wallet",
  "type": "shielded",
  "balances": {
    "transparent_zatoshis": 0,
    "sapling_zatoshis": 50000000,
    "orchard_zatoshis": 100000000,
    "total_zatoshis": 150000000,
    "total_zec": 1.5
  },
  "last_synced_height": 2500000
}
```

### Transactions and memos

Each transaction includes the kind, value in zatoshis, pool, and any decrypted memos:

```json
{
  "label": "my-wallet",
  "count": 3,
  "transactions": [
    {
      "txid": "abc123...",
      "datetime": 1710187200,
      "status": "Confirmed(2500000)",
      "blockheight": 2500000,
      "kind": "received",
      "value": 50000000,
      "pool_received": "orchard",
      "memos": ["Payment for March"]
    },
    {
      "txid": "def456...",
      "datetime": 1709500000,
      "status": "Confirmed(2499500)",
      "blockheight": 2499500,
      "kind": "sent",
      "value": 10000000,
      "fee": 10000,
      "recipient_address": "u1...",
      "memos": []
    }
  ]
}
```

Transaction `kind` values: `received`, `sent`, `shield`, `send-to-self`, `memo-to-self`, `rejection`

### Health check

```json
{
  "status": "ok",
  "watched_accounts": 2,
  "watched_addresses": 1,
  "lightwalletd_endpoint": "http://lightwalletd:9067"
}
```

## Adding Accounts

### Shielded Account (requires UFVK)

A Unified Full Viewing Key lets the service see incoming/outgoing transactions and decrypt memos without spending authority. The `birthday_height` avoids scanning the entire chain — set it to a block height before the first transaction to this key. If omitted, defaults to 2000000.

```bash
curl -s -X POST http://172.17.0.1:9101/api/accounts \
  -H 'Content-Type: application/json' \
  -d '{
    "label": "sigil-treasury",
    "viewing_key": "uview1q...",
    "birthday_height": 2400000
  }' | jq .
```

### Transparent Address

Only needs the t-address string. Balance is fetched from the chain via compact blocks.

```bash
curl -s -X POST http://172.17.0.1:9101/api/addresses \
  -H 'Content-Type: application/json' \
  -d '{
    "label": "mining-payouts",
    "address": "t1KzZ1snhtbEC..."
  }' | jq .
```

## Prometheus Metrics

Available at `http://172.17.0.1:9100/metrics`:

| Metric | Labels | Description |
|--------|--------|-------------|
| `zcash_balance_zatoshis` | `account`, `pool` | Balance per account per pool (transparent/sapling/orchard) |
| `zcash_total_balance_zec` | `account` | Total balance per account in ZEC |
| `zcash_sync_height` | `account` | Last synced block height per account |
| `zcash_chain_height` | — | Chain tip height from lightwalletd |
| `zcash_sync_lag_blocks` | `account` | Blocks behind chain tip per account |
| `zcash_last_sync_duration_seconds` | — | How long the last sync cycle took |
| `zcash_watched_accounts_total` | — | Total number of monitored accounts |

```bash
# Get all zcash metrics
curl -s http://172.17.0.1:9100/metrics | grep zcash_

# Get balance for a specific account
curl -s http://172.17.0.1:9100/metrics | grep 'zcash_total_balance_zec'

# Check sync status
curl -s http://172.17.0.1:9100/metrics | grep 'zcash_sync_lag_blocks\|zcash_chain_height'
```

## Container Operations

```bash
# View recent logs
docker logs --tail 50 zcash-sentinel

# Follow logs live
docker logs -f zcash-sentinel

# Restart the service
docker restart zcash-sentinel

# Check container health
docker inspect --format='{{.State.Health.Status}}' zcash-sentinel
```

## Transport Modes

The sentinel supports HTTP, HTTPS, and Tor connections to lightwalletd. Controlled by `SENTINEL_TRANSPORT` env var and the endpoint URI in config.toml:

- **direct** (default): HTTP or HTTPS. HTTPS works natively via zingolib's TLS stack.
- **tor**: Starts a local Tor SOCKS5 proxy and routes all traffic through it via `torsocks`. Point the endpoint at a `.onion:9067` address.

```bash
# Check current transport mode
docker inspect --format='{{range .Config.Env}}{{println .}}{{end}}' zcash-sentinel | grep SENTINEL_TRANSPORT
```

## Notes

- Balances sync every 60 seconds from lightwalletd
- Labels must be unique across both shielded accounts and transparent addresses
- 1 ZEC = 100,000,000 zatoshis
- Transactions are returned newest-first
- Memos are only available for shielded transactions where the viewing key can decrypt them
- The service persists accounts to `/var/lib/zcash-sentinel/accounts.json` — they survive restarts
- Prometheus metrics are scraped by the barad-dur monitoring stack on the `barad-dur_fukuii-network`
