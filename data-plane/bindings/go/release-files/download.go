// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package slim_bindings

import (
	"archive/zip"
	"fmt"
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
	releaseURLTemplate = "https://github.com/agntcy/slim/releases/download/%s/slim-bindings-%s.zip"
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

// GetRustTarget returns the Rust target triple for the current OS/arch.
func GetRustTarget() string {
	arch := runtime.GOARCH
	goos := runtime.GOOS

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
		return "x86_64-pc-windows-msvc"
	}

	return fmt.Sprintf("%s-unknown-%s", arch, goos)
}

// GetCacheDir returns the cache directory for SLIM bindings libraries.
func GetCacheDir() (string, error) {
	cacheHome := os.Getenv("XDG_CACHE_HOME")
	if cacheHome == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", fmt.Errorf("failed to get home directory: %w", err)
		}

		switch runtime.GOOS {
		case "windows":
			cacheHome = filepath.Join(home, "AppData", "Local")
		default:
			cacheHome = filepath.Join(home, ".cache")
		}
	}

	return filepath.Join(cacheHome, cacheDirName), nil
}

// LibraryPath returns the path to the native library for the current platform.
func LibraryPath() (string, error) {
	cacheDir, err := GetCacheDir()
	if err != nil {
		return "", err
	}

	libName := "libslim_bindings.a"
	if runtime.GOOS == "windows" {
		libName = "slim_bindings.lib"
	}

	libPath := filepath.Join(cacheDir, libName)

	if _, err := os.Stat(libPath); err != nil {
		if os.IsNotExist(err) {
			return "", fmt.Errorf("library not found - run 'slim-bindings-setup' to download")
		}
		return "", err
	}

	return libPath, nil
}

// IsLibraryInstalled checks if the library is installed.
func IsLibraryInstalled() bool {
	_, err := LibraryPath()
	return err == nil
}

// DownloadLibrary downloads the library for the current platform.
func DownloadLibrary() error {
	rustTarget := GetRustTarget()
	version := Version()

	cacheDir, err := GetCacheDir()
	if err != nil {
		return fmt.Errorf("failed to get cache directory: %w", err)
	}

	// Create cache directory
	if err := os.MkdirAll(cacheDir, 0755); err != nil {
		return fmt.Errorf("failed to create directory %s: %w", cacheDir, err)
	}

	url := fmt.Sprintf(releaseURLTemplate, version, rustTarget)
	fmt.Printf("📦 Downloading SLIM bindings library...\n")
	fmt.Printf("   Version:  %s\n", version)
	fmt.Printf("   Platform: %s\n", rustTarget)
	fmt.Printf("   URL:      %s\n", url)

	resp, err := http.Get(url)
	if err != nil {
		return fmt.Errorf("failed to download: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusNotFound {
		return fmt.Errorf("library not found for platform %s at version %s", rustTarget, version)
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
		isLibrary := strings.HasSuffix(name, ".a") || strings.HasSuffix(name, ".lib")
		if !isLibrary {
			continue
		}

		// Extract file
		rc, err := file.Open()
		if err != nil {
			return fmt.Errorf("failed to open zip entry %s: %w", name, err)
		}

		outPath := filepath.Join(cacheDir, filepath.Base(name))
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
		fmt.Printf("   Extracted: %s (%.1f MB)\n", filepath.Base(outPath), sizeMB)
		extractedFiles++
	}

	if extractedFiles == 0 {
		return fmt.Errorf("no library files found in archive")
	}

	fmt.Printf("✅ Library installed to: %s\n", cacheDir)
	return nil
}
