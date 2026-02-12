// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"runtime"
	"testing"
)

func TestGetTarget(t *testing.T) {
	tests := []struct {
		name     string
		goos     string
		arch     string
		abi      string
		expected string
	}{
		// Linux + amd64 combinations
		{
			name:     "linux amd64 gnu",
			goos:     "linux",
			arch:     "amd64",
			abi:      "gnu",
			expected: "x86_64-unknown-linux-gnu",
		},
		{
			name:     "linux amd64 musl",
			goos:     "linux",
			arch:     "amd64",
			abi:      "musl",
			expected: "x86_64-unknown-linux-musl",
		},
		{
			name:     "linux amd64 empty abi defaults to gnu",
			goos:     "linux",
			arch:     "amd64",
			abi:      "",
			expected: "x86_64-unknown-linux-gnu",
		},
		// Linux + arm64 combinations
		{
			name:     "linux arm64 gnu",
			goos:     "linux",
			arch:     "arm64",
			abi:      "gnu",
			expected: "aarch64-unknown-linux-gnu",
		},
		{
			name:     "linux arm64 musl",
			goos:     "linux",
			arch:     "arm64",
			abi:      "musl",
			expected: "aarch64-unknown-linux-musl",
		},
		{
			name:     "linux arm64 empty abi defaults to gnu",
			goos:     "linux",
			arch:     "arm64",
			abi:      "",
			expected: "aarch64-unknown-linux-gnu",
		},
		// Darwin (macOS) combinations
		{
			name:     "darwin amd64",
			goos:     "darwin",
			arch:     "amd64",
			abi:      "",
			expected: "x86_64-apple-darwin",
		},
		{
			name:     "darwin amd64 with abi ignored",
			goos:     "darwin",
			arch:     "amd64",
			abi:      "gnu",
			expected: "x86_64-apple-darwin",
		},
		{
			name:     "darwin arm64",
			goos:     "darwin",
			arch:     "arm64",
			abi:      "",
			expected: "aarch64-apple-darwin",
		},
		{
			name:     "darwin arm64 with abi ignored",
			goos:     "darwin",
			arch:     "arm64",
			abi:      "musl",
			expected: "aarch64-apple-darwin",
		},
		// Windows combinations
		{
			name:     "windows amd64",
			goos:     "windows",
			arch:     "amd64",
			abi:      "",
			expected: "x86_64-pc-windows-gnu",
		},
		{
			name:     "windows amd64 with abi ignored",
			goos:     "windows",
			arch:     "amd64",
			abi:      "gnu",
			expected: "x86_64-pc-windows-gnu",
		},
		{
			name:     "windows arm64 defaults to x86_64",
			goos:     "windows",
			arch:     "arm64",
			abi:      "",
			expected: "x86_64-pc-windows-gnu",
		},
		// Empty parameters should use runtime defaults
		{
			name:     "empty goos uses runtime.GOOS",
			goos:     "",
			arch:     "amd64",
			abi:      "gnu",
			expected: getRuntimeTarget(runtime.GOOS, "amd64", "gnu"),
		},
		{
			name:     "empty arch uses runtime.GOARCH",
			goos:     "linux",
			arch:     "",
			abi:      "gnu",
			expected: getRuntimeTarget("linux", runtime.GOARCH, "gnu"),
		},
		{
			name:     "all empty uses runtime defaults",
			goos:     "",
			arch:     "",
			abi:      "",
			expected: getRuntimeTarget(runtime.GOOS, runtime.GOARCH, ""),
		},
		// Custom/non-standard ABI on Linux
		{
			name:     "linux amd64 custom abi",
			goos:     "linux",
			arch:     "amd64",
			abi:      "custom",
			expected: "x86_64-unknown-linux-custom",
		},
		// Unsupported OS fallback
		{
			name:     "unsupported os fallback format",
			goos:     "freebsd",
			arch:     "amd64",
			abi:      "",
			expected: "amd64-unknown-freebsd",
		},
		{
			name:     "unsupported os with custom arch",
			goos:     "openbsd",
			arch:     "riscv64",
			abi:      "",
			expected: "riscv64-unknown-openbsd",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := GetTarget(tt.goos, tt.arch, tt.abi)
			if result != tt.expected {
				t.Errorf("GetTarget(%q, %q, %q) = %q, expected %q",
					tt.goos, tt.arch, tt.abi, result, tt.expected)
			}
		})
	}
}

// getRuntimeTarget is a helper function that mimics GetTarget behavior for runtime defaults
func getRuntimeTarget(goos, arch, abi string) string {
	if abi == "" {
		if goos == "linux" {
			abi = "gnu" // Default for tests (we can't reliably detect in tests)
		}
	}

	switch goos {
	case "darwin":
		if arch == "arm64" {
			return "aarch64-apple-darwin"
		}
		return "x86_64-apple-darwin"
	case "linux":
		libc := "gnu"
		if abi == "musl" {
			libc = "musl"
		} else if abi != "" && abi != "gnu" {
			libc = abi
		}
		if arch == "arm64" {
			return "aarch64-unknown-linux-" + libc
		}
		return "x86_64-unknown-linux-" + libc
	case "windows":
		return "x86_64-pc-windows-gnu"
	}
	return arch + "-unknown-" + goos
}

func TestTargetToLibraryName(t *testing.T) {
	tests := []struct {
		name     string
		target   string
		expected string
	}{
		{
			name:     "linux gnu amd64",
			target:   "x86_64-unknown-linux-gnu",
			expected: "libslim_bindings_x86_64_linux_gnu.a",
		},
		{
			name:     "linux musl amd64",
			target:   "x86_64-unknown-linux-musl",
			expected: "libslim_bindings_x86_64_linux_musl.a",
		},
		{
			name:     "linux gnu arm64",
			target:   "aarch64-unknown-linux-gnu",
			expected: "libslim_bindings_aarch64_linux_gnu.a",
		},
		{
			name:     "linux musl arm64",
			target:   "aarch64-unknown-linux-musl",
			expected: "libslim_bindings_aarch64_linux_musl.a",
		},
		{
			name:     "darwin amd64",
			target:   "x86_64-apple-darwin",
			expected: "libslim_bindings_x86_64_apple_darwin.a",
		},
		{
			name:     "darwin arm64",
			target:   "aarch64-apple-darwin",
			expected: "libslim_bindings_aarch64_apple_darwin.a",
		},
		{
			name:     "windows gnu",
			target:   "x86_64-pc-windows-gnu",
			expected: "libslim_bindings_x86_64_pc_windows_gnu.a",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := TargetToLibraryName(tt.target)
			if result != tt.expected {
				t.Errorf("TargetToLibraryName(%q) = %q, expected %q",
					tt.target, result, tt.expected)
			}
		})
	}
}

func TestGetTarget_Consistency(t *testing.T) {
	// Test that GetTarget output can be converted to library name
	testCases := []struct {
		goos string
		arch string
		abi  string
	}{
		{"linux", "amd64", "gnu"},
		{"linux", "amd64", "musl"},
		{"linux", "arm64", "gnu"},
		{"linux", "arm64", "musl"},
		{"darwin", "amd64", ""},
		{"darwin", "arm64", ""},
		{"windows", "amd64", ""},
	}

	for _, tc := range testCases {
		t.Run(tc.goos+"_"+tc.arch+"_"+tc.abi, func(t *testing.T) {
			target := GetTarget(tc.goos, tc.arch, tc.abi)
			libName := TargetToLibraryName(target)

			// Verify library name format
			if libName == "" {
				t.Error("TargetToLibraryName returned empty string")
			}
			if libName[:11] != "libslim_bin" {
				t.Errorf("Library name doesn't start with expected prefix: %s", libName)
			}
			if libName[len(libName)-2:] != ".a" {
				t.Errorf("Library name doesn't end with .a: %s", libName)
			}
		})
	}
}

// Benchmark tests
func BenchmarkGetTarget(b *testing.B) {
	for i := 0; i < b.N; i++ {
		GetTarget("linux", "amd64", "gnu")
	}
}

func BenchmarkGetTargetWithDefaults(b *testing.B) {
	for i := 0; i < b.N; i++ {
		GetTarget("", "", "")
	}
}

func BenchmarkTargetToLibraryName(b *testing.B) {
	target := "x86_64-unknown-linux-gnu"
	for i := 0; i < b.N; i++ {
		TargetToLibraryName(target)
	}
}
