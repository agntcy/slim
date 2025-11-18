#!/bin/bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Script to build Rust library and generate Go bindings

set -e

COMMON_DIR="../common"

echo "🦀 Building Rust library..."
cd "$COMMON_DIR"
cargo build --release
cd -

echo ""
echo "🔧 Generating Go bindings..."
uniffi-bindgen-go "$COMMON_DIR/src/slim_bindings.udl" --out-dir generated --config "$COMMON_DIR/uniffi.toml"

echo ""
echo "✅ Done!"
echo ""
echo "📚 Next steps:"
echo "   1. Set library path: export DYLD_LIBRARY_PATH=\$(pwd)/$COMMON_DIR/target/release:\$DYLD_LIBRARY_PATH"
echo "   2. Or on Linux: export LD_LIBRARY_PATH=\$(pwd)/$COMMON_DIR/target/release:\$LD_LIBRARY_PATH"
echo "   3. Use the generated bindings in generated/"

