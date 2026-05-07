// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

'use strict';

const { detectLinuxLibc } = require('./libc-linux.js');

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
    const libc = detectLinuxLibc();
    if (arch === 'arm64') {
      return `linux-arm64-${libc}`;
    }
    if (arch === 'x64') {
      return `linux-x64-${libc}`;
    }
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
      `Platform package ${packageName} is not installed. ` +
        `Run: npm install @agntcy/slim-bindings-node (optional dependencies include linux-*-gnu, linux-*-musl, darwin-*, win32-*). ` +
        `On Alpine or other musl Linux, ensure the linux-*-musl optional package is available for your Node arch.`
    );
  }
  throw err;
}

module.exports = bindings;
