// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Simple example demonstrating SLIM JavaScript bindings
// This is equivalent to the Go example in bindings/go/examples/simple/main.go

const {
  initializeCryptoProvider,
  getVersion,
  Name,
  BindingsAdapter,
  SessionConfig,
  SessionType
} = require('@agntcy/slim-bindings');

async function main() {
  console.log('🚀 SLIM JavaScript Bindings Example');
  console.log('====================================');

  // Initialize crypto provider (required before any operations)
  initializeCryptoProvider();
  console.log('✅ Crypto initialized');

  // Get version
  const version = getVersion();
  console.log(`📦 SLIM Bindings Version: ${version}\n`);

  // Create an app with shared secret authentication
  const appName = new Name("org", "myapp", "v1", null);

  // Note: Shared secret must be at least 32 bytes
  const sharedSecret = "my-shared-secret-value-must-be-at-least-32-bytes-long!";

  // Create shared secret provider and verifier
  const identityProvider = {
    type: "SharedSecret",
    data: sharedSecret,
    id: appName.asString()
  };

  const identityVerifier = {
    type: "SharedSecret",
    data: sharedSecret,
    id: appName.asString()
  };

  try {
    const app = new BindingsAdapter(
      appName,
      identityProvider,
      identityVerifier,
      false
    );

    console.log(`✅ App created with ID: ${app.id()}`);
    const appNameResult = app.name();
    console.log(`   Name components: ${appNameResult.components().join(', ')}\n`);

    // Create a session configuration
    const sessionConfig = {
      sessionType: SessionType.PointToPoint,
      enableMls: false,
      maxRetries: 3,
      interval: 100, // milliseconds
      metadata: {}
    };

    const destination = new Name("org", "receiver", "v1", null);

    console.log('📡 Creating session to destination...');
    const session = await app.createSessionAndWait(sessionConfig, destination);
    console.log('✅ Session created');

    // Cleanup function
    const cleanup = async () => {
      console.log('\n🗑️  Cleaning up session...');
      try {
        await app.deleteSessionAndWait(session);
        console.log('✅ Session deleted');
      } catch (err) {
        console.log(`⚠️  Failed to delete session: ${err.message}`);
      }
    };

    // Publish a message using simplified API
    const message = new TextEncoder().encode("Hello from JavaScript! 👋");

    console.log('\n📤 Publishing message...');
    try {
      await session.publishAndWait(message, null, null);
      console.log('✅ Message published successfully');
    } catch (err) {
      // This might fail without a real SLIM network - that's expected
      console.log(`⚠️  Publish failed (expected without network): ${err.message}`);
    }

    // Test subscription
    const subscriptionName = new Name("org", "myapp", "events", null);

    console.log('\n📥 Testing subscription...');
    try {
      await app.subscribe(subscriptionName, null);
      console.log('✅ Subscribed successfully');

      // Unsubscribe
      await app.unsubscribe(subscriptionName, null);
      console.log('✅ Unsubscribed successfully');
    } catch (err) {
      console.log(`⚠️  Subscribe failed (expected without network): ${err.message}`);
    }

    // Test invite (will fail for non-multicast session)
    const inviteeName = new Name("org", "guest", "v1", null);

    console.log('\n👥 Testing session invite...');
    try {
      await session.inviteAndWait(inviteeName);
      console.log('✅ Invite sent successfully');
    } catch (err) {
      console.log(`⚠️  Invite failed (expected for point-to-point session): ${err.message}`);
    }

    // Cleanup
    await cleanup();

    console.log('\n✨ Example completed successfully!');
    console.log('\n📝 Note: Some operations may fail without a running SLIM network,');
    console.log('   but the bindings are working correctly if you see this message.');

  } catch (error) {
    console.error(`❌ Error: ${error.message}`);
    console.error(error.stack);
    process.exit(1);
  }
}

// Run the example
main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});
