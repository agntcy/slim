// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * Common utilities for SLIM Node.js examples
 * 
 * This module provides shared helper functions used across examples,
 * ported from the Go bindings common package.
 */
// @ts-expect-error - tsx resolves .js imports to .ts files at runtime
import slimBindings from '../generated/slim-bindings-node.js';

// Default configuration values (matching Go examples)
export const DEFAULT_SERVER_ENDPOINT = 'http://localhost:46357';
export const DEFAULT_SHARED_SECRET = 'demo-shared-secret-min-32-chars!!';

/**
 * Split an ID of form organization/namespace/application
 * 
 * @param id - String in the canonical 'org/namespace/app-or-stream' format
 * @returns Name object for the parsed identity
 * @throws Error if the ID format is invalid
 */
export function splitId(id: string): any {
  const parts = id.split('/');
  if (parts.length !== 3) {
    throw new Error(`IDs must be in the format organization/namespace/app-or-stream, got: ${id}`);
  }
  return new slimBindings.Name(parts[0], parts[1], parts[2]);
}

/**
 * Create a SLIM app with shared secret authentication and connect it to a server
 * 
 * This is a convenience function that combines:
 * - Crypto initialization
 * - App creation with shared secret
 * - Server connection with TLS settings
 * - Subscription setup
 * 
 * @param localId - Local identity string (org/namespace/app format)
 * @param serverAddr - SLIM server endpoint URL
 * @param secret - Shared secret for authentication (min 32 chars)
 * @returns Object containing the app, connection ID, and service
 */
export async function createAndConnectApp(
  localId: string,
  serverAddr: string,
  secret: string
): Promise<{ app: any; connId: bigint; service: any }> {
  try {
    // Initialize crypto, runtime, global service and logging with defaults
    slimBindings.initializeWithDefaults();

    // Parse the local identity string
    const appName = splitId(localId);

    // Get global service
    const service = slimBindings.getGlobalService();

    // Create app with shared secret authentication
    // Note: Using synchronous version as async has FFI compatibility issues
    const app = service.createAppWithSecret(appName, secret);
    console.log(`[${app.id()}] ✅ Created app`);

    // Connect to SLIM server (returns connection ID)
    const config = slimBindings.newInsecureClientConfig(serverAddr);
    const connId = await service.connectAsync(config);
    console.log(`[${app.id()}] 🔌 Connected to ${serverAddr} (conn ID: ${connId})`);

    // Forward subscription to next node
    // Note: Generated bindings expect bigint, but connectAsync returns number
    // Convert number -> bigint for the subscription call
    await app.subscribeAsync(appName, BigInt(connId));
    console.log(`[${app.id()}] ✅ Subscribed to sessions`);

    // Return connId as bigint for consistency with TypeScript signatures
    return { app, connId: BigInt(connId), service };
  } catch (error: any) {
    let errorMsg = 'Unknown error';
    if (error && typeof error === 'object') {
      // Parse UniffiError message format: "SlimError.[object Object]"
      if (error.message && typeof error.message === 'string') {
        if (error.message.startsWith('SlimError.')) {
          // Extract error data from the error's internal state
          // Unfortunately UniffiError wraps the object in a way that loses the details
          // Try to extract from stack or other properties
          errorMsg = error.message + ' (SlimError details not accessible via UniffiError wrapper)';
        } else {
          errorMsg = error.message;
        }
      } else if (error.toString && error.toString() !== '[object Object]') {
        errorMsg = error.toString();
      } else {
        errorMsg = JSON.stringify(error, Object.getOwnPropertyNames(error), 2);
      }
    } else {
      errorMsg = String(error);
    }
    console.error('Full error object:', error);
    console.error('Error type:', typeof error);
    console.error('Error keys (own):', Object.getOwnPropertyNames(error));
    console.error('Error message:', error?.message);
    console.error('Error name:', error?.name);
    console.error('Error constructor:', error?.constructor?.name);
    throw new Error(`Create and connect failed: ${errorMsg}`);
  }
}

/**
 * Format a message with instance ID prefix for console output
 */
export function logMessage(instance: bigint | number, message: string): void {
  console.log(`[${instance}] ${message}`);
}

/**
 * Sleep utility for async operations
 */
export function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
