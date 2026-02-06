# SLIM Go Bindings

Get started with SLIM Go bindings in just a few minutes.

## Prerequisites

- [Go](https://go.dev/doc/install) 1.22 or later
- Internet connection to download dependencies

## Quick Start

### 1. Create a New Project

```bash
mkdir -p go-app
cd go-app
go mod init go-app
```

### 2. Install SLIM Go Bindings

```bash
go get github.com/agntcy/slim-bindings-go
```

### 3. Run the Setup Tool

The SLIM bindings require some additional setup to install the bindings libs. Run the setup command:

```bash
go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup
```

### 4. Create Your First SLIM Application

Create a `main.go` file with the following content:

```go
package main

import (
	"fmt"

	slim "github.com/agntcy/slim-bindings-go"
)

func main() {
	fmt.Println("🚀 SLIM Go Bindings Example")
	fmt.Println("============================")

	// Initialize crypto provider (required before any operations)
	slim.InitializeCryptoProvider()
	fmt.Println("✅ Crypto initialized")

	// Your SLIM code here...
}
```

### 5. Run Your Application

```bash
go run main.go
```

You should see:

```
🚀 SLIM Go Bindings Example
============================
✅ Crypto initialized
```

## Important Notes

- **Setup is one-time**: You only need to run `slim-bindings-setup` once
- **Native dependencies**: The bindings use native libraries under the hood via [CGO](https://go.dev/wiki/cgo), so a C compiler is required

## slimrpc (SLIM Remote Procedure Call)

For information about using slimrpc to build protobuf-based RPC services over SLIM, see the [SLIMRPC documentation](SLIMRPC.md).
