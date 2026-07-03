#!/usr/bin/env bash
# Run a SLIM control plane + 6 nodes (2 per group across 3 groups).
#
# Usage:
#   ./run-cluster.sh [controller-config.yaml]
#
# If no config is specified, only the nodes are started (no controller).
#
# Example:
#   ./run-cluster.sh config-files/controller/star-segment.yaml
#   ./run-cluster.sh   # nodes only, no controller
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

CONTROLLER_CONFIG="${1:-}"

# Resolve relative paths from script dir
if [[ -n "$CONTROLLER_CONFIG" && ! "$CONTROLLER_CONFIG" = /* ]]; then
    CONTROLLER_CONFIG="$SCRIPT_DIR/$CONTROLLER_CONFIG"
fi

if [[ -n "$CONTROLLER_CONFIG" && ! -f "$CONTROLLER_CONFIG" ]]; then
    echo "Error: controller config not found: $CONTROLLER_CONFIG"
    exit 1
fi

if [[ -n "$CONTROLLER_CONFIG" && ! -f "$CONTROLLER_BINARY" ]]; then
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
    exit 0
}
trap cleanup EXIT INT TERM

echo "=== Starting SLIM cluster ==="
if [[ -n "$CONTROLLER_CONFIG" ]]; then
    echo "Controller config: $(basename "$CONTROLLER_CONFIG")"
else
    echo "No controller config — starting nodes only"
fi
echo ""

# Start controller (if config provided)
if [[ -n "$CONTROLLER_CONFIG" ]]; then
    echo "Starting controller..."
    "$CONTROLLER_BINARY" --config "$CONTROLLER_CONFIG" \
        > "$LOG_DIR/controller.log" 2>&1 &
    PIDS+=($!)
    CONTROLLER_PID="${PIDS[-1]}"
    echo "  PID=$CONTROLLER_PID  log=$LOG_DIR/controller.log"

    # Give the controller a moment to bind its ports
    sleep 2
else
    CONTROLLER_PID=""
fi

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
if [[ -n "$CONTROLLER_PID" ]]; then
    echo "  Controller + 6 nodes (group-1: node-1,2 | group-2: node-3,4 | group-3: node-5,6)"
    echo ""
    echo "  Controller NB: 127.0.0.1:50051"
    echo "  Controller SB: 127.0.0.1:50052"
else
    echo "  6 nodes (group-1: node-1,2 | group-2: node-3,4 | group-3: node-5,6)"
    echo "  No controller — nodes will retry connecting"
fi
echo ""
echo "Press Ctrl-C to stop all. Kill a node PID to test failover."
echo ""

# Monitor children — log exits but keep running unless controller dies
while true; do
    # If the controller died, bail out
    if [[ -n "$CONTROLLER_PID" ]] && ! kill -0 "$CONTROLLER_PID" 2>/dev/null; then
        wait "$CONTROLLER_PID" 2>/dev/null
        echo "ERROR: Controller (PID $CONTROLLER_PID) exited!"
        exit 1
    fi
    # Check node processes
    for i in "${!PIDS[@]}"; do
        pid="${PIDS[$i]}"
        [[ "$pid" == "$CONTROLLER_PID" ]] && continue
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
