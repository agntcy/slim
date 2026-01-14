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
	"archive/zip"
	"flag"
	"fmt"
	"go/build"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"strings"
)

const (
	// GitHub release URL pattern - downloads from agntcy/slim releases
	releaseURLTemplate = "https://github.com/agntcy/slim/releases/download/slim-bindings-libs-%s/slim-bindings-%s.zip"
	// Cache directory name
	cacheDirName = "slim-bindings"
)

// Version returns the module version from build info.
func Version() string {
	info, ok := debug.ReadBuildInfo()
	if !ok {
		return "(unknown)"
	}
	return info.Main.Version
}

// GetTarget returns the target triple for the given OS/arch.
// If goos or arch are empty, it uses the current runtime values.
func GetTarget(goos, arch string) string {
	if goos == "" {
		goos = runtime.GOOS
	}
	if arch == "" {
		arch = runtime.GOARCH
	}

	switch goos {
	case "darwin":
		if arch == "arm64" {
			return "aarch64-apple-darwin"
		}
		return "x86_64-apple-darwin"
	case "linux":
		if arch == "arm64" {
			return "aarch64-unknown-linux-gnu"
		}
		return "x86_64-unknown-linux-gnu"
	case "windows":
		return "x86_64-pc-windows-gnu"
	}

	return fmt.Sprintf("%s-unknown-%s", arch, goos)
}

// GetCacheDir returns the cache directory for SLIM bindings libraries.
func GetCacheDir() (string, error) {
	gopath := build.Default.GOPATH
	if gopath == "" {
		return "", fmt.Errorf("failed to determine GOPATH")
	}

	return filepath.Join(gopath, ".cache", cacheDirName), nil
}

// TargetToLibraryName converts a Rust target triple to the library name format.
// Example: "aarch64-unknown-linux-gnu" -> "libslim_bindings_aarch64_linux_gnu.a"
func TargetToLibraryName(target string) string {
	// Replace hyphens with underscores and remove "unknown-" prefix
	name := strings.ReplaceAll(target, "-unknown-", "_")
	name = strings.ReplaceAll(name, "-", "_")
	return fmt.Sprintf("libslim_bindings_%s.a", name)
}

// LibraryPath returns the path to the native library for the specified target.
func LibraryPath(target string) (string, error) {
	cacheDir, err := GetCacheDir()
	if err != nil {
		return "", err
	}

	libName := TargetToLibraryName(target)
	libPath := filepath.Join(cacheDir, libName)

	if _, err := os.Stat(libPath); err != nil {
		if os.IsNotExist(err) {
			return "", fmt.Errorf("library not found - run 'slim-bindings-setup' to download")
		}
		return "", err
	}

	return libPath, nil
}

// IsLibraryInstalled checks if the library is installed for the specified target.
func IsLibraryInstalled(target string) bool {
	_, err := LibraryPath(target)
	return err == nil
}

// DownloadLibrary downloads the library for the specified platform.
func DownloadLibrary(target string) error {
	version := Version()

	cacheDir, err := GetCacheDir()
	if err != nil {
		return fmt.Errorf("failed to get cache directory: %w", err)
	}

	// Create cache directory
	if err := os.MkdirAll(cacheDir, 0755); err != nil {
		return fmt.Errorf("failed to create directory %s: %w", cacheDir, err)
	}

	url := fmt.Sprintf(releaseURLTemplate, version, target)
	fmt.Printf("📦 Downloading SLIM bindings library...\n")
	fmt.Printf("   Version:  %s\n", version)
	fmt.Printf("   Platform: %s\n", target)
	fmt.Printf("   URL:      %s\n", url)

	resp, err := http.Get(url)
	if err != nil {
		return fmt.Errorf("failed to download: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusNotFound {
		return fmt.Errorf("library not found for platform %s at version %s", target, version)
	}

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("download failed with status: %s", resp.Status)
	}

	// Download to temp file first (zip requires random access)
	tmpFile, err := os.CreateTemp("", "slim-bindings-*.zip")
	if err != nil {
		return fmt.Errorf("failed to create temp file: %w", err)
	}
	defer os.Remove(tmpFile.Name())
	defer tmpFile.Close()

	if _, err := io.Copy(tmpFile, resp.Body); err != nil {
		return fmt.Errorf("failed to download file: %w", err)
	}

	// Open zip file
	zipReader, err := zip.OpenReader(tmpFile.Name())
	if err != nil {
		return fmt.Errorf("failed to open zip file: %w", err)
	}
	defer zipReader.Close()

	extractedFiles := 0

	for _, file := range zipReader.File {
		// Only extract library files
		name := file.Name
		isLibrary := strings.HasSuffix(name, ".a") && !strings.HasSuffix(name, ".dll.a")
		if !isLibrary {
			continue
		}

		// Validate and sanitize the file name to prevent Zip Slip attacks
		// Use only the base name to prevent directory traversal
		baseName := filepath.Base(name)
		if baseName == "." || baseName == ".." || baseName == "" {
			continue // Skip invalid entries
		}

		outPath := filepath.Join(cacheDir, baseName)

		// Verify the resulting path is within the cache directory (defense in depth)
		if !strings.HasPrefix(filepath.Clean(outPath), filepath.Clean(cacheDir)) {
			return fmt.Errorf("invalid zip entry path (directory traversal attempt): %s", name)
		}

		// Extract file
		rc, err := file.Open()
		if err != nil {
			return fmt.Errorf("failed to open zip entry %s: %w", name, err)
		}

		outFile, err := os.Create(outPath)
		if err != nil {
			rc.Close()
			return fmt.Errorf("failed to create file %s: %w", outPath, err)
		}

		if _, err := io.Copy(outFile, rc); err != nil {
			outFile.Close()
			rc.Close()
			return fmt.Errorf("failed to extract file: %w", err)
		}

		outFile.Close()
		rc.Close()

		info, _ := os.Stat(outPath)
		sizeMB := float64(info.Size()) / 1024 / 1024
		fmt.Printf("   Extracted: %s (%.1f MB)\n", baseName, sizeMB)
		extractedFiles++
	}

	if extractedFiles == 0 {
		return fmt.Errorf("no library files found in archive")
	}

	fmt.Printf("✅ Library installed to: %s\n", cacheDir)
	return nil
}

func main() {
	// Parse command-line flags
	targetFlag := flag.String("target", "", "Rust target triple (e.g., x86_64-unknown-linux-gnu). If not provided, uses current OS/arch.")
	flag.Parse()

	// Determine the target
	var target string
	if *targetFlag != "" {
		target = *targetFlag
	} else {
		target = GetTarget("", "")
	}

	fmt.Println("╔═══════════════════════════════════════════════════════════╗")
	fmt.Println("║              SLIM Bindings Setup                          ║")
	fmt.Println("╚═══════════════════════════════════════════════════════════╝")
	fmt.Println()
	fmt.Printf("Version:  %s\n", Version())
	fmt.Printf("Platform: %s/%s\n", runtime.GOOS, runtime.GOARCH)
	fmt.Printf("Target:   %s\n", target)
	fmt.Println()

	if IsLibraryInstalled(target) {
		libPath, _ := LibraryPath(target)
		fmt.Println("✅ Library already installed!")
		fmt.Printf("   Location: %s\n", libPath)
		return
	}

	if err := DownloadLibrary(target); err != nil {
		fmt.Fprintf(os.Stderr, "\n❌ Error: %v\n", err)
		os.Exit(1)
	}

	fmt.Println()
	fmt.Println("✅ Setup complete! You can now build Go projects using SLIM bindings.")
}
