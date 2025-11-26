#!/bin/bash
# Build script for OpenTelemetry Collector with SLIM exporter
# This script handles the CGO requirements for building with the SLIM Go bindings

set -e

echo "🔧 Building OpenTelemetry Collector with SLIM exporter..."

# Step 1: Generate the collector code without compiling
echo "📦 Generating collector sources..."
./ocb --config builder-config.yaml --skip-compilation

# Step 2: Compile with CGO enabled
echo "🔨 Compiling with CGO enabled..."
cd otelcol-dev
CGO_ENABLED=1 go build -trimpath -o otelcol-dev -ldflags="-s -w"
cd ..

echo "✅ Build complete! Binary available at: otelcol-dev/otelcol-dev"
echo ""
echo "To run the collector:"
echo "  cd otelcol-dev"
echo "  ./otelcol-dev --config <your-config.yaml>"
