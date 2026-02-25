#!/usr/bin/env bash
# Wrapper so rustc's -target arm64/aarch64-apple-ios10.x is replaced with 13.4,
# fixing ___chkstk_darwin and deployment target. Paths relative to .cargo/.
# rustc may pass 10.0 or 10.0.0, arm64 or aarch64 depending on version.
set -e
LOG="/tmp/linker-args.log"
echo "=== $(date) linker invoked with $# args ===" >> "$LOG"
args=()
for arg in "$@"; do
  echo "Linker arg: $arg" >> "$LOG"
  case "$arg" in
    arm64-apple-ios10.0) arg="arm64-apple-ios13.4" ;;
    arm64-apple-ios10.0.0) arg="arm64-apple-ios13.4" ;;
    arm64-apple-ios10.0-simulator) arg="arm64-apple-ios13.4-simulator" ;;
    arm64-apple-ios10.0.0-simulator) arg="arm64-apple-ios13.4-simulator" ;;
    aarch64-apple-ios10.0) arg="aarch64-apple-ios13.4" ;;
    aarch64-apple-ios10.0.0) arg="aarch64-apple-ios13.4" ;;
    aarch64-apple-ios10.0-simulator) arg="aarch64-apple-ios13.4-simulator" ;;
    aarch64-apple-ios10.0.0-simulator) arg="aarch64-apple-ios13.4-simulator" ;;
    # Handle -target=value form (used by some build systems)
    -target=arm64-apple-ios10.0) arg="-target=arm64-apple-ios13.4" ;;
    -target=arm64-apple-ios10.0.0) arg="-target=arm64-apple-ios13.4" ;;
    -target=aarch64-apple-ios10.0) arg="-target=aarch64-apple-ios13.4" ;;
    -target=aarch64-apple-ios10.0.0) arg="-target=aarch64-apple-ios13.4" ;;
  esac
  args+=("$arg")
done
exec clang "${args[@]}"
