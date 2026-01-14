// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// TypeScript version of the simple example
// Demonstrates type-safe usage of SLIM JavaScript bindings

import {
  initializeCryptoProvider,
  getVersion,
  Name,
  BindingsAdapter,
  SessionConfig,
  SessionType,
  BindingsSessionContext
} from '@agntcy/slim-bindings';

async function main(): Promise<void> {
  console.log('🚀 SLIM JavaScript Bindings Example (TypeScript)');
  console.log('=================================================');

  // Initialize crypto provider (required before any operations)
  initializeCryptoProvider();
  console.log('✅ Crypto initialized');

  // Get version
  const version: string = getVersion();
  console.log(`📦 SLIM Bindings Version: ${version}\n`);

  // Create an app with shared secret authentication
  const appName: Name = new Name("org", "myapp", "v1", null);

  // Note: Shared secret must be at least 32 bytes
  const sharedSecret: string = "my-shared-secret-value-must-be-at-least-32-bytes-long!";

  // Create shared secret provider and verifier (with proper typing)
  const identityProvider = {
    type: "SharedSecret" as const,
    data: sharedSecret,
    id: appName.asString()
  };

  const identityVerifier = {
    type: "SharedSecret" as const,
    data: sharedSecret,
    id: appName.asString()
  };

  try {
    const app: BindingsAdapter = new BindingsAdapter(
      appName,
      identityProvider,
      identityVerifier,
      false
    );

    console.log(`✅ App created with ID: ${app.id()}`);
    const appNameResult: Name = app.name();
    console.log(`   Name components: ${appNameResult.components().join(', ')}\n`);

    // Create a session configuration with type safety
    const sessionConfig: SessionConfig = {
      sessionType: SessionType.PointToPoint,
      enableMls: false,
      maxRetries: 3,
      interval: 100, // milliseconds
      metadata: {}
    };

    const destination: Name = new Name("org", "receiver", "v1", null);

    console.log('📡 Creating session to destination...');
    const session: BindingsSessionContext = await app.createSessionAndWait(
      sessionConfig,
      destination
    );
    console.log('✅ Session created');

    // Cleanup function
    const cleanup = async (): Promise<void> => {
      console.log('\n🗑️  Cleaning up session...');
      try {
        await app.deleteSessionAndWait(session);
        console.log('✅ Session deleted');
      } catch (err) {
        const error = err as Error;
        console.log(`⚠️  Failed to delete session: ${error.message}`);
      }
    };

    // Publish a message using simplified API
    const message: Uint8Array = new TextEncoder().encode("Hello from TypeScript! 👋");

    console.log('\n📤 Publishing message...');
    try {
      await session.publishAndWait(message, null, null);
      console.log('✅ Message published successfully');
    } catch (err) {
      // This might fail without a real SLIM network - that's expected
      const error = err as Error;
      console.log(`⚠️  Publish failed (expected without network): ${error.message}`);
    }

    // Test subscription
    const subscriptionName: Name = new Name("org", "myapp", "events", null);

    console.log('\n📥 Testing subscription...');
    try {
      await app.subscribe(subscriptionName, null);
      console.log('✅ Subscribed successfully');

      // Unsubscribe
      await app.unsubscribe(subscriptionName, null);
      console.log('✅ Unsubscribed successfully');
    } catch (err) {
      const error = err as Error;
      console.log(`⚠️  Subscribe failed (expected without network): ${error.message}`);
    }

    // Test invite (will fail for non-multicast session)
    const inviteeName: Name = new Name("org", "guest", "v1", null);

    console.log('\n👥 Testing session invite...');
    try {
      await session.inviteAndWait(inviteeName);
      console.log('✅ Invite sent successfully');
    } catch (err) {
      const error = err as Error;
      console.log(`⚠️  Invite failed (expected for point-to-point session): ${error.message}`);
    }

    // Cleanup
    await cleanup();

    console.log('\n✨ Example completed successfully!');
    console.log('\n📝 Note: Some operations may fail without a running SLIM network,');
    console.log('   but the bindings are working correctly if you see this message.');

  } catch (error) {
    const err = error as Error;
    console.error(`❌ Error: ${err.message}`);
    console.error(err.stack);
    process.exit(1);
  }
}

// Run the example
main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});
