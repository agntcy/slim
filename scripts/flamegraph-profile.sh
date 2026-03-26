#!/usr/bin/env bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
#
# SLIM Data-Plane Flamegraph Profiling Script
#
# Profiles the in-process stress-publish binary using dtrace (macOS) or perf
# (Linux) and generates an interactive SVG flamegraph.
#
# Usage:
#   ./scripts/flamegraph-profile.sh [OPTIONS]
#
# Options:
#   --senders N        Number of concurrent sender tasks (default: 16)
#   --messages N       Messages per sender (default: 2000000)
#   --payload-size N   Payload size in bytes (default: 256)
#   --output FILE      Output SVG path (default: flamegraph-inprocess.svg)
#   --duration N       Profiling duration in seconds (default: 30)
#   -h, --help         Show this help
#
# Requirements:
#   - inferno (cargo install inferno)
#   - sudo access (dtrace requires root on macOS)
#
# The script:
#   1. Builds stress-publish in release mode with debug symbols
#   2. Starts stress-publish in background
#   3. Profiles it with dtrace for the specified duration
#   4. Collapses stacks and generates an SVG flamegraph

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DP_DIR="$REPO_ROOT/data-plane"

# Defaults
SENDERS=16
MESSAGES=2000000
PAYLOAD_SIZE=256
DURATION=30
OUTPUT="$REPO_ROOT/flamegraph-inprocess.svg"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --senders)      SENDERS="$2";      shift 2 ;;
    --messages)     MESSAGES="$2";     shift 2 ;;
    --payload-size) PAYLOAD_SIZE="$2"; shift 2 ;;
    --output)       OUTPUT="$2";       shift 2 ;;
    --duration)     DURATION="$2";     shift 2 ;;
    -h|--help)
      sed -n '/^# Usage:/,/^[^#]/{ /^[^#]/d; s/^# \{0,1\}//; p; }' "$0"
      exit 0
      ;;
    *) echo "ERROR: Unknown option: $1" >&2; exit 1 ;;
  esac
done

TOTAL=$((SENDERS * MESSAGES))
DTRACE_OUT="${OUTPUT%.svg}.dtrace"
COLLAPSED_OUT="${OUTPUT%.svg}.collapsed"

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------
check_tool() {
  if ! command -v "$1" &>/dev/null; then
    echo "ERROR: $1 not found. $2" >&2
    exit 1
  fi
}

check_tool cargo               "Install Rust: https://rustup.rs"
check_tool inferno-collapse-dtrace "Install: cargo install inferno"
check_tool inferno-flamegraph  "Install: cargo install inferno"

# ---------------------------------------------------------------------------
# Build release binary with debug symbols
# ---------------------------------------------------------------------------
echo "==> Building stress-publish (release + debug symbols)…"
cd "$DP_DIR"
CARGO_PROFILE_RELEASE_DEBUG=2 cargo build --release \
  -p agntcy-slim-testing --bin stress-publish 2>&1 | tail -3

STRESS_BIN="$DP_DIR/target/release/stress-publish"
if [[ ! -x "$STRESS_BIN" ]]; then
  echo "ERROR: $STRESS_BIN not found after build" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Cleanup helper
# ---------------------------------------------------------------------------
STRESS_PID=""
DTRACE_PID=""
cleanup() {
  echo ""
  echo "==> Cleaning up…"
  [[ -n "$DTRACE_PID" ]] && kill "$DTRACE_PID" 2>/dev/null || true
  [[ -n "$STRESS_PID" ]] && kill "$STRESS_PID" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# 1. Start stress-publish in background
# ---------------------------------------------------------------------------
echo "==> Starting stress-publish (${SENDERS} senders × ${MESSAGES} msgs × ${PAYLOAD_SIZE}B = ${TOTAL} total)…"

"$STRESS_BIN" \
  --senders "$SENDERS" \
  --messages "$MESSAGES" \
  --payload-size "$PAYLOAD_SIZE" &
STRESS_PID=$!

# Let it start up and begin sending
sleep 2

if ! kill -0 "$STRESS_PID" 2>/dev/null; then
  echo "ERROR: stress-publish exited prematurely" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 2. Profile with dtrace
# ---------------------------------------------------------------------------
echo "==> Starting dtrace profiling for ${DURATION}s (sudo required)…"
echo "    PID: $STRESS_PID"

sudo dtrace -x ustackframes=100 \
  -n "profile-997 /pid == ${STRESS_PID}/ { @[ustack()] = count(); }" \
  -o "$DTRACE_OUT" &
DTRACE_PID=$!

# Wait for profiling duration or until stress test finishes
ELAPSED=0
while [[ $ELAPSED -lt $DURATION ]] && kill -0 "$STRESS_PID" 2>/dev/null; do
  sleep 1
  ELAPSED=$((ELAPSED + 1))
done

# Stop dtrace
echo "==> Stopping dtrace…"
sudo kill -INT "$DTRACE_PID" 2>/dev/null || true
wait "$DTRACE_PID" 2>/dev/null || true
DTRACE_PID=""

# Wait for stress test to finish
echo "==> Waiting for stress-publish to complete…"
wait "$STRESS_PID" 2>/dev/null || true
STRESS_PID=""

# ---------------------------------------------------------------------------
# 3. Generate flamegraph
# ---------------------------------------------------------------------------
if [[ ! -s "$DTRACE_OUT" ]]; then
  echo "ERROR: dtrace output is empty at $DTRACE_OUT" >&2
  exit 1
fi

echo "==> Collapsing stacks…"
inferno-collapse-dtrace < "$DTRACE_OUT" > "$COLLAPSED_OUT"

TITLE="SLIM In-Process (${SENDERS}S × ${MESSAGES}M × ${PAYLOAD_SIZE}B)"
echo "==> Generating flamegraph…"
inferno-flamegraph --title "$TITLE" < "$COLLAPSED_OUT" > "$OUTPUT"

# ---------------------------------------------------------------------------
# 4. Show top functions summary
# ---------------------------------------------------------------------------
echo ""
echo "==> Top 15 hottest functions:"
awk -F';' '{print $NF}' "$COLLAPSED_OUT" \
  | awk '{sum[$1]+=$2} END {for(k in sum) print sum[k], k}' \
  | sort -rn | head -15
echo ""

# ---------------------------------------------------------------------------
# 5. Done
# ---------------------------------------------------------------------------
echo "==> Flamegraph: $OUTPUT"
echo "    Collapsed:  $COLLAPSED_OUT"
echo "    Raw dtrace: $DTRACE_OUT"
echo ""
echo "Open the flamegraph:  open $OUTPUT"
