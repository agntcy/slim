// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * FFI Compatibility utilities for Node.js bindings
 * 
 * This module provides compatibility helpers for uniffi-bindgen-node
 * which generates React Native JSI types but needs to work with ffi-rs
 * that expects standard JavaScript types.
 */

/**
 * Convert pointer value to Number for ffi-rs compatibility.
 * 
 * React Native JSI uses BigInt for pointers, but ffi-rs expects Number.
 * This helper converts BigInt pointers to Numbers before FFI calls.
 * 
 * @param ptr - Pointer value (BigInt or Number)
 * @returns Number representation of the pointer
 */
export function toFfiPointer(ptr: bigint | number): number {
  return typeof ptr === 'bigint' ? Number(ptr) : ptr;
}

/**
 * Extract error message from UniffiError
 * 
 * UniffiError wraps SlimError variants but loses details in serialization.
 * This helper extracts meaningful error messages when available.
 * 
 * @param error - Error object to extract message from
 * @returns Extracted error message
 */
export function extractErrorMessage(error: any): string {
  if (!error || typeof error !== 'object') {
    return String(error);
  }

  // Check for SlimError pattern
  if (error.message && typeof error.message === 'string') {
    if (error.message.startsWith('SlimError.')) {
      return error.message;
    }
    return error.message;
  }

  // Try toString if available
  if (error.toString && error.toString() !== '[object Object]') {
    return error.toString();
  }

  // Last resort: stringify
  try {
    return JSON.stringify(error, Object.getOwnPropertyNames(error), 2);
  } catch {
    return 'Unknown error';
  }
}

/**
 * Check if error is a SlimError
 * 
 * @param error - Error to check
 * @returns True if error is a SlimError
 */
export function isSlimError(error: any): boolean {
  return error?.message?.startsWith('SlimError.') || 
         error?.constructor?.name === 'UniffiError';
}
