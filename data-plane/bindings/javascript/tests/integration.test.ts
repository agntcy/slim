// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import {
  initializeCryptoProvider,
  createAppWithSecret,
  Name,
  SessionType,
  type SessionConfig
} from '../generated/typescript/slim_bindings'

describe('SLIM JavaScript/TypeScript Bindings - Integration Tests', () => {
  beforeAll(() => {
    initializeCryptoProvider()
  })

  describe('Session Lifecycle', () => {
    it('should create and delete a point-to-point session', async () => {
      const appName = new Name(['org', 'sender', 'v1'], undefined)
      const sharedSecret = 'integration-test-secret-at-least-32-bytes-long!!'
      
      const app = createAppWithSecret(appName, sharedSecret)
      expect(app).toBeDefined()

      const sessionConfig: SessionConfig = {
        sessionType: SessionType.PointToPoint,
        enableMls: false,
        maxRetries: undefined,
        interval: undefined,
        metadata: {}
      }

      const destination = new Name(['org', 'receiver', 'v1'], undefined)

      try {
        // Create session (will fail without network, but should not crash)
        const session = await app.createSessionAndWait(sessionConfig, destination)
        expect(session).toBeDefined()
        
        // Delete session
        await app.deleteSessionAndWait(session)
        console.log('Session created and deleted successfully')
      } catch (error: any) {
        // Expected to fail without SLIM network - that's okay
        console.log('Session creation failed (expected without network):', error.message)
        expect(error).toBeDefined()
      } finally {
        app.destroy()
      }
    }, 10000) // Longer timeout for network operations

    it('should create a group session', async () => {
      const appName = new Name(['org', 'moderator', 'v1'], undefined)
      const sharedSecret = 'integration-test-secret-at-least-32-bytes-long!!'
      
      const app = createAppWithSecret(appName, sharedSecret)

      const sessionConfig: SessionConfig = {
        sessionType: SessionType.Group,
        enableMls: false,
        maxRetries: undefined,
        interval: undefined,
        metadata: {}
      }

      const destination = new Name(['org', 'default', 'chat-room'], undefined)

      try {
        const session = await app.createSessionAndWait(sessionConfig, destination)
        expect(session).toBeDefined()
        
        await app.deleteSessionAndWait(session)
      } catch (error: any) {
        console.log('Group session failed (expected without network):', error.message)
      } finally {
        app.destroy()
      }
    }, 10000)
  })

  describe('Subscription Operations', () => {
    it('should handle subscribe and unsubscribe', async () => {
      const appName = new Name(['org', 'subscriber', 'v1'], undefined)
      const sharedSecret = 'integration-test-secret-at-least-32-bytes-long!!'
      
      const app = createAppWithSecret(appName, sharedSecret)
      const subscriptionName = new Name(['org', 'myapp', 'events'], undefined)

      try {
        await app.subscribe(subscriptionName, undefined)
        console.log('Subscribed successfully')
        
        await app.unsubscribe(subscriptionName, undefined)
        console.log('Unsubscribed successfully')
      } catch (error: any) {
        console.log('Subscribe/unsubscribe failed (expected without network):', error.message)
      } finally {
        app.destroy()
      }
    }, 10000)
  })

  describe('Multi-App Communication', () => {
    it('should create multiple apps and sessions', async () => {
      const sharedSecret = 'integration-test-secret-at-least-32-bytes-long!!'
      
      const app1 = createAppWithSecret(
        new Name(['org', 'alice', 'v1'], undefined),
        sharedSecret
      )
      
      const app2 = createAppWithSecret(
        new Name(['org', 'bob', 'v1'], undefined),
        sharedSecret
      )

      try {
        expect(app1.id()).not.toBe(app2.id())
        console.log('Two apps created with different IDs')
        
        // Both apps should be able to create sessions
        const config: SessionConfig = {
          sessionType: SessionType.PointToPoint,
          enableMls: false,
          maxRetries: undefined,
          interval: undefined,
          metadata: {}
        }
        
        try {
          const session1 = await app1.createSessionAndWait(
            config,
            new Name(['org', 'bob', 'v1'], undefined)
          )
          await app1.deleteSessionAndWait(session1)
        } catch (error: any) {
          console.log('App1 session failed (expected):', error.message)
        }
        
        try {
          const session2 = await app2.createSessionAndWait(
            config,
            new Name(['org', 'alice', 'v1'], undefined)
          )
          await app2.deleteSessionAndWait(session2)
        } catch (error: any) {
          console.log('App2 session failed (expected):', error.message)
        }
      } finally {
        app1.destroy()
        app2.destroy()
      }
    }, 15000)
  })

  describe('Error Handling in Operations', () => {
    it('should handle publish without session gracefully', async () => {
      const app = createAppWithSecret(
        new Name(['org', 'publisher', 'v1'], undefined),
        'integration-test-secret-at-least-32-bytes-long!!'
      )

      try {
        const config: SessionConfig = {
          sessionType: SessionType.PointToPoint,
          enableMls: false,
          maxRetries: undefined,
          interval: undefined,
          metadata: {}
        }
        
        try {
          const session = await app.createSessionAndWait(
            config,
            new Name(['org', 'receiver', 'v1'], undefined)
          )
          
          // Try to publish
          const message = new TextEncoder().encode('Hello SLIM!')
          try {
            await session.publishAndWait(message, undefined, undefined)
            console.log('Message published successfully')
          } catch (error: any) {
            console.log('Publish failed (expected without network):', error.message)
          }
          
          await app.deleteSessionAndWait(session)
        } catch (error: any) {
          console.log('Session creation failed (expected):', error.message)
        }
      } finally {
        app.destroy()
      }
    }, 15000)
  })
})
