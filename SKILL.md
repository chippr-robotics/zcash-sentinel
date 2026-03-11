---
name: zcash-watchman
description: Monitor Zcash balances, add/remove shielded accounts and transparent addresses via the zcash-watchman API. Use for any Zcash balance check, account management, or wallet monitoring task.
allowed-tools: Bash(curl:*)
---

# Zcash Watchman — Balance Monitoring API

The zcash-watchman service monitors Zcash balances by syncing with a lightwalletd backend. It supports both shielded accounts (via Unified Full Viewing Keys) and transparent addresses.

**Base URL:** `http://172.17.0.1:9101`

All commands use `curl` against the management API.

## Quick Reference

```bash
# Health check
curl -s http://172.17.0.1:9101/api/health | jq .

# List all monitored accounts and their balances
curl -s http://172.17.0.1:9101/api/accounts | jq .

# Get a specific account by label
curl -s http://172.17.0.1:9101/api/accounts/my-wallet | jq .

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
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/health` | Service health, watched count, lightwalletd endpoint |
| GET | `/api/accounts` | List all accounts and addresses with balances |
| GET | `/api/accounts/{label}` | Get single account by label |
| POST | `/api/accounts` | Add shielded account (UFVK) |
| POST | `/api/addresses` | Add transparent address |
| DELETE | `/api/accounts/{label}` | Remove account or address |

## Response Format

Balances are returned in both zatoshis (integer) and ZEC (float):

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

## Adding Accounts

### Shielded Account (requires UFVK)

A Unified Full Viewing Key lets the service see incoming and outgoing transactions without spending authority. The `birthday_height` avoids scanning the entire chain — set it to a block height before the first transaction to this key. If omitted, defaults to 2000000.

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

Only needs the t-address string. Balance is fetched directly from the chain.

```bash
curl -s -X POST http://172.17.0.1:9101/api/addresses \
  -H 'Content-Type: application/json' \
  -d '{
    "label": "mining-payouts",
    "address": "t1KzZ1snhtbEC..."
  }' | jq .
```

## Notes

- Balances sync every 60 seconds from lightwalletd
- Labels must be unique across both shielded accounts and transparent addresses
- 1 ZEC = 100,000,000 zatoshis
- The service also exposes Prometheus metrics on port 9100
