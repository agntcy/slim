# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

COMMON_DIR = ../common
COMMON_TARGET = $(COMMON_DIR)/target/release

.PHONY: all build generate clean test example help

# Default target
all: build generate

help:
	@echo "SLIM Go Bindings - Makefile targets:"
	@echo "  make build       - Build the Rust library (release mode)"
	@echo "  make build-debug - Build the Rust library (debug mode)"
	@echo "  make generate    - Generate Go bindings from UDL"
	@echo "  make all         - Build and generate (default)"
	@echo "  make clean       - Clean build artifacts"
	@echo "  make example     - Run the simple example"
	@echo "  make help        - Show this help message"

build:
	@echo "🦀 Building Rust library (release mode)..."
	cd $(COMMON_DIR) && cargo build --release

build-debug:
	@echo "🦀 Building Rust library (debug mode)..."
	cd $(COMMON_DIR) && cargo build

generate:
	@echo "🔧 Generating Go bindings..."
	uniffi-bindgen-go $(COMMON_DIR)/src/slim_bindings.udl --out-dir generated --config $(COMMON_DIR)/uniffi.toml
	@cd generated && go mod init github.com/agntcy/slim/bindings/generated 2>/dev/null || true
	@echo "✅ Bindings generated in generated/"

clean:
	@echo "🧹 Cleaning build artifacts..."
	cd $(COMMON_DIR) && cargo clean
	rm -rf generated/
	@echo "✅ Clean complete"

example: all
	@echo "🚀 Running simple example..."
	@cd examples/simple && \
		go mod init github.com/agntcy/slim/examples/simple 2>/dev/null || true && \
		go mod edit -replace github.com/agntcy/slim/bindings/generated=../../generated && \
		go mod tidy && \
		CGO_LDFLAGS="-L$(PWD)/$(COMMON_TARGET) -lslim_go_bindings" \
		DYLD_LIBRARY_PATH=$(PWD)/$(COMMON_TARGET):$$DYLD_LIBRARY_PATH \
		LD_LIBRARY_PATH=$(PWD)/$(COMMON_TARGET):$$LD_LIBRARY_PATH \
		go run main.go

test:
	@echo "🧪 Running Rust tests..."
	cd $(COMMON_DIR) && cargo test

