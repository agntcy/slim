// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * SLIM Server - Runs a SLIM network node that clients connect to
 * 
 * This server implementation is ported from the Go bindings example.
 */

import slimBindings from '../generated/slim-bindings-node.js';

// Server configuration
const DEFAULT_ENDPOINT = '0.0.0.0:46357';

/**
 * Parse command line arguments
 */
function parseArgs(): { endpoint: string } {
  const args = process.argv.slice(2);
  let endpoint = DEFAULT_ENDPOINT;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--endpoint' && i + 1 < args.length) {
      endpoint = args[i + 1];
      i++;
    } else if (args[i] === '--help' || args[i] === '-h') {
      console.log('Usage: npm run server -- [OPTIONS]');
      console.log('');
      console.log('Options:');
      console.log('  --endpoint <address>  Server endpoint (default: 0.0.0.0:46357)');
      console.log('  --help, -h           Show this help message');
      process.exit(0);
    }
  }

  return { endpoint };
}

/**
 * Main server function
 */
async function main() {
  const args = parseArgs();

  console.log('🚀 SLIM Server');
  console.log('==============');
  console.log(`Endpoint: ${args.endpoint}`);
  console.log();

  try {
    // Initialize crypto - this will create the global service which we will configure as server
    slimBindings.initializeWithDefaults();

    // Start server
    const config = slimBindings.newInsecureServerConfig(args.endpoint);

    console.log(`🌐 Starting server on ${args.endpoint}...`);
    console.log('   Waiting for clients to connect...');
    console.log();

    // Run server in internal tokio task
    // Note: This returns immediately - the server runs in the background
    slimBindings.getGlobalService().runServer(config);

    console.log('✅ Server running and listening');
    console.log();
    console.log('📡 Clients can now connect');
    console.log();
    console.log('Press Ctrl+C to stop');

    // Keep the process alive by keeping the event loop active
    // The setInterval prevents Node.js from exiting
    setInterval(() => {
      // Heartbeat to keep process alive
    }, 1000);

    // This promise never resolves, keeping the process alive
    await new Promise(() => {});
  } catch (error: any) {
    console.error();
    console.error('❌ Error:', error.message || String(error));
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
main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});
