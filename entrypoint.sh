#!/bin/bash
set -euo pipefail

TRANSPORT="${SENTINEL_TRANSPORT:-direct}"

case "$TRANSPORT" in
  direct)
    # Direct connection (HTTP or HTTPS) — no proxy
    exec zcash-sentinel "$@"
    ;;
  tor)
    echo "Starting local Tor SOCKS5 proxy..."

    # Write a minimal torrc for SOCKS5 proxy mode
    cat > /tmp/torrc <<TORRC
SocksPort 9050
DataDirectory /tmp/tor-data
Log notice stderr
TORRC

    mkdir -p /tmp/tor-data
    tor -f /tmp/torrc &
    TOR_PID=$!

    # Wait for Tor to bootstrap
    echo "Waiting for Tor to bootstrap..."
    for i in $(seq 1 60); do
      if curl -sf --socks5-hostname 127.0.0.1:9050 https://check.torproject.org/api/ip >/dev/null 2>&1; then
        echo "Tor is ready (took ${i}s)"
        break
      fi
      if ! kill -0 "$TOR_PID" 2>/dev/null; then
        echo "Tor process died during bootstrap"
        exit 1
      fi
      sleep 1
    done

    if ! kill -0 "$TOR_PID" 2>/dev/null; then
      echo "Tor failed to start"
      exit 1
    fi

    # Shut down Tor when the sentinel exits
    trap "kill $TOR_PID 2>/dev/null || true" EXIT

    # Run the sentinel through torsocks so all TCP goes through the SOCKS5 proxy
    exec torsocks zcash-sentinel "$@"
    ;;
  *)
    echo "Unknown SENTINEL_TRANSPORT: $TRANSPORT (expected: direct, tor)"
    exit 1
    ;;
esac
