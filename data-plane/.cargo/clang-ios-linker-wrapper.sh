#!/usr/bin/env bash
# Wrapper so rustc's -target arm64-apple-ios10.x is replaced with 13.4,
# fixing ___chkstk_darwin and deployment target. Paths relative to .cargo/.
# rustc may pass 10.0 or 10.0.0 depending on version.
set -e
args=()
for arg in "$@"; do
  case "$arg" in
    arm64-apple-ios10.0) arg="arm64-apple-ios13.4" ;;
    arm64-apple-ios10.0.0) arg="arm64-apple-ios13.4" ;;
    arm64-apple-ios10.0-simulator) arg="arm64-apple-ios13.4-simulator" ;;
    arm64-apple-ios10.0.0-simulator) arg="arm64-apple-ios13.4-simulator" ;;
  esac
  args+=("$arg")
done
exec clang "${args[@]}"
