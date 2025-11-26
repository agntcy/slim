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
cd slim-otelcol
CGO_ENABLED=1 go build -trimpath -o slim-otelcol -ldflags="-s -w"
cd ..

echo "✅ Build complete! Binary available at: slim-otelcol/slim-otelcol"
echo ""
echo "To run the collector:"
echo "  cd slim-otelcol"
echo "  ./slim-otelcol --config <your-config.yaml>"
