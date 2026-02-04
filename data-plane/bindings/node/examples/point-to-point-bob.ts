// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * SLIM Point-to-Point Bob (Sender)
 * 
 * This example demonstrates the sender side of a point-to-point messaging scenario.
 * Bob initiates a session with Alice and sends messages, waiting for replies.
 * 
 * Based on implementations from Go, Python, and React Native bindings.
 */

// @ts-expect-error - tsx resolves .js imports to .ts files at runtime
import slimBindings from '../generated/slim-bindings-node.js';
import { createAndConnectApp, splitId, logMessage, sleep, DEFAULT_SERVER_ENDPOINT, DEFAULT_SHARED_SECRET } from './common.js';

// Default configuration
const DEFAULT_LOCAL_ID = 'org/bob/app';
const DEFAULT_REMOTE_ID = 'org/alice/app';
const DEFAULT_MESSAGE = 'Hello from Bob';
const DEFAULT_ITERATIONS = 5;

/**
 * Command line arguments
 */
interface CliArgs {
  local: string;
  remote: string;
  server: string;
  sharedSecret: string;
  message: string;
  iterations: number;
}

/**
 * Parse command line arguments
 */
function parseArgs(): CliArgs {
  const args = process.argv.slice(2);
  let local = DEFAULT_LOCAL_ID;
  let remote = DEFAULT_REMOTE_ID;
  let server = DEFAULT_SERVER_ENDPOINT;
  let sharedSecret = DEFAULT_SHARED_SECRET;
  let message = DEFAULT_MESSAGE;
  let iterations = DEFAULT_ITERATIONS;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--local' && i + 1 < args.length) {
      local = args[i + 1];
      i++;
    } else if (args[i] === '--remote' && i + 1 < args.length) {
      remote = args[i + 1];
      i++;
    } else if (args[i] === '--server' && i + 1 < args.length) {
      server = args[i + 1];
      i++;
    } else if (args[i] === '--secret' && i + 1 < args.length) {
      sharedSecret = args[i + 1];
      i++;
    } else if (args[i] === '--message' && i + 1 < args.length) {
      message = args[i + 1];
      i++;
    } else if (args[i] === '--iterations' && i + 1 < args.length) {
      iterations = parseInt(args[i + 1], 10);
      if (isNaN(iterations) || iterations < 1) {
        console.error('Error: iterations must be a positive number');
        process.exit(1);
      }
      i++;
    } else if (args[i] === '--help' || args[i] === '-h') {
      console.log('Usage: npm run bob -- [OPTIONS]');
      console.log('');
      console.log('Options:');
      console.log('  --local <id>          Local identity (default: org/bob/app)');
      console.log('  --remote <id>         Remote identity (default: org/alice/app)');
      console.log('  --server <address>    Server endpoint (default: http://localhost:46357)');
      console.log('  --secret <secret>     Shared secret (default: demo-shared-secret-min-32-chars!!)');
      console.log('  --message <text>      Message to send (default: Hello from Bob)');
      console.log('  --iterations <n>      Number of send/receive cycles (default: 5)');
      console.log('  --help, -h           Show this help message');
      process.exit(0);
    }
  }

  return { local, remote, server, sharedSecret, message, iterations };
}

/**
 * Run Bob as sender
 * Creates a session with Alice and sends messages
 */
async function runSender(args: CliArgs): Promise<void> {
  try {
    // Create and connect app
    const { app, connId } = await createAndConnectApp(
      args.local,
      args.server,
      args.sharedSecret
    );

    const instance = app.id();
    logMessage(instance, `🎯 Initiating connection to ${args.remote}...`);

    // Parse remote name
    const remoteName = splitId(args.remote);
    
    // Debug connId type
    console.log('[DEBUG] connId value:', connId);
    console.log('[DEBUG] connId type:', typeof connId);
    
    // Convert connId to number for FFI (ffi-rs expects number, not bigint)
    const connIdNum = Number(connId);
    console.log('[DEBUG] connIdNum value:', connIdNum);
    console.log('[DEBUG] connIdNum type:', typeof connIdNum);
    
    logMessage(instance, '📍 Setting route to remote peer...');
    
    // Set route to Alice (synchronous, but can throw errors)
    try {
      app.setRoute(remoteName, connIdNum);
      logMessage(instance, '✅ Route established');
    } catch (error: any) {
      // Debug: log the raw error structure
      console.log('[DEBUG] setRoute error - raw:', error);
      console.log('[DEBUG] setRoute error - type:', typeof error);
      console.log('[DEBUG] setRoute error - constructor:', error?.constructor?.name);
      console.log('[DEBUG] setRoute error - is array:', Array.isArray(error));
      if (Array.isArray(error)) {
        console.log('[DEBUG] setRoute error - array length:', error.length);
        console.log('[DEBUG] setRoute error - array[0]:', error[0]);
        console.log('[DEBUG] setRoute error - array[1]:', error[1]);
        if (error[1]) {
          console.log('[DEBUG] setRoute error - SlimError object:', JSON.stringify(error[1], null, 2));
        }
      }
      
      // Parse error properly
      let errorMsg = 'Unknown error';
      if (Array.isArray(error) && error.length === 2 && error[0] === 'SlimError' && error[1]) {
        const slimError = error[1];
        console.log('[DEBUG] SlimError tag:', slimError.tag);
        switch (slimError.tag) {
          case 'sendError':
          case 'sessionError':
          case 'serviceError':
          case 'invalidArgument':
          case 'internalError':
          case 'authError':
          case 'configError':
          case 'receiveError':
            errorMsg = slimError.inner?.message || slimError.tag;
            break;
          default:
            errorMsg = `${slimError.tag}: ${JSON.stringify(slimError)}`;
        }
      } else if (error?.message) {
        errorMsg = error.message;
      } else {
        errorMsg = String(error);
      }
      throw new Error(`Failed to set route: ${errorMsg}`);
    }

    // Wait a bit for route propagation
    await sleep(100);

    // Create session configuration
    // SessionType is a string literal: "pointToPoint" or "group"
    const config = {
      sessionType: "pointToPoint" as const,
      enableMls: false,
      maxRetries: 5,
      interval: 5000, // 5 seconds in milliseconds
      metadata: new Map()
    };

    logMessage(instance, '🔗 Creating session...');
    
    // Create session and wait for establishment
    const session = await app.createSessionAndWaitAsync(config, remoteName);
    const sessionId = session.sessionId();
    logMessage(instance, `🎉 Session established! (ID: ${sessionId})`);
    console.log();

    try {
      // Send messages and wait for replies
      for (let i = 0; i < args.iterations; i++) {
        const msgText = `${args.message} (${i + 1}/${args.iterations})`;
        const payload = Buffer.from(msgText);

        logMessage(instance, `📤 Sending: ${msgText}`);
        
        // Send message
        await session.publishAndWaitAsync(payload, undefined, undefined);
        
        // Wait for reply (30 second timeout)
        try {
          const timeout = 30000; // 30 seconds in milliseconds
          const receivedMsg = await session.getMessageAsync(timeout);
          
          const replyPayload = receivedMsg.payload;
          const replyText = Buffer.from(replyPayload).toString('utf-8');
          logMessage(instance, `📨 Received reply: ${replyText}`);
        } catch (error: any) {
          // Handle timeout or receive errors
          if (error.message && error.message.toLowerCase().includes('timeout')) {
            logMessage(instance, '⏱️  No reply received (timeout)');
          } else {
            logMessage(instance, `⚠️  Error receiving reply: ${error.message || String(error)}`);
          }
        }

        // Wait a bit between iterations
        if (i < args.iterations - 1) {
          await sleep(1000);
        }
      }

      console.log();
      logMessage(instance, '✅ All messages sent');
    } finally {
      // Clean up session
      try {
        logMessage(instance, '🧹 Cleaning up session...');
        const handle = await app.deleteSessionAsync(session);
        await handle.waitAsync();
        logMessage(instance, '👋 Session closed');
      } catch (error: any) {
        logMessage(instance, `⚠️  Warning: failed to delete session: ${error.message || String(error)}`);
      }
    }
  } catch (error: any) {
    console.error();
    console.error('❌ Error:', error.message || String(error));
    throw error;
  }
}

/**
 * Main entry point
 */
async function main() {
  const args = parseArgs();

  console.log('👨 SLIM Point-to-Point Bob (Sender)');
  console.log('====================================');
  console.log(`Local ID: ${args.local}`);
  console.log(`Remote ID: ${args.remote}`);
  console.log(`Server: ${args.server}`);
  console.log(`Message: ${args.message}`);
  console.log(`Iterations: ${args.iterations}`);
  console.log();

  try {
    await runSender(args);
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
