// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

'use strict';

/**
 * Resolves the platform id for the current Node process.
 * Must match the optionalDependency package suffixes.
 */
function getCurrentPlatformId() {
  const platform = process.platform;
  const arch = process.arch;
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

const platformId = getCurrentPlatformId();
const packageName = `@agntcy/slim-bindings-node-${platformId}`;

let bindings;
try {
  bindings = require(packageName);
} catch (err) {
  if (err.code === 'MODULE_NOT_FOUND') {
    throw new Error(
      `Platform package ${packageName} is not installed. Install with: npm install @agntcy/slim-bindings-node (includes optional dependencies for your platform).`
    );
  }
  throw err;
}

module.exports = bindings;
