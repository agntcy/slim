// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * Integration tests for SLIM Node.js bindings
 * 
 * Tests point-to-point communication between sender and receiver using an embedded server.
 * Based on the alice/bob examples but in an automated test harness.
 */

import { test, describe, before, after } from 'node:test';
import assert from 'node:assert';
import { setTimeout as sleep } from 'node:timers/promises';

import slimBindings from '@agntcy/slim-bindings-node';

// Test configuration - use random high port to avoid conflicts
const TEST_SERVER_PORT = 50000 + Math.floor(Math.random() * 10000); // Random port 50000-60000
const TEST_SERVER_ENDPOINT = `127.0.0.1:${TEST_SERVER_PORT}`;
const TEST_SHARED_SECRET = 'test-shared-secret-min-32-chars!!';
const TEST_ALICE_ID = 'org/test-alice/app';
const TEST_BOB_ID = 'org/test-bob/app';

/**
 * Split an ID of form organization/namespace/application
 */
function splitId(id: string): any {
  const parts = id.split('/');
  if (parts.length !== 3) {
    throw new Error(`IDs must be in the format organization/namespace/app-or-stream, got: ${id}`);
  }
  return new slimBindings.Name(parts[0], parts[1], parts[2]);
}

/**
 * Message collector for test verification
 */
class MessageCollector {
  private messages: Array<{ payload: Uint8Array; context: any }> = [];

  add(payload: Uint8Array, context: any): void {
    this.messages.push({ payload, context });
  }

  getMessages(): Array<{ payload: Uint8Array; context: any }> {
    return this.messages;
  }

  count(): number {
    return this.messages.length;
  }

  clear(): void {
    this.messages = [];
  }
}

/**
 * Test harness for managing server and app lifecycle
 */
class TestHarness {
  private serverService: any = null;
  private serverRunning = false;

  async startServer(): Promise<void> {
    // Initialize crypto, runtime globally
    slimBindings.initializeWithDefaults();
    
    console.log(`Starting SLIM server on ${TEST_SERVER_ENDPOINT}...`);
    
    // Create a dedicated service for the server
    this.serverService = new slimBindings.Service("test-server");
    
    // Start server on test endpoint
    const config = slimBindings.newInsecureServerConfig(TEST_SERVER_ENDPOINT);
    await this.serverService.runServerAsync(config);
    this.serverRunning = true;
    
    console.log(`✅ SLIM server started on ${TEST_SERVER_ENDPOINT}`);
    
    // Wait for server to be ready
    await sleep(1000);
  }

  async stopServer(): Promise<void> {
    if (this.serverRunning && this.serverService) {
      try {
        this.serverService.stopServer(TEST_SERVER_ENDPOINT);
        this.serverRunning = false;
        console.log('🛑 Server stopped');
      } catch (error) {
        console.warn('Warning: error stopping server:', error);
      }
    }
  }

  async createAndConnectApp(localId: string, serviceId: string): Promise<{ app: any; connId: bigint; service: any }> {
    // Create a dedicated service for this app
    const service = new slimBindings.Service(serviceId);
    
    const appName = splitId(localId);
    const app = service.createAppWithSecret(appName, TEST_SHARED_SECRET);
    
    console.log(`[${app.id()}] Created app: ${localId} with service: ${serviceId}`);
    
    // Connect to server
    const config = slimBindings.newInsecureClientConfig(`http://${TEST_SERVER_ENDPOINT}`);
    
    try {
      const connId = await service.connectAsync(config);
      console.log(`[${app.id()}] Connected to server, conn ID: ${connId}`);
      
      // Subscribe
      await app.subscribeAsync(appName, BigInt(connId));
      console.log(`[${app.id()}] Subscribed`);
      
      return { app, connId: BigInt(connId), service };
    } catch (error) {
      console.error(`[${app.id()}] Error connecting or subscribing:`, error);
      throw error;
    }
  }
}

// Global test harness
let harness: TestHarness;

describe('SLIM Node.js Bindings Integration Tests', () => {
  before(async () => {
    harness = new TestHarness();
    await harness.startServer();
  });

  after(async () => {
    await harness.stopServer();
  });

  test('point-to-point communication between Alice and Bob', async () => {
    const aliceCollector = new MessageCollector();
    const bobCollector = new MessageCollector();
    
    // Create Alice (receiver)
    const { app: aliceApp, connId: aliceConnId, service: aliceService } = 
      await harness.createAndConnectApp(TEST_ALICE_ID, 'alice-service');
    const aliceInstance = aliceApp.id();
    
    // Create Bob (sender)
    const { app: bobApp, connId: bobConnId, service: bobService } = 
      await harness.createAndConnectApp(TEST_BOB_ID, 'bob-service');
    const bobInstance = bobApp.id();
    
    let aliceSession: any = null;
    let bobSession: any = null;
    
    try {
      // Start Alice receiver in background
      const alicePromise = (async () => {
        // Wait for incoming session (30 second timeout)
        const timeout = 30000;
        aliceSession = await aliceApp.listenForSessionAsync(timeout);
        
        // Receive message from Bob
        const receivedMsg = await aliceSession.getMessageAsync(30000);
        const payload = receivedMsg.payload;
        const text = Buffer.from(payload).toString('utf-8');
        aliceCollector.add(payload, receivedMsg.context);
        
        // Send reply to Bob
        await aliceSession.publishToAndWait(
          receivedMsg.context,
          Buffer.from('Hello from Alice'),
          undefined,
          undefined
        );
      })();
      
      // Give Alice time to start listening
      await sleep(500);
      
      // Bob: Set route to Alice
      const aliceRoute = splitId(TEST_ALICE_ID);
      bobApp.setRoute(aliceRoute, Number(bobConnId));
      
      // Small delay for route propagation
      await sleep(100);
      
      // Bob: Create session configuration
      const config = {
        sessionType: "pointToPoint" as const,
        enableMls: false,
        maxRetries: 5,
        interval: 5000,
        metadata: new Map()
      };
      
      // Bob: Create session with Alice
      bobSession = await bobApp.createSessionAndWaitAsync(config, aliceRoute);
      
      // Bob: Send message
      const bobMessage = 'Hello from Bob';
      await bobSession.publishAndWaitAsync(Buffer.from(bobMessage), undefined, undefined);
      
      // Bob: Wait for reply
      const bobReceivedMsg = await bobSession.getMessageAsync(30000);
      const bobReplyPayload = bobReceivedMsg.payload;
      const bobReplyText = Buffer.from(bobReplyPayload).toString('utf-8');
      bobCollector.add(bobReplyPayload, bobReceivedMsg.context);
      
      // Wait for Alice to complete
      await alicePromise;
      
      // Verify messages exchanged
      assert.strictEqual(aliceCollector.count(), 1, 'Alice should receive exactly 1 message');
      assert.strictEqual(bobCollector.count(), 1, 'Bob should receive exactly 1 reply');
      
      // Verify message content
      const aliceReceivedText = Buffer.from(aliceCollector.getMessages()[0].payload).toString('utf-8');
      assert.strictEqual(aliceReceivedText, 'Hello from Bob', 'Alice should receive correct message from Bob');
      assert.strictEqual(bobReplyText, 'Hello from Alice', 'Bob should receive correct reply from Alice');
      
      console.log('✅ Point-to-point communication test passed');
      console.log(`   Alice received: "${aliceReceivedText}"`);
      console.log(`   Bob received: "${bobReplyText}"`);
      
    } finally {
      // Cleanup sessions
      if (bobSession) {
        try {
          const handle = await bobApp.deleteSessionAsync(bobSession);
          await handle.waitAsync();
        } catch (error) {
          console.warn('Warning: failed to delete Bob session:', error);
        }
      }
      
      if (aliceSession) {
        try {
          const handle = await aliceApp.deleteSessionAsync(aliceSession);
          await handle.waitAsync();
        } catch (error) {
          console.warn('Warning: failed to delete Alice session:', error);
        }
      }
    }
  });

  test('multiple message exchanges', async () => {
    const aliceCollector = new MessageCollector();
    const bobCollector = new MessageCollector();
    
    // Create Alice (receiver)
    const { app: aliceApp, connId: aliceConnId, service: aliceService } = 
      await harness.createAndConnectApp('org/test-alice2/app', 'alice2-service');
    
    // Create Bob (sender)
    const { app: bobApp, connId: bobConnId, service: bobService } = 
      await harness.createAndConnectApp('org/test-bob2/app', 'bob2-service');
    
    let aliceSession: any = null;
    let bobSession: any = null;
    
    try {
      const numMessages = 3;
      
      // Start Alice receiver in background
      const alicePromise = (async () => {
        aliceSession = await aliceApp.listenForSessionAsync(30000);
        
        // Receive and reply to multiple messages
        for (let i = 0; i < numMessages; i++) {
          const receivedMsg = await aliceSession.getMessageAsync(30000);
          const payload = receivedMsg.payload;
          aliceCollector.add(payload, receivedMsg.context);
          
          // Send reply
          await aliceSession.publishToAndWait(
            receivedMsg.context,
            Buffer.from(`Reply ${i + 1}`),
            undefined,
            undefined
          );
        }
      })();
      
      // Give Alice time to start listening
      await sleep(500);
      
      // Bob: Set route and create session
      const aliceRoute = splitId('org/test-alice2/app');
      bobApp.setRoute(aliceRoute, Number(bobConnId));
      await sleep(100);
      
      const config = {
        sessionType: "pointToPoint" as const,
        enableMls: false,
        maxRetries: 5,
        interval: 5000,
        metadata: new Map()
      };
      
      bobSession = await bobApp.createSessionAndWaitAsync(config, aliceRoute);
      
      // Bob: Send multiple messages and receive replies
      for (let i = 0; i < numMessages; i++) {
        await bobSession.publishAndWaitAsync(Buffer.from(`Message ${i + 1}`), undefined, undefined);
        
        const bobReceivedMsg = await bobSession.getMessageAsync(30000);
        bobCollector.add(bobReceivedMsg.payload, bobReceivedMsg.context);
        
        // Small delay between messages
        await sleep(100);
      }
      
      // Wait for Alice to complete
      await alicePromise;
      
      // Verify all messages exchanged
      assert.strictEqual(aliceCollector.count(), numMessages, `Alice should receive ${numMessages} messages`);
      assert.strictEqual(bobCollector.count(), numMessages, `Bob should receive ${numMessages} replies`);
      
      // Verify message content
      for (let i = 0; i < numMessages; i++) {
        const aliceMsg = Buffer.from(aliceCollector.getMessages()[i].payload).toString('utf-8');
        const bobMsg = Buffer.from(bobCollector.getMessages()[i].payload).toString('utf-8');
        
        assert.strictEqual(aliceMsg, `Message ${i + 1}`, `Alice should receive correct message ${i + 1}`);
        assert.strictEqual(bobMsg, `Reply ${i + 1}`, `Bob should receive correct reply ${i + 1}`);
      }
      
      console.log(`✅ Multiple message exchange test passed (${numMessages} messages each way)`);
      
    } finally {
      // Cleanup sessions
      if (bobSession) {
        try {
          const handle = await bobApp.deleteSessionAsync(bobSession);
          await handle.waitAsync();
        } catch (error) {
          console.warn('Warning: failed to delete Bob session:', error);
        }
      }
      
      if (aliceSession) {
        try {
          const handle = await aliceApp.deleteSessionAsync(aliceSession);
          await handle.waitAsync();
        } catch (error) {
          console.warn('Warning: failed to delete Alice session:', error);
        }
      }
    }
  });
});
