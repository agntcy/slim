// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * SLIM Point-to-Point Alice (Receiver)
 * 
 * This example demonstrates the receiver side of a point-to-point messaging scenario.
 * Alice listens for incoming sessions and receives messages from Bob.
 * 
 * Note: Sending replies is currently blocked by uniffi-bindgen-node limitations.
 */

import { createAndConnectApp, logMessage, sleep, DEFAULT_SERVER_ENDPOINT, DEFAULT_SHARED_SECRET } from './common.js';

// Default configuration
const DEFAULT_LOCAL_ID = 'org/alice/app';

/**
 * Command line arguments
 */
interface CliArgs {
  local: string;
  server: string;
  sharedSecret: string;
}

/**
 * Parse command line arguments
 */
function parseArgs(): CliArgs {
  const args = process.argv.slice(2);
  let local = DEFAULT_LOCAL_ID;
  let server = DEFAULT_SERVER_ENDPOINT;
  let sharedSecret = DEFAULT_SHARED_SECRET;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--local' && i + 1 < args.length) {
      local = args[i + 1];
      i++;
    } else if (args[i] === '--server' && i + 1 < args.length) {
      server = args[i + 1];
      i++;
    } else if (args[i] === '--secret' && i + 1 < args.length) {
      sharedSecret = args[i + 1];
      i++;
    } else if (args[i] === '--help' || args[i] === '-h') {
      console.log('Usage: npm run alice -- [OPTIONS]');
      console.log('');
      console.log('Options:');
      console.log('  --local <id>        Local identity (default: org/alice/app)');
      console.log('  --server <address>  Server endpoint (default: http://localhost:46357)');
      console.log('  --secret <secret>   Shared secret (default: demo-shared-secret-min-32-chars!!)');
      console.log('  --help, -h         Show this help message');
      process.exit(0);
    }
  }

  return { local, server, sharedSecret };
}

/**
 * Handle an incoming session
 * Receives messages until the session closes
 * 
 * Note: Sending replies is blocked by uniffi-bindgen-node limitations
 */
async function handleSession(app: any, session: any, instance: bigint): Promise<void> {
  const sessionId = session.sessionId();
  logMessage(instance, `🎉 New session established! (ID: ${sessionId})`);

  try {
    while (true) {
      try {
        // Wait for incoming message (60 second timeout)
        const timeout = 60000; // 60 seconds in milliseconds
        const receivedMsg = await session.getMessageAsync(timeout);
        
        const payload = receivedMsg.payload;
        const text = Buffer.from(payload).toString('utf-8');
        logMessage(instance, `📨 Received: ${text}`);

        // try to send a reply using synchronous call
        try {
          session.publishToAndWait(receivedMsg.context, Buffer.from('Hello from Alice'), undefined, undefined);
          logMessage(instance, '📤 Sent reply: Hello from Alice');
        } catch (error: any) {
          // Debug: log the raw error structure to understand what we're getting
          console.log('[DEBUG] Raw error:', error);
          console.log('[DEBUG] Error type:', typeof error);
          console.log('[DEBUG] Error constructor:', error?.constructor?.name);
          console.log('[DEBUG] Is array:', Array.isArray(error));
          if (Array.isArray(error)) {
            console.log('[DEBUG] Array length:', error.length);
            console.log('[DEBUG] Array[0]:', error[0]);
            console.log('[DEBUG] Array[1]:', error[1]);
          }
          
          // Error is returned as a tuple: ["SlimError", SlimError]
          // SlimError is a tagged union with different error types
          let errorMessage = 'Unknown error';
          
          if (Array.isArray(error) && error.length === 2 && error[0] === 'SlimError' && error[1]) {
            const slimError = error[1];
            console.log('[DEBUG] SlimError object:', JSON.stringify(slimError, null, 2));
            
            // Extract message based on error tag
            switch (slimError.tag) {
              case 'sendError':
                errorMessage = `Send error: ${slimError.inner.message}`;
                break;
              case 'sessionError':
                errorMessage = `Session error: ${slimError.inner.message}`;
                break;
              case 'serviceError':
                errorMessage = `Service error: ${slimError.inner.message}`;
                break;
              case 'timeout':
                errorMessage = 'Timeout sending reply';
                break;
              case 'invalidArgument':
                errorMessage = `Invalid argument: ${slimError.inner.message}`;
                break;
              case 'internalError':
                errorMessage = `Internal error: ${slimError.inner.message}`;
                break;
              case 'authError':
                errorMessage = `Auth error: ${slimError.inner.message}`;
                break;
              case 'configError':
                errorMessage = `Config error: ${slimError.inner.message}`;
                break;
              case 'receiveError':
                errorMessage = `Receive error: ${slimError.inner.message}`;
                break;
              default:
                errorMessage = `${slimError.tag}: ${JSON.stringify(slimError)}`;
            }
          } else if (error?.message) {
            errorMessage = error.message;
          } else if (error instanceof Error) {
            errorMessage = `${error.name}: ${error.message}`;
            if (error.stack) {
              console.log('[DEBUG] Error stack:', error.stack);
            }
          } else {
            errorMessage = String(error);
          }
          
          logMessage(instance, `❌ Error sending reply: ${errorMessage}`);
        }
        
      } catch (error: any) {
        // Check if session closed or timeout
        if (error.message && error.message.toLowerCase().includes('timeout')) {
          logMessage(instance, '⏱️  No message received (timeout), continuing to listen...');
          continue;
        }
        // Session likely closed
        logMessage(instance, `🔚 Session ended: ${error.message || String(error)}`);
        break;
      }
    }
  } finally {
    // Clean up session
    try {
      const handle = await app.deleteSessionAsync(session);
      await handle.waitAsync();
      logMessage(instance, '👋 Session closed');
    } catch (error: any) {
      logMessage(instance, `⚠️  Warning: failed to delete session: ${error.message || String(error)}`);
    }
  }
}

/**
 * Main receiver loop
 * Continuously listens for new incoming sessions
 */
async function runReceiver(args: CliArgs): Promise<void> {
  try {
    // Create and connect app
    const { app, connId } = await createAndConnectApp(
      args.local,
      args.server,
      args.sharedSecret
    );

    const instance = app.id();
    logMessage(instance, '👂 Waiting for incoming sessions...');
    console.log();

    // Listen for incoming sessions indefinitely
    while (true) {
      try {
        // Block until a remote peer initiates a session to us
        // Pass undefined (not null) for indefinite wait
        const session = await app.listenForSessionAsync(undefined);
        
        // Handle session in background (don't await - allows multiple concurrent sessions)
        handleSession(app, session, instance).catch(error => {
          logMessage(instance, `❌ Session handler error: ${error.message || String(error)}`);
        });
      } catch (error: any) {
        logMessage(instance, `⏱️  Timeout waiting for session, retrying...`);
        await sleep(1000); // Brief pause before retrying
      }
    }
  } catch (error: any) {
    console.error();
    console.error('❌ Error:', error.message || String(error));
    throw error;
  }
}

async function main() {
  const args = parseArgs();

  console.log('👩 SLIM Point-to-Point Alice (Receiver)');
  console.log('=======================================');
  console.log(`Local ID: ${args.local}`);
  console.log(`Server: ${args.server}`);
  console.log();

  try {
    await runReceiver(args);
  } catch (error: any) {
    console.error('Fatal error:', error.message || String(error));
    process.exit(1);
  }
}

// Handle signals gracefully
process.on('SIGINT', () => {
  console.log('\n\n📋 Received SIGINT');
  console.log('🛑 Shutting down...');
  process.exit(0);
});

process.on('SIGTERM', () => {
  console.log('\n\n📋 Received SIGTERM');
  console.log('🛑 Shutting down...');
  process.exit(0);
});

// Run the main function
main();
