#!/usr/bin/env bash
# Run a SLIM control plane + 6 nodes (2 per group across 3 groups).
#
# Usage:
#   ./run-cluster.sh <controller-config.yaml>
#
# Example:
#   ./run-cluster.sh config-files/controller/star-segment.yaml
#
# Kill individual node PIDs to test failover. The script keeps running.
# Press Ctrl-C to stop everything.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
BINARY="$REPO_ROOT/target/release/slim"
[[ ! -f "$BINARY" ]] && BINARY="$REPO_ROOT/target/debug/slim"
CONTROLLER_BINARY="$REPO_ROOT/target/release/slim-control-plane"
[[ ! -f "$CONTROLLER_BINARY" ]] && CONTROLLER_BINARY="$REPO_ROOT/target/debug/slim-control-plane"
NODE_DIR="$SCRIPT_DIR/config-files/nodes"

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <controller-config.yaml>"
    echo ""
    echo "Available controller configs:"
    ls "$SCRIPT_DIR/config-files/controller/"*.yaml | xargs -n1 basename | sed 's/^/  /'
    exit 1
fi

CONTROLLER_CONFIG="$1"

# Resolve relative paths from script dir
if [[ ! "$CONTROLLER_CONFIG" = /* ]]; then
    CONTROLLER_CONFIG="$SCRIPT_DIR/$CONTROLLER_CONFIG"
fi

if [[ ! -f "$CONTROLLER_CONFIG" ]]; then
    echo "Error: controller config not found: $CONTROLLER_CONFIG"
    exit 1
fi

if [[ ! -f "$CONTROLLER_BINARY" ]]; then
    echo "Error: slim-control-plane binary not found at $CONTROLLER_BINARY"
    echo "Run: cargo build --release -p agntcy-slim-control-plane"
    exit 1
fi

if [[ ! -f "$BINARY" ]]; then
    echo "Error: slim binary not found at $BINARY"
    echo "Run: cargo build --release -p agntcy-slim"
    exit 1
fi

NODE_CONFIGS=(
    "$NODE_DIR/node-1-group-1.yaml"
    "$NODE_DIR/node-2-group-1.yaml"
    "$NODE_DIR/node-3-group-2.yaml"
    "$NODE_DIR/node-4-group-2.yaml"
    "$NODE_DIR/node-5-group-3.yaml"
    "$NODE_DIR/node-6-group-3.yaml"
)

# Verify all node configs exist
for cfg in "${NODE_CONFIGS[@]}"; do
    if [[ ! -f "$cfg" ]]; then
        echo "Error: node config not found: $cfg"
        exit 1
    fi
done

PIDS=()
LOG_DIR="./logs"
rm -rf "$LOG_DIR"
mkdir -p "$LOG_DIR"
echo "Logs in: $LOG_DIR"

SHUTTING_DOWN=false
cleanup() {
    $SHUTTING_DOWN && return
    SHUTTING_DOWN=true
    echo ""
    echo "Stopping all processes..."
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null
    echo "All processes stopped."
    echo "Logs saved in: $LOG_DIR"
}
trap cleanup EXIT INT TERM

echo "=== Starting SLIM cluster ==="
echo "Controller config: $(basename "$CONTROLLER_CONFIG")"
echo ""

# Start controller
echo "Starting controller..."
"$CONTROLLER_BINARY" --config "$CONTROLLER_CONFIG" \
    > "$LOG_DIR/controller.log" 2>&1 &
PIDS+=($!)
echo "  PID=${PIDS[-1]}  log=$LOG_DIR/controller.log"

# Give the controller a moment to bind its ports
sleep 2

# Start nodes
for cfg in "${NODE_CONFIGS[@]}"; do
    name=$(basename "$cfg" .yaml)
    echo "Starting $name..."
    "$BINARY" --config "$cfg" \
        > "$LOG_DIR/$name.log" 2>&1 &
    PIDS+=($!)
    echo "  PID=${PIDS[-1]}  log=$LOG_DIR/$name.log"
    sleep 0.5
done

echo ""
echo "=== Cluster running ==="
echo "  Controller + 6 nodes (group-1: node-1,2 | group-2: node-3,4 | group-3: node-5,6)"
echo ""
echo "  Controller NB: 127.0.0.1:50051"
echo "  Controller SB: 127.0.0.1:50052"
echo ""
echo "Press Ctrl-C to stop all. Kill a node PID to test failover."
echo ""

# Monitor children — log exits but keep running unless controller dies
CONTROLLER_PID="${PIDS[0]}"
while true; do
    # If the controller died, bail out
    if ! kill -0 "$CONTROLLER_PID" 2>/dev/null; then
        wait "$CONTROLLER_PID" 2>/dev/null
        echo "ERROR: Controller (PID $CONTROLLER_PID) exited!"
        exit 1
    fi
    # Check node processes
    for i in "${!PIDS[@]}"; do
        [[ $i -eq 0 ]] && continue  # skip controller
        pid="${PIDS[$i]}"
        [[ -z "$pid" ]] && continue
        if ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid" 2>/dev/null
            rc=$?
            echo "INFO: Node process $pid exited (code $rc) — link/route failover should occur"
            unset 'PIDS[$i]'
        fi
    done
    sleep 5
done
