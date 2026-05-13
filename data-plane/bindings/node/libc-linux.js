// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

'use strict';

const fs = require('fs');
const { execFileSync } = require('child_process');

/**
 * Best-effort libc family for Linux (gnu vs musl).
 * Used to pick the correct @agntcy/slim-bindings-linux-* optional package.
 */
function detectLinuxLibc() {
  try {
    const maps = fs.readFileSync('/proc/self/maps', 'utf8');
    if (maps.includes('ld-musl')) {
      return 'musl';
    }
  } catch (_) {
    // Not Linux, or no /proc (e.g. some sandboxes)
  }
  try {
    const out = execFileSync('getconf', ['GNU_LIBC_VERSION'], {
      encoding: 'utf8',
    });
    if (/GLIBC/i.test(out)) {
      return 'gnu';
    }
  } catch (_) {
    // getconf missing or non-glibc
  }
  return 'gnu';
}

module.exports = { detectLinuxLibc };
