# Architecture

Sigil Sentinel is a balance monitoring service that sits between an existing Zcash lightwalletd endpoint and a Prometheus/Grafana observability stack. It uses zingolib as a light client library to scan compact blocks with viewing keys, and wraps it with an infrastructure layer that adds a REST API, Prometheus metrics, persistent state, and multi-transport connectivity.

## System Overview

```mermaid
graph TB
    subgraph "Zcash Network"
        ZN[Zcash Peers]
    end

    subgraph "Zcash Full Node"
        ZB[zebrad :8232]
    end

    subgraph "Light Wallet Backend"
        LW[lightwalletd<br/>gRPC :9067<br/>HTTP :9068]
    end

    subgraph "Tor Hidden Services"
        TOR[tor]
        ONION_LWD[".onion :9067<br/>(lightwalletd)"]
        ONION_P2P[".onion :8233<br/>(P2P)"]
    end

    subgraph "Sigil Sentinel"
        direction TB
        EP[entrypoint.sh<br/>transport selector]
        EP -->|"SENTINEL_TRANSPORT=direct"| BIN
        EP -->|"SENTINEL_TRANSPORT=tor"| TS
        TS[torsocks + local tor<br/>SOCKS5 :9050] --> BIN

        subgraph "zcash-sentinel binary"
            BIN[main.rs<br/>orchestrator]
            BIN --> SCAN[scanner.rs<br/>zingolib LightClient]
            BIN --> API[api.rs<br/>axum REST :9101]
            BIN --> MET[metrics.rs<br/>prometheus :9100]
            SCAN --> STR[store.rs<br/>accounts.json]
            API --> STR
        end
    end

    subgraph "Observability Stack (barad-dur)"
        PROM[Prometheus :9090]
        GRAF[Grafana :3000]
    end

    ZN <-->|P2P| ZB
    ZB -->|RPC| LW
    LW -->|compact blocks| SCAN
    TOR --- ONION_LWD
    TOR --- ONION_P2P
    LW -.->|internal| TOR
    ZB -.->|internal| TOR
    MET -->|scrape /metrics| PROM
    PROM --> GRAF
```

## Transport Layer

The sentinel supports three connection modes to lightwalletd, selected by the `SENTINEL_TRANSPORT` environment variable. The transport is handled entirely at the infrastructure level — no Rust code changes between modes.

```mermaid
graph LR
    subgraph "Direct Mode (HTTP)"
        S1[sentinel] -->|"http://lightwalletd:9067"| LW1[lightwalletd<br/>same Docker network]
    end
```

```mermaid
graph LR
    subgraph "Direct Mode (HTTPS)"
        S2[sentinel] -->|"https://lwd.example.com:9067"<br/>rustls TLS| LW2[remote lightwalletd<br/>TLS termination]
    end
```

```mermaid
graph LR
    subgraph "Tor Mode"
        S3[sentinel] -->|torsocks| TP[local tor<br/>SOCKS5 :9050]
        TP -->|Tor circuit| OR[Tor network]
        OR -->|".onion:9067"| LW3[lightwalletd<br/>hidden service]
    end
```

### How transport works

| Layer | HTTP | HTTPS | Tor |
|-------|------|-------|-----|
| Config endpoint | `http://host:9067` | `https://host:9067` | `http://xyz.onion:9067` |
| `SENTINEL_TRANSPORT` | `direct` | `direct` | `tor` |
| TLS | none | zingolib rustls (native) | none (Tor provides encryption) |
| DNS resolution | standard | standard | Tor exit / hidden service |
| Extra processes | none | none | local `tor` daemon |
| Binary wrapper | none | none | `torsocks` |

The `entrypoint.sh` script handles mode selection:

- **direct**: Runs `zcash-sentinel` directly. HTTP and HTTPS are both handled by zingolib's gRPC client, which uses `hyper` with an optional `rustls` TLS layer based on the URI scheme.
- **tor**: Starts a minimal Tor daemon as a SOCKS5 proxy on `127.0.0.1:9050`, waits for bootstrap, then runs `zcash-sentinel` through `torsocks`. This transparently routes all TCP connections (including gRPC to lightwalletd) through the Tor network. No application code is aware of Tor.

## Data Flow

```mermaid
sequenceDiagram
    participant LW as lightwalletd
    participant SC as scanner (zingolib)
    participant ST as store (accounts.json)
    participant MT as metrics (prometheus)
    participant API as api (axum)
    participant PR as Prometheus

    Note over SC: Every 60s (configurable)
    SC->>LW: sync compact blocks (gRPC)
    LW-->>SC: CompactBlock stream
    SC->>SC: Decrypt notes with viewing keys
    SC->>SC: Compute pool balances

    SC->>ST: Update balances + sync height
    ST->>ST: Atomic write to accounts.json

    SC->>MT: Set zcash_balance_zatoshis{account,pool}
    SC->>MT: Set zcash_total_balance_zec{account}
    SC->>MT: Set zcash_sync_height{account}

    PR->>MT: GET /metrics (every 30s)
    MT-->>PR: Prometheus text format

    Note over API: On-demand requests
    API->>ST: Read account data
    API->>SC: Get value transfers (transactions + memos)
```

## Component Responsibilities

| Component | File | Role |
|-----------|------|------|
| **Orchestrator** | `main.rs` | Config loading, shared state init, spawns scanner + API + metrics servers |
| **Scanner** | `scanner.rs` | Manages zingolib `LightClient` instances per account, periodic sync, balance + transaction queries |
| **API** | `api.rs` | REST endpoints for account CRUD, balance queries, transaction/memo retrieval, health checks |
| **Metrics** | `metrics.rs` | Prometheus gauge registration and exposition on `:9100/metrics` |
| **Store** | `store.rs` | JSON persistence for watched accounts/addresses with atomic writes |
| **Entrypoint** | `entrypoint.sh` | Transport mode selection (direct vs tor), Tor daemon lifecycle |

## Dependency Boundary

```mermaid
graph TB
    subgraph "Sigil Sentinel (this project)"
        API_L[REST API]
        MET_L[Prometheus Metrics]
        STORE_L[Persistent Storage]
        ENTRY_L[Transport Layer]
        ORCH_L[Orchestration]
    end

    subgraph "zingolib (external dependency)"
        LC[LightClient]
        PEP[pepper-sync<br/>block scanning engine]
        WAL[LightWallet<br/>note decryption + state]
        GRP[gRPC client<br/>tonic + hyper + rustls]
    end

    ORCH_L --> LC
    LC --> PEP
    LC --> WAL
    LC --> GRP

    style API_L fill:#2d5016
    style MET_L fill:#2d5016
    style STORE_L fill:#2d5016
    style ENTRY_L fill:#2d5016
    style ORCH_L fill:#2d5016
    style LC fill:#1a1a4e
    style PEP fill:#1a1a4e
    style WAL fill:#1a1a4e
    style GRP fill:#1a1a4e
```

**Green** = Sigil Sentinel (our code): REST API, metrics, persistence, transport, orchestration

**Blue** = zingolib (upstream dependency): light client library, block scanning, wallet primitives, gRPC connectivity

The sentinel consumes zingolib as a standard Rust crate dependency. All monitoring, API, infrastructure, and transport features are implemented independently of zingolib.
