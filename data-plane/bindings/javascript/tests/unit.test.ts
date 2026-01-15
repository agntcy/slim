// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, beforeAll } from 'vitest'
import {
  initializeCryptoProvider,
  getVersion,
  createAppWithSecret,
  Name,
  SessionType,
  type SessionConfig,
  type NameInterface
} from '../generated/typescript/slim_bindings'

describe('SLIM JavaScript/TypeScript Bindings - Unit Tests', () => {
  // Note: These tests mirror the Go unit tests in tests/unit_test.go

  describe('Initialization', () => {
    it('should initialize crypto provider without error', () => {
      // Initialize crypto - should not throw
      expect(() => initializeCryptoProvider()).not.toThrow()
      
      // Multiple calls should be safe
      expect(() => initializeCryptoProvider()).not.toThrow()
    })
  })

  describe('Version', () => {
    it('should return non-empty version string', () => {
      const version = getVersion()
      expect(version).toBeTruthy()
      expect(typeof version).toBe('string')
      expect(version.length).toBeGreaterThan(0)
      console.log(`SLIM version: ${version}`)
    })
  })

  describe('Name Structure', () => {
    it('should create name with 3 components', () => {
      const name = new Name(['org', 'app', 'v1'], undefined)
      expect(name).toBeDefined()
      
      const components = name.components()
      expect(components).toHaveLength(3)
      expect(components[0]).toBe('org')
      expect(components[1]).toBe('app')
      expect(components[2]).toBe('v1')
      
      expect(name.id()).toBe(BigInt(0))
    })

    it('should create name with explicit ID', () => {
      const testId = BigInt(99999)
      const name = new Name(['org', 'app', 'v1'], testId)
      
      expect(name.id()).toBe(testId)
    })

    it('should convert name to string', () => {
      const name = new Name(['org', 'myapp', 'v1'], undefined)
      const nameStr = name.asString()
      
      expect(nameStr).toBeTruthy()
      expect(typeof nameStr).toBe('string')
      console.log(`Name as string: ${nameStr}`)
    })
  })

  describe('App Creation', () => {
    beforeAll(() => {
      initializeCryptoProvider()
    })

    it('should create app with shared secret', () => {
      const appName = new Name(['org', 'testapp', 'v1'], undefined)
      const sharedSecret = 'test-shared-secret-must-be-at-least-32-bytes-long!'
      
      const app = createAppWithSecret(appName, sharedSecret)
      
      expect(app).toBeDefined()
      expect(app.id()).toBeGreaterThan(0)
      
      const returnedName = app.name()
      const components = returnedName.components()
      expect(components).toHaveLength(3)
      expect(components[0]).toBe('org')
      expect(components[1]).toBe('testapp')
      
      console.log(`App created with ID: ${app.id()}`)
      
      // Cleanup
      app.destroy()
    })

    it('should create multiple apps with different IDs', () => {
      const sharedSecret = 'test-shared-secret-must-be-at-least-32-bytes-long!'
      
      const app1 = createAppWithSecret(
        new Name(['org', 'app1', 'v1'], undefined),
        sharedSecret
      )
      
      const app2 = createAppWithSecret(
        new Name(['org', 'app2', 'v1'], undefined),
        sharedSecret
      )
      
      expect(app1.id()).not.toBe(app2.id())
      console.log(`App1 ID: ${app1.id()}, App2 ID: ${app2.id()}`)
      
      // Cleanup
      app1.destroy()
      app2.destroy()
    })
  })

  describe('Session Configuration', () => {
    it('should create point-to-point session config', () => {
      const config: SessionConfig = {
        sessionType: SessionType.PointToPoint,
        enableMls: false,
        maxRetries: undefined,
        interval: undefined,
        metadata: {}
      }
      
      expect(config.sessionType).toBe(SessionType.PointToPoint)
      expect(config.enableMls).toBe(false)
    })

    it('should create group session config', () => {
      const config: SessionConfig = {
        sessionType: SessionType.Group,
        enableMls: true,
        maxRetries: 5,
        interval: 1000,
        metadata: { key: 'value' }
      }
      
      expect(config.sessionType).toBe(SessionType.Group)
      expect(config.enableMls).toBe(true)
      expect(config.maxRetries).toBe(5)
    })
  })

  describe('Error Handling', () => {
    beforeAll(() => {
      initializeCryptoProvider()
    })

    it('should handle short secret gracefully', () => {
      const appName = new Name(['org', 'testapp', 'v1'], undefined)
      const shortSecret = 'too-short'
      
      // Note: This may throw an error or panic depending on Rust implementation
      expect(() => {
        try {
          const app = createAppWithSecret(appName, shortSecret)
          app.destroy()
        } catch (error) {
          // Expected to throw for short secret
          throw error
        }
      }).toThrow()
    })

    it('should work with valid secret length', () => {
      const appName = new Name(['org', 'testapp', 'v1'], undefined)
      const validSecret = 'valid-shared-secret-must-be-at-least-32-bytes!'
      
      expect(() => {
        const app = createAppWithSecret(appName, validSecret)
        app.destroy()
      }).not.toThrow()
    })
  })

  describe('Cleanup', () => {
    beforeAll(() => {
      initializeCryptoProvider()
    })

    it('should cleanup app resources properly', () => {
      const app = createAppWithSecret(
        new Name(['org', 'cleanup', 'v1'], undefined),
        'test-shared-secret-must-be-at-least-32-bytes-long!'
      )
      
      expect(app).toBeDefined()
      
      // Cleanup should not throw
      expect(() => app.destroy()).not.toThrow()
      
      console.log('Cleanup completed successfully')
    })
  })

  describe('Session Type Enum', () => {
    it('should have correct session type values', () => {
      expect(SessionType.PointToPoint).toBeDefined()
      expect(SessionType.Group).toBeDefined()
      
      // Different types should have different values
      expect(SessionType.PointToPoint).not.toBe(SessionType.Group)
    })
  })
})
