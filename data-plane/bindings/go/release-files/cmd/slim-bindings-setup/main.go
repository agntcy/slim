// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// slim-bindings-setup downloads and installs the SLIM bindings native library.
//
// Usage:
//
//	go install github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup@latest
//	slim-bindings-setup
package main

import (
	"fmt"
	"os"
	"runtime"

	slim "github.com/agntcy/slim-bindings-go"
)

func main() {
	fmt.Println("╔═══════════════════════════════════════════════════════════╗")
	fmt.Println("║              SLIM Bindings Setup                          ║")
	fmt.Println("╚═══════════════════════════════════════════════════════════╝")
	fmt.Println()
	fmt.Printf("Version:  %s\n", slim.Version())
	fmt.Printf("Platform: %s/%s\n", runtime.GOOS, runtime.GOARCH)
	fmt.Println()

	if slim.IsLibraryInstalled() {
		libPath, _ := slim.LibraryPath()
		fmt.Println("✅ Library already installed!")
		fmt.Printf("   Location: %s\n", libPath)
		return
	}

	if err := slim.DownloadLibrary(); err != nil {
		fmt.Fprintf(os.Stderr, "\n❌ Error: %v\n", err)
		os.Exit(1)
	}

	fmt.Println()
	fmt.Println("✅ Setup complete! You can now build Go projects using SLIM bindings.")
}
