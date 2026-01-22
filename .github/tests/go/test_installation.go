package main

import (
	"fmt"
	"os"

	slim "github.com/agntcy/slim-bindings-go"
)

// Installation test for SLIM Go bindings.
//
// This test verifies that the slim_bindings package can be imported
// and initialized successfully.
func main() {
	fmt.Println("🚀 SLIM Go Bindings Installation Test")
	fmt.Println("==================================================")

	// Initialize SLIM (required before any operations)
	slim.InitializeWithDefaults()
	fmt.Println("✅ SLIM initialized successfully")

	fmt.Println("✅ Installation test passed!")
	os.Exit(0)
}