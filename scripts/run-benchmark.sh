#!/usr/bin/env bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
#
# SLIM Data-Plane Benchmark Runner
#
# Runs the in-process stress-publish binary across a matrix of configurations
# and prints a summary table of throughput results.
#
# Usage:
#   ./scripts/run-benchmark.sh [OPTIONS]
#
# Options:
#   --senders LIST        Comma-separated sender counts (default: 1,4,8,16)
#   --messages N          Messages per sender (default: 1000000)
#   --payload-sizes LIST  Comma-separated payload sizes in bytes (default: 8,16,64,256,1024,4096)
#   --runs N              Repetitions per configuration (default: 3)
#   --output FILE         Results CSV path (default: benchmark-results.csv)
#   --quick               Quick mode: 1 run, fewer configs
#   -h, --help            Show this help
#
# Requirements:
#   - cargo (Rust toolchain)
#
# Note: This benchmark uses in-process message routing (bypasses gRPC).
# No external SLIM node is required. However, if you want to benchmark
# against a running SLIM node over gRPC instead, start one with:
#
#   slimctl slim start -c data-plane/config/base/server-config.yaml
#
# Output:
#   - Prints a summary table to stdout
#   - Writes detailed results to CSV

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DP_DIR="$REPO_ROOT/data-plane"

# Defaults
SENDER_LIST="1,4,8,16"
MESSAGES=1000000
PAYLOAD_LIST="8,16,64,256,1024,4096"
RUNS=3
OUTPUT="$REPO_ROOT/benchmark-results.csv"
QUICK=false

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --senders)       SENDER_LIST="$2";  shift 2 ;;
    --messages)      MESSAGES="$2";     shift 2 ;;
    --payload-sizes) PAYLOAD_LIST="$2"; shift 2 ;;
    --runs)          RUNS="$2";         shift 2 ;;
    --output)        OUTPUT="$2";       shift 2 ;;
    --quick)         QUICK=true;        shift ;;
    -h|--help)
      sed -n '/^# Usage:/,/^[^#]/{ /^[^#]/d; s/^# \{0,1\}//; p; }' "$0"
      exit 0
      ;;
    *) echo "ERROR: Unknown option: $1" >&2; exit 1 ;;
  esac
done

if [[ "$QUICK" == true ]]; then
  SENDER_LIST="4,16"
  PAYLOAD_LIST="8,16,64,1024"
  RUNS=1
  MESSAGES=500000
fi

IFS=',' read -ra SENDERS <<< "$SENDER_LIST"
IFS=',' read -ra PAYLOADS <<< "$PAYLOAD_LIST"

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
echo "==> Building stress-publish (release)…"
cd "$DP_DIR"
cargo build --release -p agntcy-slim-testing --bin stress-publish 2>&1 | tail -3

STRESS_BIN="$DP_DIR/target/release/stress-publish"
if [[ ! -x "$STRESS_BIN" ]]; then
  echo "ERROR: $STRESS_BIN not found after build" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# CSV header
# ---------------------------------------------------------------------------
echo "senders,messages_per_sender,payload_bytes,total_messages,run,send_elapsed_s,total_elapsed_s,total_received,send_mps,recv_mps" > "$OUTPUT"

# ---------------------------------------------------------------------------
# Run matrix
# ---------------------------------------------------------------------------
TOTAL_CONFIGS=$(( ${#SENDERS[@]} * ${#PAYLOADS[@]} * RUNS ))
echo "==> Running $TOTAL_CONFIGS benchmark configurations"
echo "    Senders:  ${SENDER_LIST}"
echo "    Payloads: ${PAYLOAD_LIST}"
echo "    Messages: ${MESSAGES}/sender"
echo "    Runs:     ${RUNS}"
echo ""

printf "%-8s %-8s %-10s %-12s %-14s %-14s\n" \
  "Senders" "Payload" "Total" "Send MPS" "Recv MPS" "Delivery%"
printf "%-8s %-8s %-10s %-12s %-14s %-14s\n" \
  "-------" "-------" "---------" "-----------" "-------------" "---------"

for senders in "${SENDERS[@]}"; do
  for payload in "${PAYLOADS[@]}"; do
    total=$((senders * MESSAGES))

    # Accumulate across runs for averaging
    sum_send_mps=0
    sum_recv_mps=0
    sum_delivery=0

    for run in $(seq 1 "$RUNS"); do
      # Run stress-publish and capture output
      result=$("$STRESS_BIN" \
        --senders "$senders" \
        --messages "$MESSAGES" \
        --payload-size "$payload" 2>&1)

      # Parse results from output
      send_elapsed=$(echo "$result" | grep "Send elapsed:" | awk '{print $3}' | tr -d 's')
      total_elapsed=$(echo "$result" | grep "Total elapsed:" | awk '{print $3}' | tr -d 's')
      total_received=$(echo "$result" | grep "Total received:" | awk '{print $3}')
      send_mps=$(echo "$result" | grep "Throughput (send):" | awk '{print $3}')
      recv_mps=$(echo "$result" | grep "Throughput (recv):" | awk '{print $3}')

      # Write CSV row
      echo "${senders},${MESSAGES},${payload},${total},${run},${send_elapsed},${total_elapsed},${total_received},${send_mps},${recv_mps}" >> "$OUTPUT"

      # Accumulate for averages
      sum_send_mps=$(echo "$sum_send_mps + $send_mps" | bc)
      sum_recv_mps=$(echo "$sum_recv_mps + $recv_mps" | bc)
      delivery_pct=$(echo "scale=1; $total_received * 100 / $total" | bc)
      sum_delivery=$(echo "$sum_delivery + $delivery_pct" | bc)
    done

    # Compute averages
    avg_send_mps=$(echo "scale=0; $sum_send_mps / $RUNS" | bc)
    avg_recv_mps=$(echo "scale=0; $sum_recv_mps / $RUNS" | bc)
    avg_delivery=$(echo "scale=1; $sum_delivery / $RUNS" | bc)

    printf "%-8s %-8s %-10s %-12s %-14s %-14s\n" \
      "$senders" "${payload}B" "$total" "$avg_send_mps" "$avg_recv_mps" "${avg_delivery}%"
  done
done

echo ""
echo "==> Results written to: $OUTPUT"
