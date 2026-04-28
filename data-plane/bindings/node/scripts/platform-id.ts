#!/usr/bin/env tsx
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * Platform identifier mapping for @agntcy/slim-bindings-node.
 * Used for: (1) CI: Rust TARGET → npm platform package name
 *           (2) Runtime: process.platform/arch → platform id to require().
 */

/** Rust target triple → npm platform id (package suffix) */
export const RUST_TARGET_TO_PLATFORM_ID: Record<string, string> = {
  'aarch64-apple-darwin': 'darwin-arm64',
  'x86_64-apple-darwin': 'darwin-x64',
  'x86_64-unknown-linux-gnu': 'linux-x64-gnu',
  'aarch64-unknown-linux-gnu': 'linux-arm64-gnu',
  'x86_64-unknown-linux-musl': 'linux-x64-musl',
  'aarch64-unknown-linux-musl': 'linux-arm64-musl',
  'x86_64-pc-windows-msvc': 'win32-x64-msvc',
  'aarch64-pc-windows-msvc': 'win32-arm64-msvc',
  'x86_64-pc-windows-gnu': 'win32-x64-gnu',
};

/** All platform ids we publish (for optionalDependencies list) */
export const ALL_PLATFORM_IDS = [
  'darwin-arm64',
  'darwin-x64',
  'linux-x64-gnu',
  'linux-arm64-gnu',
  'linux-x64-musl',
  'linux-arm64-musl',
  'win32-x64-msvc',
  'win32-arm64-msvc',
  'win32-x64-gnu',
] as const;

export type PlatformId = (typeof ALL_PLATFORM_IDS)[number];

/**
 * Map Rust TARGET to npm platform id. Used in CI when building platform packages.
 */
export function rustTargetToPlatformId(rustTarget: string): string {
  const id = RUST_TARGET_TO_PLATFORM_ID[rustTarget];
  if (!id) {
    throw new Error(`Unknown Rust target for platform id: ${rustTarget}`);
  }
  return id;
}

/**
 * Get the platform id for the current Node process (for runtime require).
 * Node does not expose musl vs gnu; we default to gnu for Linux.
 */
export function getCurrentPlatformId(): PlatformId {
  const platform = process.platform;
  const arch = process.arch; // 'x64' | 'arm64' | 'ia32' | ...
  if (platform === 'darwin') {
    return arch === 'arm64' ? 'darwin-arm64' : 'darwin-x64';
  }
  if (platform === 'linux') {
    return arch === 'arm64' ? 'linux-arm64-gnu' : 'linux-x64-gnu';
  }
  if (platform === 'win32') {
    return arch === 'arm64' ? 'win32-arm64-msvc' : 'win32-x64-msvc';
  }
  throw new Error(`Unsupported platform: ${platform} ${arch}`);
}

/** Package name for a platform-specific optional dependency */
export function getPlatformPackageName(platformId: PlatformId): string {
  return `@agntcy/slim-bindings-node-${platformId}`;
}

// CLI: print platform id for given Rust TARGET (for use in Taskfile/shell)
// Only runs when invoked as script with an argument (not when imported).
const arg = process.argv[2];
if (arg === '--current') {
  console.log(getCurrentPlatformId());
} else if (arg && !arg.startsWith('-')) {
  console.log(rustTargetToPlatformId(arg));
} else if (arg !== undefined) {
  console.error('Usage: platform-id.ts <RUST_TARGET> | --current');
  process.exit(1);
}
