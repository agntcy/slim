# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

SERVICE_DIR = ../../core/service
SERVICE_TARGET = ../../target/release
LIB_NAME = slim_service

.PHONY: all build generate clean test example help

# Default target
all: build generate example

help:
	@echo "SLIM Go Bindings (Proc Macro Approach) - Makefile targets:"
	@echo "  make build       - Build the Rust library with UniFFI (release mode)"
	@echo "  make build-debug - Build the Rust library (debug mode)"
	@echo "  make generate    - Generate Go bindings from compiled library"
	@echo "  make all         - Build and generate (default)"
	@echo "  make clean       - Clean build artifacts"
	@echo "  make example     - Run the simple example"
	@echo "  make help        - Show this help message"

build:
	@echo "🦀 Building Rust library with UniFFI (release mode)..."
	cd $(SERVICE_DIR) && cargo build --release

build-debug:
	@echo "🦀 Building Rust library (debug mode)..."
	cd $(SERVICE_DIR) && cargo build

generate:
	@echo "🔧 Generating Go bindings from compiled library..."
	uniffi-bindgen-go \
		--library '$(SERVICE_TARGET)/lib$(LIB_NAME).dylib' \
		--out-dir generated
	@cd generated && go mod init github.com/agntcy/slim/bindings/generated 2>/dev/null || true
	@echo "✅ Bindings generated in generated/"

clean:
	@echo "🧹 Cleaning build artifacts..."
	cd $(SERVICE_DIR) && cargo clean
	rm -rf generated/
	@echo "✅ Clean complete"

example:
	@echo "🚀 Running simple example..."
	@cd examples/simple && \
		go mod init github.com/agntcy/slim/examples/simple 2>/dev/null || true && \
		go mod edit -replace github.com/agntcy/slim/bindings/generated=../../generated && \
		go mod tidy && \
		CGO_LDFLAGS="-L$(PWD)/$(SERVICE_TARGET) -l$(LIB_NAME)" \
		DYLD_LIBRARY_PATH=$(PWD)/$(SERVICE_TARGET):$$DYLD_LIBRARY_PATH \
		LD_LIBRARY_PATH=$(PWD)/$(SERVICE_TARGET):$$LD_LIBRARY_PATH \
		go run main.go
