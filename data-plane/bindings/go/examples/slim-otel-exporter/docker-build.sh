#!/bin/bash
# Docker build script for OpenTelemetry Collector with SLIM exporter
# This script prepares the build context with necessary dependencies

set -e

# Create a temporary build directory
BUILD_DIR=$(mktemp -d)
trap "rm -rf $BUILD_DIR" EXIT

echo "Copying files to build context at $BUILD_DIR..."

# Copy current directory contents
cp -r ./* "$BUILD_DIR/"

# Copy the SLIM bindings
echo "   - Copying SLIM generated bindings..."
cp -r ../../generated "$BUILD_DIR/generated"

echo "   - Copying common utilities..."
cp -r ../common "$BUILD_DIR/common"

echo "   - Copying data-plane for Rust build ..."
rsync -a --exclude='target' --exclude='.git' ../../../../ "$BUILD_DIR/data-plane/"

# Build the Docker image
echo " Building Docker image..."
cd "$BUILD_DIR"
docker build -t slim-otel-collector .

echo " Docker build complete!"
echo ""
echo "To run the collector, you must mount your config file:"
echo "  docker run -p 4317:4317 -p 4318:4318 -v /path/to/your/collector-config.yaml:/otelcol/config.yaml:ro slim-otel-collector"
